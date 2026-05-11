# AppFS Agent Unified Input Router Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the current broad AppFS event watcher with a staged unified input routing foundation that preserves model-call boundary injection and enables safe future idle wake.

**Architecture:** First remove the unsafe broad wake path, then extract AppFS event reading/classification into testable runtime primitives. Boundary injection remains the only active delivery path until classification and cursor semantics are stable. Idle wake is reintroduced later using attention-only routing and separate wake cursors.

**Tech Stack:** Rust workspace under `appfs-agent/rust`, runtime crate, rusty-claude-cli crate, JSONL session persistence, AppFS event streams.

---

## Reference Documents

- Requirements: `docs/plans/2026-05-09-appfs-agent-event-boundary-and-idle-wake.md`
- Current runtime event sync: `appfs-agent/rust/crates/runtime/src/appfs.rs`
- Current turn loop: `appfs-agent/rust/crates/runtime/src/conversation.rs`
- Current session cursor persistence: `appfs-agent/rust/crates/runtime/src/session.rs`
- Current CLI watcher and REPL: `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`
- Current non-blocking input prototype: `appfs-agent/rust/crates/rusty-claude-cli/src/input.rs`

## Current Code Findings

### Boundary Injection Already Exists

`ConversationRuntime::run_turn()` calls `sync_appfs_events_before_model_call()` before every model call. That calls `sync_appfs_event_reminders()` and appends an `AttachmentKind::AppfsEvents` user attachment when new events exist.

This is the good path. Preserve it.

### Broad Watcher Is the Problem

The old CLI had two broad watcher paths:

- `--watch-appfs-events`, which switched the normal REPL into a watcher-oriented REPL loop;
- `appfs-events watch`, which ran a standalone watcher loop.

Both eventually call `drive_pending_appfs_events()` or equivalent. That function uses the same boundary-sync function and then runs a normal user turn with `APPFS_EVENT_LOOP_PROMPT`.

This is unsafe because it wakes for every event, including `action.completed`, `message.sent`, and `profile.credentials.ready`.

### Cursor State Is Single-Lane

`Session` currently has only `appfs_event_cursors`. This tracks model-facing injected events. It should not be reused for idle wake scans once idle wake is reintroduced.

### Event Reader and Renderer Are Coupled

`appfs.rs` currently combines:

- event stream discovery;
- JSONL parsing;
- cursor baseline logic;
- reminder rendering;
- session mutation.

This is workable but hard to reuse for a router. We need small internal primitives first.

## Implementation Strategy

Do this in small PRs. Do not jump directly to a new event loop.

Recommended PR order:

1. Stop the unsafe broad wake behavior.
2. Extract event classification primitives.
3. Route boundary injection through classification.
4. Add input envelope and delivery mode types.
5. Add pending input queue scaffolding.
6. Add attention-only idle wake with separate cursor.

The first three PRs are stabilization. PRs 4-6 are new architecture.

---

## PR 1: Remove or Disable Broad AppFS Watcher

**Goal:** Stop `--watch-appfs-events` / `appfs-events watch` from automatically waking the model on every event, while keeping existing boundary injection intact.

**Files:**

- Modify: `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`
- Modify: `appfs-agent/rust/README.md`
- Modify: `docs/APPFS-multi-agent-identity-and-app-visibility-v0-design.md`
- Test: `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`

### Step 1: Write Parser Regression Tests

Update tests currently expecting these interfaces:

- `parses_watch_appfs_events_flag_for_repl`
- `parses_appfs_events_watch_subcommand`

Choose one of two behaviors:

1. preferred: parsing `--watch-appfs-events` returns an error explaining that broad watcher is disabled;
2. acceptable: parsing succeeds but sets a mode that does not auto-run model turns.

Recommended for PR 1: return an explicit error.

Add tests:

```rust
#[test]
fn rejects_watch_appfs_events_until_router_idle_wake_is_ready() {
    let _guard = env_lock();
    let args = vec!["--watch-appfs-events".to_string()];
    let err = parse_args(&args).expect_err("broad watcher should be disabled");
    assert!(err.contains("disabled"));
}

#[test]
fn rejects_appfs_events_watch_until_router_idle_wake_is_ready() {
    let _guard = env_lock();
    let args = vec!["appfs-events".to_string(), "watch".to_string()];
    let err = parse_args(&args).expect_err("broad watcher should be disabled");
    assert!(err.contains("disabled"));
}
```

### Step 2: Run Tests and Verify Failure

Run:

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli watch_appfs -- --test-threads=1
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli appfs_events -- --test-threads=1
```

Expected: tests fail until parser behavior is changed.

### Step 3: Disable Parser Paths

In `parse_args()`:

- remove or reject `--watch-appfs-events`;
- remove or reject `appfs-events watch`;
- remove `watch_appfs_events` from `CliAction::Repl` if no longer needed.

If removing fields creates a large diff, it is acceptable in PR 1 to keep dead functions and only make parser reject the options. Remove dead code in PR 2 or PR 3.

### Step 4: Remove User-Facing Help Text

Remove or rewrite help text that says:

- `claw --watch-appfs-events`
- `claw appfs-events watch ...`

Replace with a note only if useful:

```text
The broad AppFS watcher is disabled because it wakes on every AppFS event. Model-call boundary injection remains enabled automatically; use `--appfs-idle-wake` for attention-only idle wake.
```

### Step 5: Update Docs

In `appfs-agent/rust/README.md` and `docs/APPFS-multi-agent-identity-and-app-visibility-v0-design.md`, remove instructions that recommend the current watcher.

Point to the frozen requirements doc for the new design.

### Step 6: Verify

Run:

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli appfs -- --test-threads=1
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime appfs -- --test-threads=1
cargo check --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli
git diff --check
```

**Rollback:** restore parser acceptance for the watcher flags. Boundary injection is untouched.

---

## PR 2: Extract AppFS Event Classification Primitives

**Goal:** Add event classification without changing runtime behavior.

**Files:**

- Modify: `appfs-agent/rust/crates/runtime/src/appfs.rs`
- Test: `appfs-agent/rust/crates/runtime/src/appfs.rs`

### Step 1: Add Internal Types

Add internal enums near `AppfsEventRecord`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppfsInputClass {
    Guidance,
    Task,
    Receipt,
    Attention,
    Status,
    Noise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppfsDeliveryMode {
    InjectAtNextBoundary,
    QueueAfterTurn,
    WakeIfIdle,
    ContextOnly,
    Drop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AppfsEventClassification {
    input_class: AppfsInputClass,
    running_delivery: AppfsDeliveryMode,
    idle_delivery: AppfsDeliveryMode,
}
```

Keep these private in PR 2.

### Step 2: Write Classification Tests

Add tests under `appfs::tests`:

```rust
#[test]
fn classifies_attention_message_received() {
    let event = appfs_event_record_for_test(
        "message.received",
        Some(json!({"requires_attention": true, "text_preview": "hello"})),
        None,
    );
    let class = classify_appfs_event(&event);
    assert_eq!(class.input_class, AppfsInputClass::Attention);
    assert_eq!(class.running_delivery, AppfsDeliveryMode::InjectAtNextBoundary);
    assert_eq!(class.idle_delivery, AppfsDeliveryMode::WakeIfIdle);
}
```

Also cover:

- `message.received` without attention;
- `action.completed`;
- `action.failed`;
- `message.sent`;
- `profile.credentials.ready`;
- `inbox.updated`;
- unknown event.

### Step 3: Implement Minimal Classifier

Implement:

```rust
fn classify_appfs_event(event: &AppfsEventRecord) -> AppfsEventClassification
```

Suggested defaults:

- `message.received` + `content.requires_attention=true`: `Attention`, running inject, idle wake.
- `message.received`: `Guidance`, running inject, idle context-only.
- `action.completed`: `Receipt`, running context-only, idle context-only.
- `action.failed`: `Receipt`, running inject, idle context-only.
- `message.sent`: `Receipt`, running context-only, idle context-only.
- `profile.credentials.ready`: `Status`, running context-only, idle context-only.
- `inbox.updated`: `Noise`, drop/drop for now.
- unknown: `Status`, context-only/context-only.

### Step 4: Verify No Behavior Change

Run:

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime appfs -- --test-threads=1
```

Expected: existing event injection tests still pass.

**Rollback:** remove classifier types and tests. No public behavior changed.

---

## PR 3: Route Boundary Injection Through Classification

**Goal:** Keep current boundary injection semantics but make it classification-aware.

**Files:**

- Modify: `appfs-agent/rust/crates/runtime/src/appfs.rs`
- Test: `appfs-agent/rust/crates/runtime/src/appfs.rs`

### Step 1: Add Boundary Selection Helper

Add:

```rust
fn should_inject_at_model_boundary(event: &AppfsEventRecord) -> bool {
    matches!(
        classify_appfs_event(event).running_delivery,
        AppfsDeliveryMode::InjectAtNextBoundary | AppfsDeliveryMode::ContextOnly
    )
}
```

### Step 2: Update `sync_appfs_event_reminders_with_outcome()`

When extending `new_events`, filter through `should_inject_at_model_boundary`.

Important: cursor update should still advance to max seq for visible streams, even if some events are dropped as noise. Otherwise noise events will be reread forever.

### Step 3: Add Tests

Add a test where a stream contains:

- `message.received requires_attention=true`;
- `inbox.updated`;
- `action.completed`.

Assert:

- reminder includes `message.received`;
- reminder includes `action.completed`;
- reminder does not include `inbox.updated` if classified as drop;
- cursor advances past all three.

### Step 4: Verify Same-Turn Receipts Still Work

Existing tests:

- `sync_appfs_event_reminders_baselines_then_injects_new_events`
- `sync_appfs_event_reminders_reports_new_event_count`
- `sync_appfs_event_reminders_filters_private_streams_by_principal`

Run:

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime sync_appfs_event_reminders -- --test-threads=1
```

**Rollback:** remove the boundary filter call; classifier remains harmless.

---

## PR 4: Add Input Envelope Scaffolding

**Goal:** Introduce a common input representation without changing CLI behavior.

**Files:**

- Modify: `appfs-agent/rust/crates/runtime/src/appfs.rs`
- Optional create: `appfs-agent/rust/crates/runtime/src/input_router.rs`
- Modify: `appfs-agent/rust/crates/runtime/src/lib.rs` if creating module
- Test: new module tests or `appfs.rs` tests

### Step 1: Decide Module Placement

Recommended: create `runtime/src/input_router.rs` only if the types grow beyond AppFS.

For this PR, a small private module is enough:

```rust
mod input_router;
```

Types:

```rust
pub enum InputSource {
    UserTerminal,
    AppFsEvent,
    AgentMessage,
    System,
}

pub struct InputEnvelope {
    pub id: String,
    pub source: InputSource,
    pub input_type: String,
    pub principal_id: Option<String>,
    pub app_id: Option<String>,
    pub stream_id: Option<String>,
    pub seq: Option<i64>,
    pub requires_attention: bool,
    pub payload: serde_json::Value,
}
```

Keep public surface minimal. Do not expose from runtime root until needed.

### Step 2: Convert AppFS Event to Envelope

Add helper:

```rust
fn appfs_event_to_input_envelope(event: &AppfsEventRecord) -> InputEnvelope
```

### Step 3: Tests

Test conversion preserves:

- event type;
- app id;
- seq;
- requires_attention from `content.requires_attention`;
- raw payload.

### Step 4: Verify

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime input_router appfs -- --test-threads=1
```

**Rollback:** remove scaffolding. No behavior changed.

---

## PR 5: Add Pending Input Queue Scaffolding

**Goal:** Represent guide/queue inputs for future active-turn user input and idle wake, without fully wiring non-blocking CLI yet.

**Files:**

- Modify: `appfs-agent/rust/crates/runtime/src/conversation.rs`
- Modify/create: `appfs-agent/rust/crates/runtime/src/input_router.rs`
- Test: `appfs-agent/rust/crates/runtime/src/conversation.rs` or input router tests

### Step 1: Add Queue Types

Suggested:

```rust
pub enum PendingInputDelivery {
    InjectAtNextBoundary,
    QueueAfterTurn,
}

pub struct PendingInput {
    pub envelope: InputEnvelope,
    pub delivery: PendingInputDelivery,
}
```

### Step 2: Add Runtime Queue Field

Add to `ConversationRuntime`:

```rust
pending_inputs: VecDeque<PendingInput>
```

### Step 3: Add Injection Before Model Call

Before AppFS sync or after AppFS sync, decide order.

Recommended order:

1. inject pending user guidance;
2. sync AppFS boundary events;
3. model call.

This ensures user typed corrections appear before external status events.

### Step 4: Tests

Use fake API client to verify:

- queued guidance is inserted before next model call;
- queue item is consumed once;
- queued-after-turn item is not injected during current turn.

This PR can keep CLI unchanged. Tests can call runtime methods directly.

### Step 5: Verify

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime conversation pending -- --test-threads=1
```

**Rollback:** remove queue field and tests.

---

## PR 6: Implement Attention-Only Idle Wake

**Goal:** Reintroduce idle wake safely: only attention-worthy events wake the agent, with separate wake cursor.

**Files:**

- Modify: `appfs-agent/rust/crates/runtime/src/appfs.rs`
- Modify: `appfs-agent/rust/crates/runtime/src/session.rs`
- Modify: `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`
- Modify: `appfs-agent/rust/crates/rusty-claude-cli/src/input.rs` if keeping non-blocking editor
- Test: runtime appfs tests, CLI parser tests

### Step 1: Add Wake Cursors

Add to `Session`:

```rust
pub appfs_wake_event_cursors: BTreeMap<String, i64>
```

Add methods:

```rust
pub fn appfs_wake_event_cursor(&self, stream_id: &str) -> Option<i64>
pub fn update_appfs_wake_event_cursors<I>(&mut self, updates: I) -> Result<(), SessionError>
```

Persist in session meta separately from `appfs_event_cursors`.

### Step 2: Add Idle Wake Scanner

In runtime AppFS module:

```rust
pub fn scan_appfs_attention_events_for_idle_wake(
    session: &mut Session,
    cwd: &Path,
) -> Result<AppfsIdleWakeScanOutcome, SessionError>
```

Behavior:

- read visible streams;
- compare against wake cursors;
- classify events;
- select only idle delivery `WakeIfIdle`;
- return selected events as `PendingInput` / `InputEnvelope`;
- do not append an AppFS-specific reminder directly to the session;
- advance wake cursor for scanned streams;
- advance model cursor for events handed to the unified input router, so the next model boundary does not collect the same event twice.

### Step 3: Tests

Cases:

- `message.received requires_attention=true` wakes once.
- Re-running scan does not wake again.
- `action.completed` does not wake.
- `message.sent` does not wake.
- `profile.credentials.ready` does not wake.
- private event for other principal does not wake.
- wake cursor and model cursor do not mask each other incorrectly.

### Step 4: Event Turn API

Avoid:

```rust
run_turn(APPFS_EVENT_LOOP_PROMPT)
```

Add a dedicated event turn method:

```rust
fn run_event_turn(&mut self) -> Result<(), Box<dyn std::error::Error>>
```

Final PR6/route-closure behavior:

- `scan_appfs_attention_events_for_idle_wake()` returns pending inputs for attention-worthy AppFS events;
- the normal REPL path enqueues those pending inputs into the prepared runtime before `run_event_turn()`;
- the running-input REPL path enqueues those pending inputs into `SharedPendingInputQueue`;
- `ConversationRuntime::sync_pending_inputs_before_model_call()` renders a single `AttachmentKind::InputRouter` reminder for both user guidance and AppFS events;
- the old `AttachmentKind::AppfsEvents` reminder path is retained only as compatibility/test scaffolding, not as the main delivery path.

### Step 5: CLI Flag

Do not reuse `--watch-appfs-events` unless semantics are changed clearly.

Recommended:

```powershell
claw --appfs-idle-wake
```

Parser tests:

- flag only allowed for REPL mode;
- flag starts normal REPL with idle wake;
- does not accept old `--watch-appfs-events`.

### Step 6: Manual Smoke

Use Tinode-only compose:

1. start AppFS;
2. start default agent with idle wake;
3. start `code-implementer` agent with idle wake;
4. default sends message to `code-implementer`;
5. code-implementer wakes exactly once;
6. code-implementer replies;
7. default wakes exactly once;
8. neither wakes from own `action.completed`.

**Rollback:** remove new CLI flag and idle scanner. Boundary injection remains.

---

## PR 7: Running Guidance Input And Router Closure

**Goal:** Support terminal input while a turn is running as guidance by default.

This is harder than AppFS idle wake because the current CLI runs a synchronous `cli.run_turn()` and does not read user input while the model/tool loop is active.

Decision after PR 6: split the input-layer work into a separate plan. PR 1-6 fixed the unsafe broad watcher and restored attention-only AppFS idle wake. PR 7 changes how terminal input is collected while a model/tool turn is running, so it is implemented behind explicit running-input/idle-wake paths instead of replacing the default synchronous REPL all at once.

PR 7 has now been split into its own implementation plan:

```text
docs/plans/2026-05-10-appfs-agent-pr7-running-guidance-input.md
```

Use that plan for implementation. This document remains the completed PR1-6 plan and historical rationale.

**Options:**

1. keep current synchronous input; document that user guidance while running is future work;
2. introduce a background input thread that appends `UserTerminal` envelopes;
3. design a new non-blocking terminal input layer to poll input and event queues from one loop.

Current state after the router closure work:

- `InputEnvelope`, `PendingInput`, and `SharedPendingInputQueue` are the shared routing model.
- User running guidance enters `SharedPendingInputQueue`.
- Attention-worthy AppFS idle wake events enter the same queue on the running-input path, or the runtime-local pending queue on the normal REPL path.
- `ConversationRuntime` drains local/external pending inputs before each model call and emits one `AttachmentKind::InputRouter` attachment.
- Full default-REPL replacement remains intentionally out of scope; keep the synchronous REPL path stable until the non-blocking terminal controller has more soak time.

---

## Cross-Cutting Tests

Always run before merging each PR:

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime appfs -- --test-threads=1
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli appfs -- --test-threads=1
cargo check --manifest-path appfs-agent\rust\Cargo.toml -p runtime
cargo check --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli
git diff --check
```

Run broader tests before final integration PR:

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime -- --test-threads=1
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli -- --test-threads=1
```

## Manual Verification Commands

Tinode-only smoke still starts with:

```powershell
cd C:\Users\esp3j\rep\appfs-platform

$env:APPFS_TINODE_ENDPOINT = "http://101.34.216.193:6060"
$env:APPFS_TINODE_LOGIN_PREFIX = "appfsmanual$(Get-Date -Format yyyyMMddHHmmss)"
$env:APPFS_TINODE_CREDENTIAL_POLICY = "auto-create"

cargo run --manifest-path appfs\cli\Cargo.toml --target-dir C:\tmp\appfs-local-target -- appfs compose up -f appfs\appfs-compose.tinode.local.yaml
```

After PR 1-5, do not expect idle wake. Boundary injection remains only during active turns.

After PR 6, use the new idle wake flag selected during implementation.

## Risks and Mitigations

### Risk: Removing Watcher Hurts Manual Testing

Mitigation: keep manual PowerShell `Get-Content ...events.evt.jsonl` and inbox checks. The current watcher is unsafe, so correctness wins over convenience.

### Risk: Classifier Drops Useful Events

Mitigation: PR 3 should initially be conservative. Only drop clear noise such as redundant `inbox.updated`. Keep unknown events as `ContextOnly`.

### Risk: Cursor Bugs Lose Events

Mitigation: add explicit tests for cursor advancement across injected, dropped, and wake-only events.

### Risk: Event Turn Pollutes Session

Mitigation: do not implement event turn until after classification and wake cursor exist. Prefer a dedicated runtime method over fake user prompt.

### Risk: Non-Blocking Terminal UI Becomes a Sinkhole

Mitigation: keep PR 7 separate. The first safe implementation can defer wake while user is typing.

## Recommended Next Action

Start with PR 1 only:

1. disable broad watcher CLI paths;
2. update docs;
3. verify normal REPL and boundary injection tests still pass.

This stabilizes the product immediately and creates a clean base for the router work.
