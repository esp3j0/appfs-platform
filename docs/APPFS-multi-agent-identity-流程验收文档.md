# AppFS Multi-Agent Identity 流程验收文档

> 基于 `APPFS-multi-agent-identity-and-app-visibility-v0-design.md`，覆盖 compose 启动、agent 身份创建、私有 app 实例化、首次业务使用透明凭据、principal fork 全链路。

---

## 当前实现说明

本文最初按“第一个 agent 启动时创建 default principal”描述。当前实现已经前移了一步：

1. `appfs compose up` 启动后，如果 compose 中存在 `visibility: private` app policy，AppFS supervisor 会自动确保 `default` principal 存在。
2. AppFS supervisor 会立即为已有 principal 物化 private app，例如 `/private/default/tinode`。
3. appfs-agent 启动时仍会解析当前 `principal_id`，但它不再是 `default` principal 的唯一创建者。
4. appfs-agent 的 `/principal create` 和 `/principal fork` 仍用于新增非 default principal。

这意味着：即使还没有启动 agent，也可以在 mount 根下看到 `/private/default/tinode`。这是预期行为。

自动化 smoke:

```powershell
cd C:\Users\esp3j\rep\appfs-platform
$env:APPFS_TINODE_ENDPOINT = "http://101.34.216.193:6060"
$env:APPFS_TINODE_API_KEY = "<tinode-api-key>"
.\integration\scripts\test-windows-appfs-tinode-multi-agent-smoke.ps1
```

该脚本覆盖 compose 启动、`default` private app 物化、创建 `code-implementer` principal、两个 principal 的 `appfs-tinode` skill/status、Tinode 凭据、双向私聊和 inbox read-through。

---

## 验收场景总览

```
Compose 启动
 │
 ├─→ 场景 A: Compose 自动准备 default principal
 │   ├─ A1: default principal 自动创建
 │   ├─ A2: private app 自动实例化
 │   └─ A3: agent 以 default 身份使用 private app
 │
 └─→ 场景 B: 第二个 Agent 创建（principal fork）
     ├─ B1: principal fork + 新 private app 实例
     └─ B2: 两个 agent 并行运行，身份完全隔离
```

---

## 前置条件

```yaml
# appfs-compose.yaml
version: 1
name: multi-agent-smoke
runtime:
  db: .agentfs/smoke.db
  mountpoint: C:/mnt/appfs-smoke
  backend: winfsp
  init: if_missing

connectors:
  aiim-http:
    transport: http
    endpoint: http://127.0.0.1:8080
    mode: command
    command:
      program: python
      args: ["-u", "bridge_server.py"]
      env: { APPFS_HTTP_BRIDGE_BACKEND: aiim }

  tinode-http:
    transport: http
    endpoint: http://127.0.0.1:6061
    mode: command
    command:
      program: node
      args: ["connectors/tinode-bridge/server.mjs"]
      env:
        APPFS_TINODE_ENDPOINT: http://101.34.216.193:6060
        APPFS_TINODE_API_KEY: "<tinode-api-key>"

apps:
  aiim:
    connector: aiim-http
    visibility: public
    path: public/aiim

  tinode:
    connector: tinode-http
    visibility: private
    credential_policy: auto-create
    path_template: private/{principal_id}/tinode
    profile_template: tinode:{principal_id}
```

说明：

1. `connectors.tinode-http.endpoint` 是 AppFS Tinode connector bridge 的地址，不是 Tinode 服务端地址。
2. Tinode 服务端地址通过 connector env，例如 `APPFS_TINODE_ENDPOINT`，传给 bridge。
3. Tinode action path 在本文中是示例路径；最终联系人/群聊树以后以 Tinode app tree 设计为准，本验收重点是 identity、profile、凭据和事件隔离流程。

```bash
# 启动 AppFS compose
agentfs appfs compose up -f appfs-compose.yaml
```

---

## 场景 A：default principal

### Step A1.1 — compose 启动完成后的状态

AppFS mount 已挂载，supervisor poll loop 已启动。因为 compose 中声明了 private `tinode` app policy，supervisor 会自动创建 `default` principal 并物化 `private/default/tinode`。

**`/_appfs/app-policies.registry.json`**（app 模板注册表）：

```json
{
  "version": 1,
  "apps": [
    {
      "app_id": "aiim",
      "visibility": "public",
      "connector": "aiim-http",
      "path": "public/aiim",
      "legacy_aliases": ["aiim"]
    },
    {
      "app_id": "tinode",
      "visibility": "private",
      "connector": "tinode-http",
      "credential_policy": "auto-create",
      "path_template": "private/{principal_id}/tinode",
      "profile_template": "tinode:{principal_id}"
    }
  ]
}
```

**`/_appfs/apps.registry.json`**（app 实例注册表）至少包含：

```json
{
  "version": 1,
  "apps": [
    {
      "instance_id": "aiim",
      "app_id": "aiim",
      "visibility": "public",
      "path": "public/aiim",
      "transport": { "kind": "http", "endpoint": "http://127.0.0.1:8080" }
    },
    {
      "instance_id": "tinode--default",
      "app_id": "tinode",
      "visibility": "private_instance",
      "principal_id": "default",
      "path": "private/default/tinode",
      "profile_id": "tinode:default",
      "transport": { "kind": "http", "endpoint": "http://127.0.0.1:6061" }
    }
  ]
}
```

**`/_appfs/principals.registry.json`**：包含 `default` principal。

**Verification**：
```bash
agentfs fs cat smoke.db /_appfs/apps.registry.json | jq '.apps | length'  # → 至少 2 (aiim + tinode--default)
agentfs fs cat smoke.db /_appfs/app-policies.registry.json | jq '.apps | length'  # → 2
agentfs fs cat smoke.db /_appfs/principals.registry.json | jq '.principals[].principal_id'  # → "default"
```

---

### Step A1.2 — default Agent 启动

```bash
claw
```

appfs-agent 执行 `detect_appfs_environment()`：

1. 发现 `/.well-known/appfs/runtime.json` → 拿到 mount_root、runtime_session_id。
2. 发现 `_appfs/` 控制面目录存在。
3. 读 `apps.registry.json` → 看到 aiim（public）和 tinode--default（private_instance）。
4. 读 `principals.registry.json` → 看到 default。
5. 未设置 `APPFS_PRINCIPAL_ID` → fallback 到 `default`。
6. appfs-agent 不需要再创建 default，只需要以 default 身份过滤 app、skills 和 events。

**Verification**：agent status 输出中可见 `principal_id: default`。

---

### Step A1.3 — Supervisor 自动准备 default private app

当前实现里，`default` 不再依赖第一个 agent 写 `create_principal.act`。Compose 启动后如果存在 private app policy，supervisor 会主动执行一次安全的 default bootstrap：

1. 确保 `/_appfs/principals.registry.json` 存在。
2. 如果 `default` 不存在，则创建 default principal。
3. 生成或刷新派生视图 `/_appfs/principals/default.res.json`。
4. 读取 `/_appfs/app-policies.registry.json`。
5. 找到 `tinode`（visibility=private）。
6. 自动实例化：
   ```
   profile_template: "tinode:{principal_id}" → "tinode:default"
   path: private/{principal_id}/tinode → private/default/tinode
   ```
7. 在 `apps.registry.json` 中追加：
   ```json
   {
     "instance_id": "tinode--default",
     "app_id": "tinode",
     "visibility": "private_instance",
     "principal_id": "default",
     "path": "private/default/tinode",
     "profile_id": "tinode:default",
     "transport": { "kind": "http", "endpoint": "http://127.0.0.1:6061" }
   }
   ```
8. 为 private/default/tinode 创建 adapter：
   - 连接 tinode-http bridge
   - tree sync → 物化 `_meta/`、`_stream/`、`_app/`、`contacts/`、`topics/` 目录
9. 发射事件：`principal.created`、`app.instance.created`。

注意：tree sync / app structure bootstrap 不应该创建 Tinode 上游账号或 token。`private/default/tinode` 可以先被物化为安全骨架，真正的 `tinode:default` 凭据应等首次 credential-required 业务操作时再创建。

显式 `/_appfs/principals/create_principal.act` 仍然保留，用于创建 `default` 之外的语义身份；如果并发或重复创建同一个 principal，supervisor 应按幂等路径处理，已存在则复用。

**Verification**：
```bash
agentfs fs cat smoke.db /_appfs/principals.registry.json | jq '.principals | length'  # → 1
agentfs fs cat smoke.db /_appfs/apps.registry.json | jq '.apps | length'               # → 2 (aiim + tinode--default)
agentfs fs ls smoke.db /private/default/tinode                                         # → _meta/ _stream/ _app/ contacts/ topics/
# connector 私有存储中此时还不应有 tinode:default credentials
```

---

### Step A1.4 — Agent 完成启动

Agent 继续启动流程：

1. **System prompt** 注入身份段：

   ```
   You are attached to an AppFS project.

   Current agent identity:
   - principal_id: default
   - display_name: Default agent
   - description: The default project agent.
   - attach_id: attach-20260506-001
   - private_root: /private/default

   Known project principals:
   - default: Default agent. The default project agent.

   AppFS app layout:
   - /public contains apps shared by all principals.
   - /private/<principal_id> contains private app instances for each principal.
   - Your private app root is /private/default.
   - Do not operate on another principal's private app unless the user explicitly asks.
   ```

2. **Skill listing**：扫描 `/public/` 和 `/private/default/`
   - 发现 `public/aiim` → 生成 skill `appfs-aiim`
   - 发现 `private/default/tinode` → 生成 skill `appfs-tinode`

3. **Event subscription**：订阅 `/_appfs/_stream/events.evt.jsonl` + `/public/aiim/_stream/events.evt.jsonl` + `/private/default/tinode/_stream/events.evt.jsonl`

**Verification**：`/status` 命令输出包含 AppFS identity section。`/skills list` 包含 `appfs-aiim` 和 `appfs-tinode`。

---

### Step A3 — 首次业务调用：透明凭据创建 + 发消息

**状态**：`private/default/tinode` 已实例化。**未**调用过 `ensure_credentials.act`。connector 私有存储中无 `tinode:default` 凭据。

用户说：**"给张三说明天开会"**

```
[Iteration 1]
sync_appfs_events_before_model_call → 无新事件
LLM → Skill("appfs-tinode")
Skill 全文注入（base directory: /private/default/tinode）

[Iteration 2]
sync_appfs_events_before_model_call → 无新事件
LLM → bash: printf '{"text":"明天十点开会"}' >> /private/default/tinode/contacts/zhangsan/send_message.act

[Iteration 3]
sync_appfs_events_before_model_call → 读事件流
```

**关键：模型没有先写 `ensure_credentials.act`。直接写了 `send_message.act`。**

connector 内部 `submit_action` 流程：

```
1. 从 ConnectorContext 拿到 profile_id = "tinode:default"
                       principal_id = "default"
   从 SubmitActionRequest 拿到 path = "contacts/zhangsan/send_message.act"
   ↓
2. 检查私有存储: connector:tinode-http:profile:tinode:default:credentials
   ↓
3. 不存在 → 透明创建:
   3.1 调 Tinode API {acc} → 注册账号
   3.2 获取 usrXXX + token + refresh_token + expires_at
   3.3 存入私有存储
   3.4 发射: {type:"profile.credentials.ready", profile_id:"tinode:default", upstream_user_id:"usrXXX"}
   ↓
4. 用新 token 调 Tinode API {pub} → 发送消息
   ↓
5. 返回 SubmitActionResponse(ok)
   ↓
6. adapter 至少发射 action.completed；也可以额外发射 Tinode app 事件 {type:"message.sent", ...}
```

**Verification**：
```bash
# connector 的私有存储中应有凭据（不通过 AppFS tree 暴露）
# appfs-agent 的 event log 中应有:
#   {"type":"profile.credentials.ready","principal_id":"default","profile_id":"tinode:default","upstream_user_id":"usrXXX"}
#   {"type":"action.completed",...}
#   可选: {"type":"message.sent",...}
```

**Agent 看到的 event**（在 iteration 3 或下一轮中通过 `<system-reminder>` 注入）：

```
<system-reminder>
New AppFS events were received since the previous model call.
- [AppFS app `tinode`] profile.credentials.ready profile_id=tinode:default upstream_user_id=usrXXX
- [AppFS app `tinode`] type=action.completed path=/contacts/zhangsan/send_message.act
</system-reminder>
```

模型看到后回复："已发送成功，账号已就绪 usrXXX"。

---

### Step A3-bis — 后续操作无需额外步骤

用户说：**"给李四也说一下"**

模型直接写当前 Tinode app skill 中描述的发送消息 action，例如 `/private/default/tinode/contacts/lisi/send_message.act`。

connector 这次检查私有存储 → `tinode:default` 有凭据 → token 未过期 → 直接发 → 返回。

**Verification**：第二次 send_message 不需要凭据创建，响应时间明显短于第一次。

---

## 场景 B：第二个 Agent（principal fork）

### Step B1.1 — 当前 agent 触发 principal fork

用户在 default agent 对话中说：**"创建一个 incident-reporter agent，专门负责事故通知"**

当前实现使用 `/principal fork` 完成“创建/复用 principal + fork 当前 session + 给子会话写入 bootstrap 消息”。它不会直接后台启动子进程，而是输出可复制的启动命令，让用户或后续 launcher 明确启动新的 agent 进程。

`/principal fork incident-reporter 创建一个 agent 专门负责事故通知`

命令内部：

1. 写入 `/_appfs/principals/create_principal.act`：

   ```json
   {
     "principal_id": "incident-reporter",
     "display_name": "Incident reporter",
     "description": "Summarizes incidents and sends chat updates.",
     "kind": "agent",
     "client_token": "create-incident-reporter"
   }
   ```

2. Supervisor poll 到该 action，执行和 A1.3 完全相同的流程：
   - `principals.registry.json` 追加 incident-reporter。
   - `principals/incident-reporter.res.json` 生成。
   - 读 `app-policies.registry.json`。
   - 发现 tinode 是 private → 自动实例化：
     ```
     instance_id: "tinode--incident-reporter"
     principal_id: "incident-reporter"
     path: "private/incident-reporter/tinode"
     profile_id: "tinode:incident-reporter"
     ```
   - `apps.registry.json` 追加新 instance。
   - 创建 adapter，物化 `private/incident-reporter/tinode/` 目录。
   - 发射 `principal.created`、`app.instance.created`。

3. appfs-agent fork 当前 session 文件，并注入一条 bootstrap 消息，说明新 principal 的任务、父 session 和启动意图。

4. 输出子进程启动命令：

   ```powershell
   $env:APPFS_PRINCIPAL_ID="incident-reporter"; claw --session "<child-session-file>"
   ```

这和 `/session fork` 的关键区别是：`/session fork` 只在同一 principal 下分叉会话；`/principal fork` 会为新 principal 准备 private app，并让新进程以新的 `APPFS_PRINCIPAL_ID` 启动。

---

### Step B1.2 — 子 Agent 启动

用户在新终端运行 `/principal fork` 输出的命令后，子 agent（incident-reporter）启动并执行 `detect_appfs_environment()`：

1. 读 `principals.registry.json` → 找到自己的 `principal_id=incident-reporter` 信息。
2. 读 `apps.registry.json` → 现在有 3 条记录：
   - `aiim`（public） ✅ 可见
   - `tinode--default`（private_instance, principal_id=default） ❌ 不可见
   - `tinode--incident-reporter`（private_instance, principal_id=incident-reporter） ✅ 可见

3. System prompt 注入：

   ```
   Current agent identity:
   - principal_id: incident-reporter
   - display_name: Incident reporter
   - description: Summarizes incidents and sends chat updates.
   - attach_id: attach-incident-001
   - private_root: /private/incident-reporter

   Known project principals:
   - default: Default agent. The default project agent.
   - incident-reporter: Incident reporter. Summarizes incidents and sends chat updates.
   ```

4. Skill listing：
   - `public/aiim` → `appfs-aiim`
   - `private/incident-reporter/tinode` → `appfs-tinode`
   - **不生成** `appfs-tinode` for default 的实例。

5. Event subscription：
   - 订阅 `/_appfs/_stream/events.evt.jsonl`
   - 订阅 `/public/aiim/_stream/events.evt.jsonl`
   - 订阅 `/private/incident-reporter/tinode/_stream/events.evt.jsonl`
   - **不订阅** `/private/default/tinode/_stream/events.evt.jsonl`。

**Verification**：
```bash
# 第二个 agent 的 /status 输出中:
# principal_id = incident-reporter
# 可见的 app 不包括 private/default/tinode
```

---

### Step B2 — 两个 Agent 并行运行

**状态**：

| 组件 | default agent | incident-reporter agent |
|------|--------------|------------------------|
| principal_id | default | incident-reporter |
| private_root | /private/default | /private/incident-reporter |
| 可见 Tinode | /private/default/tinode | /private/incident-reporter/tinode |
| profile_id | tinode:default | tinode:incident-reporter |
| connector 凭据 | 已创建（Step A3） | 未创建 |

**default agent 发消息**：

```bash
# default agent 写:
printf '{"text":"明天十点开会"}' >> /private/default/tinode/contacts/zhangsan/send_message.act

# connector 内部:
profile_id = "tinode:default" → 有凭据 → 直接调 Tinode API → 成功
```

**incident-reporter agent 首次发消息**：

```bash
# incident-reporter agent 写:
printf '{"text":"发现事故 #1234"}' >> /private/incident-reporter/tinode/contacts/zhangsan/send_message.act

# connector 内部:
profile_id = "tinode:incident-reporter" → 无凭据 → 自动创建新 Tinode 账号
  → 获取 usrYYY + token_2 → 存入私有存储（key=tinode:incident-reporter，和 tinode:default 不同）
  → 用 token_2 发消息 → 成功
  → 发射 profile.credentials.ready (incident-reporter)
```

**Verification**：

Connector 私有存储中有两份凭据：

```
connector:tinode-http:profile:tinode:default:credentials
  → {upstream_user_id: "usrXXX", token: "t1", refresh_token: "rt1"}

connector:tinode-http:profile:tinode:incident-reporter:credentials
  → {upstream_user_id: "usrYYY", token: "t2", refresh_token: "rt2"}
```

Tinode 上游有两个独立账号（usrXXX 和 usrYYY），各自独立收发消息。

两个 agent 的 `<system-reminder>` 互不干扰：
- default agent 看不到 incident-reporter 的消息事件。
- incident-reporter agent 看不到 default 的消息事件。

---

## 验收检查清单

### Compose 启动
- [ ] `app-policies.registry.json` 包含 aiim(public) + tinode(private)
- [ ] `principals.registry.json` 启动后包含 default
- [ ] `apps.registry.json` 启动后包含 aiim（public）和 `tinode--default`
- [ ] `private/default/tinode/` 在 agent 启动前已物化
- [ ] tree sync / app structure bootstrap 不创建 `tinode:default` 凭据

### 第一个 Agent（default principal）
- [ ] 未设置 `APPFS_PRINCIPAL_ID` 时，appfs-agent fallback 到 `default`
- [ ] system prompt 包含 principal_id=default 和 private_root=/private/default
- [ ] skill listing 包含 appfs-aiim 和 appfs-tinode
- [ ] event subscription 包含 platform、public/aiim、private/default/tinode 事件流

### 首次业务调用（透明凭据创建）
- [ ] 模型直接写 send_message.act，不先写 ensure_credentials.act
- [ ] connector 在 submit_action 时检测到无凭据
- [ ] connector 自动创建上游账号，存储 token
- [ ] 发射 profile.credentials.ready event
- [ ] 用新凭据成功执行原始业务 action
- [ ] event 通过 `<system-reminder>` 注入 agent
- [ ] 第二次 send_message 无需凭据创建（耗时明显更短）

### 第二个 Agent（principal fork）
- [ ] `/principal fork <id> <task>` 写入 create_principal.act
- [ ] `/principal fork` fork 当前 session，并写入 bootstrap 消息
- [ ] `/principal fork` 输出 `$env:APPFS_PRINCIPAL_ID="<id>"; claw --session "<child-session-file>"`
- [ ] supervisor 自动实例化 `private/incident-reporter/tinode`
- [ ] `principals.registry.json` 包含两个 principal
- [ ] `apps.registry.json` 包含三条记录（aiim + 两个 tinode instance）
- [ ] 子 agent 的 system prompt 包含 principal_id=incident-reporter
- [ ] 子 agent 的 skill listing 不包含 private/default/tinode
- [ ] 子 agent 的事件订阅包含 platform、public/aiim、private/incident-reporter/tinode
- [ ] 子 agent 的事件订阅不包含 default 的 tinode 事件流

### 并行运行
- [ ] default 和 incident-reporter 使用不同的 profile_id
- [ ] connector 私有存储中有两份独立凭据
- [ ] 上游 Tinode 有两个独立账号
- [ ] 两个 agent 的 `<system-reminder>` 互不干扰
