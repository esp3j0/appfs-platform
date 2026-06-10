# AppFS Platform：以文件系统为中心的多 Agent 协作基础设施

> 研发期刊 — 用于内部同步

---

## 1. 核心思想：一切皆文件

AppFS（Agent Application File System）的核心设计理念来自 Unix 哲学——**把 AI Agent 的"应用状态"建模为文件树**。

传统 AI Agent 系统中，应用状态散落在数据库、API、消息队列等不同位置。Agent 需要为每个外部系统写不同的集成代码。AppFS 把这一切统一到一棵文件树下，\_appfs为系统软件的访问、控制面，public/private为三方软件的访问、控制面：

```
/appfs-mount/
├── _appfs/                          ← 控制面（注册 app、管理 principal 身份）
│   ├── apps.registry.json           ← 已注册的 app 列表
│   ├── app-policies.registry.json   ← app 可见性策略（public/private）
│   ├── principals.registry.json     ← principal 注册表
│   ├── principals/
│   │   ├── create_principal.act     ← append JSON → 创建 principal
│   │   ├── attach_principal.act     ← append JSON → 绑定 agent 到 principal
│   │   ├── update_principal.act     ← append JSON → 更新状态（含 heartbeat）
│   │   └── delete_principal.act     ← append JSON → 删除 principal
│   └── _stream/
│       └── events.evt.jsonl         ← append-only 事件流
├── public/<app>/                    ← 共享 app 树（所有 principal 可见）
└── private/<principal>/<app>/       ← 私有 app 树（仅该 principal 可见）
```

**Agent 读文件 = 读状态读数据，Agent 写.act文件 = 写动作。** AppFS supervisor 监听 `.act` 文件的变更，消费动作、产生事件。Agent 通过读取 `events.evt.jsonl` 消费事件。

这种设计带来几个关键优势：

- **提示词管理**：能通过文件系统快速抽出当前环境信息进行注入

- **语言无关**：任何能读写文件的程序都能与 AppFS 交互，无需 SDK
- **可观测性**：`ls`、`cat`、`tail -f` 就是最好的调试工具
- **原子性**：每个 `.act` 文件是 append-only JSONL，天然支持多生产者
- **可组合**：不同 app 的文件树相互隔离，通过事件流通信，可以通过shell命令串联操作

底层存储使用 SQLite（via Turso），提供虚拟文件系统、KV 存储、工具调用审计三层存储模型。Agent 通过 mount（FUSE/NFS/WinFsp）挂载后，像操作本地文件一样操作远程状态。

---

## 2. 为什么需要系统软件（AppFS Runtime）：从单 Agent 到多 Agent

单个 Agent 只需要 Claude Code 那样的 REPL。但当我们需要**多个 Agent 协作、接入第三方系统、管理身份和权限**时，就需要一个系统软件层来管理以下问题：

```
┌─────────────────────────────────────────────────────────────┐
│                    AppFS Runtime Supervisor                  │
│                                                             │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐                 │
│  │Principal │  │  App      │  │ Runtime  │                 │
│  │Identity  │  │ Registry  │  │ Lifecycle│                 │
│  └──────────┘  └──────────┘  └──────────┘                 │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐    │
│  │              Connector Host                          │    │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐         │    │
│  │  │ Tinode   │  │ gRPC     │  │ HTTP     │         │    │
│  │  │ Connector│  │ Bridge   │  │ Bridge   │         │    │
│  │  └──────────┘  └──────────┘  └──────────┘         │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

AppFS supervisor 解决的核心问题：

| 问题 | AppFS 怎么做 |
|------|-------------|
| Agent 身份管理 | Principal registry + attach lease + heartbeat |
| 第三方软件接入 | Connector 抽象 + compose 配置 |
| Agent 间协作 | 通过 public/private 文件树 + 事件流 |
| 状态持久化 | SQLite-backed 文件系统 |
| 生命周期管理 | `.act` action 文件 + `events.evt.jsonl` 事件流 |

第三方软件（如 Tinode 聊天、火眼软件）通过实现 **Connector** 接入。Connector 负责：将外部系统的消息/事件转为 AppFS 文件树上的动作，反之亦然。

---

## 3. 统一输入层：一切输入皆信封

Agent 运行时面临四种输入源：

```
                    ┌─────────────────────┐
                    │    Input Router      │
                    │  (input_router.rs)   │
                    └────────┬────────────┘
                             │
        ┌────────────────────┼────────────────────┐
        │                    │                    │
   UserTerminal         AppfsEvent          System
   (用户终端输入)      (AppFS 事件流)     (系统指令)
   - 直接输入           - 外部消息到达      - 超时提醒
   - guidance           - 状态变更通知      - 优雅关闭
   - 队列化输入          - credential 就绪   - 健康检查
```

每种输入被封装为统一的 `InputEnvelope`：

```rust
struct InputEnvelope {
    source: InputSource,        // UserTerminal | AppfsEvent | AgentMessage | System
    input_type: String,         // user_prompt | appfs_event | system_shutdown ...
    text: String,               // 输入文本
    principal_id: Option<String>,
    app_id: Option<String>,
    stream_id: Option<String>,
    seq: Option<i64>,
    requires_attention: bool,   // 是否需要立即打断当前 turn
    // ...
}
```

投递策略有两种：

- **`InjectAtNextBoundary`** — 打断当前 turn，立即处理（如紧急通知）
- **`QueueAfterTurn`** — 排队等待当前 turn 完成后处理（如普通消息）

这意味着 AppFS 事件（第三方系统消息到达、状态变更等）和用户终端输入被**统一路由**到同一个对话循环。Agent 不需要关心输入来自哪里——它只看到 `InputEnvelope`。

**关键设计**：AppFS idle wake 机制。当 Agent 空闲时，它不会死等用户输入，而是定期扫描 `events.evt.jsonl`，检查是否有新事件需要处理。这使得 Agent 能被外部事件"唤醒"。

---

## 4. 会话持久化与映射层：Session 不是 API 的原始记录

Agent 的会话（session）以 JSONL 文件形式持久化在 `.claw/sessions/` 下，但文件里的内容**不是 Anthropic API 的原始 request/response**。它经过了一层映射，存储的是 Agent 运行时视角的结构化消息：

```
.claw/sessions/
├── <workspace-fingerprint>/
│   ├── <session-id>.jsonl        ← 运行时结构化消息流
│   └── <session-id>.meta.json    ← 元数据（model、provider、archived 等）
```

### 4.1 为什么需要映射？

直接把 Anthropic API 的消息存盘有几个问题：

1. **不同 provider 格式不同**：Anthropic 用 `content_block`，OpenAI 用 `tool_calls`。如果直接存原始格式，换 provider 就读不回来
2. **AppFS 事件不是 API 消息**：`message.received`、`action.completed` 这些事件需要以某种形式进入对话历史，但它们不是 `user` / `assistant` 消息
3. **压缩需要摘要**：对话太长时需要把旧消息总结成一段文字，这个"摘要消息"需要一个和普通消息不同的类型标记

### 4.2 Session 内部消息模型

Session 内部使用统一的 `ConversationMessage` 结构：

```rust
struct ConversationMessage {
    uuid: String,
    role: MessageRole,                         // User | Assistant | Tool | System
    blocks: Vec<ContentBlock>,                 // 消息内容的组成块
    usage: Option<TokenUsage>,
    is_compact_summary: bool,                  // 是否是压缩摘要
    compact_metadata: Option<CompactBoundaryMetadata>,
    attachment_metadata: Option<AttachmentMetadata>,
    hook_result_metadata: Option<HookResultMetadata>,
    timestamp_ms: u64,
}
```

其中 `ContentBlock` 是关键的统一抽象：

```rust
enum ContentBlock {
    Text { text },                             // 普通文本
    Thinking { thinking, signature },          // 思维链
    RedactedThinking { data },                 // 被隐藏的思维链
    InputRouter { inputs },                    // ← AppFS 事件的简化载体
    ToolUse { id, name, input },               // 工具调用
    ToolResult { tool_use_id, tool_name, output, is_error },  // 工具结果
}
```

### 4.3 AppFS 事件的简化映射

这是最关键的设计。AppFS 事件（来自 `events.evt.jsonl`）不是以原始 JSON 直接塞进对话历史，而是经过 `input_router.rs` 中的**渲染函数**转换成一段人类（和 LLM）可读的简短文本。

映射流程：

```
AppFS events.evt.jsonl 原始事件
    │  {"type":"action.completed", "content":{...大 JSON...}, "seq":42, ...}
    │
    ▼  input_router 消费事件 → 封装为 InputEnvelope
    │  source=AppfsEvent, input_type="action.completed",
    │  payload={...}, event_render_metadata={...}
    │
    ▼  渲染为 InputRouter ContentBlock
    │  ContentBlock::InputRouter { inputs: [...] }
    │
    ▼  发送给 API 前，render_input_router_block() 将其渲染为文本
    │  "<system-reminder>
    │   AppFS: credential ready for tinode:code-implementer
    │   AppFS: message received from user-alice in tinode:general"
    │  </system-reminder>"
    │
    ▼  API 实际收到的是 text content block，不包含原始事件结构
```

渲染策略按事件类型区分：

| 事件类型 | 渲染策略 |
|---------|---------|
| `message.received` | 渲染为独立的外部消息（不包裹 system-reminder） |
| `action.completed` | 简短一句话：`AppFS: <action> completed` |
| `action.failed` | 优先级最高，包含错误原因 |
| `profile.credentials.ready` | `AppFS: credential ready for <profile>` |
| `action.progress` | 简短进度提示 |
| `message.sent` | 本方消息发送确认 |
| platform 事件 | 统一用 `AppFS: <summary>` |
| 其他 AppFS 事件 | 按输入类型渲染简短摘要 |

还有一层**聚合**：同一 `correlation_id` 的多个事件（如 `action.accepted` + `action.completed` + `action.progress`）会被合并渲染，优先显示最重要的那个（failed > sent > ready > completed > progress > accepted）。

### 4.4 Compaction：上下文窗口的滑动摘要

当对话历史超过 context window 阈值时，运行时会自动触发 **compaction**：

```
┌──────────────────────────────────────────────────────────────┐
│ Session Messages                                             │
│                                                              │
│  [old user/assistant pairs]  ← compacted into summary text   │
│  [compact boundary marker]  ← metadata: trigger, token count │
│  [summary user message]     ← "Summary of conversation..."   │
│  [recent messages]          ← preserved verbatim (last N)    │
└──────────────────────────────────────────────────────────────┘
```

Compaction 的核心逻辑：
1. 保留最近 N 条消息不动（`preserve_recent_messages`）
2. 对之前的消息调用 LLM 生成结构化摘要（含 Primary Request、Files、Errors、Pending Tasks 等 9 个段落）
3. 如果已有旧摘要，**合并新旧摘要**而非覆盖
4. 保留 `compact_metadata`（触发原因、压缩前 token 数、保留的工具列表）

InputRouter 类型的事件在 compaction 时会被特殊处理——它们被识别为 `attachment`（附带信息），而不是核心对话内容，摘要时可以有不同的保留策略。

### 4.5 Session 发送给 API 前的最终组装

每次 model call 之前，运行时组装 `ApiRequest`：

```rust
let request = ApiRequest::conversation(
    self.system_prompt.clone(),     // 系统提示词（含 AppFS 环境信息）
    self.session.messages.clone(),  // 全部历史消息
);
tool_context.apply_to_api_request(&mut request);  // 可能覆盖 model/reasoning
```

`ApiRequest` 携带的消息列表就是 `ConversationMessage` 数组。API client 负责将它们转换为具体 provider 的格式（Anthropic 的 `input` / `content` blocks，或 OpenAI 的 `messages` format）。`ContentBlock::InputRouter` 在转换时被渲染为 `text` content block——LLM 看到的是可读文本，不是结构化事件 JSON。

### 4.6 持久化格式

Session JSONL 文件按行存储，每行是一条 `ConversationMessage` 的 JSON 序列化。读取时支持两种格式：
- **JSON 对象格式**：整个文件是一个 `{ "messages": [...] }` JSON（旧格式）
- **JSONL 格式**：每行一个独立 JSON 对象（当前格式）

每条消息记录了 `uuid`、`role`、`blocks`（含完整内容）、`timestamp_ms`、`usage` 等。Dashboard 通过 file watcher 监听这些文件，使用 **k-way 归并**算法将多个 session 的事件流合并为统一时间线。

---

## 5. 身份系统：Principal、Attach、私有 App 实例化

### 5.1 Principal — Agent 的身份

Principal 是 Agent 在 AppFS 世界中的身份标识，私有软件的认证也通过身份绑定。每个 Principal 有：

```json
{
  "principal_id": "code-implementer",
  "display_name": "Code Implementer",
  "kind": "agent",
  "active_attaches": [
    {
      "attach_id": "dashboard-code-implementer",
      "session_id": "session-abc123",
      "attached_at": "2026-06-05T10:00:00Z",
      "last_seen_at": "2026-06-05T10:01:30Z"  // 由 heartbeat 刷新
    }
  ],
  "agent_status": { "state": "idle" }
}
```

### 5.2 Attach Lease — 绑定关系

Agent 启动时，启动代码通过 `attach_principal.act` 向 AppFS 声明自己的身份。AppFS supervisor 会：

1. 检查该 principal 是否已存在（不存在则自动创建）
2. 检查是否有冲突的 active attach（同一 principal 不同 attach_id 默认被拒绝，除非 takeover）
3. 记录 attach lease（含 `last_seen_at` 时间戳）
4. 为该 principal 实例化私有 app

Attach 有心跳机制：Agent 每 30s 写 `update_principal.act` 刷新 `last_seen_at`。如果 90s 无心跳，AppFS 的 stale attach sweep 会自动清理该 attach。

```
Agent 启动 ──→ attach_principal.act ──→ AppFS 注册 attach
Agent 运行 ──→ update_principal.act ──→ 刷新 last_seen_at (heartbeat)
Agent 停止 ──→ detach_principal.act ──→ AppFS 注销 attach
Agent 崩溃 ──→ 90s 无 heartbeat  ──→ AppFS sweep 自动清理
```

### 5.3 私有 App 实例化

当 Principal 被创建时，AppFS supervisor 根据 app policy registry 中 `visibility: Private` 的 app，为该 principal 自动实例化私有 app：

```
app policy: tinode (visibility: Private)
                    ↓ principal "code-implementer" 被创建
实例化: instance_id = "tinode--code-implementer"
        path = "private/code-implementer/tinode"
        profile_id = "tinode:code-implementer"
```

每个私有 app 实例有独立的文件树、独立的 credential profile 认证配置。这使得同一套 compose 配置可以为不同 principal 生成完全隔离的第三方软件实例。

---

## 6. 第三方软件接入：Connector + Skills

### 6.1 Connector

Connector 是第三方软件与 AppFS 之间的桥梁。目前支持三种类型：

| Connector | 用途 | 协议 |
|-----------|------|------|
| inprocess Bridge（Tinode聊天软件为例） | rs程序内部集成 |  |
| gRPC Bridge | 通用 RPC 集成 | tonic (gRPC) |
| HTTP Bridge | REST API 集成 | HTTP |

Connector 负责：
- **方向一**：外部事件 → AppFS 文件树（如 Tinode 收到消息 → 写入 `_stream/events.evt.jsonl`）
- **方向二**：AppFS 动作 → 外部系统（如 Agent 写 `/_app/send_message.act` → Tinode 发送消息）

Connector 通过 compose YAML 配置：

```yaml
apps:
  - app_id: tinode
    visibility: private
    connector:
      kind: tinode
      endpoint: ${APPFS_TINODE_ENDPOINT}
      login_prefix: ${APPFS_TINODE_LOGIN_PREFIX}
    private_path_template: "private/{principal_id}/tinode"
```

### 6.2 Skills

Skills 是 Agent 的能力扩展。它们定义在 agent runtime 中，不是 AppFS 层面的概念。Skills 通过 slash commands 触发，可以：

- 查询第三方 API
- 执行特定业务逻辑
- 操作 AppFS 文件树

Skills 和 Connector 的关系：Connector 提供"管道"（数据如何流入流出），Skills 提供"语义"（Agent 如何理解和使用这些数据）。

---

## 7. Agent Runtime：Claude Code 的扩展

AppFS Agent Runtime 基于 Claude Code（`claw-code`）做了扩展，核心改动在以下几处：

### 7.1 AppFS Attach

Agent 启动时检测 AppFS 环境（通过 `APPFS_MOUNT_ROOT` env 或 `runtime.json` manifest），自动完成：

1. **发现** AppFS mount
2. **创建/附加** principal（写 `create_principal.act` + `attach_principal.act`）
3. **预热** 私有 app 的 credential（写 `ensure_credentials.act`，等待 ready/failed/timed_out）
4. **启动** heartbeat 线程（每 30s 写 `update_principal.act`）

### 7.2 事件消费与输入路由

Agent 的 `input_router.rs` 在每次 model call 之前，扫描 `events.evt.jsonl` 的新事件，将它们转化为 `InputEnvelope` 注入对话循环。

### 7.3 系统提示词增强

Agent 的 system prompt 中会自动注入 AppFS 环境信息：已注册的 app 列表、principal identity、mount 路径等。这让 LLM 能理解自己所在的环境，做出更准确的决策。

### 7.4 AppFS Idle Wake

`--appfs-idle-wake` 标志让 Agent 在空闲时不阻塞等待用户输入，而是定期检查 AppFS 事件流。这使 Agent 能被外部事件（第三方系统消息、状态变更等）唤醒。

---

## 8. Dashboard：可观测性层

Dashboard 是开发调试用的管理界面（未来会封装为面向客户的产品 UI）：

```
Dashboard Server (Fastify)          Dashboard Client (React)
├── Agent Registry                  ├── Agent Sidebar
│   └── 从 .claw/sessions/ 发现      │   └── 项目分组、状态、操作按钮
├── File Watcher                    ├── Timeline Panel
│   └── 监听 JSONL 变更              │   └── 单 Agent 列表 / 多 Agent 泳道
├── JSONL Parser                    ├── Message Bubble
│   └── k-way 归并时间线             │   └── assistant_delta 流式渲染
├── SSE Event Bus                   ├── Playground Panel
│   └── 实时推送 agent 状态           │   └── 向 headless agent 发送 prompt
└── Principal Lifecycle             └── Debug Dump Viewer
    └── CRUD + attach 管理              └── compaction archive 查看
```

Server 端的核心是 **Agent Registry**：它从三个路径发现 agent session（managed process、discovered JSONL、external process），并维护一个统一的 agent 列表。Principal Lifecycle Service 在此基础上提供完整的生命周期管理（create → start → stop → detach → delete → archive）。

---

## 9. 生命周期管理：从创建到归档

一个完整的 Agent 生命周期：

```
创建 Principal          POST /api/projects/:id/principals
    │                      → append create_principal.act
    ↓                      → AppFS 注册 principal + 实例化私有 app
启动 Agent             POST /api/projects/:id/principals/:pid/start
    │                      → processManager.spawn()
    │                      → Agent 进程启动 → attach_principal.act
    ↓                      → heartbeat 线程开始
运行中
    │                      → heartbeat 每 30s 刷新 last_seen_at
    │                      → input_router 消费事件流
    │                      → model turn loop 处理输入
    ↓
停止 Agent             POST /api/projects/:id/principals/:pid/stop
    │                      → 等待进程树终止
    │                      → 写 detach_principal.act → 等 AppFS 确认
    ↓
删除/归档             DELETE /api/projects/:id/principals/:pid
    │                      → 检查无 active attach
    │                      → 如有 stale attach: best-effort detach + force delete
    │                      → append delete_principal.act → 等 AppFS 确认
    │                      → archive sessions（标记 archived，不删文件）
    ↓
已归档
                           → UI 移入 "Archived agents" 区域
                           → resume 过滤跳过 archived agent
```

异常路径：
- **Agent 崩溃**：heartbeat 停止 → 90s 后 AppFS sweep 自动清理 stale attach → 可立即删除
- **Dashboard 重启**：从磁盘 JSONL 重新 discover session → managed process 需手动 resume

---

*本文基于 2026-06-05 的代码状态撰写。随着项目迭代，具体实现可能有所变化，但核心架构理念保持一致。*
