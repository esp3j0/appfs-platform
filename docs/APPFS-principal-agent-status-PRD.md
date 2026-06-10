# APPFS Principal Agent Status PRD

## Background

`_appfs` is the platform control app exposed by the AppFS runtime. It owns global app and principal management rather than a business app's data plane.

Current control files include:

```text
_appfs/apps.registry.json
_appfs/app-policies.registry.json
_appfs/principals.registry.json
_appfs/principals/<principal-id>.res.json
_appfs/principals/create_principal.act
_appfs/principals/update_principal.act
_appfs/principals/attach_principal.act
_appfs/principals/detach_principal.act
```

The current principal model can represent identity and attach leases, but it cannot represent the execution status of the agent bound to that principal. For multi-agent collaboration, an agent needs a concise way to know whether another principal is online, idle, running, stale, or failed.

## Goals

1. Extend principal records with a runtime-owned agent status view.
2. Make one principal map to at most one live agent instance in a single AppFS runtime.
3. Let appfs-agent automatically update principal status during its lifecycle.
4. Let models read principal status files when they need coordination context.
5. Keep exposed status concise, safe, and free of credentials or full prompts.

## Non-Goals

1. Do not support multiple live agent instances for the same principal in this version.
2. Do not expose process IDs, API keys, full prompts, full tool arguments, connector credentials, or raw stack traces.
3. Do not require models to read principal status files on every turn.
4. Do not redesign `_appfs` directory layout.

## Product Semantics

The intended model is:

```text
principal = collaboration identity
agent instance = the current runtime process representing that identity
attach = a lease proving that the agent instance is currently online
agent_status = execution state of that principal's current agent instance
```

A principal may exist without an active agent. Once online, a principal may have at most one live agent attach.

`active_attach_count` remains for compatibility, but its valid live values become `0` or `1`.

## User Stories

1. As an agent, I can check whether another principal is online before sending a task.
2. As an agent, I can tell whether another principal is busy or idle before requesting work.
3. As a user, I do not want two `default` agents to produce conflicting states such as one idle and one running.
4. As the dashboard/runtime, I can reject duplicate live attaches unless the existing attach is stale or explicitly taken over.

## Data Contract

Extend:

```text
_appfs/principals/<principal-id>.res.json
_appfs/principals.registry.json
```

Example:

```json
{
  "principal_id": "coder",
  "display_name": "coder",
  "description": "AppFS principal created by appfs-agent attach.",
  "kind": "agent",
  "created_at": "2026-05-27T14:00:00Z",
  "updated_at": "2026-05-27T14:01:00Z",
  "active_attach_count": 1,
  "active_attaches": [
    {
      "attach_id": "attach-abc",
      "session_id": "session-abc",
      "attached_at": "2026-05-27T14:00:00Z",
      "last_seen_at": "2026-05-27T14:01:00Z"
    }
  ],
  "presence": "online",
  "agent_status": {
    "state": "running",
    "current_task_preview": "Tinode message from default: please review the last change...",
    "current_task_source": "tinode",
    "turn_id": "turn-abc",
    "attach_id": "attach-abc",
    "session_id": "session-abc",
    "model": "deepseek-v4-flash",
    "updated_at": "2026-05-27T14:01:00Z",
    "last_activity_at": "2026-05-27T14:01:00Z",
    "last_outcome": null
  }
}
```

### `presence`

`presence` is derived by the runtime:

```text
online | offline | stale
```

Recommended semantics:

1. `online`: one active attach exists and is not stale.
2. `offline`: no active attach exists.
3. `stale`: an active attach exists but its `last_seen_at` exceeds the configured stale threshold.

### `agent_status.state`

Allowed values:

```text
idle | running | stopping | error | stopped | unknown
```

Recommended semantics:

1. `idle`: agent is ready for new input.
2. `running`: agent is currently processing a turn.
3. `stopping`: a stop request has been issued but not fully settled.
4. `error`: the last turn failed and the agent has not yet returned to a clean idle state.
5. `stopped`: agent is intentionally shut down.
6. `unknown`: runtime cannot determine current execution state.

### `current_task_preview`

`current_task_preview` is a short, sanitized one-line summary. It must not be a raw system prompt or full user prompt.

Recommended sources:

1. User input: preview of the user message.
2. Tinode message: `Tinode message from <sender>: <body preview>`.
3. AppFS event: concise event summary rendered from the event envelope.

Requirements:

1. Limit to 200-240 characters.
2. Remove newlines/control characters.
3. Redact obvious secrets.
4. Clear when the agent returns to idle.

## Attach Rules

`attach_principal.act` should enforce one live attach per principal:

1. If no active attach exists, accept.
2. If the same `attach_id` already exists, treat as refresh/heartbeat.
3. If a different non-stale attach exists, reject with a `principal.attach_conflict` event.
4. If a different stale attach exists, allow takeover.
5. If the request includes explicit `takeover: true`, allow forced takeover and emit a takeover event.

Proposed request extension:

```json
{
  "principal_id": "coder",
  "attach_id": "attach-new",
  "session_id": "session-new",
  "role": "agent",
  "takeover": false
}
```

## Status Update Action

Extend:

```text
_appfs/principals/update_principal.act
```

Proposed request:

```json
{
  "principal_id": "coder",
  "attach_id": "attach-current",
  "agent_status": {
    "state": "running",
    "current_task_preview": "Tinode message from default: please review...",
    "current_task_source": "tinode",
    "turn_id": "turn-123"
  }
}
```

Rules:

1. `attach_id` is required when updating `agent_status`.
2. The request `attach_id` must match the current active attach for the principal.
3. Mismatched attach IDs must be rejected to prevent stale processes from overwriting current state.
4. Omitted fields are left unchanged.
5. `current_task_preview: null` clears the current task.
6. The runtime owns `updated_at` and `last_activity_at`.
7. Invalid enum values, oversized previews, and unsafe principal IDs are rejected.

## appfs-agent Lifecycle Updates

appfs-agent should automatically update status:

| Lifecycle Event | Status Update |
| --- | --- |
| Attach success | `state=idle`, clear task |
| User/AppFS input turn starts | `state=running`, set task preview |
| Turn completes | `state=idle`, clear task, `last_outcome=completed` |
| User stop requested | `state=stopping` |
| Stop settled | `state=idle`, clear task, `last_outcome=cancelled` |
| Turn fails | `state=error`, `last_outcome=failed`, optional short error summary |
| Graceful shutdown | `state=stopped` before detach |

For long-running turns, appfs-agent should refresh the active attach periodically so stale detection remains accurate.

## Model Guidance

The appfs-agent system prompt should include a short instruction:

```text
When coordinating with other AppFS principals, you may read `_appfs/principals/<principal-id>.res.json` or `_appfs/principals.registry.json` to check whether an agent is online, idle, running, stale, or stopped. These files are maintained by AppFS runtime and should not be modified.
```

This should be guidance, not an instruction to read status every turn.

## Security and Privacy

The status view must not contain:

1. Process IDs.
2. API keys, tokens, or credentials.
3. Full prompts.
4. Full tool call arguments.
5. Raw stack traces.
6. Connector credential material.
7. Sensitive absolute paths unless already part of public workspace context.

Any error status should expose only a short, sanitized summary.

## Acceptance Criteria

1. A principal cannot have two live non-stale agent attaches at the same time.
2. A stale old attach cannot overwrite the status of a newer active attach.
3. `_appfs/principals/<principal-id>.res.json` exposes `presence` and `agent_status`.
4. `update_principal.act` can update `agent_status` with attach validation.
5. appfs-agent sets `running` at turn start and returns to `idle` after normal completion.
6. appfs-agent marks stop/cancel and error outcomes accurately.
7. The current task preview is short, sanitized, and cleared after the turn.
8. Model-facing guidance explains how to read principal status without encouraging constant reads.
9. Existing consumers of `active_attach_count` and `active_attaches` remain compatible.

## Future Work

If agents frequently need a compact overview of all principals, add:

```text
_appfs/principals/status.res.json
```

This file should be a runtime-owned aggregate projection containing only:

```text
principal_id
display_name
presence
agent_state
current_task_preview
last_activity_at
model
```

This aggregate is not required for the first implementation.
