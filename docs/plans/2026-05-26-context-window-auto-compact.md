# Context Window Auto Compact Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace appfs-agent's fixed cumulative-input auto compact trigger with a context-window-aware trigger modeled after `C:\Users\esp3j\rep\claude-code`.

**Architecture:** Keep the existing appfs-agent full compact pipeline, summary prompt, compact boundary, and post-compact layout. Change only the proactive auto compact trigger so it measures current context size instead of cumulative billing-style input tokens, computes thresholds from the selected model context window and max output tokens, and avoids repeated failed compaction attempts. Dashboard model config already writes `contextWindowTokens` and `maxOutputTokens`; the CLI/runtime should use those values for both preflight and auto compact.

**Tech Stack:** Rust workspace under `appfs-agent/rust`, existing `runtime::ConversationRuntime`, `runtime::compact`, `api::providers`, and `rusty-claude-cli` runtime model config.

---

## Reference Behavior From `C:\Users\esp3j\rep\claude-code`

Relevant files:

- `src/services/compact/autoCompact.ts`
- `src/services/compact/compact.ts`
- `src/utils/tokens.ts`
- `src/query.ts`

Key behavior to port conceptually:

- `tokenCountWithEstimation(messages)` is the canonical context-size estimator. It uses the last real API response usage as a baseline, then adds a rough estimate for messages appended since that response. It explicitly avoids cumulative usage because cumulative usage double-counts a growing context.
- `getEffectiveContextWindowSize(model)` subtracts reserved summary output from the model context window. It caps reserved summary output at `20_000`.
- `getAutoCompactThreshold(model)` subtracts a fixed safety buffer of `13_000` from the effective window. There is also an env percent override for testing.
- Auto compact has recursion guards and a circuit breaker that stops retrying after repeated failures.
- The full compact output layout is boundary + summary + preserved tail + attachments/hooks. appfs-agent already has the same core boundary + summary + preserved segment pattern in `runtime/src/compact.rs`.

## Current appfs-agent Behavior

Relevant files:

- `appfs-agent/rust/crates/runtime/src/conversation.rs`
- `appfs-agent/rust/crates/runtime/src/compact.rs`
- `appfs-agent/rust/crates/runtime/src/usage.rs`
- `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`
- `appfs-agent/rust/crates/api/src/providers/mod.rs`

Current problems:

- Auto compact uses `DEFAULT_AUTO_COMPACTION_INPUT_TOKENS_THRESHOLD = 100_000`.
- Trigger condition is `usage_tracker.cumulative_usage().input_tokens >= threshold`.
- `contextWindowTokens` from dashboard is passed into API preflight, but does not affect auto compact.
- Cumulative input token counting double-counts repeated prompt context, so sessions can compact after a few turns even on 200k or larger context windows.
- After auto compact in the same runtime process, `usage_tracker` is not rebuilt from the compacted session, so the in-memory cumulative counter can remain above threshold.

## Target Design

### Auto Compact Configuration

Add a runtime-side config struct:

```rust
pub struct AutoCompactionConfig {
    pub context_window_tokens: Option<u32>,
    pub max_output_tokens: u32,
    pub fixed_threshold_tokens: Option<u32>,
}
```

Constants:

```rust
const MAX_OUTPUT_TOKENS_FOR_SUMMARY: u32 = 20_000;
const AUTOCOMPACT_BUFFER_TOKENS: u32 = 13_000;
const DEFAULT_AUTO_COMPACTION_CONTEXT_WINDOW_TOKENS: u32 = 200_000;
const AUTO_COMPACTION_WINDOW_ENV_VAR: &str = "CLAUDE_CODE_AUTO_COMPACT_WINDOW";
const AUTO_COMPACTION_PCT_ENV_VAR: &str = "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE";
```

Keep `CLAUDE_CODE_AUTO_COMPACT_INPUT_TOKENS` and `fixed_threshold_tokens` temporarily as compatibility escape hatches, but make them override the computed context threshold rather than be the default strategy.

### Threshold Formula

```
effective_window = min(context_window, CLAUDE_CODE_AUTO_COMPACT_WINDOW if set)
reserved_summary_output = min(max_output_tokens, 20_000)
auto_threshold = effective_window - reserved_summary_output - 13_000
```

Clamp low values defensively:

- if subtraction underflows, fall back to `effective_window * 80%`
- percent override returns `min(percent_threshold, auto_threshold)`

### Context Size Estimation

Add `UsageTracker::context_window_usage_estimate(&Session)` or equivalent helper:

- Find the last message with usage.
- Use `usage.input_tokens + usage.output_tokens + cache_creation_input_tokens + cache_read_input_tokens` as baseline.
- Add rough token estimate for messages after that usage-bearing message.
- If no usage exists, use `estimate_session_tokens(session)`.

This mirrors claude-code's `tokenCountWithEstimation()`.

### Trigger Point

Replace `maybe_auto_compact()`:

- compute `estimated_context_tokens`
- compute `threshold`
- compact only when estimate >= threshold
- include `estimated_context_tokens` and `threshold` in `AutoCompactionEvent`
- after successful compact, assign `self.session = result.compacted_session` and reset `self.usage_tracker = UsageTracker::from_session(&self.session)`

### CLI Wiring

In `rusty-claude-cli/src/main.rs`:

- Headless runtime already loads `RuntimeModelConfigOverride`.
- When constructing `ConversationRuntime`, call `.with_auto_compaction_config(...)` using `context_window_tokens` and resolved `max_output_tokens`.
- If no runtime model config exists, fall back to provider registry defaults where available, otherwise 200k context and current `max_tokens_for_model`.

### Tests

Add focused tests in `runtime/src/conversation.rs` and `runtime/src/usage.rs`:

- below threshold does not compact even when cumulative usage exceeds 100k
- above computed context threshold compacts
- threshold uses context window and max output tokens
- compact success resets in-memory usage tracker
- env fixed threshold and percent override parse correctly

Run:

```powershell
cargo test --manifest-path appfs-agent/rust/Cargo.toml -p runtime auto_compact -- --nocapture
cargo test --manifest-path appfs-agent/rust/Cargo.toml -p runtime usage -- --nocapture
cargo check --manifest-path appfs-agent/rust/Cargo.toml -p runtime -p api -p rusty-claude-cli
```

## Implementation Tasks

### Task 1: Add Current Context Estimation

**Files:**

- Modify: `appfs-agent/rust/crates/runtime/src/usage.rs`
- Test: `appfs-agent/rust/crates/runtime/src/usage.rs`

**Steps:**

1. Add `TokenUsage::context_window_tokens()`.
2. Add `UsageTracker::estimated_context_window_tokens(session: &Session) -> u32`.
3. Use `runtime::compact::estimate_message_tokens` or a local equivalent if needed; avoid circular dependencies if `usage.rs` cannot import `compact.rs`.
4. Test that it uses last usage plus tail estimate, not cumulative usage.

### Task 2: Add Auto Compaction Threshold Config

**Files:**

- Modify: `appfs-agent/rust/crates/runtime/src/conversation.rs`
- Test: `appfs-agent/rust/crates/runtime/src/conversation.rs`

**Steps:**

1. Add `AutoCompactionConfig` and constants.
2. Add helper functions for effective window and threshold.
3. Preserve env overrides.
4. Add unit tests for threshold math.

### Task 3: Switch `maybe_auto_compact`

**Files:**

- Modify: `appfs-agent/rust/crates/runtime/src/conversation.rs`

**Steps:**

1. Replace cumulative input comparison with estimated context comparison.
2. Add threshold/estimate to `AutoCompactionEvent`.
3. Reset `usage_tracker` after successful compaction.
4. Update existing auto compact tests.

### Task 4: Wire Model Config Into Runtime

**Files:**

- Modify: `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`

**Steps:**

1. Resolve auto compact context window from runtime model config.
2. Resolve max output tokens from runtime model config or model default.
3. Apply the config to each `ConversationRuntime` constructor used by headless/interative paths.
4. Add or update tests if runtime construction has existing coverage.

### Task 5: Verify

**Commands:**

```powershell
cargo test --manifest-path appfs-agent/rust/Cargo.toml -p runtime auto_compact -- --nocapture
cargo test --manifest-path appfs-agent/rust/Cargo.toml -p runtime usage -- --nocapture
cargo check --manifest-path appfs-agent/rust/Cargo.toml -p runtime -p api -p commands -p tools -p rusty-claude-cli
```

**Expected:**

- Auto compact no longer fires merely because cumulative input crosses 100k.
- It fires when estimated current context approaches the configured model window.
- Dashboard-selected large-context models delay compact appropriately.
