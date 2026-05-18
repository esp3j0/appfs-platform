# Structured Input Router Model View Implementation Plan

**Goal:** Store AppFS/unified-input events as structured session blocks and render them into model-visible text only when building model requests.

**Architecture:** Match the existing tool-result pattern: session JSONL keeps raw structured data, while model request conversion renders model-facing text. AppFS `message.received` remains wake-capable, but the rendered external-message body and `<system-reminder>` are generated at model-boundary time instead of being pre-rendered into the session.

**Tech Stack:** Rust runtime/session schema, Rust CLI model request conversion, TypeScript dashboard session parsing/model view.

---

## PR 1: Runtime Structured Input Router Block

**Files:**
- Modify: `appfs-agent/rust/crates/runtime/src/session.rs`
- Modify: `appfs-agent/rust/crates/runtime/src/input_router.rs`
- Modify: `appfs-agent/rust/crates/runtime/src/conversation.rs`
- Modify: `appfs-agent/rust/crates/runtime/src/lib.rs`
- Modify: `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`
- Modify: `appfs-agent/rust/crates/tools/src/lib.rs`

**Steps:**
1. Add a structured `ContentBlock::InputRouter { inputs }` session block.
2. Add serde/load support for the new block while keeping old text attachments readable.
3. Add input-router conversion helpers: `PendingInput -> persisted input block -> PendingInput`.
4. Add a model-visible renderer that reuses the current `render_pending_input_reminder` behavior.
5. Change `sync_pending_inputs_before_model_call()` to store structured input-router blocks.
6. Update both `convert_messages()` call sites to render `ContentBlock::InputRouter` into model-visible text.
7. Add tests for session roundtrip, model-visible rendering, and event-turn behavior.

**Acceptance:**
- Raw session contains `{"type":"input_router","inputs":[...]}` for new routed inputs.
- Model request still sees the same user-facing event text/reminder as before.
- Old sessions containing pre-rendered `input_router` text attachments still load and work.

## PR 2: Dashboard Structured Input Router Support

**Files:**
- Modify: `dashboard/server/src/types.ts`
- Modify: `dashboard/server/src/routes/timeline.ts`
- Modify: `dashboard/src/types.ts`
- Modify: `dashboard/src/components/MessageBubble.tsx`
- Modify existing Model View only if raw session fallback rendering is needed.

**Steps:**
1. Extend dashboard block types to recognize `input_router`.
2. Extract AppFS interaction metadata from structured inputs instead of parsing only rendered text.
3. Show Raw View as structured event records.
4. Keep Model View backed by debug-dump model requests; do not infer model text from raw session if a debug dump exists.

**Acceptance:**
- Timeline still shows cross-agent Tinode interactions.
- Raw message display no longer makes structured input-router blocks look like opaque/invalid content.
- Model View continues to show actual model requests.

## PR 3: Cleanup And Documentation

**Files:**
- Modify: `appfs-agent/rust/README.md`
- Modify relevant AppFS/Tinode design docs if needed.
- Remove obsolete `#[cfg(test)]` rendered AppFS-event reminder helpers if all tests have moved to structured input-router rendering.

**Steps:**
1. Document raw session vs model-visible conversion for tools and input-router events.
2. Document the event classification split: wake policy vs model renderer.
3. Remove dead compatibility helpers only after dashboard and runtime tests are stable.

**Acceptance:**
- Docs explain why raw session and model view differ.
- No duplicate event-rendering paths remain except deliberate legacy compatibility.
