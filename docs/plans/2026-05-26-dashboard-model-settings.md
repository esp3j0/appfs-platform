# Dashboard Model Settings Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use executing-plans to implement this plan task-by-task.

**Goal:** Add durable dashboard-managed provider/model settings, make spawned appfs-agent processes honor selected model, provider, context window, and max output tokens, and leave a clean path for live headless model switching.

**Architecture:** Dashboard owns the long-lived editable catalog at `~/.appfs-platform/models.json`. Each spawn resolves one selected catalog entry into an immutable runtime model-config file under `~/.appfs-platform/runtime/model-configs/`, then passes that file to appfs-agent via `--model-config`. appfs-agent treats the file as a runtime overlay on top of existing `.claw` settings: it can override provider wiring, model name, context window, and max output tokens without changing project config.

**Tech Stack:** TypeScript Fastify dashboard server, React dashboard UI, Rust appfs-agent CLI/runtime/API crates.

---

### Task 1: Add Dashboard Model Config Store

**Files:**
- Create: `dashboard/server/src/model-config-store.ts`
- Test: `dashboard/server/src/model-config-store.test.ts`

**Steps:**
1. Define catalog types: provider id/name/type/baseUrl/credential/models.
2. Resolve storage directory from `APPFS_PLATFORM_HOME` or `~/.appfs-platform`.
3. Load `models.json`; create a default Anthropic provider when missing.
4. Normalize/validate provider ids, selected model ids, token limits.
5. Write tests for default creation and update/read round trip.

### Task 2: Add Dashboard Model Config API

**Files:**
- Create: `dashboard/server/src/routes/model-configs.ts`
- Modify: `dashboard/server/src/index.ts`

**Steps:**
1. Add `GET /api/model-configs`.
2. Add `PUT /api/model-configs`.
3. Register the route in the dashboard server.
4. Test with Fastify injection.

### Task 3: Resolve Spawn Model Configs

**Files:**
- Modify: `dashboard/server/src/process-manager.ts`
- Modify: `dashboard/server/src/routes/process.ts`
- Modify: `dashboard/src/types.ts`

**Steps:**
1. Extend `SpawnConfig` with `modelProviderId`, `modelId`, `contextWindowTokens`, `maxOutputTokens`, and `runtimeModelConfigPath`.
2. Inject `ModelConfigStore` into `AgentProcessManager`.
3. Before spawn, resolve selected provider/model into a runtime JSON file.
4. Add `--model-config <path>` to cargo and binary launch args when present.
5. Preserve existing env-based behavior when no model store exists.

### Task 4: Teach appfs-agent `--model-config`

**Files:**
- Modify: `appfs-agent/rust/crates/api/src/providers/mod.rs`
- Modify: `appfs-agent/rust/crates/api/src/lib.rs`
- Modify: `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`

**Steps:**
1. Add runtime model config structs to CLI.
2. Parse `--model-config <path>`.
3. Load resolved model config before creating `LiveCli`.
4. Convert provider config to `ProviderOverride`.
5. Override `max_tokens` and context preflight with model-config values.
6. Keep old `--model` behavior unchanged when no model-config is passed.

### Task 5: Add Spawn UI Controls

**Files:**
- Modify: `dashboard/src/components/AgentSidebar.tsx`
- Modify: `dashboard/src/index.css`
- Modify: `dashboard/src/types.ts`

**Steps:**
1. Fetch `/api/model-configs` with spawn defaults.
2. Show provider select, model select, context tokens input, max output tokens input.
3. Keep manual model input as advanced fallback.
4. Update spawn body with selected provider/model/token overrides.
5. Avoid showing API keys in UI for this phase.

### Task 6: Future Live Switch Follow-Up

**Files:**
- Future: `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`
- Future: `dashboard/server/src/process-manager.ts`
- Future: dashboard model switch UI

**Steps:**
1. Extend headless control protocol with `set_model_config`.
2. Idle agents apply immediately; busy agents apply at next model boundary.
3. Dashboard sends `cancel_turn` first when user wants immediate interruption.
4. Persist active runtime model metadata into session/registry.

### Verification

Run:
- `npm --prefix dashboard/server run build`
- `npm --prefix dashboard run build`
- `cargo test --manifest-path appfs-agent/rust/Cargo.toml -p api model_token_limit -- --nocapture`
- `cargo test --manifest-path appfs-agent/rust/Cargo.toml -p rusty-claude-cli model_config -- --nocapture`

