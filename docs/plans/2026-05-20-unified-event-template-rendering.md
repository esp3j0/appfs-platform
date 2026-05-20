# Unified Event Template Rendering Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Unify AppFS event rendering so model-facing reminders and terminal wake cards both use the same safe template engine, while keeping separate rendering targets and safety rules.

**Architecture:** AppFS events remain stored as rich structured `InputEnvelope` / `InputRouter` data. Rendering becomes target-aware: `model_render` produces LLM-safe text with no ANSI/control escapes, while `terminal_render` produces CLI-only output and may use a small allowlist of ANSI tokens. Runtime policy continues to own wake/delivery/reply safety decisions; templates only control presentation.

**Tech Stack:** Rust, appfs-agent runtime `input_router`, rusty-claude-cli terminal UI, dashboard App Control overrides, AppFS `_app/events.res.json` metadata.

---

## Current State

AppFS event ingestion already has the right data shape:

- `appfs-agent/rust/crates/runtime/src/appfs.rs` reads app `_app/events.res.json` plus `.claw/appfs-event-render-overrides.json`.
- `appfs-agent/rust/crates/runtime/src/appfs.rs` attaches matched `event_render_metadata` to each `InputEnvelope`.
- `appfs-agent/rust/crates/runtime/src/input_router.rs` persists rich event data into session `InputRouter` blocks.
- `appfs-agent/rust/crates/runtime/src/input_router.rs` renders model-visible text through `render_input_router_block`.
- `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs` separately renders terminal wake cards through `appfs_event_card_lines`.

The gap is that model rendering already supports app-provided templates, but terminal wake cards still have hardcoded extraction/layout rules, especially for `message.received`.

## Design Principles

- Preserve raw event/session/debug data.
- Keep model rendering and terminal rendering as separate targets.
- Never allow ANSI/control escape tokens in model-visible output.
- Allow a small ANSI token allowlist only for terminal rendering.
- Prefer one canonical metadata key: `terminal_render`.
- Read legacy `ui_render` / `user_render` only as compatibility aliases.
- Keep safety and reply semantics in runtime policy, not free-form templates.
- Keep old hardcoded CLI card rendering as fallback when no terminal template exists.

## Metadata Schema

Preferred schema:

```json
{
  "class": "external_message",
  "wake": true,
  "running_delivery": "inject_at_next_boundary",
  "idle_delivery": "wake_if_idle",
  "model_render": {
    "mode": "body_with_source_reminder",
    "body_template": "{{message.body}}",
    "source_template": "来源：{{app.display_name}}，from={{message.sender}}，contact_key={{content.contact_key}}，seq={{seq}}"
  },
  "terminal_render": {
    "mode": "card",
    "lines": [
      "{{ansi.cyan}}{{app.display_name}} · from {{message.sender}}{{ansi.reset}}",
      "{{message.body}}"
    ]
  }
}
```

Compatibility aliases:

- `ui_render` may be read if `terminal_render` is absent.
- `user_render` may be read if both `terminal_render` and `ui_render` are absent.
- New descriptors and dashboard overrides should write `terminal_render`.

## Target Rules

### Model Target

Allowed variables:

- `{{type}}`
- `{{path}}`
- `{{seq}}`
- `{{principal_id}}`
- `{{app_id}}`
- `{{stream_id}}`
- `{{app.display_name}}`
- `{{content.field}}`
- `{{error.field}}`
- `{{payload.field}}`
- `{{message.sender}}`
- `{{message.body}}`

Disallowed variables:

- `{{ansi.*}}` renders as an empty string.
- Raw escape/control output must be sanitized.

### Terminal Target

Allowed variables:

- Everything in Model Target.
- ANSI allowlist:
  - `{{ansi.bold}}` -> `\x1b[1m`
  - `{{ansi.dim}}` -> `\x1b[2m`
  - `{{ansi.italic}}` -> `\x1b[3m`
  - `{{ansi.underline}}` -> `\x1b[4m`
  - `{{ansi.reset}}` -> `\x1b[0m`
  - `{{ansi.cyan}}` -> `\x1b[36m`
  - `{{ansi.green}}` -> `\x1b[32m`
  - `{{ansi.yellow}}` -> `\x1b[33m`
  - `{{ansi.blue}}` -> `\x1b[34m`
  - `{{ansi.magenta}}` -> `\x1b[35m`
  - `{{ansi.red}}` -> `\x1b[31m`
  - `{{ansi.gray}}` -> `\x1b[90m`

Message helper fallbacks:

- `{{message.sender}}`: `content.from_display_name` -> `content.from_principal` -> `content.contact_key` -> `payload.from_display_name` -> `payload.from_principal` -> `payload.contact_key` -> `unknown`
- `{{message.body}}`: `content.text` -> `content.text_preview` -> `payload.text` -> `payload.text_preview` -> `envelope.text`

## Non-Goals

- Do not introduce a full template language.
- Do not add loops or conditionals.
- Do not let app descriptors create system/developer instructions.
- Do not remove CLI fallback rendering in this phase.
- Do not change the AppFS event JSONL schema.

---

## Task 1: Add Target-Aware Template Rendering

**Files:**

- Modify: `appfs-agent/rust/crates/runtime/src/input_router.rs`
- Modify: `appfs-agent/rust/crates/runtime/src/lib.rs`

**Step 1: Write failing tests for target-specific ANSI behavior**

Add tests near the existing `input_router` tests:

```rust
#[test]
fn event_template_omits_ansi_for_model_target() {
    let mut envelope = InputEnvelope::new(InputSource::AppfsEvent, "message.received", "fallback body");
    envelope.app_id = Some("tinode".to_string());
    envelope.payload = Some(json!({
        "from_display_name": "AppFS Agent default",
        "text_preview": "hello"
    }));

    let rendered = render_event_template_for_target(
        &envelope,
        "{{ansi.cyan}}{{message.sender}}{{ansi.reset}}: {{message.body}}",
        EventTemplateTarget::Model,
    );

    assert_eq!(rendered, "AppFS Agent default: hello");
}

#[test]
fn event_template_allows_ansi_for_terminal_target() {
    let mut envelope = InputEnvelope::new(InputSource::AppfsEvent, "message.received", "fallback body");
    envelope.app_id = Some("tinode".to_string());
    envelope.payload = Some(json!({
        "from_display_name": "AppFS Agent default",
        "text_preview": "hello"
    }));

    let rendered = render_event_template_for_target(
        &envelope,
        "{{ansi.cyan}}{{message.sender}}{{ansi.reset}}: {{message.body}}",
        EventTemplateTarget::Terminal,
    );

    assert_eq!(rendered, "\x1b[36mAppFS Agent default\x1b[0m: hello");
}
```

**Step 2: Run tests and verify they fail**

Run:

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime input_router -- --test-threads=1
```

Expected:

- FAIL because `EventTemplateTarget` and `render_event_template_for_target` do not exist.

**Step 3: Implement target enum and public renderer**

Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventTemplateTarget {
    Model,
    Terminal,
}

#[must_use]
pub fn render_event_template_for_target(
    envelope: &InputEnvelope,
    template: &str,
    target: EventTemplateTarget,
) -> String {
    render_event_template_inner(envelope, template, target)
}
```

Refactor existing private `render_event_template` into:

```rust
fn render_event_template(envelope: &InputEnvelope, template: &str) -> String {
    render_event_template_inner(envelope, template, EventTemplateTarget::Model)
}
```

**Step 4: Add target-aware variable resolution**

Change `template_value` to accept target:

```rust
fn template_value(
    envelope: &InputEnvelope,
    variable: &str,
    target: EventTemplateTarget,
) -> Option<String>
```

Add:

```rust
fn ansi_template_value(variable: &str, target: EventTemplateTarget) -> Option<String> {
    if target != EventTemplateTarget::Terminal {
        return Some(String::new());
    }
    let value = match variable {
        "ansi.bold" => "\x1b[1m",
        "ansi.dim" => "\x1b[2m",
        "ansi.italic" => "\x1b[3m",
        "ansi.underline" => "\x1b[4m",
        "ansi.reset" => "\x1b[0m",
        "ansi.cyan" => "\x1b[36m",
        "ansi.green" => "\x1b[32m",
        "ansi.yellow" => "\x1b[33m",
        "ansi.blue" => "\x1b[34m",
        "ansi.magenta" => "\x1b[35m",
        "ansi.red" => "\x1b[31m",
        "ansi.gray" => "\x1b[90m",
        _ => return None,
    };
    Some(value.to_string())
}
```

Add message helpers:

```rust
fn template_message_value(envelope: &InputEnvelope, field: &str) -> Option<String> {
    match field {
        "sender" => payload_str(envelope, "from_display_name")
            .or_else(|| payload_str(envelope, "from_principal"))
            .or_else(|| payload_str(envelope, "contact_key"))
            .map(ToOwned::to_owned)
            .or_else(|| Some("unknown".to_string())),
        "body" => payload_str(envelope, "text")
            .or_else(|| payload_str(envelope, "text_preview"))
            .map(ToOwned::to_owned)
            .or_else(|| Some(envelope.text.trim().to_string()))
            .filter(|value| !value.is_empty()),
        _ => None,
    }
}
```

**Step 5: Export new symbols**

In `appfs-agent/rust/crates/runtime/src/lib.rs`, export:

```rust
pub use input_router::{
    render_event_template_for_target, EventTemplateTarget,
    // existing exports...
};
```

**Step 6: Run tests**

Run:

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime input_router -- --test-threads=1
```

Expected:

- PASS.

**Step 7: Commit**

```powershell
git add appfs-agent\rust\crates\runtime\src\input_router.rs appfs-agent\rust\crates\runtime\src\lib.rs
git commit -m "Add target-aware AppFS event template renderer"
```

---

## Task 2: Use `terminal_render` for CLI Wake Cards

**Files:**

- Modify: `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`

**Step 1: Write failing CLI tests**

Add tests near existing CLI tests or create a focused test module around `appfs_event_card_lines`:

```rust
#[test]
fn appfs_event_card_lines_uses_terminal_render_lines() {
    let mut envelope = runtime::InputEnvelope::new(
        runtime::InputSource::AppfsEvent,
        "message.received",
        "fallback body",
    );
    envelope.app_id = Some("tinode".to_string());
    envelope.payload = Some(json!({
        "from_display_name": "AppFS Agent default",
        "text_preview": "hello"
    }));
    envelope.event_render_metadata = Some(json!({
        "terminal_render": {
            "mode": "card",
            "lines": [
                "{{ansi.cyan}}{{app.display_name}} from {{message.sender}}{{ansi.reset}}",
                "{{message.body}}"
            ]
        }
    }));

    let lines = appfs_event_card_lines(&envelope);

    assert_eq!(lines[0], "\x1b[36mTinode from AppFS Agent default\x1b[0m");
    assert_eq!(lines[1], "hello");
}
```

**Step 2: Run test and verify it fails**

Run:

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli appfs_event -- --test-threads=1
```

Expected:

- FAIL because CLI does not read `terminal_render`.

**Step 3: Add terminal render lookup**

Add helper:

```rust
fn event_terminal_render(envelope: &runtime::InputEnvelope) -> Option<&serde_json::Value> {
    let metadata = envelope.event_render_metadata.as_ref()?;
    metadata
        .get("terminal_render")
        .or_else(|| metadata.get("ui_render"))
        .or_else(|| metadata.get("user_render"))
}
```

Add helper:

```rust
fn appfs_event_card_lines_from_terminal_render(
    envelope: &runtime::InputEnvelope,
) -> Option<Vec<String>> {
    let render = event_terminal_render(envelope)?;
    let mode = render
        .get("mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("card");
    if matches!(mode, "hidden" | "drop" | "debug_only") {
        return Some(Vec::new());
    }

    if let Some(lines) = render.get("lines").and_then(serde_json::Value::as_array) {
        let rendered = lines
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(|template| {
                runtime::render_event_template_for_target(
                    envelope,
                    template,
                    runtime::EventTemplateTarget::Terminal,
                )
            })
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        return Some(rendered);
    }

    let template = render.get("template").and_then(serde_json::Value::as_str)?;
    let rendered = runtime::render_event_template_for_target(
        envelope,
        template,
        runtime::EventTemplateTarget::Terminal,
    );
    Some(rendered.lines().map(ToOwned::to_owned).collect())
}
```

**Step 4: Wire fallback into `appfs_event_card_lines`**

At the top of `appfs_event_card_lines`:

```rust
if let Some(lines) = appfs_event_card_lines_from_terminal_render(envelope) {
    return lines;
}
```

Keep existing hardcoded logic below as fallback.

**Step 5: Run CLI tests**

Run:

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli appfs_event -- --test-threads=1
```

Expected:

- PASS.

**Step 6: Run runtime tests**

Run:

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime input_router -- --test-threads=1
```

Expected:

- PASS.

**Step 7: Commit**

```powershell
git add appfs-agent\rust\crates\rusty-claude-cli\src\main.rs
git commit -m "Render AppFS wake cards from terminal templates"
```

---

## Task 3: Add Dashboard Override Support for `terminal_render`

**Files:**

- Modify: `dashboard/src/components/AppControlPanel.tsx`
- Modify: `dashboard/src/types.ts`
- Optional Modify: `dashboard/server/src/types.ts`

**Step 1: Add terminal render fields to frontend draft model**

Extend the event draft state:

```ts
interface EventDraft {
  classValue: string;
  mode: string;
  wake: boolean;
  runningDelivery: string;
  idleDelivery: string;
  template: string;
  bodyTemplate: string;
  sourceTemplate: string;
  terminalLines: string;
}
```

Use newline-separated text in the UI for `terminal_render.lines`.

**Step 2: Read terminal render metadata**

In `draftFromMetadata`, read:

```ts
const terminalRender = metadata?.terminal_render ?? metadata?.ui_render ?? metadata?.user_render ?? {};
const terminalLines = Array.isArray(terminalRender.lines)
  ? terminalRender.lines.filter((line): line is string => typeof line === 'string').join('\n')
  : stringOrDefault(terminalRender.template, '');
```

**Step 3: Write terminal render overrides**

In `draftToOverride`, add:

```ts
const terminalLines = draft.terminalLines
  .split('\n')
  .map(line => line.trimEnd())
  .filter(Boolean);
if (terminalLines.length > 0) {
  metadata.terminal_render = {
    mode: 'card',
    lines: terminalLines,
  };
}
```

**Step 4: Add terminal preview**

Add a preview block that renders ANSI tokens as visible text or as styled spans.

Minimum acceptable v0:

- Show raw rendered terminal lines.
- Do not attempt full terminal emulation.

**Step 5: Build dashboard**

Run:

```powershell
npm run build
```

Working directory:

```powershell
dashboard
```

Expected:

- PASS.

**Step 6: Typecheck server if types changed**

Run:

```powershell
npm exec -- tsc --noEmit -p tsconfig.json
```

Working directory:

```powershell
dashboard/server
```

Expected:

- PASS.

**Step 7: Commit**

```powershell
git add dashboard/src/components/AppControlPanel.tsx dashboard/src/types.ts dashboard/server/src/types.ts
git commit -m "Add terminal render controls to App Control"
```

---

## Task 4: Add Documentation and Manual Smoke

**Files:**

- Modify: `docs/plans/2026-05-19-appfs-event-model-rendering.md`
- Optional Modify: Tinode/AppFS docs if this becomes public user-facing behavior.

**Step 1: Document the new rendering split**

Add a short note:

```markdown
Event rendering now has two targets:

- `model_render`: LLM-safe, no ANSI/control escape output.
- `terminal_render`: CLI-only wake-card rendering with a small ANSI token allowlist.

Runtime policy still owns delivery, wake, and reply behavior. Templates only control presentation.
```

**Step 2: Manual smoke override**

Use a local AppFS mount and write:

```json
{
  "version": 1,
  "apps": {
    "tinode": {
      "events": {
        "message.received": {
          "terminal_render": {
            "mode": "card",
            "lines": [
              "{{ansi.cyan}}Tinode from {{message.sender}}{{ansi.reset}}",
              "{{message.body}}"
            ]
          }
        }
      }
    }
  }
}
```

to:

```text
<mount-root>/.claw/appfs-event-render-overrides.json
```

**Step 3: Trigger an AppFS message**

Send a Tinode message to the active principal and verify:

- The terminal wake card uses the custom `terminal_render`.
- The model-visible reminder still uses `model_render`.
- Model-visible output contains no ANSI escape sequences.

**Step 4: Run final verification**

Run:

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime input_router -- --test-threads=1
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli appfs_event -- --test-threads=1
```

Optional broader verification:

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime appfs -- --test-threads=1
npm run build
```

Working directory for dashboard build:

```powershell
dashboard
```

**Step 5: Commit**

```powershell
git add docs/plans/2026-05-19-appfs-event-model-rendering.md
git commit -m "Document target-specific AppFS event rendering"
```

---

## Acceptance Criteria

- `model_render` remains LLM-safe and never emits ANSI/control escape tokens.
- `terminal_render` can customize CLI wake card lines.
- `terminal_render` supports `{{ansi.*}}`, `{{message.sender}}`, and `{{message.body}}`.
- `message.received` no longer requires a CLI hardcoded special case when terminal metadata is present.
- Existing hardcoded CLI rendering remains as fallback.
- Existing Tinode direct-message flow still works.
- Dashboard overrides can configure terminal rendering without changing AppFS event JSONL.
- Raw session/debug/dashboard data remains rich and unchanged.

## Risks and Mitigations

- **Risk:** ANSI tokens leak into model context.
  - **Mitigation:** Target-aware renderer; tests assert ANSI renders empty for `Model`.

- **Risk:** App descriptors become prompt-injection vectors.
  - **Mitigation:** Templates remain presentation-only; reply and trust policy stay in runtime.

- **Risk:** Too much schema naming drift.
  - **Mitigation:** Canonicalize on `terminal_render`; read `ui_render` / `user_render` only as aliases.

- **Risk:** CLI tests are hard to isolate because rendering lives in `main.rs`.
  - **Mitigation:** Keep the first implementation minimal; if tests become awkward, extract `appfs_event_card_lines` helpers into a small module in a follow-up.

