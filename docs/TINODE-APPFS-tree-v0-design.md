# Tinode AppFS Tree v0 Design

## Status

Design proposal for the first real Tinode AppFS app tree.

This document fixes the v0 path layout, resource files, action files, event contract, and skill expectations for the `tinode` private app. It depends on:

1. [Tinode AppFS v0 Design](./TINODE-APPFS-v0-design.md)
2. [AppFS Multi-Agent Identity And App Visibility v0 Design](./APPFS-multi-agent-identity-and-app-visibility-v0-design.md)

## Goals

1. Let the model send a direct message with one business action.
2. Let the model read recent direct and group messages through resource files.
3. Let inbound user messages reach appfs-agent through AppFS event reminders.
4. Support multiple AppFS principals, each with its own Tinode profile.
5. Support group creation, member invitation, and group messages for multi-agent collaboration.
6. Keep credentials, tokens, API keys, and cookies outside the model-visible AppFS tree.
7. Keep the tree stable enough for connector implementation and skill generation.

## Non-Goals

1. Do not implement a full IM client UI.
2. Do not model every Tinode protocol feature.
3. Do not support attachments, reactions, read receipts, push notification settings, or moderation in v0.
4. Do not expose raw Tinode tokens, refresh tokens, API keys, cookies, or passwords.
5. Do not use `by-login` as a path-level concept.
6. Do not use display names as the only stable identity.

## App Instance Root

Tinode is a private account-backed app:

```text
/private/<principal-id>/tinode
```

Examples:

```text
/private/default/tinode
/private/incident-reporter/tinode
/private/code-reviewer/tinode
```

Identity binding:

```text
principal_id = default
profile_id = tinode:default
app root = /private/default/tinode
```

All actions under one Tinode app root execute as that root's `profile_id`. The model should never put `profile_id` in action payloads as an authority claim. AppFS/connector context provides the effective `profile_id`.

## Naming Rules

Collection directories use plural names:

1. `contacts`
2. `groups`
3. `inbox`

Connector-private topic mappings may still exist internally, but `topics/`
is not part of the v0 model-facing tree.

Action names use singular verbs:

1. `send_message.act`
2. `create_group.act`
3. `invite_members.act`
4. `mark_read.act`

Resource streams use AppFS resource suffixes:

1. `*.res.json`
2. `*.res.jsonl`
3. `*.evt.jsonl`

Do not use plain `messages.jsonl` in v0. Use `messages.res.jsonl` so the resource is clearly model-readable AppFS data, not an action sink or internal connector file.

## Contact And Group Keys

Directory names under `contacts/<contact-key>` and `groups/<group-key>` are connector-generated stable keys.

Rules:

1. The key must be filesystem-safe on Windows, macOS, and Linux.
2. The key should be human-readable when possible.
3. The key may be a display name such as `张三` only when it is unique and safe.
4. If a display name is duplicated, append a stable suffix such as `张三--usrab12`.
5. If no good display name exists, use a stable technical key such as `usr_ab12cd` or `grp_ab12cd`.
6. The model should read `contacts/index.res.jsonl` or `groups/index.res.jsonl` before assuming a key exists.

This keeps paths friendly without making display names the source of truth.

## Generated App Metadata

The connector exposes three generated metadata files under `_app/`:

1. `_app/skill.res.json` - safe skill metadata used by appfs-agent skill discovery.
2. `_app/actions.res.json` - recommended action index for the model.
3. `_app/control.res.json` - control-plane action index for credentials and inbox recovery.

These are part of the v0 model-facing tree.

## Root Tree

Recommended v0 tree:

```text
/private/<principal-id>/tinode/
  _app/
    actions.res.json
    control.res.json
    skill.res.json
    self.res.json
    message_layers.res.json
    ensure_credentials.act
    forget_credentials.act
    refresh_structure.act
    refresh_inbox.act
  _stream/
    events.evt.jsonl
  contacts/
    index.res.jsonl
    send_message.act
    resolve.act
    <contact-key>/
      messages.res.jsonl
      send_message.act
  groups/
    index.res.jsonl
    create_group.act
    <group-key>/
      group.res.json
      messages.res.jsonl
      send_message.act
      invite_members.act
  inbox/
    recent.res.jsonl
    unread.res.jsonl
    mark_read.act
```

The v0 contract intentionally does not include a separate `topics/` tree. If an
implementation keeps internal topic mappings or search placeholders, they should
remain connector-private and not be treated as model-facing contract paths.

## Bootstrap Tree

`get_app_structure` during app instance materialization should return a safe skeleton without creating upstream Tinode credentials.

Initial skeleton:

```text
_app/actions.res.json
_app/control.res.json
_app/skill.res.json
_app/self.res.json
_app/message_layers.res.json
_app/ensure_credentials.act
_app/forget_credentials.act
_app/refresh_structure.act
_app/refresh_inbox.act
_stream/events.evt.jsonl
contacts/index.res.jsonl
contacts/send_message.act
contacts/resolve.act
groups/index.res.jsonl
groups/create_group.act
inbox/recent.res.jsonl
inbox/unread.res.jsonl
inbox/mark_read.act
```

Dynamic directories such as `contacts/张三/` and `groups/事故同步群/` appear after the connector learns or creates them. The connector should bump app structure revision or otherwise request/allow structure refresh after contact resolution, first message, group creation, or group invitation.

## `_app/self.res.json`

Safe account summary for the current principal's Tinode profile.

Example before credentials exist:

```json
{
  "app_id": "tinode",
  "principal_id": "default",
  "profile_id": "tinode:default",
  "credential_policy": "auto-create",
  "credential_status": "missing",
  "tinode_user_id": null,
  "login": null,
  "display_name": "Default agent",
  "owner_ref": "basic:esp3j0"
}
```

Example after credentials are ready:

```json
{
  "app_id": "tinode",
  "principal_id": "default",
  "profile_id": "tinode:default",
  "credential_policy": "auto-create",
  "credential_status": "ready",
  "tinode_user_id": "usrAbCdEf",
  "login": "appfs_default",
  "display_name": "Default agent",
  "owner_ref": "basic:esp3j0",
  "last_ready_at": "2026-05-06T08:00:00Z"
}
```

Never include tokens, refresh tokens, passwords, API keys, cookies, or raw Tinode secrets.

## `_app/ensure_credentials.act`

Optional pre-check action for Tinode because Tinode uses `credential_policy = auto-create`.

Normal model behavior should not call this before sending a message. The first credential-required business action should create or reuse credentials transparently.

Payload:

```json
{
  "client_token": "ensure-tinode-default"
}
```

Optional guardrail:

```json
{
  "expected_profile_id": "tinode:default",
  "client_token": "ensure-tinode-default"
}
```

If `expected_profile_id` is present, it must match connector context. It is not authority.

## `_app/forget_credentials.act`

Best-effort credential cleanup for the current principal's Tinode profile.

Use this when the principal is being deleted, when an operator wants to reset a Tinode profile, or when a test needs to clear shared connector state before re-running a smoke flow.

Payload:

```json
{
  "client_token": "forget-tinode-default"
}
```

The connector must remove the current profile's private credential record and forget any shared principal-to-profile mapping for that profile. It must not expose or require tokens, refresh tokens, or passwords.

## Contact Index

`contacts/index.res.jsonl` lists known contacts and stable paths.

Each line:

```json
{
  "contact_key": "张三",
  "tinode_user_id": "usrZhangSan",
  "basic_login": "zhangsan",
  "display_name": "张三"
}
```

Fields:

1. `contact_key`: canonical path key under `contacts/`.
2. `display_name`: human-facing name.
3. `tinode_user_id`: safe upstream user id.
4. `basic_login`: optional Tinode basic login without the `basic:` prefix.

In v0 the connector may keep richer contact metadata internally, but
`contacts/index.res.jsonl` only needs the stable summary above.

## Direct Message Paths

Canonical per-contact path:

```text
contacts/<contact-key>/messages.res.jsonl
contacts/<contact-key>/send_message.act
```

Example:

```text
contacts/张三/messages.res.jsonl
contacts/张三/send_message.act
```

The connector may expose `contacts/张三/` if `张三` is unique. If ambiguous, the path may be `contacts/张三--usrab12/`.

## `contacts/<contact-key>/messages.res.jsonl`

Direct conversation messages.

Each line:

```json
{
  "message_id": "tinode:usrZhangSan:42",
  "contact_key": "张三",
  "direction": "outbound",
  "text": "明天十点开会",
  "created_at_ms": 1778300000000,
  "seq": 42
}
```

The v0 archive row is intentionally minimal:

1. `message_id`
2. `contact_key`
3. `direction`
4. `text`
5. `created_at_ms`
6. `seq`

## `contacts/<contact-key>/send_message.act`

Send one direct message to a known contact.

Payload:

```json
{
  "text": "明天十点开会",
  "client_token": "msg-001"
}
```

Optional fields:

```json
{
  "text": "明天十点开会",
  "priority": "normal",
  "client_token": "msg-001",
  "metadata": {
    "source": "appfs-agent"
  }
}
```

Rules:

1. `text` is required.
2. `client_token` is strongly recommended for idempotency.
3. Credentials are auto-created on first use if missing.
4. The connector sends as the current app instance `profile_id`.
5. The action result must produce `action.completed` or `action.failed`.
6. A successful send should also emit `message.sent`.

## `contacts/send_message.act`

Convenience action for sending to a person when the exact contact path is not known.

Payload:

```json
{
  "to": "张三",
  "text": "明天十点开会",
  "client_token": "msg-quick-001"
}
```

More explicit payload:

```json
{
  "to": {
    "kind": "basic",
    "value": "zhangsan"
  },
  "text": "明天十点开会",
  "client_token": "msg-quick-001"
}
```

Allowed `to.kind` values:

1. `contact_key`
2. `basic`
3. `tinode_user_id`
4. `principal_id`
5. `search`

Behavior:

1. Resolve the recipient.
2. Create or update the contact entry if needed.
3. Send the message.
4. Emit `contact.resolved` when a new contact path is created.
5. Emit `message.sent` and standard action completion events.

This action is the best default when the user says "给张三说..." and the skill cannot confidently identify an existing contact path.

## `contacts/resolve.act`

Resolve a person without sending a message.

Payload:

```json
{
  "query": "张三",
  "client_token": "resolve-zhangsan"
}
```

Optional explicit search:

```json
{
  "query": "basic:zhangsan",
  "create_contact": true,
  "client_token": "resolve-zhangsan"
}
```

The connector should resolve the contact and emit `contact.resolved` or
`contact.resolve.failed` events. If an implementation keeps a transient search
buffer, it is connector-private and not part of the v0 model-facing tree.

`basic:<username>` may appear in payloads and results because it is a Tinode search/ref format. It should not be used as a path layer.

## Groups Index

`groups/index.res.jsonl` lists known groups.

Each line:

```json
{
  "group_key": "事故同步群",
  "title": "事故同步群",
  "topic_id": "grpIncident001",
  "path": "groups/事故同步群",
  "members": [
    {
      "kind": "self",
      "display_name": "Default agent",
      "profile_id": "tinode:default"
    },
    {
      "kind": "contact",
      "contact_key": "张三",
      "display_name": "张三",
      "tinode_user_id": "usrZhangSan"
    },
    {
      "kind": "principal",
      "principal_id": "incident-reporter",
      "profile_id": "tinode:incident-reporter",
      "tinode_user_id": "usrIncident"
    }
  ],
  "member_count": 3,
  "created_at_ms": 1778300000000,
  "last_message_at_ms": 1778300000000
}
```

If group titles duplicate, use a stable suffix such as `事故同步群--grpab12`.

## Group Paths

Canonical group paths:

```text
groups/<group-key>/group.res.json
groups/<group-key>/messages.res.jsonl
groups/<group-key>/send_message.act
groups/<group-key>/invite_members.act
```

## `groups/create_group.act`

Create a group and optionally send an initial message.

Payload:

```json
{
  "title": "事故同步群",
  "members": ["basic:zhangsan", "principal:incident-reporter"],
  "initial_message": "这个群用于同步事故 #1234",
  "client_token": "create-incident-group-001"
}
```

Allowed member refs:

1. `contact:<contact-key>`
2. `basic:<username>`
3. `tinode_user:<usr-id>`
4. `principal:<principal-id>`

Rules:

1. Current profile is the group creator.
2. `principal:<principal-id>` requires the target principal's Tinode profile to be ready.
3. If target principal credentials are missing, fail with a recoverable `PROFILE_NOT_READY` style error and a clear hint.
4. Do not silently create another principal's Tinode credentials from the current principal's action in v0.

On success, the connector creates `groups/<group-key>/` and emits `group.created`.

## `groups/<group-key>/group.res.json`

Safe group metadata:

```json
{
  "group_key": "事故同步群",
  "title": "事故同步群",
  "topic_id": "grpIncident001",
  "path": "groups/事故同步群",
  "members": [
    {
      "kind": "self",
      "display_name": "Default agent",
      "profile_id": "tinode:default"
    },
    {
      "kind": "contact",
      "contact_key": "张三",
      "display_name": "张三",
      "tinode_user_id": "usrZhangSan"
    },
    {
      "kind": "principal",
      "principal_id": "incident-reporter",
      "profile_id": "tinode:incident-reporter",
      "tinode_user_id": "usrIncident"
    }
  ],
  "member_count": 3,
  "created_at_ms": 1778300000000,
  "last_message_at_ms": 1778300000000
}
```

## `groups/<group-key>/send_message.act`

Send one group message.

Payload:

```json
{
  "text": "事故 #1234 已确认，开始同步处理。",
  "client_token": "grp-msg-001"
}
```

Rules are the same as direct `send_message.act`, except `conversation_type = group`.

## `groups/<group-key>/invite_members.act`

Invite members into an existing group.

Payload:

```json
{
  "members": ["basic:lisi", "principal:code-reviewer"],
  "client_token": "invite-001"
}
```

The same member ref rules from `groups/create_group.act` apply.

## Group Messages

`groups/<group-key>/messages.res.jsonl` uses the same message schema as direct messages, with:

```json
{
  "message_id": "tinode:grpIncident001:7",
  "conversation_type": "group",
  "group_key": "事故同步群",
  "topic_id": "grpIncident001",
  "direction": "inbound",
  "text": "收到，我来处理。",
  "created_at_ms": 1778300000000,
  "seq": 7
}
```

## Inbox

The inbox provides a task-oriented view across contacts and groups.

### `inbox/recent.res.jsonl`

Recent messages across all conversations:

```json
{
  "conversation_type": "direct",
  "contact_key": "张三",
  "message_id": "tinode:usrZhangSan:42",
  "text": "收到",
  "created_at_ms": 1778300000000,
  "from_tinode_user_id": "usrZhangSan",
  "from_display_name": "张三",
  "text_preview": "收到",
  "requires_attention": true,
  "unread": true
}
```

### `inbox/unread.res.jsonl`

Unread or not-yet-summarized inbound messages:

```json
{
  "conversation_type": "group",
  "contact_key": "事故同步群",
  "message_id": "tinode:grpIncident001:7",
  "from_display_name": "张三",
  "text": "收到，我来处理。",
  "created_at_ms": 1778300000000,
  "from_tinode_user_id": "usrZhangSan",
  "text_preview": "收到，我来处理。",
  "requires_attention": true,
  "unread": true
}
```

### `inbox/mark_read.act`

Mark messages as handled by the current principal.

Supported payloads:

Mark one or more message ids:

```json
{
  "scope": "message",
  "message_ids": ["tinode:grpIncident001:7"],
  "client_token": "mark-read-001"
}
```

Mark a whole direct conversation:

```json
{
  "scope": "thread",
  "contact_key": "张三",
  "client_token": "mark-read-zhangsan"
}
```

Mark a whole group conversation:

```json
{
  "scope": "thread",
  "group_key": "事故同步群",
  "client_token": "mark-read-incident-group"
}
```

Mark everything currently unread:

```json
{
  "scope": "all",
  "client_token": "mark-read-all"
}
```

Compatibility payloads are also accepted:

1. `{"all": true}` is equivalent to `{"scope": "all"}`.
2. `{"message_id": "tinode:usrZhangSan:42"}` is equivalent to `{"scope": "message", "message_ids": ["tinode:usrZhangSan:42"]}`.

`mark_read.act` only affects AppFS/connector-side unread state in v0. It does not need to map to Tinode read receipts.

The action result should include:

1. `cleared`: message ids removed from local unread state.
2. `scope`: the effective scope.
3. `unread_count`: remaining local unread count.

## Message Layers

Tinode uses three different layers for different jobs:

1. `_stream/events.evt.jsonl` is the wake and reminder boundary. It carries concise `message.received`, `message.sent`, `action.completed`, and `action.failed` signals for the current principal.
2. `inbox/recent.res.jsonl` and `inbox/unread.res.jsonl` are the operational summary layer. They are ideal for offline catch-up, attach-time summaries, and unread counts.
3. `contacts/<contact-key>/messages.res.jsonl` and `groups/<group-key>/messages.res.jsonl` are the full history/archive layer. Read them only when the model needs conversation context.

Practical rules:

1. If the agent is online and attached, prefer event reminders over rereading inbox resources every turn.
2. If the agent is offline or just attached, surface unread counts and recent inbox items first.
3. Use full message history only when the current task needs prior context or a specific thread recap.
4. `inbox.updated` is bookkeeping. It should not be treated as a standalone wake signal.
5. `message.received` with `requires_attention=true` is the signal that should wake the agent.
6. `delete_principal.act` may trigger `_app/forget_credentials.act` on affected private Tinode app instances so connector-private credential state is actually cleaned up, not merely requested.
7. The model should treat `inbox/` as a summary and triage layer, not as the primary live input stream.
8. The model should treat `messages.res.jsonl` as archive/history, not as a replacement for the event stream.

## Operational Usage

When you are actually using the tree:

1. If the agent is online and attached, let `_stream/events.evt.jsonl` drive the current turn.
2. If the agent has just attached or is catching up after being offline, read `inbox/recent.res.jsonl` and `inbox/unread.res.jsonl` first.
3. If the task needs the full conversation, read `contacts/<contact-key>/messages.res.jsonl` or `groups/<group-key>/messages.res.jsonl`.
4. Treat `refresh_inbox.act` as a recovery/debug action, not the normal daily path.
5. Treat `mark_read.act` as local AppFS unread-state bookkeeping in v0.

## Event Contract

Tinode emits domain events into:

```text
_stream/events.evt.jsonl
```

Events must include `principal_id` and `profile_id`.

### Credential Events

```json
{"type":"profile.credentials.ready","principal_id":"default","profile_id":"tinode:default","tinode_user_id":"usrAbCdEf","login":"appfs_default"}
{"type":"profile.credentials.failed","principal_id":"default","profile_id":"tinode:default","error":"upstream auth rejected"}
{"type":"profile.credentials.expired","principal_id":"default","profile_id":"tinode:default","recoverable":true}
```

### Contact Events

```json
{"type":"contact.resolved","principal_id":"default","profile_id":"tinode:default","contact_key":"张三","display_name":"张三","path":"contacts/张三"}
{"type":"contact.resolve.failed","principal_id":"default","profile_id":"tinode:default","query":"张三","error":"not found"}
```

### Message Events

```json
{"type":"message.sent","principal_id":"default","profile_id":"tinode:default","conversation_type":"direct","path":"contacts/张三/send_message.act","message_id":"tinode:usrZhangSan:42","to_display_name":"张三","text_preview":"明天十点开会","client_token":"msg-001"}
{"type":"message.received","principal_id":"default","profile_id":"tinode:default","conversation_type":"direct","path":"contacts/张三/messages.res.jsonl","message_id":"tinode:usrZhangSan:43","from_display_name":"张三","text_preview":"收到","requires_attention":true}
```

### Group Events

```json
{"type":"group.created","principal_id":"default","profile_id":"tinode:default","group_key":"事故同步群","title":"事故同步群","path":"groups/事故同步群","client_token":"create-incident-group-001"}
{"type":"group.member.invited","principal_id":"default","profile_id":"tinode:default","group_key":"事故同步群","member_ref":"basic:lisi","client_token":"invite-001"}
```

### Standard Action Events

The connector or AppFS runtime should also preserve standard action completion semantics:

```json
{"type":"action.completed","principal_id":"default","profile_id":"tinode:default","path":"/contacts/张三/send_message.act","request_id":"req-001","content":{"ok":true,"message_id":"tinode:usrZhangSan:42"}}
{"type":"action.failed","principal_id":"default","profile_id":"tinode:default","path":"/contacts/张三/send_message.act","request_id":"req-002","error":"recipient not found"}
```

`message.sent` is the human/domain event. `action.completed` is the action acknowledgement. Both are useful.

## Credential Behavior

Tinode has `credential_policy = auto-create`.

Credential creation must happen on credential-required actions, not passive tree bootstrap.

Credential-required actions:

1. `_app/ensure_credentials.act`
2. `contacts/send_message.act`
3. `contacts/<contact-key>/send_message.act`
4. `contacts/resolve.act`
5. `groups/create_group.act`
6. `groups/<group-key>/send_message.act`
7. `groups/<group-key>/invite_members.act`
8. `inbox/mark_read.act` if it needs upstream state

Passive reads should not create credentials in v0:

1. `get_app_structure`
2. skill discovery
3. `_app/self.res.json`
4. `contacts/index.res.jsonl`
5. `groups/index.res.jsonl`

If credentials are missing, passive reads should return safe local/cache data and include `credential_status = missing` where useful.

## Skill Contract

The generated `appfs-tinode` skill should teach the model these rules:

1. This app is a private Tinode chat app for the current principal.
2. Credentials are created automatically on first business action.
3. Do not write raw credentials into action files.
4. For a known contact, append one JSON line to `contacts/<contact-key>/send_message.act`.
5. If unsure which contact path to use, append one JSON line to `contacts/send_message.act` with `to` and `text`.
6. For live attention and wakeups, rely on AppFS event reminders from `_stream/events.evt.jsonl`.
7. If an AppFS reminder includes a `message.received` event with `requires_attention=true`, treat it as an active task to answer or act on in the current turn before asking for more context.
8. For unread summaries after attach or reconnect, read `inbox/unread.res.jsonl` or `inbox/recent.res.jsonl`.
9. For full conversation history, read `contacts/<contact-key>/messages.res.jsonl` or `groups/<group-key>/messages.res.jsonl`.
10. For group creation, append one JSON line to `groups/create_group.act`.
11. To clear local unread state, use `inbox/mark_read.act` with `scope = message`, `thread`, or `all`.
12. After any action, rely on AppFS event reminders or read `_stream/events.evt.jsonl` for debugging.

Skill example for known contact:

To send 张三 a message when `contacts/index.res.jsonl` shows `contact_key = 张三`:

```bash
printf '%s\n' '{"text":"明天十点开会","client_token":"msg-001"}' \
  >> contacts/张三/send_message.act
```

Skill example for unknown or ambiguous contact:

If you are not sure which contact path maps to 张三:

```bash
printf '%s\n' '{"to":"张三","text":"明天十点开会","client_token":"msg-001"}' \
  >> contacts/send_message.act
```

The skill should use the app root as its base directory, so examples should be relative paths.

## Model Usage Flows

### Send To Known Contact

1. User says: `给张三说明天十点开会`.
2. Model loads `appfs-tinode`.
3. Model sees `contacts/index.res.jsonl` maps `张三` to `contacts/张三`.
4. Model appends to `contacts/张三/send_message.act`.
5. Connector auto-creates credentials if missing.
6. Connector sends message and emits `message.sent` plus `action.completed`.
7. appfs-agent injects event reminder.
8. Model replies that the message was sent.

### Send To Unknown Contact

1. User says: `给李四说事故更新了`.
2. Model loads `appfs-tinode`.
3. Model cannot confidently find 李四 in `contacts/index.res.jsonl`.
4. Model appends to `contacts/send_message.act` with `to = 李四`.
5. Connector resolves, creates contact path if found, sends message.
6. Connector emits `contact.resolved`, `message.sent`, and `action.completed`.
7. Model replies with the resolved recipient and send result.

### Receive Message

1. User sends a message to the agent from Tinode.
2. Connector receives or polls Tinode data.
3. Connector appends/updates `contacts/<contact-key>/messages.res.jsonl` or `groups/<group-key>/messages.res.jsonl` as the archive.
4. Connector appends summary state to `inbox/recent.res.jsonl` and `inbox/unread.res.jsonl`.
5. Connector emits `message.received` and `inbox.updated`.
6. appfs-agent injects the event reminder before the next model call when the agent is online.
7. If the agent was offline, the next attach can first show unread inbox summary and then read full history only if needed.
8. Model can answer, continue a workflow, or ask the user what to do.

### Create Multi-Agent Group

1. User says: `创建一个事故同步群，拉张三和 incident-reporter agent 进去`.
2. Model loads `appfs-tinode`.
3. Model ensures `incident-reporter` has a ready Tinode profile or asks/fails with a clear reason if not.
4. Model appends to `groups/create_group.act`.
5. Connector creates group, invites members, and sends optional initial message.
6. Connector creates `groups/<group-key>/`.
7. Connector emits `group.created` and invitation events.

## Error Semantics

Common app-level errors:

1. `RECIPIENT_NOT_FOUND`
2. `RECIPIENT_AMBIGUOUS`
3. `PROFILE_NOT_READY`
4. `GROUP_NOT_FOUND`
5. `AUTH_EXPIRED`
6. `UPSTREAM_UNAVAILABLE`
7. `RATE_LIMITED`

Error event example:

```json
{
  "type": "action.failed",
  "principal_id": "default",
  "profile_id": "tinode:default",
  "path": "/contacts/send_message.act",
  "error_code": "RECIPIENT_AMBIGUOUS",
  "error": "Multiple contacts match 张三",
  "hint": "Read contacts/index.res.jsonl again or retry with contacts/send_message.act and an explicit to field"
}
```

## Implementation Notes

1. The connector should keep Tinode credentials in connector private state keyed by `profile_id`.
2. The connector should keep idempotency records keyed by `profile_id + client_token`.
3. The connector should maintain per-topic cursors keyed by `profile_id + topic_id`.
4. The connector should expose safe resources only.
5. Dynamic contact/group directories require app structure revision updates.
6. `contacts/send_message.act` can be implemented first, then per-contact dirs can be added once structure refresh is solid.
7. `inbox/unread.res.jsonl` can initially be a derived view from recent inbound events.
8. Connector resolves `principal:<principal-id>` member refs through an AppFS-provided app instance registry/resolver. In v0 this may be implemented from an `apps.registry.json` snapshot: find the private app instance with `principal_id` matching the ref and `app_id = tinode`, take that instance's `profile_id`, then look up the target's `tinode_user_id` from Tinode connector private state. The connector must not infer another principal's profile by string concatenation alone.
9. The Tinode connector credential store should be shared by `profile_id` across Tinode app instances in the same AppFS runtime, so resolving `principal:<principal-id>` can see already-ready profiles such as `tinode:incident-reporter`.
10. If the target principal has no Tinode instance, no `profile_id`, or no ready Tinode user id, group creation or invitation should fail with `PROFILE_NOT_READY` and an actionable hint. The current principal's action must not silently create another principal's Tinode credentials in v0.

## v0 Minimum Slice

The minimum useful implementation is:

1. `_app/actions.res.json`
2. `_app/control.res.json`
3. `_app/skill.res.json`
4. `_app/self.res.json`
5. `_app/ensure_credentials.act`
6. `_app/forget_credentials.act`
7. `_app/message_layers.res.json`
8. `_app/refresh_structure.act`
9. `_app/refresh_inbox.act`
10. `_stream/events.evt.jsonl`
11. `contacts/index.res.jsonl`
12. `contacts/send_message.act`
13. `contacts/<contact-key>/messages.res.jsonl`
14. `contacts/<contact-key>/send_message.act`
15. `inbox/recent.res.jsonl`
16. `inbox/unread.res.jsonl`
17. `inbox/mark_read.act`
18. `groups/index.res.jsonl`
19. `groups/create_group.act`
20. `groups/<group-key>/group.res.json`
21. `groups/<group-key>/messages.res.jsonl`
22. `groups/<group-key>/send_message.act`
23. `groups/<group-key>/invite_members.act`

If implementation pressure is high, the connector can keep extra internal indexes for search or topic recovery, but they should not expand the v0 model-facing tree.

## Acceptance Checklist

### Direct Message

- [ ] `get_app_structure` returns the safe skeleton without creating credentials.
- [ ] `contacts/send_message.act` creates Tinode credentials on first use.
- [ ] `contacts/send_message.act` resolves a recipient and sends a message.
- [ ] Connector stores credentials under `profile_id`.
- [ ] Connector emits `profile.credentials.ready`, `message.sent`, and `action.completed`.
- [ ] The recipient sees the message in Tinode.

### Inbound Message

- [ ] A human user's Tinode message becomes `message.received`.
- [ ] The message is visible in `inbox/unread.res.jsonl`.
- [ ] appfs-agent receives the event reminder only for the matching principal's Tinode app.

### Multi-Agent

- [ ] Two principals get different `profile_id` values.
- [ ] Two principals get separate Tinode accounts.
- [ ] One principal does not receive the other principal's private Tinode events.
- [ ] A group can include a human contact and a ready agent principal.

### Safety

- [ ] No token, refresh token, API key, password, or cookie appears in AppFS resources or events.
- [ ] Duplicate contact display names produce distinct `contact_key` values.
- [ ] Unknown or ambiguous recipients fail with actionable hints.
