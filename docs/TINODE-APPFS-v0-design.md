# Tinode AppFS v0 Design

## Status

This document is the high-level Tinode/AppFS integration design. The concrete,
model-facing tree contract now lives in
[`TINODE-APPFS-tree-v0-design.md`](./TINODE-APPFS-tree-v0-design.md) and should
be treated as the source of truth for paths and resource fields.

Implemented v0 vertical slice in `appfs-platform/main`.

Tinode should be implemented as a new app/connector named `tinode`. The current `aiim` demo app should stay unchanged and continue serving as a stable integration-test fixture.

Dependency:

1. [AppFS Multi-Agent Identity And App Visibility v0 Design](./APPFS-multi-agent-identity-and-app-visibility-v0-design.md)

Implemented scope:

1. compose can declare `tinode` as a private app policy;
2. AppFS materializes `/private/<principal-id>/tinode` per principal;
3. `profile_id = tinode:<principal-id>` flows through `ConnectorContext`;
4. Tinode credentials are stored in connector-private state, not AppFS files;
5. credentials can be created through `_app/ensure_credentials.act` or lazily on first business action;
6. direct principal-to-principal messages work through `contacts/send_message.act`;
7. inbound/direct messages are exposed through `_stream/events.evt.jsonl` and `inbox/*.res.jsonl` read-through;
8. appfs-agent lists `appfs-tinode` only for the current principal's private instance.

Current v0 is a bridgeable agent-chat foundation, not a complete Tinode client.

## Goal

把 Tinode 从一个手动验证过的聊天服务，收敛成可以被 AppFS 稳定桥接的真实聊天 app。

v0 追求一个可靠闭环：

1. 每个 `principal_id` 有独立 Tinode 身份。
2. agent 可以给用户发私聊消息。
3. 用户可以在 Tinode 客户端里看到 agent 发来的消息，也可以给 agent 回复。
4. 多个 principal 可以在同一个 AppFS project/root 下使用不同 Tinode 身份。
5. agent 可以创建群聊、邀请成员、发送群消息，用聊天软件承载多 agent 协作。
6. Tinode 动作结果和新消息通过 AppFS event stream 暴露给 appfs-agent。

不在 v0 做完整 IM 产品能力，例如文件上传、已读回执、推送、多端同步冲突处理、复杂权限 UI。

## Non-Goals

1. 不替换、不重命名、不改造当前 `aiim` demo。
2. 不把 Tinode 身份模型做成 Tinode connector 私有协议。
3. 不在本文件重复定义 Tinode contact/message tree；具体树以 `TINODE-APPFS-tree-v0-design.md` 为准。
4. 不使用 `team` app visibility。
5. 不依赖 `by-login` 作为 Tinode 路径层设计。

## Relationship To AppFS Identity

Tinode 是一个 private account-backed app。

Tinode app instance path:

```text
/private/<principal-id>/tinode
```

Tinode auth binding:

```text
profile_id = tinode:<principal_id>
tinode account/session key = profile_id
```

Important rules:

1. `attach_id` is per-run and should not own Tinode credentials.
2. `principal_id` is stable and owns Tinode credentials.
3. missing principal defaults to `default`.
4. a fork-created principal gets a new Tinode account by default.
5. sharing a Tinode account across agents requires explicitly sharing `principal_id` or `profile_id`.

Tinode connector private state should store:

1. `profile_id -> Tinode login`
2. `profile_id -> Tinode user id`
3. `profile_id -> Tinode token/password`
4. `profile_id + topic -> cursor`
5. `profile_id + client_token -> action result`

Visible app resources must not expose token/password/API key.

## Human Owner

用户自己的 Tinode 身份不应靠模型猜，也不应从自然语言里临时推断。

v0 建议通过 connector env 配置默认 human owner：

```yaml
connectors:
  tinode-http:
    command:
      env:
        APPFS_TINODE_ENDPOINT: http://101.34.216.193:6060
        APPFS_TINODE_API_KEY: AQEAAAABAAD_rAp4DJh05a1HAwFT3A6K
        APPFS_TINODE_OWNER_REF: basic:esp3j0
```

说明：

1. 当前 compose schema 对未知字段使用 `deny_unknown_fields`，所以 v0 先不要加 `apps.tinode.identity`。
2. 后续可以扩展 compose schema，把 `APPFS_TINODE_OWNER_REF` 收敛成正式字段。
3. owner 是默认联系人或默认通知对象，不代表所有权限都来自 owner。

## Protocol Findings

Tinode 的最小 connector 流程和 Web UI 行为一致：

1. 每个连接先走 WebSocket `/v0/channels?apikey=...`，发送 `{hi}`。
2. Basic 登录名会被索引成 `basic:<username>` tag。
3. 用户搜索通过 `fnd` topic 完成：先 `sub fnd`，再 `set fnd desc.public` 为查询语句，然后 `get fnd what=sub`。
4. 单聊 topic 对当前用户表现为对方的 `usr...` ID。订阅对方 `usr...` 后即可发消息。
5. 群聊通过 `sub new` 创建，返回真实 `grp...` topic。
6. 群成员邀请通过 `set <grp> sub.user=<usr...>`。
7. 消息通过 `pub <topic>` 发送，接收方收到 `{data}`。

这意味着 AppFS connector 不应该依赖 Tinode Web UI 的搜索/建群流程，而应该直接持有 Tinode 协议会话。

可以用下面的脚本反复验证协议闭环：

```powershell
cd C:\Users\esp3j\rep\appfs-platform
node integration/scripts/tinode-smoke.mjs --endpoint http://101.34.216.193:6060
```

脚本会创建两个临时账号，验证 `basic:` 搜索、单聊发送、群创建、成员邀请、群消息发送，然后默认清理临时账号和群。调试时可以加 `--keep --verbose` 保留现场。

一个实现细节：Tinode WebSocket JSON 包中的 `secret` 字段在服务端 Go 结构里是 `[]byte`，实际要传标准 base64 字符串；不要使用去掉 padding 的 base64url。

## App Policy

Tinode app definition:

```json
{
  "app_id": "tinode",
  "display_name": "Tinode",
  "connector": "tinode-http",
  "visibility": "private",
  "path_template": "private/{principal_id}/tinode",
  "profile_template": "tinode:{principal_id}"
}
```

Examples:

```text
/private/default/tinode
/private/incident-reporter/tinode
/private/code-reviewer/tinode
```

## Tree Design v0

The Tinode AppFS tree is defined in:

1. [Tinode AppFS Tree v0 Design](./TINODE-APPFS-tree-v0-design.md)

Key v0 decisions:

1. app root is `/private/<principal-id>/tinode`;
2. direct chats live under `contacts/<contact-key>/`;
3. group chats live under `groups/<group-key>/`;
4. `contacts/send_message.act` is the convenience action when the exact contact path is unknown;
5. canonical message resources use `messages.res.jsonl`;
6. action files use singular names such as `send_message.act`;
7. contact and group keys are connector-generated, filesystem-safe, and may use display names only when unambiguous.

Identity and auth binding remain:

```text
sender identity = current principal's Tinode profile
credential key = tinode:<principal-id>
app root = /private/<principal-id>/tinode
```

## Minimal Action Requirements

Whatever final tree is chosen, Tinode actions should follow these rules:

1. all `*.act` files are append-only JSONL;
2. actions under `/private/<principal-id>/tinode` execute as `tinode:<principal-id>`;
3. action payloads should not contain passwords or tokens;
4. action results should be observable through `_stream/events.evt.jsonl`;
5. connector should deduplicate by `client_token` where available.

Example payload shape, not final path shape:

```json
{
  "to": "张三",
  "text": "明天十点开会",
  "client_token": "msg-001"
}
```

## Resource Requirements

Tinode should expose safe identity resources.

Example:

```json
{
  "principal_id": "default",
  "profile_id": "tinode:default",
  "tinode_user_id": "usr...",
  "login": "appfs_default",
  "display_name": "Default agent",
  "owner_ref": "basic:esp3j0"
}
```

Do not expose:

1. Tinode password。
2. Tinode auth token。
3. API key。
4. connector 私有 session cookie。

## Event Contract v0

`_stream/events.evt.jsonl` 面向 appfs-agent 的会话提醒，应该尽量可读但保持结构化。

Events should include `principal_id` and `profile_id`.

Examples:

```json
{"type":"profile.credentials.ready","principal_id":"default","profile_id":"tinode:default","tinode_user_id":"usr...","login":"appfs_default"}
{"type":"message.sent","principal_id":"default","profile_id":"tinode:default","conversation_type":"direct","path":"/contacts/send_message.act","client_token":"msg-001","text_preview":"明天十点开会"}
{"type":"message.received","principal_id":"default","profile_id":"tinode:default","conversation_type":"direct","path":"contacts/张三/messages.res.jsonl","message_id":"tinode:usrZhangSan:42","from_display_name":"张三","text_preview":"收到","requires_attention":true}
{"type":"group.created","principal_id":"incident-reporter","profile_id":"tinode:incident-reporter","group_key":"事故同步群","title":"事故同步群","path":"groups/事故同步群","client_token":"grp-001"}
{"type":"action.failed","principal_id":"default","profile_id":"tinode:default","path":"/contacts/send_message.act","client_token":"msg-001","error":"recipient not found"}
```

事件被 appfs-agent 注入 `<system-reminder>` 后，模型应该能判断：

1. 哪个 app 收到了事件。
2. 哪个 principal 收到了事件。
3. 这条事件是 action 结果还是外部用户发来的消息。
4. 是否需要继续行动或只向用户汇报。

## Skill Requirements

`appfs-tinode` skill 应从当前 principal 的 private app root 生成。

skill 中应包含：

```markdown
## Current AppFS identity
- principal_id: default
- profile_id: tinode:default
- app root: /private/default/tinode

Use only this principal's Tinode app root unless the user explicitly asks to inspect another principal.
```

The skill may include current v0 path examples from
`TINODE-APPFS-tree-v0-design.md`, but should avoid inventing paths that are not
part of that contract.

## Implementation Plan

1. 保持 `aiim` demo 原样，继续作为 public integration fixture。
2. 先实现 AppFS 多 agent identity/public/private 基础层。
3. 扩展 `integration/scripts/tinode-smoke.mjs`，模拟两个 `principal_id` / `profile_id`，验证两个 Tinode agent 账号能分别给同一个 owner 发消息。
4. 在 HTTP bridge 中新增 `tinode` backend，不替换 `mock_aiim`。
5. v0 compose 通过 connector `command.env` 配置 Tinode endpoint、API key、owner ref。
6. 注册 `tinode` app，visibility 设为 `private`。
7. 实现 Tinode account ensure 行为，根据当前 principal/profile 创建或复用 Tinode agent 账号。
8. 按 [Tinode AppFS Tree v0 Design](./TINODE-APPFS-tree-v0-design.md) 实现 Tinode contact/message/group tree。
9. 把 Tinode inbound message 转换成 `message.received` event，让 appfs-agent 的 AppFS event reminder 在下一轮模型调用前注入。

## Open Decisions

1. Tinode 登录名派生规则：需要避免泄漏敏感 principal，也要稳定可复用。
2. recipient policy 默认值：建议 v0 允许搜索和发送，后续加 allowlist。
3. agent 启动后是否自动给 owner 发消息：建议作为 compose/env 开关，默认关闭；手测或 demo 可以打开。
