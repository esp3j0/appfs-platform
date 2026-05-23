# AppFS ↔ appfs-agent 协作方向讨论

> 2026-04-26 · v0.1 · 供内部讨论

---

## 1. 背景：两个项目是什么

`appfs-platform` 是一个 monorepo，包含两个协作的子项目：

| 项目 | 角色 | 技术栈 |
|------|------|--------|
| **appfs** | 文件系统原生 App 协议 + 引擎。把不同 App 后端挂载成统一虚拟文件系统树，AI agent 通过标准 shell 原语（`cat`/`>>`/`ls`/`tail -f`）交互。版本 `0.7.0-beta.1` | Rust, SQLite (Turso), FUSE/NFS/WinFsp |
| **appfs-agent** | AI coding agent（Claw Code — Claude Code 的 Rust 重写）。交互式 REPL、40 种工具、多 LLM 支持。挂载到 AppFS 文件系统上运行 | Rust (~20K LOC), tokio, 9 crates |

**核心设计理念**：AI 训练数据中文件系统操作的语料极其丰富，把 App 协议映射成文件系统能显著降低 agent 的学习成本，也便于做强化学习训练。文件系统在这里是"最小公分母协议"。

**协作方式**（attach contract v1.1）：appfs 把运行时清单写在 `/.well-known/appfs/runtime.json`，appfs-agent 启动时通过三层发现（环境变量 → manifest → 目录启发式）自动 attach。

---

## 2. 当前阶段

- ✅ AppFS 核心引擎（虚拟文件系统、OverlayFS、KV Store、远程同步）
- ✅ AppFS 协议层（compose up、多 App 运行时、HTTP/gRPC 桥接、分页、scope）
- ✅ 三平台挂载（Linux FUSE / macOS NFS / Windows WinFsp）
- ✅ appfs-agent Rust 实现基本完整
- ✅ 四个集成检查点全部完成（冒烟、HTTP 桥接、多 agent attach、Launcher 模式）
- 🔨 真实 App 试点进行中（huoyan、aiim）
- 📋 **下一步：agent 和 AppFS 的深度协同**

---

## 3. 现状中的关键 gap

当前 appfs-agent **能发现** AppFS 环境（`detect_appfs_environment()` 已完整实现三层发现），但这些信息**只在 `/status` 命令里展示**，从未进入 LLM 对话上下文：

- ✗ 系统提示词里没有 AppFS 环境信息
- ✗ 没有 AppFS 相关工具定义
- ✗ 没有事件流订阅
- ✗ agent 不知道自己运行在 AppFS 环境中

这意味着 agent 虽然运行在 AppFS 挂载的文件系统上，但对 App 的存在、能力、事件流一无所知。

---

## 4. 提出的协作方向：三层模型

```
┌─────────────────────────────────────────────────────────┐
│ Layer 1: System Prompt（启动时注入，零交互成本）          │
│                                                         │
│ "你在 AppFS 环境中，mount_root=/mnt/appfs"               │
│ "注册的 app: aiim, huoyan"                               │
│ ".act = 动作  .res.jsonl = 快照资源  .evt.jsonl = 事件流"│
│ "用标准文件操作即可与 App 交互"                           │
│ "需要详细了解某个 App 时，使用 /aiim 查看"                │
│                                                         │
│ 成本：~200-300 tokens，占 system prompt 总量比例极小       │
├─────────────────────────────────────────────────────────┤
│ Layer 2: Skills 模式（按需注入，不浪费 context window）    │
│                                                         │
│ 每个 App 生成一份 SKILL.md，放在 .claw/apps/<app>/ 下    │
│ ┌─────────────────────────────────────────────────┐     │
│ │ .claw/apps/aiim/SKILL.md                        │     │
│ │                                                  │     │
│ │ ---                                              │     │
│ │ name: aiim                                       │     │
│ │ description: Factory incident management         │     │
│ │ when_to_use: "When working with incidents..."    │     │
│ │ allowed-tools: [Bash, Read]                      │     │
│ │ ---                                              │     │
│ │ # AIIM Incident Management                       │     │
│ │                                                  │     │
│ │ ## Available Actions                             │     │
│ │ | Action | Path |                                │     │
│ │ |--------|------|                                │     │
│ │ | Send   | /aiim/chats/{id}/send_message.act     │     │
│ │                                                  │     │
│ │ ## App Layout                                    │     │
│ │ aiim/                                            │     │
│ │   chats/{id}/messages.res.jsonl                  │     │
│ │   cases/{id}/detail.res.json                     │     │
│ └─────────────────────────────────────────────────┘     │
│                                                         │
│ 触发方式：                                               │
│ • 模型需要时调用 Skill(aiim)，markdown 内容注入到上下文 │
│ • 用户输入 /aiim，等价于 skill 调用                     │
│ • 条件激活：访问 App 目录文件时自动激活该 skill          │
│                                                         │
│ 优势：完全复用现有 skills 机制，零新协议、零新工具        │
├─────────────────────────────────────────────────────────┤
│ Layer 3: Event Stream 自动订阅（后台持续运行）            │
│                                                         │
│ appfs-agent 启动时：                                     │
│ 1. 从 AppFS 发现控制面事件流 + 所有已注册 App 的事件流   │
│ 2. 读 cursor.res.json 获取上次消费位置                   │
│ 3. 从 from-seq/{seq}.evt.jsonl 回放未读事件（catch-up）  │
│ 4. 进入 tail -f 模式，持续监听 events.evt.jsonl          │
│                                                         │
│ 收到新事件后：                                           │
│ • 写入 session 的 pending_appfs_events 队列             │
│ • 每轮 turn 开始前，把新事件打包注入到 LLM 上下文        │
│ • 用 (app, session_id, seq) 去重                         │
│ • cursor 持久化到 session 目录，支持断点续传             │
│                                                         │
│ 效果：agent 无需主动轮询，自动感知 App 的状态变化        │
│  （action 完成/失败、资源更新、其他 agent 的操作结果）   │
└─────────────────────────────────────────────────────────┘
```

---

## 5. 为什么这个方向可行

### 5.1 "App 即 Skill" 是自然的抽象

Skills 机制已有的能力和 AppFS App 的需求高度对应：

| Skills 机制 | AppFS App 需求 |
|-------------|---------------|
| `SKILL.md` markdown 格式 | App 的操作说明、actions、resources 列表 |
| `when_to_use` 触发条件 | 告诉模型什么场景该用这个 App |
| `allowed-tools` 工具白名单 | 限制该 App 只能用 Bash/Read，防止越权 |
| 按需注入（不占 context window） | agent 可能挂载几十个 App，不能全量注入 |
| 条件激活（`paths:` glob） | 访问 App 目录文件时自动激活对应 skill |

**改动量极小**：不需要新工具、不需要新协议。只需要：
1. 在 system prompt 里加 AppFS 环境摘要（~50 行）
2. 把 AppFS manifest 翻译成 SKILL.md 格式（一个转换器）
3. 在 skills 搜索路径里加 `.claw/apps/`

### 5.2 Bash 工具天然就是 AppFS 的客户端

这是 AppFS 设计的核心优势——

```
模型: ls /mnt/appfs/                     → bash 工具 → 看到 aiim/ huoyan/
模型: cat /mnt/appfs/aiim/chats/123/messages.res.jsonl → bash 工具 → 拿到消息列表
模型: echo '{"content":"hello"}' >> /mnt/appfs/aiim/chats/123/send_message.act → bash 工具 → 发消息
模型: tail -f /mnt/appfs/aiim/_stream/events.evt.jsonl → bash 工具 → 实时事件
```

不需要为每个 App 定义专用工具——bash 一个通用工具就是全部 App 的客户端。

### 5.3 Event 订阅让 agent 从"瞎子"变"有感知"

当前 agent 发出 action 后完全不知道结果——只能通过轮询或等待超时。自动订阅后：
- action 完成/失败即时感知
- 其他 agent 的操作结果可见（多 agent 协作的基础）
- App 状态变更自动同步

---

## 6. 实施优先级建议

| 优先级 | 事项 | 预估工作量 | 依赖 |
|--------|------|-----------|------|
| P0 | System prompt 注入 AppFS 环境摘要 | ~50 行 | 无 |
| P0 | Event stream 自动订阅（含 cursor 管理、断点回放） | ~300 行 | appfs.rs 已有环境检测 |
| P1 | Manifest → SKILL.md 转换器 | ~200 行 | 需确定 SKILL.md 模板格式 |
| P1 | Skills 搜索路径添加 `.claw/apps/` | ~30 行 | P0 完成后 |
| P2 | 条件激活（访问 App 目录自动激活 skill） | ~100 行 | P1 完成后 |
| P3 | 权限联动（App manifest 权限 → agent PermissionPolicy） | 待评估 | 真实 App 试点反馈后 |

---

## 7. 已知风险与待解决问题

| 风险/问题 | 影响 | 应对 |
|-----------|------|------|
| **多平台挂载后端可靠性不一致** | Linux FUSE 最成熟，macOS NFS 无状态语义冲突，Windows WinFsp edge case 多 | 收敛测试面到 Linux FUSE，macOS/Windows 先 best-effort |
| **`_stream/` 目录职责混杂** | 对外事件流和内部实现状态混在同一目录 | 后续考虑把 `action-cursors`、`inflight.jobs` 等内部状态挪到 `_internal/` |
| **App 数量增长后 context window 压力** | 注册几十个 App 后，即使按需注入也可能超出窗口 | Layer 2 的按需注入 + 条件激活机制是设计时已考虑的对策 |
| **权限模型落地不足** | 当前 AppFS 的 permissions/observer 声明了但未强制执行 | 暂不纳入当前迭代，等真实场景暴露需求 |

---

## 8. 关键文件参考

| 文件 | 说明 |
|------|------|
| `appfs-agent/rust/crates/runtime/src/appfs.rs` | agent 侧 AppFS 环境检测（三层发现已实现） |
| `appfs-agent/rust/crates/runtime/src/prompt.rs` | system prompt 构建，`__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` 为注入点 |
| `appfs-agent/rust/crates/commands/src/skill_docs.rs` | Skill 文档解析（frontmatter + markdown） |
| `appfs-agent/rust/crates/commands/src/lib.rs` | Skill 发现（`discover_skill_roots`）、加载、条件激活 |
| `appfs-agent/rust/crates/commands/src/bundled_skills.rs` | 内置 skills（verify/remember/stuck/skillify），可作模板参考 |
| `appfs/cli/src/cmd/appfs/events.rs` | AppFS 侧事件发射（`emit_event`、cursor 更新、from-seq 回放） |
| `appfs/cli/src/cmd/appfs/runtime_manifest.rs` | 运行时自描述清单（`.well-known/appfs/runtime.json`） |
| `appfs/examples/appfs/fixtures/aiim/_meta/manifest.res.json` | App manifest 示例，SKILL.md 的数据来源 |
