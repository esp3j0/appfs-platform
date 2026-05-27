# APPFS Compose Typed Connector Config Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Move Tinode startup configuration out of shell env vars and into typed AppFS compose config, so dashboard and compose templates can own endpoint / login prefix / credential policy directly.

**Architecture:** Compose becomes the declarative source of truth for connector-specific settings. The compose loader parses and validates a typed `config.kind: tinode` block, `compose up` resolves it once per run into registry/runtime state, and `TinodeConnector` consumes the resolved config with env fallback only as a temporary compatibility path.

**Tech Stack:** Rust, serde_yaml, AgentFS registry files, Tinode connector, existing AppFS compose loader.

---

## Task 1: Add Typed Tinode Config to Compose Schema

**Files:**
- Modify: `appfs/cli/src/cmd/appfs/compose/schema.rs`
- Test: `appfs/cli/src/cmd/appfs/compose/schema.rs` tests
- Test: `appfs/cli/src/cmd/appfs/compose/loader.rs` tests

**Step 1: Write the failing test**

Add a compose fixture that declares a Tinode connector with structured config, for example:

```yaml
connectors:
  tinode-in-process:
    mode: in_process
    transport: in_process
    config:
      kind: tinode
      endpoint: http://101.34.216.193:6060
      login_prefix_template: dash{compose_run_id}
```

Assert that:
- the loader accepts the new block,
- the endpoint is preserved,
- the login prefix template is preserved,
- invalid values fail validation.

**Step 2: Run the test and verify it fails**

Run:

```powershell
cargo test --manifest-path appfs\cli\Cargo.toml parses_and_normalizes_relative_compose_paths -- --nocapture
cargo test --manifest-path appfs\cli\Cargo.toml tinode_compose_config -- --nocapture
```

Expected:
- the new Tinode config test fails because the schema does not yet accept the block.

**Step 3: Write minimal implementation**

Add a typed Tinode connector config block under `connectors.*.config`.

Suggested shape:

```rust
pub(crate) struct AppfsComposeConnectorConfig {
    pub(crate) kind: String,
    pub(crate) endpoint: String,
    pub(crate) login_prefix_template: String,
}
```

Add validation rules:
- endpoint must be `http://` or `https://`
- login prefix template must be non-empty
- `in_process` Tinode connector must still reject `command` / `healthcheck`

**Step 4: Run the test to verify it passes**

Run:

```powershell
cargo test --manifest-path appfs\cli\Cargo.toml parses_and_normalizes_relative_compose_paths -- --nocapture
cargo test --manifest-path appfs\cli\Cargo.toml tinode_compose_config -- --nocapture
```

Expected:
- both tests pass
- `cargo fmt --manifest-path appfs\cli\Cargo.toml --all -- --check` passes

**Step 5: Commit**

Commit message:

```bash
git commit -m "feat: add typed tinode compose config"
```

---

## Task 2: Thread Tinode Compose Config into Runtime Registry

**Files:**
- Modify: `appfs/cli/src/cmd/appfs/compose/connector_supervisor.rs`
- Modify: `appfs/cli/src/cmd/appfs/compose/reconcile.rs`
- Modify: `appfs/cli/src/cmd/appfs/compose/schema.rs`
- Modify: `appfs/cli/src/cmd/appfs.rs`
- Test: `appfs/cli/src/cmd/appfs/compose/reconcile.rs` tests

**Step 1: Write the failing test**

Add a test that resolves a compose doc with a Tinode connector config and verifies the resolved app/registry snapshot retains:
- endpoint
- login prefix template
- credential policy
- resolved compose run id / resolved login prefix

Use a small fixture with one private Tinode app.

**Step 2: Run the test and verify it fails**

Run:

```powershell
cargo test --manifest-path appfs\cli\Cargo.toml bootstrap_registry_from_resolved_apps -- --nocapture
cargo test --manifest-path appfs\cli\Cargo.toml resolve_apps -- --nocapture
```

Expected:
- the new assertion fails because resolved runtime state does not yet carry the structured Tinode config.

**Step 3: Write minimal implementation**

Thread the new Tinode config through:
- compose resolve result
- registry bootstrap
- runtime app materialization
- adapter-side runtime connector initialization
- mount-side read-through connector initialization

Keep app policy and connector config separate:
- app policy keeps `credential_policy`, `visibility`, `path_template`, `profile_template`
- connector config keeps `kind`, `endpoint`, `login_prefix_template`

Important: AppFS has two connector initialization paths. The adapter runtime consumes actions and inbound events, while the mount-side read-through runtime opens app files and snapshot resources. Both paths must receive the same resolved `connector_config` from `apps.registry.json`; otherwise normal file reads such as `_app/skill.res.json` can fail before the adapter even processes a business action.

**Step 4: Run the test to verify it passes**

Run:

```powershell
cargo test --manifest-path appfs\cli\Cargo.toml bootstrap_registry_from_resolved_apps -- --nocapture
cargo test --manifest-path appfs\cli\Cargo.toml resolve_apps -- --nocapture
```

Expected:
- registry snapshot contains the Tinode config fields in a connector-specific field, not in transport
- adapter-side and mount-side runtimes can both reconstruct the Tinode connector from registry config
- `cargo fmt --manifest-path appfs\cli\Cargo.toml --all -- --check` passes

**Step 5: Commit**

Commit message:

```bash
git commit -m "feat: thread tinode config through compose runtime"
```

---

## Task 3: Make Tinode Connector Consume Compose Config First

**Files:**
- Modify: `appfs/sdk/rust/src/tinode_connector.rs`
- Modify: `appfs/cli/src/cmd/appfs/core.rs`
- Modify: `appfs/cli/src/cmd/appfs.rs` tests
- Test: `appfs/sdk/rust/src/tinode_connector.rs` tests

**Step 1: Write the failing test**

Add tests that prove:
- Tinode connector can be constructed from structured compose config,
- `APPFS_TINODE_*` remains only a fallback,
- `login_prefix_template` is used to derive the effective login prefix,
- `credential_policy` still validates to `auto-create`.

**Step 2: Run the test and verify it fails**

Run:

```powershell
cargo test --manifest-path appfs\sdk\rust\Cargo.toml tinode_config_requires_endpoint_policy_and_safe_prefix -- --nocapture
cargo test --manifest-path appfs\sdk\rust\Cargo.toml tinode_generated_basic_login_respects_server_policy -- --nocapture
```

Expected:
- at least one test fails because the connector still depends on env-first initialization.

**Step 3: Write minimal implementation**

Change Tinode connector initialization order to:
1. use compose config if present
2. fall back to env only if the compose config is absent

Keep the connector validation strict:
- endpoint required
- safe login prefix required
- `auto-create` only for v0

**Step 4: Run the test to verify it passes**

Run:

```powershell
cargo test --manifest-path appfs\sdk\rust\Cargo.toml tinode_config_requires_endpoint_policy_and_safe_prefix -- --nocapture
cargo test --manifest-path appfs\sdk\rust\Cargo.toml tinode_generated_basic_login_respects_server_policy -- --nocapture
```

Expected:
- tests pass
- `cargo check --manifest-path appfs\sdk\rust\Cargo.toml -p agentfs-sdk --features debug-dump` passes if relevant

**Step 5: Commit**

Commit message:

```bash
git commit -m "feat: read tinode connector config from compose"
```

---

## Task 4: Update Docs and Add Compose Examples

**Files:**
- Modify: `appfs/appfs-compose.tinode.local.yaml`
- Modify: `appfs/appfs-compose.aiim-tinode.local.yaml`
- Modify: `docs/TINODE-APPFS-tree-v0-design.md`
- Modify: `docs/APPFS-multi-agent-identity-and-app-visibility-v0-design.md`

**Step 1: Write the failing test**

This is documentation-first. Add a compose example and ensure the existing example still parses after the schema change.

**Step 2: Run the test and verify it fails**

Run:

```powershell
cargo test --manifest-path appfs\cli\Cargo.toml parses_public_and_private_app_visibility_fields -- --nocapture
```

Expected:
- the old example should fail until updated to the new typed Tinode config.

**Step 3: Write minimal implementation**

Update example compose files so they show the structured Tinode config instead of shell env instructions.

**Step 4: Run the test to verify it passes**

Run:

```powershell
cargo test --manifest-path appfs\cli\Cargo.toml parses_public_and_private_app_visibility_fields -- --nocapture
```

Expected:
- example compose files parse cleanly

**Step 5: Commit**

Commit message:

```bash
git commit -m "docs: describe typed tinode compose config"
```

---

## Acceptance Criteria

1. Tinode startup no longer requires hand-exported `APPFS_TINODE_ENDPOINT`, `APPFS_TINODE_LOGIN_PREFIX`, and `APPFS_TINODE_CREDENTIAL_POLICY` for normal compose-driven runs.
2. The compose file explicitly owns Tinode connector configuration.
3. `credential_policy` remains app policy, not duplicated as env.
4. Dashboard can later render the same fields as a form without inventing a separate env editor.
5. Existing tests for compose parsing, Tinode config validation, and registry bootstrap all pass.
6. Mounted private Tinode app files are readable without Tinode env vars, including static resources such as `_app/skill.res.json` and `_app/actions.res.json`.
7. Mount-side read-through uses the resolved registry connector config, not env-only fallback, when constructing the Tinode connector.

## Rollout Notes

- Keep env fallback only as a compatibility fallback during implementation.
- Do not introduce a new generic env bucket unless a later connector genuinely needs it.
- Prefer typed config over stringly env once the compose schema is extended.
