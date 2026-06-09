# Agent Lifecycle Architecture

> 技术文档 — Agent 从创建到归档的完整生命周期、身份系统、三层改动设计。
> 最后更新：2026-06-05

---

## 1. 系统架构总览

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Dashboard (Fastify + React)                  │
│  Principal Lifecycle Service · Agent Registry · SSE Event Bus       │
│  管理界面：创建/启动/停止/删除 Agent，展示时间线和状态                   │
└──────────────────────────────┬──────────────────────────────────────┘
                               │ HTTP / SSE
┌──────────────────────────────▼──────────────────────────────────────┐
│                     Agent Process (rusty-claude-cli --headless)     │
│  ┌─────────────┐  ┌──────────────┐  ┌───────────────┐              │
│  │ AppFS Attach │  │ Input Router │  │ Heartbeat     │              │
│  │ 身份注册     │  │ 统一输入层   │  │ 30s 刷新      │              │
│  └──────┬──────┘  └──────┬───────┘  └───────┬───────┘              │
│         │                │                   │                      │
│         └────────────────┼───────────────────┘                      │
│                          │ 读写文件 = 读写状态                        │
└──────────────────────────┼──────────────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────────────┐
│                  AppFS Runtime Supervisor (Rust)                     │
│  tokio::select! 主循环                                               │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────────────┐  │
│  │Action Wake │ │Fallback    │ │Inbound     │ │Stale Sweep     │  │
│  │.act 变更   │ │定时轮询    │ │外部事件    │ │每30s 清理      │  │
│  └────────────┘ └────────────┘ └────────────┘ └────────────────┘  │
│                                                                     │
│  Principal Registry · App Registry · Connector Host                  │
│  Tinode Connector · gRPC Bridge · HTTP Bridge                        │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 2. Agent 启动全流程

### 2.1 触发：Dashboard Frontend → Backend

```
用户点击 "Spawn Headless Agent"
    │
    ├─ POST /api/projects/:id/principals
    │  → lifecycle.createPrincipal()
    │  → append create_principal.act → AppFS 注册 principal + 实例化私有 app
    │
    └─ POST /api/projects/:id/principals/:pid/start
       → lifecycle.startPrincipal()
```

### 2.2 Dashboard 后端准备

`startPrincipal` (`principal-lifecycle.ts:320`) 做三件事：

1. **查重**：`findManagedAgentByPrincipal` 检查是否已有 managed agent
2. **构建配置**：`buildSpawnConfig` 合并默认配置和用户输入
3. **Spawn**：`processManager.spawn(config)` 启动子进程

构建的 `SpawnConfig` 关键字段：

```typescript
{
  principalId: "code-implementer",
  projectId: "project-uuid",
  projectRoot: "/path/to/project",
  cwd: "/path/to/project",
  appfsMountRoot: "/path/to/project/.appfs",
  model: "claude-opus-4-8",
  permissionMode: "dangerous",
  launchSpec: { kind: "cargo" | "binary", ... },
}
```

注入给子进程的环境变量 (`process-manager.ts:883`)：

```typescript
{
  APPFS_PRINCIPAL_ID: config.principalId,
  APPFS_ATTACH_ID: buildManagedAppfsAttachId(config.principalId),
  //     → "dashboard-code-implementer"
  APPFS_MOUNT_ROOT: "/path/to/project/.appfs",
  APPFS_RUNTIME_MANIFEST: ".appfs/.well-known/appfs/runtime.json",
}
```

### 2.3 Agent 进程启动

`run_headless` (`main.rs:4034`) → `LiveCli::new_headless()` 构造函数中依次执行：

#### Step 1: 身份注册 — `ensure_appfs_attach_identity`

`appfs.rs:395` — AppFS 环境检测三级策略：

```
APPFS_MOUNT_ROOT env ──→ 直接使用
        ↓ 没有则
runtime.json manifest ──→ 读 manifest.mount_root
        ↓ 没有则
CWD 向上查找 .appfs/ 目录 ──→ heuristic 发现
```

检测到 AppFS 环境后：

1. 如果 principal 不存在 → `create_appfs_principal_from_environment`
   - 写 `create_principal.act`：
     ```json
     { "principal_id": "code-implementer",
       "display_name": "code-implementer",
       "kind": "agent",
       "client_token": "principal-create-<timestamp>" }
     ```
   - 等待 AppFS supervisor 消费，返回 `principal.created`

2. 如果 principal 已存在 → 跳过创建，返回 `Exists`

#### Step 2: Attach 绑定 — `attach_appfs_principal`

`appfs.rs:697` — 写 `attach_principal.act`：

```json
{
  "principal_id": "code-implementer",
  "attach_id": "dashboard-code-implementer",
  "role": null,
  "session_id": null,
  "client_token": "principal-attach-<timestamp>"
}
```

返回 `AppfsAttachLease`：
```rust
{
  principal_id: "code-implementer",
  attach_id: "dashboard-code-implementer",
  action_path: "/path/.appfs/_appfs/principals/detach_principal.act",
  update_action_path: Some("/path/.appfs/_appfs/principals/update_principal.act"),
}
```

#### Step 3: 私有 App 预热 — `warmup_appfs_private_apps`

`appfs.rs:736` — 对每个 `visibility: PrivateInstance` 且匹配当前 principal 的 app：

1. 写 `ensure_credentials.act`：
   ```json
   { "expected_profile_id": "tinode:code-implementer",
     "client_token": "appfs-agent-warmup-tinode--code-implementer-<ts>" }
   ```
2. 等待事件流返回 `profile.credentials.ready` / `failed` / `timed_out`

#### Step 4: 启动 Heartbeat 线程

`main.rs:4096` — spawn 后台线程：

```rust
const APPFS_HEARTBEAT_INTERVAL_SECS: u64 = 30;
loop {
    let shutdown = heartbeat_shutdown_rx.recv_timeout(Duration::from_secs(30));
    match shutdown {
        Ok(()) | Err(Disconnected) => break,
        Err(Timeout) => heartbeat_appfs_principal(lease),
    }
}
```

`heartbeat_appfs_principal` (`appfs.rs:594`) 写轻量 payload：

```json
{
  "principal_id": "code-implementer",
  "attach_id": "dashboard-code-implementer",
  "client_token": "principal-heartbeat-<timestamp>"
}
```

**注意**：不带 `agent_status`。AppFS supervisor 对这种 payload 走 `else if` 分支，
只刷新 `last_seen_at`，不改 agent 状态。

#### Step 5: 进入 Headless 主循环

- 绑定 TCP control endpoint（host: 127.0.0.1, port: 0 随机）
- 输出 `session_started` JSON 到 stdout（Dashboard 通过 stdout 捕获）
- 主循环接收 prompt + AppFS idle wake

### 2.4 AppFS Supervisor 侧的消费

每个 `.act` action 的处理：

**`create_principal.act`** → `handle_create_principal` (runtime_supervisor.rs:335)

```
1. 验证 principal_id 安全性
2. 检查是否已存在 → 已存在则返回 principal.exists
3. 创建 PrincipalRecord
4. 写入 principals.registry.json
5. 写入 _appfs/principals/{principal}.res.json
6. materialize_private_apps_for_principal()
   → 过滤 visibility = Private 的 app
   → 为该 principal 渲染 instance:
     instance_id = "{app_id}--{principal_id}"
     path = "private/{principal_id}/{app_id}"
     profile_id = "{connector}:{principal_id}"
   → 创建 runtime entry + 写 app registry
7. emit principal.created
```

**`attach_principal.act`** → `handle_attach_principal` (runtime_supervisor.rs:601)

```
1. 查找 principal（不存在则失败）
2. 检查是否 refresh（同 attach_id 已存在）
   → 是：刷新 last_seen_at，emit principal.attach_refreshed
3. 冲突检测：是否有 live（非 stale）attach
   → 有冲突且 !takeover → 拒绝 PRINCIPAL_ATTACH_CONFLICT
   → takeover=true → 清理所有旧 attach
4. 创建新 PrincipalAttachLease
5. 更新 active_attach_count, presence = Online
6. 写回 registry + record view
7. emit principal.attached
```

**`update_principal.act`** → `handle_update_principal` (runtime_supervisor.rs:439)

```
1. 更新元数据（display_name, description, kind）——如果提供了
2. 如果有 agent_status + attach_id:
   → 验证 attach_id 匹配 → 更新 last_seen_at → 应用 status patch
3. 否则如果有 attach_id 但没 agent_status (heartbeat):
   → 找到 matching attach → 刷新 last_seen_at
4. 写回 registry + record view
5. emit principal.updated / principal.status.updated
```

---

## 3. 身份系统详解

### 3.1 Principal — Agent 的身份

Principal 是 Agent 在 AppFS 世界中的身份标识。存储在 `_appfs/principals.registry.json`：

```json
{
  "version": 1,
  "principals": [
    {
      "principal_id": "code-implementer",
      "display_name": "Code Implementer",
      "kind": "agent",
      "created_at": "2026-06-05T10:00:00Z",
      "updated_at": "2026-06-05T10:01:30Z",
      "active_attach_count": 1,
      "active_attaches": [
        {
          "attach_id": "dashboard-code-implementer",
          "role": null,
          "session_id": "session-abc123",
          "attached_at": "2026-06-05T10:00:00Z",
          "last_seen_at": "2026-06-05T10:01:30Z"
        }
      ],
      "agent_status": { "state": "idle" }
    }
  ]
}
```

### 3.2 Attach Lease — 运行时绑定

Attach Lease 表示一个 Agent 进程当前绑定到某个 Principal。关键规则：

- 同一 Principal 可以有多个 attach（不同 `attach_id`）
- 但同一时刻只能有**一个非 stale** 的 attach（除非 takeover）
- 新 attach 对 stale attach 自动 takeover
- 新 attach 对 fresh attach 默认拒绝

### 3.3 私有 App 实例化

当 Principal 被创建时，supervisor 根据 app policy registry 中 `visibility: Private` 的 app 自动实例化：

```
app policy: tinode (visibility: Private, path_template: "private/{principal_id}/tinode")
                    ↓ principal "code-implementer" 被创建
实例化: instance_id = "tinode--code-implementer"
        path = "private/code-implementer/tinode"
        profile_id = "tinode:code-implementer"
```

每个私有 app 实例有独立的文件树和 credential profile。

### 3.4 Presence 状态

`principal_presence` (registry.rs:770) 计算派生状态：

```
active_attaches 为空                   → Offline
active_attaches 中有 non-stale (>90s)  → Online
active_attaches 全部 stale             → Stale
```

Stale 是内部过渡状态。在三层改动生效后，正常运行的 Agent 通过 heartbeat 保持 Online，
崩溃后最多 90s 被 sweep 清理为 Offline。Dashboard 不区分 Stale，统一视为 inactive。

---

## 4. Heartbeat + Sweep + Dashboard 三层改动

### 4.1 设计目标

将 attach 状态从三态 (Online / Stale / Offline) 简化为二态 (Online / Offline)。
Stale 只作为内部过渡，在一个 sweep 周期内自动消除。

### 4.2 Layer 1: AppFS Stale Attach Auto-Sweep

**文件**: `runtime_supervisor.rs`, `appfs.rs`

**改动**: 新增 `sweep_stale_attaches_once()` 方法，挂到主 `tokio::select!` 循环。

```rust
pub(super) fn sweep_stale_attaches_once(&mut self) -> Result<()> {
    let now = chrono::Utc::now();
    let mut doc = self.load_principal_registry()?;
    let mut any_changed = false;

    for record in &mut doc.principals {
        let before = record.active_attaches.len();
        record.active_attaches.retain(|lease| {
            !registry::is_principal_attach_stale(lease, now)
        });
        let removed = before - record.active_attaches.len();
        if removed > 0 {
            any_changed = true;
            record.active_attach_count = record.active_attaches.len() as u32;
            if record.active_attaches.is_empty() {
                record.agent_status = None;
            }
        }
    }

    if !any_changed { return Ok(()); }
    registry::write_principal_registry(&self.root, &doc)?;
    // update individual record views
    Ok(())
}
```

主循环接入 (appfs.rs):

```rust
let mut stale_sweep_interval = tokio::time::interval(Duration::from_secs(30));
stale_sweep_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

tokio::select! {
    // ... 原有三个分支 ...
    _ = stale_sweep_interval.tick() => {
        supervisor.sweep_stale_attaches_once();
    }
}
```

### 4.3 Layer 2: Agent Runtime Periodic Heartbeat

**文件**: `appfs.rs`, `lib.rs`, `main.rs`

**改动**: Agent 在 headless 模式下启动后台线程，每 30s 写 `update_principal.act`。

Heartbeat 函数 (`appfs.rs`):

```rust
pub fn heartbeat_appfs_principal(lease: &AppfsAttachLease) -> Result<(), String> {
    let Some(action_path) = lease.update_action_path.as_ref() else {
        return Err("AppFS update principal action path not available".to_string());
    };
    append_principal_lifecycle_action(
        action_path,
        serde_json::json!({
            "principal_id": lease.principal_id,
            "attach_id": lease.attach_id,
            "client_token": format!("principal-heartbeat-{}", now_millis()),
        }),
        "heartbeat",
    )
}
```

AppFS 侧的处理 (`handle_update_principal`):

```rust
if let Some(agent_status) = request.agent_status {
    // 完整状态更新：last_seen_at + agent_status patch
} else if let Some(request_attach_id) = request.attach_id.as_deref() {
    // Heartbeat 路径：只刷新 last_seen_at
    if let Some(active_attach) = record.active_attaches.iter_mut()
        .find(|lease| lease.attach_id == request_attach_id)
    {
        active_attach.last_seen_at = chrono::Utc::now().to_rfc3339();
    }
}
```

后台线程 (`main.rs`):

```rust
let heartbeat_lease = cli.appfs_attach_lease.clone();
let (heartbeat_shutdown_tx, heartbeat_shutdown_rx) = mpsc::channel::<()>();
if heartbeat_lease.is_some() {
    thread::spawn(move || {
        const APPFS_HEARTBEAT_INTERVAL_SECS: u64 = 30;
        loop {
            match heartbeat_shutdown_rx.recv_timeout(Duration::from_secs(
                APPFS_HEARTBEAT_INTERVAL_SECS,
            )) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(lease) = heartbeat_lease.as_ref() {
                        let _ = heartbeat_appfs_principal(lease);
                    }
                }
            }
        }
    });
}
```

### 4.4 Layer 3: Dashboard Binary State Simplification

**文件**: `principal-lifecycle.ts`, `AgentSidebar.tsx`

**改动**: Dashboard 不再区分 stale vs active，统一为 "attach 存在且 fresh" vs "不存在或 stale"。

`principalAttachState` 返回值：

```typescript
// 之前: { active: boolean; stale: boolean }
// 现在: { active: boolean; hasStaleAttaches: boolean }
```

`deletePrincipal` 遇到 stale attach 时：

```typescript
if (attachState.hasStaleAttaches && principal) {
  const attach = freshestAttach(principal);
  if (attach?.attach_id) {
    // Best-effort detach，不等确认
    appendPrincipalAction(project.mountRoot, 'detach_principal.act', {
      principal_id: principalId,
      attach_id: attach.attach_id,
      reason: 'pre_delete_stale_cleanup',
    });
  }
}
// 然后带 force: true delete
const clientToken = appendPrincipalAction(project.mountRoot, 'delete_principal.act', {
  principal_id: principalId,
  ...(attachState.hasStaleAttaches ? { force: true } : {}),
});
```

前端 `isPrincipalActive` 简化：

```typescript
// 之前: 检查 isAttachStale + hasFreshPrincipalAttach + ACTIVE_PRINCIPAL_STATUSES
// 现在: 只看 active_attaches.length > 0
function isPrincipalActive(principal?: PrincipalLifecycleInfo): boolean {
  if (!principal) return false;
  const attaches = principal.active_attaches ?? [];
  if (attaches.length > 0) return true;
  return ACTIVE_PRINCIPAL_STATUSES.has(principal.status) && principal.status !== 'online';
}
```

### 4.5 时间常数

| 常量 | 值 | 含义 |
|------|-----|------|
| Agent heartbeat interval | 30s | Agent 多久发一次心跳 |
| Stale threshold | 90s | 多久没心跳认为 agent 死了（3 次 heartbeat） |
| AppFS sweep interval | 30s | Supervisor 多久扫一次 stale attach |
| Dashboard stale threshold | 90s | Dashboard 侧判断 stale 的阈值（已简化，不再暴露给前端） |

### 4.6 效果对比

| 场景 | 改动前 | 改动后 |
|------|--------|--------|
| Agent idle 90s | attach stale（不正确） | heartbeat 保持 Online |
| Agent 崩溃 | attach 永远 stale，需手动清理 | 90s 后 sweep 自动清理为 Offline |
| Stop 后立即 Delete | 可能被拒（attach online/stale） | Stop 走 lifecycle detach → Delete 可用 |
| Dashboard 遇 stale attach Delete | 返回 409 或带 force | 自动 best-effort detach + force delete |
| 前端展示 | 显示 stale 状态（令人困惑） | 只有 active / inactive |

---

## 5. 完整生命周期流程图

```
Create Principal          POST /api/projects/:id/principals
    │                        → append create_principal.act
    │                        → AppFS: 注册 + 实例化私有 app + emit created
    ↓
Start Agent              POST /api/projects/:id/principals/:pid/start
    │                        → processManager.spawn()
    │                        → Agent: detect AppFS → attach → warmup → heartbeat
    ↓
Running
    │                        → heartbeat 每 30s 刷新 last_seen_at
    │                        → input_router 消费事件流
    │                        → model turn loop 处理输入
    ↓
Stop Agent               POST /api/projects/:id/principals/:pid/stop
    │                        → await terminateChildProcessTree()
    │                        → append detach_principal.act
    │                        → await principal.detached
    ↓
Delete / Archive         DELETE /api/projects/:id/principals/:pid
    │                        → 检查无 managed agent / online agent / active attach
    │                        → 如 stale: best-effort detach + force delete
    │                        → append delete_principal.act → await principal.deleted
    │                        → AppFS: 清理 private app runtime + credentials
    │                        → archiveSessionsForPrincipal
    ↓
Archived
                             → UI 移入 "Archived agents" 区域
                             → resume 过滤跳过 archived agent

异常路径:
    Agent 崩溃 → heartbeat 停止
               → 90s 后 AppFS sweep 清理 stale attach
               → Dashboard 下次发现 stale → 自动 detach + 可 Delete
```

---

## 6. 文件清单

### AppFS (Rust)

| 文件 | 改动 |
|------|------|
| `appfs/cli/src/cmd/appfs/runtime_supervisor.rs` | `sweep_stale_attaches_once()` + `handle_update_principal` heartbeat 分支 |
| `appfs/cli/src/cmd/appfs/appfs.rs` | 主循环新增 sweep interval |
| `appfs/cli/src/cmd/appfs/registry.rs` | 未改（`is_principal_attach_stale` 90s 阈值保持不变） |

### Agent Runtime (Rust)

| 文件 | 改动 |
|------|------|
| `appfs-agent/rust/crates/runtime/src/appfs.rs` | `heartbeat_appfs_principal()` |
| `appfs-agent/rust/crates/runtime/src/lib.rs` | 导出 `heartbeat_appfs_principal` |
| `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs` | headless heartbeat 线程 |

### Dashboard (TypeScript)

| 文件 | 改动 |
|------|------|
| `dashboard/server/src/principal-lifecycle.ts` | `principalAttachState` 二态化 + stale 自动 detach |
| `dashboard/server/src/process-manager.ts` | `latestResumableAgentPerPrincipal` 加 archived 过滤 |
| `dashboard/src/components/AgentSidebar.tsx` | 去 stale 逻辑，`stopAgent` 走 lifecycle API |
| `dashboard/server/src/routes/process.ts` | 旧 stop route 弃用注释 |
| `dashboard/server/src/process-manager.test.ts` | archived / missing-principalId 测试 |

---

*本文档基于 2026-06-05 代码状态撰写。*
