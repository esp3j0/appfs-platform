# AppFS Agent Unified Input Router and Turn Scheduler

Status: frozen for implementation
Date: 2026-05-09
Scope: appfs-agent runtime, CLI input, AppFS event handling, future multi-agent messaging

## Summary

The previous `--watch-appfs-events` design treated AppFS events as a side-channel that could directly wake the agent. That proved the concept, but the abstraction is too narrow and causes duplicated turns: an action receipt such as `action.completed` can be misread as a new task.

The better architecture is a unified **Agent Input Router + Turn Scheduler**.

All inputs should enter the same routing layer:

- user terminal input;
- AppFS app events;
- messages from other agents;
- user guidance delivered through an app;
- platform/system control events;
- future timers, webhooks, Feishu, GitHub, or other integrations.

The router classifies each input, applies configurable filters, and decides how it should be delivered to the model:

- inject at the next model-call boundary;
- queue as a later turn;
- wake the agent if idle;
- keep as context only;
- drop or log as noise.

This makes AppFS events first-class inputs without giving them a special, fragile event loop.

## Design Goal

The user should be able to interact with an agent in a natural way:

```text
Agent is running a task.
User sends another line.
Default behavior: treat it as guidance and inject it into the current task at the next safe model boundary.
```

Similarly:

```text
Agent is running a task.
Another agent sends a Tinode message with requires_attention=true.
Default behavior: inject it into the current task at the next safe model boundary.
```

If the agent is idle, attention-worthy input can wake it and start a new event-driven turn.

## Core Concepts

### Agent State

The scheduler should distinguish at least:

```text
Idle:
  no model turn or tool execution is currently running;
  the agent is waiting for user input or external attention input.

Running:
  the agent is inside a model/tool loop;
  new inputs can be accepted by the router but cannot interrupt unsafe execution points.
```

Future states may include:

- `Stopping`;
- `WaitingForPermission`;
- `Compacting`;
- `Paused`.

For v0, `Idle` and `Running` are enough.

### Input Source

Every inbound item should record its source:

```text
UserTerminal:
  typed by the local user in the CLI.

AppFsEvent:
  emitted by AppFS platform or an AppFS app event stream.

AgentMessage:
  delivered by another principal through Tinode or another app.

System:
  internal runtime control message.

Webhook / Timer / FutureExternal:
  reserved for future integrations.
```

### Input Envelope

Before routing, each input should be normalized into an `InputEnvelope`.

Suggested shape:

```json
{
  "id": "input-...",
  "source": "user_terminal | appfs_event | agent_message | system",
  "type": "user.guidance | user.task | message.received | action.completed | action.failed | ...",
  "principal_id": "default",
  "app_id": "tinode",
  "stream_id": "app:tinode--default",
  "seq": 12,
  "correlation_id": "req-...",
  "requires_attention": true,
  "priority": "normal",
  "created_at_ms": 1778300000000,
  "payload": {}
}
```

Not every field is required for every input. The important point is that user input and external event input share a common routing vocabulary.

## Delivery Modes

Inputs should not go straight to the model. The router should assign a delivery mode.

### `InjectAtNextBoundary`

Inject into the current model context before the next model call.

Use for:

- user guidance typed while the agent is running;
- another agent's attention message while the agent is running;
- action receipts needed for the current turn;
- app failures that should affect current reasoning.

This is the most important mode for active tasks.

### `QueueAfterTurn`

Hold until the current turn fully finishes, then process as a later turn.

Use for:

- explicit "do this after current task" user messages;
- lower-priority tasks;
- external events that should not disturb the current task.

### `WakeIfIdle`

If the agent is idle, wake it and start a controlled event-driven turn.

Use for:

- direct message received from another agent;
- user guidance delivered through an app;
- urgent app events with `requires_attention=true`.

If the agent is running, this should usually degrade to `InjectAtNextBoundary`.

### `ContextOnly`

Make available as context when a model call is already happening, but do not wake an idle agent.

Use for:

- `action.completed`;
- `message.sent`;
- `profile.credentials.ready`;
- non-critical progress/status events.

### `Drop`

Ignore or log without injecting into model context.

Use for:

- noisy cache/read-through events;
- repeated `inbox.updated` when a richer `message.received` event already exists;
- irrelevant events for another principal.

## Default Policy

The default policy should be simple and human-friendly.

### User Terminal Input

Default: `InjectAtNextBoundary`.

Rationale: most users wait for the model to finish before sending unrelated new instructions. If they type while the model is still working, they usually intend to correct, clarify, or guide the current task.

Future explicit override:

```text
/queue <message>
```

or:

```text
/after-turn <message>
```

These can force `QueueAfterTurn`.

### AppFS / Agent Messages

Default by event type:

| Input | Running Agent | Idle Agent |
| --- | --- | --- |
| `message.received` + `requires_attention=true` | `InjectAtNextBoundary` | `WakeIfIdle` |
| `message.received` without attention | `InjectAtNextBoundary` or `ContextOnly` | no wake |
| `action.completed` | `ContextOnly` | no wake |
| `action.failed` | `InjectAtNextBoundary` if correlated with active turn, otherwise `ContextOnly` | no wake by default |
| `message.sent` | `ContextOnly` | no wake |
| `profile.credentials.ready` | `ContextOnly` | no wake |
| `inbox.updated` | `Drop` or `ContextOnly` | no wake |

### Platform/System Events

Default:

- safety-critical events can be `InjectAtNextBoundary` or `WakeIfIdle`;
- routine status events should be `ContextOnly` or `Drop`.

## Filters

Routing should be configurable through layered filters.

### Visibility Filter

Reject inputs the current agent should not see.

Rules:

- current principal can see public app events;
- current principal can see private app events only for `private/<principal-id>/...`;
- another principal's private events must not be injected.

### Security Filter

Normalize and label untrusted inputs.

External messages should not silently become system instructions. They should remain source-labeled:

```text
Message from principal `code-implementer` via AppFS app `tinode`:
...
```

### Classification Filter

Map raw event/user input into semantic classes:

```text
guidance:
  intended to steer current behavior

task:
  new work item

receipt:
  result of an action

attention:
  external input that may require response

status:
  low-priority runtime state

noise:
  ignored or summarized
```

### Delivery Filter

Map source/type/class/state to delivery mode:

```text
(source=user_terminal, state=running) -> InjectAtNextBoundary
(type=message.received, requires_attention=true, state=idle) -> WakeIfIdle
(type=action.completed, state=idle) -> ContextOnly
```

### Deduplication Filter

Avoid injecting semantically duplicate events.

Example:

- `message.received` and `inbox.updated` for the same message should not both wake the model.
- `action.completed` should not wake the sender after the sender already saw the action result during boundary injection.

## Model-Call Boundary Injection

Boundary injection is the mechanism that runs before each model call.

It should:

1. collect pending inputs with delivery mode `InjectAtNextBoundary` or `ContextOnly`;
2. render them as source-labeled context attachments;
3. append them to the session before the model call;
4. advance the model-facing cursor only after successful attachment;
5. preserve current per-principal visibility guarantees.

This replaces the narrower phrase "current action result injection." It covers:

- current action receipts;
- failures;
- user guidance typed during the turn;
- other-agent messages received during the turn;
- app events that should redirect current reasoning.

### Example: User Guidance While Running

```text
User starts: "Refactor the Tinode connector."
Agent begins editing.
User types: "先别动认证模块，只改 event router."
Router classifies as user.guidance.
Scheduler injects at the next model-call boundary.
Model sees the guidance before continuing.
```

### Example: Other Agent Guidance While Running

```text
default agent is implementing.
code-implementer sends: "I already changed runtime_supervisor.rs."
Tinode emits message.received requires_attention=true.
Router classifies as attention guidance.
Scheduler injects at the next model-call boundary.
Model avoids conflicting edits.
```

## Idle Wake

Idle wake should only run when the agent is idle.

It should:

1. scan pending inputs;
2. filter to `WakeIfIdle`;
3. inject a source-labeled event reminder;
4. start exactly one event-driven turn;
5. avoid waking for receipts/status/noise;
6. not corrupt a partially typed user line.

Current `--watch-appfs-events` wakes on any new event and should be removed or replaced.

## Event-Driven Turn

The event-driven turn should not look like a normal user prompt.

Avoid:

```text
run_turn("New AppFS events were received...")
```

Prefer a dedicated entry point:

```text
run_event_turn()
```

or an internal message:

```text
<system-reminder>
External input requiring attention was received...
</system-reminder>
```

The model should understand:

- this is not a human's direct new command;
- it should inspect the injected input;
- it should decide whether to respond, act, or do nothing;
- it must not repeat completed actions unless asked.

## Queue Semantics

Queued messages are not ignored. They wait until the current turn is complete.

Potential future CLI:

```text
/queue after current turn, write tests for the event router
/guide stop editing auth code
```

For v0:

- user terminal input defaults to guidance;
- explicit queue commands can be added later;
- external events can carry `delivery_hint=queue` later.

## Cursor Model

The current session stores `appfs_event_cursors` for model-facing AppFS event injection.

The redesigned system should avoid a single cursor doing every job.

Recommended:

```text
model_input_cursors:
  tracks what has been injected into model context

wake_scan_cursors:
  tracks what idle wake has already considered

queue_cursors or queue ids:
  tracks inputs deferred until after current turn
```

For v0, `model_input_cursors` can continue using the existing `appfs_event_cursors`, but idle wake should not advance it unless the event is actually injected into model context.

## Configuration

The policy should be configurable, but defaults should be safe.

Possible config shape:

```toml
[appfs.input_router]
default_user_input = "guide"
idle_wake = true

[[appfs.input_router.rules]]
source = "appfs_event"
event_type = "message.received"
requires_attention = true
running = "inject_at_next_boundary"
idle = "wake_if_idle"

[[appfs.input_router.rules]]
source = "appfs_event"
event_type = "action.completed"
running = "context_only"
idle = "context_only"
```

Config is not required for the first implementation, but the code should be structured so these policies are not hardcoded everywhere.

## Non-Goals

- Do not make every AppFS event wake the model.
- Do not turn `*.evt.jsonl` into read-through resources.
- Do not remove app-side inbound polling/subscription; connectors still need to bring backend events into AppFS.
- Do not interrupt active tool execution unsafely.
- Do not hardcode Tinode-only routing in appfs-agent.
- Do not treat external agent messages as trusted system instructions.

## Implementation Plan

### Step 1. Remove the Current Broad Event Loop

- Remove or disable `--watch-appfs-events` automatic wake.
- Remove or hide `appfs-events watch` if it uses broad wake semantics.
- Update docs so users do not rely on the current watcher.
- Keep model-call boundary injection working.

### Step 2. Introduce InputEnvelope Types

- Add an internal representation for normalized inputs.
- Convert AppFS event records into envelopes.
- Later convert user terminal input into envelopes.

### Step 3. Add Event/Input Classification

- Implement source/type/classification logic.
- Add tests for:
  - `message.received requires_attention=true`;
  - `action.completed`;
  - `message.sent`;
  - `profile.credentials.ready`;
  - private event from another principal.

### Step 4. Refactor Boundary Injection

- Build existing AppFS event reminder injection on top of the router.
- Include `InjectAtNextBoundary` and `ContextOnly` inputs.
- Preserve existing same-turn action receipt behavior.

### Step 5. Add Queue Support

- Add an in-memory pending input queue for current process.
- Future persistence can be added after semantics stabilize.
- User input while running should enter the queue as guidance and be injected at boundary.

### Step 6. Redesign Idle Wake

- Add idle scanner that only selects `WakeIfIdle` inputs.
- Keep separate wake scan cursor.
- Run one event-driven turn using a dedicated event-turn API.

### Step 7. Reintroduce CLI

Possible names:

```text
--appfs-idle-wake
--agent-input-router
appfs-events idle-wake
```

Avoid reusing `--watch-appfs-events` unless it is clearly redefined.

## Acceptance Criteria

### Active Turn

- User input typed while agent is running is treated as guidance by default.
- Guidance is injected before the next model call.
- Other-agent `message.received requires_attention=true` is injected before the next model call.
- Current action `action.completed` is still visible to the model before it finishes the turn.

### Idle Agent

- Other-agent `message.received requires_attention=true` wakes the agent exactly once.
- Own `action.completed` does not wake the agent.
- Own `message.sent` does not wake the agent.
- `profile.credentials.ready` does not wake the agent.
- `inbox.updated` alone does not wake the agent.

### Safety

- Events from another principal's private app are not injected.
- External messages are source-labeled and not treated as system instructions.
- Wake does not corrupt partially typed terminal input.
- Cursors do not cause boundary injection to miss events consumed by idle scanning.

### Multi-Agent Tinode Smoke

1. Start AppFS with Tinode-only compose.
2. Start default agent.
3. Start `code-implementer` agent.
4. Agent attach auto-creates principal and warms private app credentials.
5. default sends Tinode message to `principal:code-implementer`.
6. code-implementer wakes once because `message.received requires_attention=true`.
7. code-implementer replies.
8. default wakes once.
9. Neither agent wakes from its own `action.completed`.

## Open Questions

1. Should queued inputs be persisted across process restart in v0?
2. Should user terminal guidance while running require an explicit UI affordance, or is default guidance enough?
3. Should there be a visible `/queue` command in the first implementation?
4. Should idle wake be enabled by default once the router is implemented?
5. Should app connectors provide richer event metadata such as `delivery_hint`, `attention_reason`, and `dedupe_key`?

## Recommendation

Adopt the unified input router model.

Immediate next step:

1. remove or disable the current broad `--watch-appfs-events` wake behavior;
2. keep boundary injection;
3. implement the event/input classifier;
4. only then reintroduce idle wake on top of attention filtering and separate cursors.

This gives us a clean path for multi-agent collaboration without making AppFS events a brittle side channel.
