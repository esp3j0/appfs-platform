# AppFS Principal Agent Status Implementation Plan

> Related PRD: `docs/APPFS-principal-agent-status-PRD.md`

## Goal

Implement a runtime-owned agent status view for AppFS principals, enforce one live agent attach per principal, and make appfs-agent automatically publish status transitions during its lifecycle.

## Architecture

The AppFS runtime remains the source of truth for `_appfs/principals/*.res.json`. appfs-agent reports status by appending JSON lines to `_appfs/principals/update_principal.act`; the AppFS runtime validates the active `attach_id`, updates the principal record, and materializes the per-principal `.res.json` view.

The system keeps `active_attach_count` and `active_attaches` for compatibility, but enforces a single live attach for agent principals. If a second attach appears, the runtime rejects it unless the existing attach is stale or the request explicitly asks for takeover.

## Key Files

AppFS runtime:

```text
appfs/cli/src/cmd/appfs/registry.rs
appfs/cli/src/cmd/appfs/action_dispatcher.rs
appfs/cli/src/cmd/appfs/runtime_supervisor.rs
appfs/cli/src/cmd/appfs/supervisor_control.rs
appfs/cli/src/cmd/appfs/runtime_manifest.rs
appfs/cli/src/cmd/appfs/tests.rs
```

appfs-agent:

```text
appfs-agent/rust/crates/runtime/src/appfs.rs
appfs-agent/rust/crates/runtime/src/conversation.rs
appfs-agent/rust/crates/runtime/src/input_router.rs
appfs-agent/rust/crates/runtime/src/prompt.rs
appfs-agent/rust/crates/rusty-claude-cli/src/main.rs
```

Dashboard follow-up, if needed:

```text
dashboard/server/src/routes/principals.ts
dashboard/src/components/AppControlPanel.tsx
```

## Phase 1: Extend Principal Data Model

### Task 1.1 Add serializable status structs

Modify `appfs/cli/src/cmd/appfs/registry.rs`.

Add:

```rust
pub(crate) struct PrincipalAgentStatus {
    pub(crate) state: PrincipalAgentState,
    pub(crate) current_task_preview: Option<String>,
    pub(crate) current_task_source: Option<String>,
    pub(crate) turn_id: Option<String>,
    pub(crate) attach_id: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) updated_at: String,
    pub(crate) last_activity_at: Option<String>,
    pub(crate) last_outcome: Option<PrincipalAgentOutcome>,
}
```

Add enums:

```text
PrincipalAgentState: idle | running | stopping | error | stopped | unknown
PrincipalAgentOutcome: completed | cancelled | failed
```

Add `agent_status: Option<PrincipalAgentStatus>` to `PrincipalRecord`.

### Task 1.2 Add `presence`

Add a runtime-derived `presence` field to materialized principal views.

Recommended values:

```text
online | offline | stale
```

Implementation options:

1. Store `presence` in `PrincipalRecord`; or
2. Preferably derive it when writing `principals/<id>.res.json`.

Prefer option 2 so `presence` does not become a second source of truth.

### Task 1.3 Validate principal status

Extend `validate_principal_registry` in `registry.rs`:

1. `active_attach_count == active_attaches.len()`.
2. For new single-attach invariant, `active_attaches.len() <= 1`.
3. `agent_status.state` must be known.
4. `current_task_preview` length must be bounded.
5. `agent_status.attach_id`, when present, must match the active attach.

Add tests in `appfs/cli/src/cmd/appfs/tests.rs`.

## Phase 2: Extend Control Actions

### Task 2.1 Extend `UpdatePrincipalRequest`

Modify `appfs/cli/src/cmd/appfs/action_dispatcher.rs`.

Current request supports profile-like fields. Extend it with:

```rust
pub(super) attach_id: Option<String>,
pub(super) agent_status: Option<PrincipalAgentStatusPatch>,
```

Patch semantics:

1. Missing fields mean no change.
2. Explicit `null` clears nullable fields.
3. `attach_id` is required if `agent_status` is present.
4. Validate state/outcome enum values during parse.
5. Reject oversized previews.

Add parser tests:

```text
parse_update_principal_request_accepts_agent_status_patch
parse_update_principal_request_requires_attach_id_for_agent_status
parse_update_principal_request_rejects_oversized_task_preview
parse_update_principal_request_rejects_invalid_state
```

### Task 2.2 Handle status updates

Modify `handle_update_principal` in `appfs/cli/src/cmd/appfs/runtime_supervisor.rs`.

Rules:

1. Find the principal record.
2. If `agent_status` is present, validate `request.attach_id` equals the active attach ID.
3. If no active attach exists, reject with an action failure.
4. Apply patch.
5. Runtime sets `updated_at`.
6. Runtime sets `last_activity_at` for `running`, terminal outcomes, and non-idle state changes.
7. Write `principals.registry.json`.
8. Write `_appfs/principals/<principal-id>.res.json`.
9. Emit a platform event, for example `principal.status.updated`.

Add tests:

```text
update_principal_agent_status_requires_current_attach
update_principal_agent_status_rejects_stale_attach
update_principal_agent_status_clears_current_task_on_null
```

## Phase 3: Enforce One Live Attach Per Principal

### Task 3.1 Add takeover support to attach request

Modify `AttachPrincipalRequest` in `action_dispatcher.rs`:

```rust
pub(super) takeover: bool,
```

Default is `false`.

### Task 3.2 Attach conflict behavior

Modify `handle_attach_principal` in `runtime_supervisor.rs`.

Rules:

1. Same `attach_id`: refresh existing lease and update `last_seen_at`.
2. No active attach: create lease.
3. Different active non-stale attach and `takeover=false`: reject with `principal.attach_conflict`.
4. Different stale attach: replace old lease.
5. Different active attach and `takeover=true`: replace old lease and emit takeover event.

Define a stale threshold constant. Suggested starting point:

```text
APPFS_PRINCIPAL_ATTACH_STALE_AFTER_MS = 90000
```

The implementation can use a hard-coded constant first; later it can move into config.

### Task 3.3 Preserve compatibility

Keep output fields:

```text
active_attach_count
active_attaches
```

But enforce:

```text
active_attach_count is 0 or 1
active_attaches length is 0 or 1
```

Add tests:

```text
attach_principal_rejects_second_live_attach
attach_principal_refreshes_same_attach
attach_principal_replaces_stale_attach
attach_principal_takeover_replaces_live_attach
```

## Phase 4: Add appfs-agent Status Update Helpers

### Task 4.1 Add helper API

Modify `appfs-agent/rust/crates/runtime/src/appfs.rs`.

Add helper functions:

```rust
pub fn update_appfs_principal_agent_status(
    lease: &AppfsAttachLease,
    update: AppfsAgentStatusUpdate,
) -> Result<(), String>
```

Add structs:

```rust
pub struct AppfsAgentStatusUpdate {
    pub state: AppfsAgentState,
    pub current_task_preview: Option<Option<String>>,
    pub current_task_source: Option<Option<String>>,
    pub turn_id: Option<Option<String>>,
    pub model: Option<Option<String>>,
    pub last_outcome: Option<Option<AppfsAgentOutcome>>,
}
```

Use `Option<Option<T>>` for patch semantics:

```text
None = omit field
Some(None) = clear field
Some(Some(value)) = set field
```

The helper should append one JSON line to:

```text
_appfs/principals/update_principal.act
```

### Task 4.2 Add preview sanitizer

Add:

```rust
pub fn sanitize_appfs_task_preview(input: &str) -> Option<String>
```

Requirements:

1. Trim whitespace.
2. Collapse newlines/control characters to spaces.
3. Limit to 200-240 chars.
4. Return `None` for empty input.
5. Avoid including system reminder boilerplate.

Add tests in `appfs.rs`.

## Phase 5: Wire appfs-agent Lifecycle

### Task 5.1 Attach success status

In `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`, after successful `attach_appfs_principal`, call:

```text
state=idle
clear current_task_preview
clear current_task_source
last_outcome=null
```

### Task 5.2 Turn start status

Before calling `runtime.run_turn(...)`, publish:

```text
state=running
current_task_preview=<sanitized preview>
current_task_source=user | appfs_event | tinode | unknown
last_outcome=null
```

Known call sites to inspect:

```text
run_turn
run_turn_with_output
run_appfs_idle_wake_turn
headless JSON turn handling
slash skill dispatch paths that invoke run_turn
```

### Task 5.3 Turn completion status

After `runtime.run_turn(...)` returns:

1. If success and `summary.cancelled == false`: `state=idle`, clear task, `last_outcome=completed`.
2. If success and `summary.cancelled == true`: `state=idle`, clear task, `last_outcome=cancelled`.
3. If error: `state=error`, keep or clear task depending on desired UX, `last_outcome=failed`, include only a short sanitized error summary if implemented.

Use `finally`-style cleanup so status is updated even when the turn returns early.

### Task 5.4 Stop handling

When a stop request is received and before aborting the active turn:

```text
state=stopping
```

When the cancelled turn settles:

```text
state=idle
last_outcome=cancelled
clear current task
```

### Task 5.5 Shutdown status

Before `detach_appfs_principal(..., "process_exit")`:

```text
state=stopped
clear current task
```

Then detach as today.

## Phase 6: Add Heartbeat / Stale Safety

### Task 6.1 Reuse attach refresh or add lightweight status refresh

appfs-agent should refresh the active attach periodically while running. Preferred low-risk approach:

1. Re-append `attach_principal.act` with the same `attach_id`.
2. Runtime treats same attach as refresh and updates `last_seen_at`.

Suggested interval:

```text
30 seconds
```

### Task 6.2 Avoid noisy writes

Only status transitions should write full status updates. Heartbeat should refresh attach lease only.

## Phase 7: Prompt Guidance

Modify AppFS prompt generation in:

```text
appfs-agent/rust/crates/runtime/src/appfs.rs
appfs-agent/rust/crates/runtime/src/prompt.rs
```

Add concise guidance:

```text
When coordinating with other AppFS principals, you may read `_appfs/principals/<principal-id>.res.json` or `_appfs/principals.registry.json` to check whether an agent is online, idle, running, stale, or stopped. These files are maintained by AppFS runtime and should not be modified.
```

Do not instruct the model to read these files on every turn.

## Phase 8: Dashboard Compatibility

Dashboard can remain unchanged for v1 if it already reads `principals.registry.json` dynamically.

If the UI validates schemas or renders principal fields, update:

```text
dashboard/server/src/routes/principals.ts
dashboard/src/components/AppControlPanel.tsx
```

Expected UI behavior:

1. Unknown new fields should not break existing views.
2. Optional future display: show `presence`, `state`, and `current_task_preview`.

## Test Plan

AppFS CLI:

```powershell
cargo test --manifest-path appfs/cli/Cargo.toml parse_update_principal_request
cargo test --manifest-path appfs/cli/Cargo.toml attach_principal
cargo test --manifest-path appfs/cli/Cargo.toml principal
cargo check --manifest-path appfs/cli/Cargo.toml
```

appfs-agent runtime:

```powershell
cargo test --manifest-path appfs-agent/rust/Cargo.toml -p runtime appfs
cargo test --manifest-path appfs-agent/rust/Cargo.toml -p rusty-claude-cli appfs
cargo check --manifest-path appfs-agent/rust/Cargo.toml -p runtime -p rusty-claude-cli
```

Manual smoke:

1. Start AppFS desktop app.
2. Spawn `default`.
3. Verify `_appfs/principals/default.res.json` shows `presence=online`, `state=idle`.
4. Send a message.
5. Verify state changes to `running` during the turn.
6. Stop the turn.
7. Verify state returns to `idle` with `last_outcome=cancelled`.
8. Try spawning a second `default`.
9. Verify duplicate attach is rejected or requires explicit takeover.
10. Kill the process without detach.
11. Verify status eventually becomes `stale`.

## Rollout Notes

1. Keep all new fields optional for backward compatibility.
2. Make registry parser tolerate old records without `agent_status`.
3. Preserve `active_attach_count` and `active_attaches`.
4. Enforce single live attach only after tests cover dashboard resume/restart behavior.
5. If duplicate agent startup currently happens during dashboard resume, update dashboard first to resume/focus existing agents or request explicit takeover.

## Open Decisions

1. Exact stale threshold: recommended initial value is 90 seconds.
2. Whether `presence` should be stored or derived. Recommended: derived.
3. Whether duplicate attach should be rejected by default or dashboard should always use takeover during resume. Recommended: reject by default; takeover must be explicit.
4. Whether to add `_appfs/principals/status.res.json` in v1. Recommended: defer.
