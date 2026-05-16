# Auto Mark-as-Read Requirements

Status: draft
Date: 2026-05-15
Scope: appfs-agent runtime, Tinode connector, AppFS event propagation

## Background

### Current Behavior

When `default` agent sends a Tinode message to `code-implementer`:

1. `default` appends to `contacts/send_message.act` with `"to": "principal:code-implementer"`.
2. Tinode connector processes the action, sends the message upstream, and emits `message.sent` + `action.completed` into `default`'s event stream.
3. Tinode server delivers the message to `code-implementer`'s Tinode account.
4. `code-implementer`'s Tinode connector polls/receives the inbound message, generates `message.received` + `inbox.updated` side events, and writes them to `code-implementer`'s event stream.
5. `code-implementer`'s idle wake scan detects `message.received` with `requires_attention=true` and wakes the agent.
6. The agent runs `run_event_turn()` and processes the message.

### Problem

After step 6, `code-implementer` has processed the message but:

- The message stays in the connector's local `unread_message_ids: HashSet<String>` and is never removed.
- No `{note, what="read"}` packet is sent to the Tinode server.
- `default` never sees a `message.read` event for the message it sent.
- `default` cannot determine whether `code-implementer` has processed or even seen the message.

### Root Cause

The original v0 design explicitly excluded read receipts:

- `TINODE-APPFS-v0-design.md`: "不在 v0 做完整 IM 产品能力，例如文件上传、已读回执..."
- `TINODE-APPFS-tree-v0-design.md`: "Do not support attachments, reactions, read receipts..."
- `2026-05-06-appfs-multi-agent-tinode.md` cut lines: "read receipts"

The `inbox/mark_read.act` action path and `submit_mark_read()` handler exist for local unread state bookkeeping, but:

1. **No automatic trigger**: The agent runtime never writes `inbox/mark_read.act` automatically after waking for a `message.received` event.
2. **No upstream notification**: `submit_mark_read()` only clears the local `unread_message_ids` HashSet and generates a local `message.read` side event. It does NOT send a `{note, what="read"}` packet to the Tinode WebSocket server.
3. **No cross-principal propagation**: Even if a local `message.read` event were generated, there is no mechanism to propagate the read receipt back to the sender's event stream.

## Requirements

### R1: Automatic mark-as-read on wake

When an agent wakes from idle due to a `message.received` event with `requires_attention=true`, the runtime should automatically write an `inbox/mark_read.act` action for the message's `message_id` before or during the event turn.

**Trigger point**: After `scan_appfs_attention_events_for_idle_wake()` returns attention-worthy events but before `run_event_turn()` executes.

**Scope**: Only `message.received` events that caused the idle wake (`WakeIfIdle` delivery mode). The agent should NOT auto-mark messages that were injected during an active turn (`InjectAtNextBoundary`) because the model may still be deciding whether to act on them.

**Action payload**:

```json
{
  "scope": "message",
  "message_ids": ["tinode:usrCodeImpl:42"],
  "client_token": "auto-mark-read-wake-<seq>"
}
```

### R2: Upstream read notification

When `submit_mark_read()` processes a `mark_read.act` action, the Tinode connector must:

1. Clear local `unread_message_ids` (already implemented).
2. For each cleared `message_id`, resolve the corresponding Tinode topic and seq.
3. Send a `{note}` packet to the Tinode WebSocket server for each topic with `what: "read"` and the appropriate `seq`.

**Tinode protocol**:

```json
{
  "note": {
    "topic": "usrCodeImpl",
    "what": "read",
    "seq": 42
  }
}
```

This is a fire-and-forget notification. The Tinode server does not respond with `{ctrl}` for `{note}` packets. The connector should send the note and not wait for a response.

**Error handling**: If the WebSocket connection is not available, log a warning and continue. Upstream read notification is best-effort and must not block the `mark_read.act` action from completing.

### R3: Cross-principal read receipt propagation

When the Tinode connector receives a `{kp}` (KP = key press / notification) packet with `what: "read"` from the server, the connector should:

1. Map the `topic` back to the local contact/profile.
2. Generate a `message.read` inbound event for the sender's event stream.
3. Include sufficient metadata for the sender to know which message was read.

**Event shape**:

```json
{
  "type": "message.read",
  "principal_id": "default",
  "profile_id": "tinode:default",
  "conversation_type": "direct",
  "contact_key": "code-implementer",
  "topic": "usrCodeImpl",
  "seq": 42,
  "message_id": "tinode:usrCodeImpl:42",
  "from_display_name": "code-implementer",
  "path": "contacts/code-implementer/messages.res.jsonl"
}
```

**Classification**: `message.read` should be classified as `Receipt` with `ContextOnly` delivery (no wake, inject as context if running).

### R4: Classification update for `message.read`

Add `message.read` to `classify_appfs_event()` in `appfs.rs`:

```rust
"message.read" => AppfsEventClassification {
    input_class: Receipt,
    running_delivery: ContextOnly,
    idle_delivery: ContextOnly,
},
```

This means `message.read` does not wake an idle agent but is visible as context if the agent is already running.

## Design Decisions

### DD1: Auto-mark timing

**Decision**: Auto-mark happens between wake detection and event turn execution.

**Alternatives considered**:

1. **After model response**: Would require waiting for the full model turn, which is slow. The read receipt should be sent as soon as the agent "sees" the message.
2. **Manual only**: Relies on the model deciding to call `mark_read.act`, which is unreliable for multi-agent coordination.
3. **At boundary injection**: Would mark messages even for running agents that haven't decided to act on them yet.

**Rationale**: The idle wake is the clearest signal that the agent has been activated by this specific message. Marking it as read at this point is semantically correct: the agent is about to process it.

### DD2: Scope limited to idle wake

**Decision**: Only messages that trigger `WakeIfIdle` are auto-marked. Messages injected via `InjectAtNextBoundary` during an active turn are NOT auto-marked.

**Rationale**: When an agent is already running, it may receive multiple `message.received` events at a boundary. Auto-marking all of them would be premature -- the model may decide to ignore or defer some. The model can still manually mark messages via `inbox/mark_read.act` when needed.

### DD3: Best-effort upstream notification

**Decision**: Sending `{note, what="read"}` to Tinode is best-effort. Failure does not block the `mark_read.act` action from completing.

**Rationale**: Read receipts are informational, not transactional. A dropped read receipt is acceptable. The local unread state is the authoritative state within AppFS.

### DD4: In-process connector required

**Decision**: The auto mark-read requires the Tinode connector to be running in-process (WebSocket session active). If the connector is out-of-process or not yet initialized, auto-mark is deferred to the first connector action.

**Rationale**: Sending `{note}` packets requires an active Tinode WebSocket connection. The in-process Tinode connector already maintains this session.

## Implementation Plan

### Step 1: Add upstream note sending to Tinode connector

**Files**: `appfs/sdk/rust/src/tinode_connector.rs`

1. Add a `send_read_note(topic: &str, seq: i64)` method to `TinodeSession` that sends `{note: {topic, what: "read", seq}}` via `send_packet()`.
2. In `submit_mark_read()`, after clearing local `unread_message_ids`, resolve each cleared message to its topic and seq, then call `send_read_note()`.
3. Parse `message_id` format `tinode:<topic>:<seq>` to extract topic and seq for the note.
4. Add unit tests for the note packet construction.

**Acceptance**: When `submit_mark_read` is called with message IDs, `{note}` packets are sent to the Tinode server with correct topic and seq.

### Step 2: Handle inbound `{kp}` read receipts

**Files**: `appfs/sdk/rust/src/tinode_connector.rs`

1. In the Tinode session message loop, detect `{kp}` packets where `what == "read"`.
2. Map `topic` to the local contact key.
3. Generate a `ConnectorInboundEvent` with `event_type: "message.read"`.
4. Include metadata: `topic`, `seq`, `message_id`, `from_display_name`, `contact_key`, `conversation_type`.

**Acceptance**: When another user reads a message in Tinode, the sender's Tinode connector generates a `message.read` inbound event.

### Step 3: Add `message.read` event classification

**Files**: `appfs-agent/rust/crates/runtime/src/appfs.rs`

1. Add `"message.read"` match arm to `classify_appfs_event()`:
   ```rust
   "message.read" => AppfsEventClassification {
       input_class: Receipt,
       running_delivery: ContextOnly,
       idle_delivery: ContextOnly,
   },
   ```
2. Add test coverage for the new event type.

**Acceptance**: `message.read` events are classified as Receipt/ContextOnly and do not trigger idle wake.

### Step 4: Add auto mark-read on idle wake

**Files**:
- `appfs-agent/rust/crates/runtime/src/appfs.rs`
- `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`

1. After `scan_appfs_attention_events_for_idle_wake()` returns pending inputs, extract all `message.received` events from the wake events.
2. Collect `message_id` values from those events.
3. For each app that has a `mark_read.act` action path, write the auto-mark payload.
4. Execute this before `run_event_turn()`.

**Implementation options**:

- **Option A (preferred)**: In `drive_appfs_idle_wake()`, after getting `pending_inputs` from `sync_appfs_events_for_idle()`, extract message IDs and write `inbox/mark_read.act` for the relevant app before calling `run_event_turn()`.
- **Option B**: Add a new function `auto_mark_read_for_wake_events()` in `appfs.rs` that takes the wake events and the AppFS environment, writes the mark-read actions, and returns.

**Payload per app**:

```json
{
  "scope": "message",
  "message_ids": ["tinode:usrCodeImpl:42", "tinode:usrCodeImpl:43"],
  "client_token": "auto-wake-<stream-seq>"
}
```

**Write mechanism**: Append to `inbox/mark_read.act` under the relevant private app root, e.g., `/private/code-implementer/tinode/inbox/mark_read.act`.

**Acceptance**: When `code-implementer` wakes due to `message.received` from `default`, the runtime auto-writes `inbox/mark_read.act` before the event turn starts.

### Step 5: Integration smoke test

**Files**: `integration/scripts/test-windows-appfs-tinode-multi-agent-smoke.ps1` (extend existing)

Extend the existing multi-agent Tinode smoke script to verify:

1. `default` sends message to `code-implementer`.
2. `code-implementer` wakes.
3. `default`'s event stream eventually contains a `message.read` event for the sent message.
4. The `message.read` event includes the correct `message_id`, `contact_key`, and `from_display_name`.

**Acceptance**: End-to-end flow: send -> wake -> auto-mark-read -> upstream note -> kp receipt -> `message.read` event in sender stream.

## Affected Code

| Layer | File | Change |
|-------|------|--------|
| Tinode connector | `appfs/sdk/rust/src/tinode_connector.rs` | Add `send_read_note()`, handle `{kp}` read receipts, update `submit_mark_read()` |
| Event classification | `appfs-agent/rust/crates/runtime/src/appfs.rs` | Add `message.read` to `classify_appfs_event()` |
| Idle wake flow | `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs` | Add auto mark-read before `run_event_turn()` |
| AppFS runtime | `appfs/cli/src/cmd/appfs/core.rs` | No change needed (side events already handled) |
| Integration | `integration/scripts/test-windows-appfs-tinode-multi-agent-smoke.ps1` | Add read receipt verification steps |

## Event Classification Summary (Updated)

| Event Type | Input Class | Running Delivery | Idle Delivery |
|------------|-------------|------------------|---------------|
| `message.received` + attention | Attention | InjectAtNextBoundary | WakeIfIdle |
| `message.received` (no attention) | Guidance | InjectAtNextBoundary | ContextOnly |
| `action.completed` | Receipt | ContextOnly | ContextOnly |
| `action.failed` | Receipt | InjectAtNextBoundary | ContextOnly |
| `message.sent` | Receipt | ContextOnly | ContextOnly |
| **`message.read`** (new) | **Receipt** | **ContextOnly** | **ContextOnly** |
| `profile.credentials.ready` | Status | ContextOnly | ContextOnly |
| `inbox.updated` | Noise | Drop | Drop |

## Flow Diagram

### Before (Current)

```text
default --send_message.act--> Tinode server
Tinode server --delivers--> code-implementer's Tinode account
code-implementer connector --message.received--> code-implementer event stream
code-implementer agent wakes, processes message
(NO mark-read action written)
(NO read note sent to Tinode server)
(NO message.read event in default's stream)
```

### After (With Auto Mark-Read)

```text
default --send_message.act--> Tinode server
Tinode server --delivers--> code-implementer's Tinode account
code-implementer connector --message.received--> code-implementer event stream
code-implementer agent wakes
  |
  v  [NEW] auto-write inbox/mark_read.act
  |
  v  [NEW] Tinode connector sends {note what="read"} to server
  |
  v  run_event_turn() processes the message
Tinode server --{kp what="read"}--> default's Tinode connector
default connector --message.read--> default's event stream
default sees "code-implementer read your message" (ContextOnly)
```

## Open Questions

1. **Should auto-mark-read also fire for `InjectAtNextBoundary` messages?**
   Current decision: No. Only `WakeIfIdle`. The model can manually mark-read during an active turn.

2. **Should `message.read` events be rendered in the model's system reminder?**
   Current decision: Yes, but only as `ContextOnly`. They appear as receipt-level context, similar to `action.completed`.

3. **Should the auto-mark-read be configurable per-principal or per-app?**
   Current decision: No configuration in the initial implementation. All principals auto-mark on wake.

4. **What happens if `mark_read.act` write fails?**
   Current decision: Log a warning and continue. The event turn should proceed even if mark-read fails.

5. **Should `message.read` events be emitted for group conversations?**
   Current decision: Yes, if Tinode sends `{kp what="read"}` for group topics. The connector should handle both direct and group read receipts uniformly.

## Non-Goals

- Do not implement typing indicators.
- Do not implement delivery receipts (`message.delivered`).
- Do not implement per-message read receipt tracking in the sender's model context.
- Do not change the `inbox/mark_read.act` payload schema.
- Do not remove the manual `mark_read.act` capability -- the model can still explicitly mark messages as read.
- Do not make `{note}` sending a blocking operation.
