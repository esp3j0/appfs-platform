# AppFS Multi-Agent Tinode Implementation Plan

> **For Codex:** implement this plan task-by-task. Do not skip the validation steps. If an implementation discovery changes the design, update the relevant design document in the same PR.

**Goal:** Build the generic AppFS multi-agent identity and private-app visibility foundation, then add a real Tinode AppFS connector on top of it.

**Architecture:** Implement the foundation in vertical slices. AppFS owns project-level principals, app policies, and app instance registries; appfs-agent owns the current process identity, model-visible prompt context, skill discovery, and event filtering; connectors own app-specific credential state keyed by `profile_id`. Tinode is introduced only after the generic identity and visibility layer can be tested without Tinode.

**Tech Stack:** Rust AppFS CLI/runtime, Rust appfs-agent, AppFS Rust SDK, JSON/JSONL control files, AppFS compose YAML, Tinode bridge/connector, Windows WinFsp plus Linux/macOS compatibility paths.

---

## Source Documents

Read these before implementation:

- `docs/APPFS-multi-agent-identity-and-app-visibility-v0-design.md`
- `docs/APPFS-multi-agent-identity-流程验收文档.md`
- `docs/TINODE-APPFS-v0-design.md`
- `docs/TINODE-APPFS-tree-v0-design.md`

This plan is intentionally more operational than the design docs. If the plan and design disagree, stop and reconcile the docs before writing code.

## Implementation Strategy

Use small PRs with one clear acceptance target each. The correct order is:

1. Teach the code about principals and app policies without changing user behavior.
2. Let AppFS create and persist principals.
3. Let AppFS materialize private app instances from compose policies.
4. Let appfs-agent see only the apps/events relevant to the current principal.
5. Extend connector context and credential storage.
6. Implement Tinode skeleton.
7. Implement Tinode credentials and direct messaging.
8. Implement inbound messages, inbox, groups, and principal fork flows.

Do not start with Tinode connector code. Tinode will otherwise force identity, visibility, credentials, and event streaming to be debugged at the same time.

## Non-Negotiable Invariants

These invariants should appear in code comments, tests, or PR descriptions where relevant:

- `attach_id` remains ephemeral and process-scoped.
- `principal_id` is the stable semantic agent identity. Default is `APPFS_PRINCIPAL_ID` or `default`.
- `profile_id` is app-specific and credential-scoped. For private Tinode, use `tinode:{principal_id}`.
- No global `/_appfs/current_identity.res.json`.
- No `/_views/<principal-id>` root in v0.
- No `team` visibility in v0.
- No generic `by-login` path layer in v0.
- No Tinode tokens, refresh tokens, passwords, API keys, cookies, or raw secrets in AppFS files, events, prompts, session logs, or skill output.
- AIIM stays available as a public demo and regression fixture.
- `/aiim` compatibility must not create duplicate `appfs-aiim` skills when `/public/aiim` exists.
- Private event reminders must be principal-aware before private Tinode events are enabled.
- Bootstrap app structure must not create upstream Tinode credentials.
- `principal:<principal-id>` references must resolve through AppFS/app registry plus connector private credential state; never infer upstream IDs from string concatenation alone.

## Repository Map

AppFS runtime and compose:

- `appfs/cli/src/cmd/appfs/compose/schema.rs`
- `appfs/cli/src/cmd/appfs/compose/reconcile.rs`
- `appfs/cli/src/cmd/appfs/compose/connector_supervisor.rs`
- `appfs/cli/src/cmd/appfs/registry.rs`
- `appfs/cli/src/cmd/appfs/runtime_supervisor.rs`
- `appfs/cli/src/cmd/appfs/supervisor_control.rs`
- `appfs/cli/src/cmd/appfs/action_dispatcher.rs`
- `appfs/cli/src/cmd/appfs/runtime_manifest.rs`
- `appfs/cli/src/cmd/appfs/mount_runtime.rs`

AppFS SDK and connector protocol:

- `appfs/sdk/rust/src/appfs_connector.rs`

appfs-agent runtime:

- `appfs-agent/rust/crates/runtime/src/appfs.rs`
- `appfs-agent/rust/crates/runtime/src/session.rs`
- `appfs-agent/rust/crates/runtime/src/conversation.rs`
- `appfs-agent/rust/crates/runtime/src/session_control.rs`
- `appfs-agent/rust/crates/tools/src/lib.rs`
- `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`

Existing AIIM fixtures:

- `appfs/appfs-compose.aiim.local.yaml`
- AIIM connector/demo files under the current appfs compose test fixtures

Future Tinode work:

- `integration/scripts/tinode-smoke.mjs`
- new Tinode connector/bridge location to be decided during the Tinode connector PR

## Validation Commands

Use these commands as the default local validation set. Narrow tests are listed per task.

```powershell
cargo test --manifest-path appfs\cli\Cargo.toml
cargo test --manifest-path appfs\sdk\rust\Cargo.toml
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p tools
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli
```

On Windows manual mount smoke, keep a separate target dir:

```powershell
cargo run --manifest-path appfs\cli\Cargo.toml --target-dir C:\tmp\appfs-local-target -- appfs compose up -f appfs\appfs-compose.aiim.local.yaml
```

For Tinode PRs only, require a Tinode endpoint:

```powershell
$env:APPFS_TINODE_ENDPOINT = "http://101.34.216.193:6060"
node integration\scripts\tinode-smoke.mjs
```

## PR 0: Documentation Baseline

**Status:** current docs PR.

**Goal:** Land the design and implementation plan before code changes.

**Files:**

- Modify: `docs/APPFS-multi-agent-identity-and-app-visibility-v0-design.md`
- Modify: `docs/APPFS-multi-agent-identity-流程验收文档.md`
- Modify: `docs/TINODE-APPFS-v0-design.md`
- Modify: `docs/TINODE-APPFS-tree-v0-design.md`
- Create: `docs/plans/2026-05-06-appfs-multi-agent-tinode.md`

**Acceptance:**

- The docs clearly distinguish `attach_id`, `principal_id`, and `profile_id`.
- The docs clearly distinguish work fork from principal fork.
- The docs say `create_principal.act` is consumed by AppFS supervisor, not by an app connector.
- The docs say `principals.registry.json` is the source of truth and `principals/<id>.res.json` is derived.
- The docs say private app instances are materialized from compose app policies.
- The Tinode tree has no `refresh_messages.act` in v0.

**Validation:**

```powershell
git diff --check
```

## PR 1: Principal Data Model and Read-Only Parsing

**Goal:** Add principal and app policy data types without changing runtime behavior.

**Files:**

- Modify: `appfs/cli/src/cmd/appfs/registry.rs`
- Modify: `appfs/cli/src/cmd/appfs/compose/schema.rs`
- Modify: `appfs/cli/src/cmd/appfs/compose/reconcile.rs`
- Test: existing module tests in the same files

**Step 1: Add failing schema tests**

Add tests that parse compose apps with:

- `visibility: public`
- `visibility: private`
- `path: public/aiim`
- `path_template: private/{principal_id}/tinode`
- `profile_template: tinode:{principal_id}`
- `credential_policy: auto-create`

Expected before implementation: compose parsing rejects unknown fields.

**Step 2: Add compose schema fields**

Extend compose app structs with:

- `visibility: Option<AppfsComposeAppVisibility>`
- `path: Option<String>`
- `path_template: Option<String>`
- `profile_template: Option<String>`
- `credential_policy: Option<String>`

Validation rules:

- default visibility is `public` for existing compose files;
- `public` apps may use `path`;
- `private` apps must use `path_template`;
- private `path_template` must contain `{principal_id}`;
- `profile_template`, if present, must contain `{principal_id}` for private apps;
- reject unknown visibility values.

**Step 3: Add registry structs**

Add structs for:

- `PrincipalRegistryDoc`
- `PrincipalRecord`
- `AppPolicyRegistryDoc`
- `AppPolicyRecord`
- extended app instance record fields: `instance_id`, `visibility`, `principal_id`, `profile_id`, `parent_app_id`, `path`

Only the new format is supported. The software has not shipped, so there is no legacy format to preserve. If an old-format `apps.registry.json` is found, treat it as a data error.

**Step 4: Render app policies during compose bootstrap**

`bootstrap_registry_from_resolved_apps` should write the new app-instance `apps.registry.json` format for public apps. Add a separate render path for `/_appfs/app-policies.registry.json`, but do not yet wire it into runtime boot if that would make the PR too large.

**Step 5: Run tests**

```powershell
cargo test --manifest-path appfs\cli\Cargo.toml compose
cargo test --manifest-path appfs\cli\Cargo.toml registry
```

**Acceptance:**

- Existing compose files still parse.
- New public/private compose fields parse.
- No runtime principal creation yet.
- AIIM compose still behaves as public by default.

**Rollback Point:**

If compose schema changes become too invasive, keep the new structs in `registry.rs` first and defer YAML schema support to PR 2.

## PR 2: Principal Control Plane

**Goal:** AppFS can create, update, delete, and list principals through `/_appfs/principals/*.act`.

**Files:**

- Modify: `appfs/cli/src/cmd/appfs/runtime_manifest.rs`
- Modify: `appfs/cli/src/cmd/appfs/supervisor_control.rs`
- Modify: `appfs/cli/src/cmd/appfs/action_dispatcher.rs`
- Modify: `appfs/cli/src/cmd/appfs/runtime_supervisor.rs`
- Modify: `appfs/cli/src/cmd/appfs/mount_runtime.rs`
- Modify: `appfs/cli/src/cmd/appfs/registry.rs`
- Test: `appfs/cli/src/cmd/appfs/tests.rs`

**Step 1: Add failing parser tests**

Add tests for action payloads:

```json
{"principal_id":"default","display_name":"Default agent","description":"The default project agent.","kind":"agent"}
```

```json
{"principal_id":"incident-reporter","display_name":"Incident reporter","description":"Summarizes incidents.","kind":"agent"}
```

Reject:

- empty `principal_id`;
- path separators in `principal_id`;
- `.` or `..`;
- principal IDs that are not safe for Windows paths.

**Step 2: Add control action discovery**

Add actions under:

- `/_appfs/principals/create_principal.act`
- `/_appfs/principals/update_principal.act`
- `/_appfs/principals/delete_principal.act`

The action wake path filter must treat these as relevant action paths.

**Step 3: Implement supervisor handlers**

Add runtime supervisor handlers:

- create principal;
- update principal metadata;
- delete principal metadata.

For create:

- if the principal already exists, treat it as idempotent success;
- write/update `principals.registry.json`;
- write derived `principals/<principal-id>.res.json`;
- emit `principal.created` or `principal.exists`.

For update:

- update only metadata fields;
- do not mutate `principal_id`;
- emit `principal.updated`.

For delete:

- mark tombstone or delete according to the design decision in the implementation PR;
- do not remove connector credentials yet unless PR 6 credential cleanup has landed;
- emit `principal.deleted` or `principal.delete_requested`.

**Step 4: Add registry persistence tests**

Test that:

- `principals.registry.json` is authoritative;
- derived `principals/default.res.json` matches registry;
- duplicate default creation is idempotent.

**Step 5: Run tests**

```powershell
cargo test --manifest-path appfs\cli\Cargo.toml principal
cargo test --manifest-path appfs\cli\Cargo.toml supervisor_control
cargo test --manifest-path appfs\cli\Cargo.toml runtime_supervisor
```

**Acceptance:**

- AppFS can process `create_principal.act` without any app connector.
- Concurrent default creation can be retried safely.
- No private app auto-instantiation yet.

**Rollback Point:**

If delete semantics are risky, implement create/update first and leave delete as a parsed but unsupported action with a clear `UNIMPLEMENTED` event.

## PR 3: App Policies and Private App Auto-Instantiation

**Goal:** Compose declares app policies, public apps are instantiated at startup, and private apps are instantiated per principal.

**Files:**

- Modify: `appfs/cli/src/cmd/appfs/compose/reconcile.rs`
- Modify: `appfs/cli/src/cmd/appfs/compose/connector_supervisor.rs`
- Modify: `appfs/cli/src/cmd/appfs/runtime_supervisor.rs`
- Modify: `appfs/cli/src/cmd/appfs/registry.rs`
- Modify: `appfs/cli/src/cmd/appfs/runtime_manifest.rs`
- Test: compose and runtime supervisor tests

**Step 1: Add failing compose bootstrap test**

Input compose:

```yaml
apps:
  aiim:
    connector: aiim-http
    visibility: public
    path: public/aiim
  tinode:
    connector: tinode-http
    visibility: private
    credential_policy: auto-create
    path_template: private/{principal_id}/tinode
    profile_template: tinode:{principal_id}
```

Expected output:

- `app-policies.registry.json` contains both AIIM and Tinode policies;
- `apps.registry.json` contains only the public AIIM instance at compose startup.

**Step 2: Write app policy registry**

During compose bootstrap, write:

- `/_appfs/app-policies.registry.json`
- `/_appfs/apps.registry.json`

Public app instance fields:

- `instance_id = app_id`
- `app_id`
- `visibility = public`
- `path`
- transport/connector metadata as currently required
- optional `legacy_aliases`

Private app policies stay out of `apps.registry.json` until a principal exists.

**Step 3: Materialize private instance on principal creation**

When `create_principal.act` succeeds:

- read `app-policies.registry.json`;
- find private policies;
- derive `path` and `profile_id`;
- create an app instance record with `visibility = private_instance`;
- set `instance_id` to a stable form such as `tinode--default`;
- copy transport from the compose-resolved connector policy;
- create runtime adapter for that instance.

**Step 4: Keep dynamic `register_app.act`**

Do not remove runtime `register_app.act`.

Rules:

- keep it for dynamic public app registration;
- do not require it for normal private per-principal app instances;
- if used for private instances, require explicit `principal_id` and reject missing policy unless intentionally supporting advanced mode.

**Step 5: Add AIIM compatibility handling**

Keep `/aiim` compatibility without treating it as a second public app instance. Prefer:

- canonical path: `/public/aiim`;
- compatibility path: `/aiim`;
- appfs-agent skill discovery must dedupe these later in PR 5.

**Step 6: Run tests**

```powershell
cargo test --manifest-path appfs\cli\Cargo.toml compose
cargo test --manifest-path appfs\cli\Cargo.toml registry
cargo test --manifest-path appfs\cli\Cargo.toml runtime_supervisor
```

**Acceptance:**

- Compose with public AIIM and private Tinode policy starts without creating Tinode credentials.
- Creating `default` principal adds `tinode--default` to `apps.registry.json`.
- Creating `incident-reporter` adds `tinode--incident-reporter`.
- Existing AIIM tests still pass.

**Rollback Point:**

If runtime adapter creation for private instances is too large, first persist the private instance record and emit `app.instance.created`; create adapters in the next PR.

## PR 4: Connector Context Principal/Profile Fields

**Goal:** AppFS passes effective `principal_id` and `profile_id` to connectors without trusting model-provided payload fields.

**Files:**

- Modify: `appfs/sdk/rust/src/appfs_connector.rs`
- Modify: AppFS connector adapter code under `appfs/cli/src/cmd/appfs/`
- Modify: HTTP bridge serialization/deserialization code if present
- Test: SDK and AppFS connector adapter tests

**Step 1: Add failing SDK serialization tests**

`ConnectorContext` should round-trip:

```json
{
  "session_id": "sess-1",
  "app_id": "tinode",
  "request_id": "req-1",
  "path": "/contacts/send_message.act",
  "principal_id": "default",
  "profile_id": "tinode:default"
}
```

**Step 2: Extend `ConnectorContext`**

Add:

- `principal_id: Option<String>`
- `profile_id: Option<String>`
- optionally `instance_id: Option<String>` if app instance routing needs it

These fields are optional because public apps may not have a principal/profile binding. Do not treat missing `principal_id` or `profile_id` as a legacy migration path for private apps; private app instances must get these values from the new app instance registry.

**Step 3: Fill context from app instance registry**

When submitting an action to a connector:

- use the effective app instance record;
- fill `principal_id` and `profile_id` from registry;
- ignore any payload `profile_id` as authority.

**Step 4: Define credential error behavior**

Use existing `AUTH_EXPIRED` for expired credentials. Add or document companion codes if needed:

- `PROFILE_NOT_READY`
- `PROFILE_NOT_FOUND`
- `CREDENTIALS_FAILED`

Do not over-expand error codes unless Tinode needs them.

**Step 5: Run tests**

```powershell
cargo test --manifest-path appfs\sdk\rust\Cargo.toml
cargo test --manifest-path appfs\cli\Cargo.toml connector
```

**Acceptance:**

- Public connectors still work.
- Private connector actions receive `principal_id` and `profile_id`.
- Model payload cannot impersonate a different principal/profile.

**Rollback Point:**

If HTTP bridge compatibility is uncertain, make fields optional and additive only; do not require Tinode until PR 7.

## PR 5: appfs-agent Principal Awareness, Skill Discovery, and Event Filtering

**Goal:** appfs-agent knows "who I am", lists only relevant app skills, and injects only relevant AppFS event reminders.

**Files:**

- Modify: `appfs-agent/rust/crates/runtime/src/appfs.rs`
- Modify: `appfs-agent/rust/crates/runtime/src/session.rs`
- Modify: `appfs-agent/rust/crates/runtime/src/conversation.rs`
- Modify: `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`
- Test: runtime and CLI tests

**Step 1: Add failing environment detection tests**

Cases:

- no `APPFS_PRINCIPAL_ID` => `principal_id = default`;
- explicit `APPFS_PRINCIPAL_ID=incident-reporter`;
- no `principals.registry.json` => appfs-agent requests/defaults to `default` creation path;
- existing registry => prompt lists known principals.

**Step 2: Add `APPFS_PRINCIPAL_ID_ENV`**

Add:

```rust
pub const APPFS_PRINCIPAL_ID_ENV: &str = "APPFS_PRINCIPAL_ID";
```

Extend `AppfsEnvironment` with:

- `principal_id`
- known principal summaries
- visible registered apps with `visibility`, `principal_id`, `profile_id`, and `path`

**Step 3: Extend registered app loading**

Update `AppfsRegisteredApp` to parse the new registry fields (`instance_id`, `app_id`, `visibility`, `principal_id`, `profile_id`, `path`, `active_scope`).

Visibility filter:

- include `visibility = public`;
- include `visibility = private_instance` only when `principal_id == current principal_id`;
- exclude private instances for other principals.

**Step 4: Update system prompt AppFS section**

Prompt should say:

- what AppFS is;
- current mount root;
- current `attach_id`;
- current `principal_id`;
- known principals and descriptions;
- visible public apps;
- visible private apps for current principal;
- `/private/<principal-id>` contains per-agent apps and the current agent should operate on its own private app root unless explicitly coordinating.

Do not add AIIM-specific or Tinode-specific instructions to the general AppFS prompt.

**Step 5: Update skill listing/discovery**

Rules:

- generated AppFS skills come from visible apps only;
- dedupe `/aiim` and `/public/aiim`;
- app-specific skill description should come from app-provided descriptors/readme where available;
- generated skill body may include action rules, app root, and app-specific descriptor content.

**Step 6: Update event reminder filtering**

`sync_appfs_event_reminders` should collect:

- platform events from `/_appfs/_stream/events.evt.jsonl`;
- public app events;
- private app events only for the current `principal_id`.

It must not inject Tinode events from another principal.

**Step 7: Ensure compaction preserves identity context**

After compaction, reconstruct or re-inject AppFS identity context. The compacted conversation should not forget `principal_id`.

**Step 8: Run tests**

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime appfs
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli appfs
```

**Acceptance:**

- From mount root, agent sees public AIIM and current principal's private apps.
- From `/public/aiim`, agent still sees the correct AIIM skill.
- From mount root, appfs-agent no longer says it cannot detect an app when apps exist at root.
- A `default` agent does not receive `incident-reporter` Tinode events.

**Rollback Point:**

If automatic default principal creation from appfs-agent is too risky, first implement read-only identity detection and a clear warning; wire creation in PR 6.

## PR 6: Credential Store Interface and Generic `ensure_credentials.act` Contract

**Goal:** Define and implement a generic connector credential state mechanism before Tinode depends on it.

**Files:**

- Modify: `appfs/cli/src/cmd/appfs/registry.rs`
- Modify: connector adapter/runtime files under `appfs/cli/src/cmd/appfs/`
- Modify: `appfs/sdk/rust/src/appfs_connector.rs`
- Test: AppFS runtime and SDK tests

**Step 1: Decide credential store backend**

Preferred v0:

- AppFS runtime DB key-value store, not model-visible tree files.

If the DB API is not ready:

- connector-owned SQLite/file is acceptable for Tinode v0, but document the limitation.

**Step 2: Add key shape helper**

Use:

```text
connector:<connector-name>:profile:<profile-id>:credentials
```

Do not store credentials in:

- `apps.registry.json`;
- `principals.registry.json`;
- events;
- skill descriptors;
- session files.

**Step 3: Define safe credential summary**

The model-visible summary can include:

- `credential_status`;
- `profile_id`;
- `upstream_user_id`;
- `login`;
- `display_name`;
- `last_ready_at`;

It must not include secrets.

**Step 4: Define `ensure_credentials.act` flow**

For private app roots:

```text
/private/<principal-id>/<app-id>/_app/ensure_credentials.act
```

Payload v0:

```json
{"client_token":"ensure-tinode-default"}
```

Optional payload:

```json
{"expected_profile_id":"tinode:default","client_token":"ensure-tinode-default"}
```

The effective `profile_id` comes from connector context.

**Step 5: Add principal deletion cleanup hook design**

At minimum, emit cleanup request events. Full credential deletion can land with Tinode once credential store is real.

**Step 6: Run tests**

```powershell
cargo test --manifest-path appfs\cli\Cargo.toml credential
cargo test --manifest-path appfs\sdk\rust\Cargo.toml
```

**Acceptance:**

- Credential store API exists or is explicitly deferred with a safe Tinode-owned store plan.
- No secret can be rendered into AppFS tree by generic code.
- `ensure_credentials.act` contract is generic and not Tinode-specific.

**Rollback Point:**

If a generic credential API is too large, create a narrow `ConnectorPrivateState` trait and implement only enough for Tinode, but keep the no-secrets invariant.

## PR 7: Tinode Connector Skeleton and AppFS Tree

**Goal:** Add the Tinode AppFS app skeleton without sending real messages.

**Files:**

- Create: Tinode connector/bridge files under the chosen connector directory
- Modify: compose fixture for Tinode smoke if needed
- Modify: docs if connector location differs from this plan
- Test: Tinode connector unit tests and tree tests

**Step 1: Create Tinode connector skeleton**

Implement:

- `connector_id`
- `health`
- `get_app_structure`
- safe `self.res.json`
- empty indexes/resources

Do not create Tinode accounts in `get_app_structure`.

**Step 2: Materialize tree**

Tree must match `docs/TINODE-APPFS-tree-v0-design.md`:

```text
_app/self.res.json
_app/ensure_credentials.act
_app/refresh_structure.act
_app/refresh_inbox.act
_stream/events.evt.jsonl
contacts/index.res.jsonl
contacts/send_message.act
contacts/resolve.act
contacts/search_results.res.jsonl
groups/index.res.jsonl
groups/create_group.act
inbox/recent.res.jsonl
inbox/unread.res.jsonl
inbox/mark_read.act
topics/index.res.jsonl
```

**Step 3: Add connector config validation**

Require:

- Tinode server endpoint;
- credential policy;
- safe login prefix or account naming policy.

Reject missing endpoint early.

**Step 4: Add tests**

Test:

- skeleton tree;
- no credential creation during structure fetch;
- no secrets in resources;
- non-ASCII display names do not break resource JSON.

**Step 5: Run tests**

```powershell
cargo test --manifest-path appfs\cli\Cargo.toml tinode
node integration\scripts\tinode-smoke.mjs
```

**Acceptance:**

- Compose can mount private Tinode skeleton for `default`.
- `_app/self.res.json` says credentials are missing.
- No Tinode account is created before a credential-required action.

**Rollback Point:**

If full AppFS connector integration is too heavy, first implement the bridge server plus a standalone smoke test, then wire it into compose.

## PR 8: Tinode Credentials and Direct Messages

**Goal:** First real business flow: agent sends a direct message to a user/contact through Tinode.

**Files:**

- Modify: Tinode connector files
- Modify: credential store if needed
- Modify: Tinode smoke script
- Test: Tinode connector tests

**Step 1: Implement auto-create credentials**

On first credential-required action:

- derive effective `profile_id` from connector context;
- check credential store;
- if missing, create or reuse Tinode account;
- store token/refresh token privately;
- update safe `_app/self.res.json` summary;
- emit `profile.credentials.ready`.

**Step 2: Implement token refresh**

Before Tinode API calls:

- check expiry;
- refresh if needed;
- update private store;
- on failure, return `AUTH_EXPIRED`.

**Step 3: Implement contact resolution**

Support:

- direct `contacts/<contact-key>/send_message.act` when contact exists;
- `contacts/send_message.act` with recipient reference;
- `contacts/resolve.act` to resolve/search.

Do not add `by-login` as a path layer.

**Step 4: Implement `send_message.act`**

Payload:

```json
{
  "to": "basic:zhangsan",
  "text": "明天开会",
  "client_token": "send-001"
}
```

For contact-specific action:

```json
{
  "text": "明天开会",
  "client_token": "send-001"
}
```

Emit:

- `action.accepted`;
- `message.sent`;
- `action.completed`;
- `action.failed` on failure.

**Step 5: Add tests**

Test:

- first send creates credentials;
- second send reuses credentials;
- action payload cannot override `profile_id`;
- bad recipient yields useful failure;
- event reminder summary is model-readable and has no secrets.

**Step 6: Run tests**

```powershell
cargo test --manifest-path appfs\cli\Cargo.toml tinode
node integration\scripts\tinode-smoke.mjs
```

**Acceptance:**

- From appfs-agent, user can ask: "给张三发消息说明天开会".
- Model loads `appfs-tinode` skill, appends one JSONL action, and does not manually tail events if event reminders are enabled.
- Event reminder confirms success.
- Tinode client UI shows the message from the agent account.

**Rollback Point:**

If contact resolution is unstable, support explicit `basic:<login>` recipient in root `contacts/send_message.act` first, then add contact folders.

## PR 9: Inbound Messages and Inbox

**Goal:** User-to-agent Tinode messages appear as AppFS events and readable inbox resources.

**Files:**

- Modify: Tinode connector files
- Modify: AppFS event integration if needed
- Modify: appfs-agent event reminder tests if event shape changes
- Test: Tinode inbound smoke tests

**Step 1: Implement connector-side inbound handling**

Tinode connector maintains an internal WebSocket connection to the Tinode server. Received `{data}` and `{pres}` messages are translated into AppFS events and resource updates directly inside the connector:

- write new messages to `contacts/<key>/messages.res.jsonl` or `groups/<key>/messages.res.jsonl`;
- append to `inbox/recent.res.jsonl` and `inbox/unread.res.jsonl`;
- emit `message.received` event to `_stream/events.evt.jsonl`.

Inbound handling does not go through the `AppConnector` trait in v0. The adapter poll loop continues to drive `.act` file processing; connector-internal WebSocket runs alongside it. Do not add OS file watchers.

**Step 2: Update resources**

Maintain:

- `inbox/recent.res.jsonl`
- `inbox/unread.res.jsonl`
- `contacts/<contact-key>/messages.res.jsonl`
- `groups/<group-key>/messages.res.jsonl`

**Step 3: Emit inbound events**

Event examples:

- `message.received`
- `inbox.updated`
- `message.read`

Include:

- contact/group key;
- safe sender display;
- text preview or full text according to v0 policy;
- Tinode topic/message IDs if safe.

Do not include secrets.

**Step 4: Test appfs-agent reminder flow**

User sends message from Tinode UI. appfs-agent receives next-turn system reminder:

```text
New AppFS events were received since the previous model call.
- [AppFS app `tinode`] type=message.received ...
```

**Step 5: Run tests**

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime appfs
node integration\scripts\tinode-smoke.mjs inbound
```

**Acceptance:**

- User can message the agent from Tinode UI.
- Agent sees inbound event without manually reading `_stream/events.evt.jsonl`.
- Agent can read recent messages from `inbox/recent.res.jsonl`.

**Rollback Point:**

If the internal Tinode WebSocket is unstable, fall back to periodic polling of `{get what="data"}` inside the connector. The connector still writes events and resources directly; only the data source changes. Do not add new actions or trait methods.

## PR 10: Principal Fork and Multi-Agent Tinode

**Goal:** Let an agent create a new semantic principal and use Tinode to communicate privately or in groups.

**Files:**

- Modify: `appfs-agent/rust/crates/runtime/src/session_control.rs`
- Modify: `appfs-agent/rust/crates/runtime/src/conversation.rs`
- Modify: `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`
- Modify: `appfs-agent/rust/crates/tools/src/lib.rs` if adding a tool
- Modify: AppFS principal action handling if needed
- Modify: Tinode connector group actions
- Test: runtime, CLI, Tinode group smoke tests

**Step 1: Explicitly separate fork types in code/UI**

Keep existing `/session fork` as work/session fork unless intentionally changed.

Add one of:

- a new command such as `/principal create`;
- a tool such as `CreatePrincipal`;
- an option on future agent spawn tooling that clearly means "new principal".

Do not silently change current subagent/skill fork behavior to create a new principal.

**Step 2: Implement principal creation workflow**

Tool/command flow:

1. write `/_appfs/principals/create_principal.act`;
2. wait or check for principal registry update;
3. start or instruct a new agent process with `APPFS_PRINCIPAL_ID=<new-id>`;
4. ensure private apps auto-instantiate for that principal.

**Step 3: Implement group creation and invitations**

Tinode group actions:

- `groups/create_group.act`
- `groups/<group-key>/invite_members.act`
- `groups/<group-key>/send_message.act`

Member refs:

- `principal:<principal-id>`
- `basic:<login>` or other explicit Tinode-safe refs

For `principal:<principal-id>`:

- resolve principal through AppFS registry/app instances;
- find Tinode private app instance;
- get `profile_id`;
- look up `tinode_user_id` in connector private credential state;
- if not ready, fail with `PROFILE_NOT_READY`.

Do not auto-create another principal's Tinode credentials as a side effect of the current principal's group action.

**Step 4: Add multi-agent acceptance tests**

Scenarios:

- `default` creates `incident-reporter`;
- both have Tinode private instances;
- both have separate Tinode accounts;
- default sends direct message to incident reporter;
- default creates group and invites incident reporter;
- each agent receives only its own private Tinode events.

**Step 5: Run tests**

```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime session
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli session
node integration\scripts\tinode-smoke.mjs multi-agent
```

**Acceptance:**

- Work fork and principal fork are visibly different operations.
- Principal fork creates registry entry and private Tinode app instance.
- Tinode group chat works between principals.
- Event reminders remain principal-isolated.

**Rollback Point:**

If process spawning is too large, land principal creation plus manual launch instructions first.

## Cross-PR Test Matrix

| Capability | PR Expected | Automated Test | Manual Test |
| --- | --- | --- | --- |
| Existing AIIM still works | PR 1 onward | AppFS/appfs-agent existing tests | AIIM compose smoke |
| Compose public/private policy parse | PR 1 | AppFS compose tests | N/A |
| `create_principal.act` works | PR 2 | runtime supervisor tests | append action manually |
| Private app auto-instantiation | PR 3 | compose/runtime tests | inspect `apps.registry.json` |
| Principal-aware skill listing | PR 5 | appfs-agent runtime tests | `skills --output-format json` |
| Principal-aware event filtering | PR 5 | appfs-agent event tests | two private streams smoke |
| Connector gets `profile_id` | PR 4 | SDK/context tests | connector log smoke |
| Tinode skeleton tree | PR 7 | Tinode connector tests | mount tree listing |
| First send creates credentials | PR 8 | Tinode smoke | Tinode UI shows message |
| Inbound message reminder | PR 9 | Tinode inbound smoke | user sends Tinode message |
| Multi-agent group | PR 10 | Tinode multi-agent smoke | two agents plus group |

## CI Expectations

Before opening each implementation PR:

```powershell
git diff --check
cargo test --manifest-path appfs\cli\Cargo.toml
cargo test --manifest-path appfs\sdk\rust\Cargo.toml
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p tools
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli
```

Tinode-dependent PRs should either:

- run against a configured Tinode endpoint; or
- clearly skip Tinode integration tests when `APPFS_TINODE_ENDPOINT` is missing, while still running unit tests.

Do not make ordinary CI depend on a fragile external Tinode server unless the runner owns that service lifecycle.

## Risk Register

### Risk: Compose schema change breaks existing files

Mitigation:

- add the new fields (`visibility`, `path`, `path_template`, `credential_policy`, `profile_template`) as optional additions to the compose schema;
- update existing compose YAML files (AIIM, Huoyan) to include the new fields in the same PR;
- add tests parsing the updated compose files.

### Risk: `apps.registry.json` becomes ambiguous

Mitigation:

- store app definitions in `app-policies.registry.json`;
- store only materialized app instances in `apps.registry.json`;
- use `instance_id` for uniqueness.

### Risk: Principal registry and derived `.res.json` diverge

Mitigation:

- treat `principals.registry.json` as source of truth;
- always regenerate derived `principals/<id>.res.json` from registry writes;
- appfs-agent reads registry first.

### Risk: appfs-agent sees another principal's private app/events

Mitigation:

- add filtering tests before Tinode private data ships;
- event reminder stream IDs should include private instance identity internally;
- only visible private apps generate skills.

### Risk: Connector credentials leak into model context

Mitigation:

- credential store is not under mount tree;
- events/resources use safe summaries only;
- tests assert secrets are absent from rendered resources and events.

### Risk: Tinode account creation has irreversible side effects

Mitigation:

- no account creation during tree bootstrap;
- credential-required action triggers creation;
- use explicit login prefix and cleanup instructions for smoke accounts.

### Risk: Windows path behavior with non-ASCII contacts

Mitigation:

- connector-generated contact keys must be filesystem-safe;
- use display name plus stable suffix only when safe;
- tests should include Chinese display names and fallback keys.

### Risk: External Tinode CI flakes

Mitigation:

- keep unit tests offline;
- make external integration tests opt-in unless runner manages Tinode service;
- document local Tinode smoke commands.

## Manual Smoke Scripts To Add Later

These are not required in PR 1-6.

### Smoke: default principal startup

```powershell
cargo run --manifest-path appfs\cli\Cargo.toml -- appfs compose up -f appfs\appfs-compose.multi-agent.local.yaml
```

Expected:

- `/_appfs/app-policies.registry.json` exists;
- `/_appfs/apps.registry.json` contains public AIIM only before agent starts;
- first appfs-agent startup creates `default`.

### Smoke: Tinode first send

```powershell
$env:APPFS_PRINCIPAL_ID = "default"
cargo run --manifest-path appfs-agent\rust\Cargo.toml -p rusty-claude-cli -- --dangerously-skip-permissions prompt "给张三发消息说明天开会"
```

Expected:

- `appfs-tinode` skill is loaded;
- one `send_message.act` append happens;
- event reminder reports send success;
- Tinode UI shows message.

### Smoke: principal isolation

```powershell
$env:APPFS_PRINCIPAL_ID = "default"
# start first agent

$env:APPFS_PRINCIPAL_ID = "incident-reporter"
# start second agent
```

Expected:

- both see public AIIM;
- each sees only own private Tinode app;
- each receives only own Tinode private events.

## Cut Lines

Do not include these in v0 unless explicitly re-planned:

- attachments;
- read receipts;
- message reactions;
- push notification settings;
- Tinode moderation/admin flows;
- true OS-level access isolation between principals;
- generic `team` namespace;
- `by-login` path hierarchy;
- replacing AIIM tests with Tinode tests;
- automatic creation of another principal's credentials from a group invite.

## Suggested PR Sequence Summary

1. Docs and plan.
2. Principal/app policy structs and compose parsing.
3. Principal control plane actions.
4. App policies and private app auto-instantiation.
5. Connector context `principal_id`/`profile_id`.
6. appfs-agent principal-aware prompt, skills, and events.
7. Generic credential store/contract.
8. Tinode skeleton tree.
9. Tinode credentials and direct messages.
10. Tinode inbound inbox/events.
11. Principal fork and multi-agent Tinode group chat.

The first five code PRs should be testable without a real Tinode server. That is the main guardrail against scope drift.
