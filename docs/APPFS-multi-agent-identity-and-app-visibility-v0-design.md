# AppFS Multi-Agent Identity And App Visibility v0 Design

## Status

Design proposal before implementing account-backed apps such as Tinode.

This document is about the generic AppFS/appfs-agent identity, visibility, and app registration layer. It intentionally does not finalize the Tinode contact/message tree.

## Goals

1. Keep the current `aiim` demo usable for existing integration tests.
2. Let one project root contain multiple stable agent identities.
3. Keep `attach_id` as a per-run process attach id.
4. Add `principal_id` as the stable semantic agent identity.
5. Let private app instances bind to `principal_id`, not random process instances.
6. Let appfs-agent tell the model who "I" am and which other principals exist.
7. Keep the agent running from the project root instead of forcing it into `/_views/<principal-id>`.
8. Separate app definitions from app instances, so private apps can be auto-instantiated per principal.

## Non-Goals

1. Do not finalize the Tinode contact/message tree in this document.
2. Do not add `team` app visibility in v0.
3. Do not use `by-login` as a generic AppFS concept.
4. Do not claim strong OS security isolation between principals in v0.
5. Do not replace current `aiim` tests while introducing `/public`.

## Current Code Reality

Current implemented pieces:

1. `APPFS_ATTACH_ID` exists and is surfaced by appfs-agent status.
2. `attach_id` may be explicit, but appfs-agent can also generate an ephemeral `attach-*`.
3. `agentfs appfs launch` supports `--attach-id` and `--attach-role`.
4. appfs-agent has `/session fork [branch-name]`, but this forks the session file, not a new AppFS principal or a new child process.
5. appfs-agent tools include subagent/forked skill execution, but those workers are not yet wired to AppFS principal identity.
6. Compose startup currently bootstraps `/_appfs/apps.registry.json` from declared apps.
7. Runtime `register_app.act` can dynamically add an app instance, but callers must provide transport details.
8. appfs-agent currently loads registered apps as `{ app_id, active_scope }` only.
9. appfs-agent event reminder sync currently considers registered app event streams without principal-aware filtering.
10. `ConnectorContext` currently carries app/request/session metadata, but not `principal_id` or `profile_id`.
11. `SubmitActionRequest` currently carries the app-relative action path and payload.
12. Connector error code `AUTH_EXPIRED` exists as the v0 expired-auth surface; automatic token refresh is a future connector enhancement.

Missing pieces for this design:

1. `APPFS_PRINCIPAL_ID`.
2. principal registry.
3. principal-aware system prompt and post-compaction identity context.
4. `/public` and `/private/<principal-id>` namespace.
5. app policy registry.
6. private app auto-instantiation.
7. principal management supervisor handlers.
8. principal-aware fork/spawn integration.
9. principal-aware event filtering.
10. profile-aware connector context and credential lifecycle.

## Identity Model

### `runtime_session_id`

Runtime identity.

Meaning:

1. identifies one AppFS runtime lifecycle;
2. shared by all agents attached to that runtime.

### `attach_id`

Process attach identity.

Meaning:

1. identifies this appfs-agent process instance;
2. may be random or generated each run;
3. useful for logs, events, and debugging;
4. must not be used as the stable app account identity.

The current random/ephemeral behavior should be preserved.

### `principal_id`

Stable semantic agent identity.

Meaning:

1. identifies the agent persona inside the current project;
2. is reused across runs;
3. owns private app instances;
4. is the default key for connector account binding.

Default:

```text
principal_id = APPFS_PRINCIPAL_ID if provided else "default"
```

The first AppFS/appfs-agent run in a project should create a `default` principal if none exists.

### `profile_id`

Optional app-specific identity under one principal.

Recommended format:

```text
<app-id>:<principal-id>
```

Example:

```text
tinode:default
tinode:incident-reporter
```

Generation:

1. if an app policy provides `profile_template`, derive `profile_id` from it when the private app instance is materialized;
2. otherwise, private apps may default to `<app-id>:<principal-id>`;
3. public apps may omit `profile_id` unless they need an app-specific shared profile;
4. materialized `profile_id` should be stored in `apps.registry.json` with the app instance;
5. derived principal views may list profile summaries, but should not become the source of truth.

Example policy:

```json
{
  "app_id": "tinode",
  "visibility": "private",
  "path_template": "private/{principal_id}/tinode",
  "profile_template": "tinode:{principal_id}"
}
```

Example materialization:

```text
principal_id = default
profile_template = tinode:{principal_id}
profile_id = tinode:default
```

`profile_id` is the stable key for connector-owned account state. It is not the same as `attach_id`.

## Responsibility Split

### appfs-agent / launcher

Owns current process declaration:

1. choose or receive `principal_id`;
2. generate or receive `attach_id`;
3. inject `APPFS_ATTACH_ID`, `APPFS_AGENT_ROLE`, and future `APPFS_PRINCIPAL_ID`;
4. inject principal identity into system prompt;
5. reconstruct principal identity context after compaction;
6. expose a tool or command for creating/forking principals.

On attach, appfs-agent should also ensure the selected principal exists before
building its system prompt, skill listing, or event cursor baseline:

1. resolve `principal_id` from `APPFS_PRINCIPAL_ID`, falling back to `default`;
2. if the principal is missing, append `/_appfs/principals/create_principal.act`;
3. wait for `principals.registry.json` to contain the principal;
4. wait for private app policies to materialize into `apps.registry.json`;
5. re-detect the AppFS environment so prompt/skills/events see
   `/private/<principal-id>/<app-id>`.
6. append `/_appfs/principals/attach_principal.act` with the current `attach_id`;
7. on normal process exit, append `/_appfs/principals/detach_principal.act`.

This attach ensure step must not create app-specific upstream credentials. For
example, Tinode account creation still belongs to the Tinode connector's
credential action flow.

AppFS startup should not create the `default` principal just because private app
policies exist. Compose startup owns app definitions and public instances.
Private instances are created only after a principal is explicitly created, most
commonly by appfs-agent attach.

### AppFS runtime supervisor

Owns project-level identity, app policy, and app instance state:

1. persist principal registry;
2. persist app policy registry;
3. persist app instance registry;
4. materialize `/public` and `/private`;
5. route private app instances by `principal_id`;
6. consume principal management actions;
7. auto-instantiate private app instances when principals are created;
8. track best-effort attach leases for running agent processes.

Principal management is an AppFS internal control-plane transaction. It should be handled by the AppFS runtime supervisor, alongside `register_app.act`, `unregister_app.act`, and `list_apps.act`. It should not go through an app connector or bridge.

AppFS should not expose one global `current_identity.res.json`, because two appfs-agent processes may attach to the same mount with different principals at the same time. The current identity is process-local and should come from appfs-agent attach environment/status, then be injected into the system prompt.

### Connectors

Own app-specific authentication:

1. bind credentials to `profile_id` or `principal_id`;
2. never bind long-lived credentials to ephemeral `attach_id`;
3. expose safe identity summaries inside app resources;
4. keep passwords, tokens, and API keys private.

## Namespace v0

Agents continue to run from the project root.

Recommended root shape:

```text
/
  _appfs/
    runtime.json
    app-policies.registry.json
    apps.registry.json
    principals.registry.json
    principals/
      create_principal.act
      update_principal.act
      delete_principal.act
      default.res.json
      <principal-id>.res.json
  public/
    <app-id>/
  private/
    default/
      <app-id>/
    <principal-id>/
      <app-id>/
  aiim/
```

Notes:

1. `/public/<app-id>` is the canonical path for public apps.
2. `/private/<principal-id>/<app-id>` is the canonical path for private apps.
3. `/aiim` remains as a compatibility path for existing tests during migration.
4. `/private` may list all known principals, which helps humans and agents understand project participants.
5. v0 relies on prompt/tool policy to tell an agent to operate on its own private root only.
6. v0 compatibility for `/aiim` should be implemented in appfs-agent skill discovery first by additionally scanning the legacy root-level `/aiim`; do not require a filesystem symlink or AppFS mount-layer alias in P1/P2.

## Visibility Model

Only two app visibility classes exist in v0.

### Public App

Public apps are shared by all principals.

Use for:

1. current `aiim` demo;
2. sample apps;
3. public documentation-like apps;
4. apps with no per-agent credentials.

Policy:

```json
{
  "app_id": "aiim",
  "visibility": "public",
  "path": "public/aiim",
  "legacy_aliases": ["aiim"]
}
```

### Private App

Private apps create one instance per principal.

Use for:

1. Tinode agent accounts;
2. email;
3. calendars;
4. apps with per-agent upstream credentials or data.

Policy:

```json
{
  "app_id": "tinode",
  "visibility": "private",
  "credential_policy": "auto-create",
  "path_template": "private/{principal_id}/tinode",
  "profile_template": "tinode:{principal_id}"
}
```

## App Registration Model

There should be three related but separate concepts:

1. connector configuration;
2. app policy or app definition;
3. app instance.

### Layer 1. Compose Declares Infrastructure And App Policies

Compose should remain the declarative place for connector and transport configuration.

Future compose shape:

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
```

Compose startup should write `/_appfs/app-policies.registry.json`.

Example:

```json
{
  "version": 1,
  "apps": [
    {
      "app_id": "aiim",
      "visibility": "public",
      "connector": "aiim-http",
      "path": "public/aiim",
      "legacy_aliases": ["aiim"]
    },
    {
      "app_id": "tinode",
      "visibility": "private",
      "connector": "tinode-http",
      "credential_policy": "auto-create",
      "path_template": "private/{principal_id}/tinode",
      "profile_template": "tinode:{principal_id}"
    }
  ]
}
```

For public apps, compose startup should also materialize the shared public instance in `apps.registry.json`.

For private apps, compose startup should register the policy/template, but instances are created per principal.

### Layer 2. AppFS Supervisor Materializes App Instances

The AppFS supervisor owns app instance materialization.

Triggers:

1. compose bootstrap for public apps;
2. `register_app.act` for dynamic public apps or admin-defined templates;
3. `create_principal.act` for creating a principal;
4. `attach_principal.act` for marking an appfs-agent process as attached to an existing principal and refreshing/materializing that principal's private app instances.

When `create_principal.act` succeeds, the supervisor should:

1. update `principals.registry.json`;
2. regenerate `principals/<principal-id>.res.json`;
3. materialize `/private/<principal-id>`;
4. read `app-policies.registry.json`;
5. find all apps with `visibility = private`;
6. create one private app instance per private app for that principal;
7. derive each instance `profile_id` from the app policy `profile_template`;
8. inherit connector/transport settings from the app policy;
9. write the materialized instances to `apps.registry.json`;
10. emit principal and app instance events.

This keeps agents from needing to know connector endpoints, transport kinds, retry settings, or healthcheck policy.

Private app instance materialization creates the AppFS instance and profile identity. It does not have to eagerly create upstream credentials. Account creation may be lazy and connector-specific, but it must be keyed by the materialized `profile_id`.

Current v0 behavior:

1. compose startup registers app policies and public app instances only;
2. compose startup should not eagerly create `default` or materialize `/private/default/tinode`;
3. appfs-agent attach/identity ensure creates the selected principal if needed;
4. successful create/attach materializes that principal's private app instances;
5. appfs-agent may then warm up credential-backed private apps through their generic `_app/ensure_credentials.act` when configured to do so.

### Layer 3. appfs-agent Filters Visible Apps

appfs-agent should load app instance metadata and generate skills only for:

1. public instances;
2. private instances whose `principal_id` matches the current process principal;
3. the legacy root-level `/aiim` compatibility path while tests still need it, deduplicated against `/public/aiim`.

It should not generate normal-use skills for another principal's private app instances.

### `apps.registry.json` As Instance Registry

`apps.registry.json` should describe materialized app instances, not only app definitions.

Future instance shape:

```json
{
  "version": 1,
  "apps": [
    {
      "instance_id": "aiim",
      "app_id": "aiim",
      "visibility": "public",
      "path": "public/aiim",
      "transport": {
        "kind": "http",
        "endpoint": "http://127.0.0.1:8080"
      }
    },
    {
      "instance_id": "tinode--default",
      "app_id": "tinode",
      "visibility": "private_instance",
      "principal_id": "default",
      "path": "private/default/tinode",
      "profile_id": "tinode:default",
      "transport": {
        "kind": "http",
        "endpoint": "http://127.0.0.1:6060"
      }
    },
    {
      "instance_id": "tinode--incident-reporter",
      "app_id": "tinode",
      "visibility": "private_instance",
      "principal_id": "incident-reporter",
      "path": "private/incident-reporter/tinode",
      "profile_id": "tinode:incident-reporter",
      "transport": {
        "kind": "http",
        "endpoint": "http://127.0.0.1:6060"
      }
    }
  ]
}
```

Use a filesystem-safe `instance_id`. Avoid using `:` in instance ids or path segments because Windows treats `:` specially.

### Dynamic `register_app.act`

`register_app.act` should remain available, but its v0 role should be narrower:

1. dynamic admin registration for public apps;
2. optional dynamic app policy registration;
3. compatibility with existing tests and clients.

It should not be the normal path for daily private app instantiation. Private app instances should usually be derived from compose/app policies when a principal is created.

## Principal Registry

### Principal Lifecycle Actions

The principal control plane lives under `/_appfs/principals/`:

```text
/_appfs/principals/create_principal.act
/_appfs/principals/update_principal.act
/_appfs/principals/delete_principal.act
/_appfs/principals/attach_principal.act
/_appfs/principals/detach_principal.act
```

`create_principal.act` records durable principal metadata. It is the path used
by explicit `/principal create` and by appfs-agent attach when the selected
principal does not exist.

`attach_principal.act` and `detach_principal.act` record runtime leases for
running appfs-agent processes. They update `active_attaches` and
`active_attach_count` as best-effort runtime status. A successful attach also
refreshes/materializes private app instances for that principal. Detach does
not have to unmount or delete private app instances in v0; whether inactive
private apps are later unloaded is a policy decision.

### No Global Current Identity File

There should be no authoritative `/_appfs/current_identity.res.json` in v0.

Reason:

1. many agents may share one AppFS mount;
2. each process may have a different `principal_id`;
3. a single global file would be overwritten by the last attached process;
4. agents could then read another process's identity by accident.

The current identity should be resolved by appfs-agent from:

1. `APPFS_PRINCIPAL_ID` or `default_principal_id`;
2. `APPFS_ATTACH_ID`;
3. `APPFS_AGENT_ROLE`;
4. `/_appfs/principals.registry.json` for descriptive metadata.

appfs-agent should inject the resolved current identity directly into the system prompt and status output.

### `/_appfs/principals.registry.json`

This file lists all known principals, not only currently running ones.

`default_principal_id` means "fallback principal when the current process did not receive an explicit `APPFS_PRINCIPAL_ID`".

`principals.registry.json` is the source of truth for principal metadata in v0.

`/_appfs/principals/<principal-id>.res.json` is a derived view generated from the registry for convenient inspection. If the registry and a derived file disagree, the registry wins.

```json
{
  "version": 1,
  "default_principal_id": "default",
  "principals": [
    {
      "principal_id": "default",
      "display_name": "Default agent",
      "description": "The default project agent.",
      "kind": "agent",
      "created_at": "2026-05-06T00:00:00Z",
      "updated_at": "2026-05-06T00:01:00Z",
      "active_attach_count": 1,
      "active_attaches": [
        {
          "attach_id": "attach-1778300000-1234-1",
          "role": "coordinator",
          "session_id": "session-1778300000-0",
          "attached_at": "2026-05-06T00:01:00Z",
          "last_seen_at": "2026-05-06T00:01:00Z"
        }
      ]
    },
    {
      "principal_id": "incident-reporter",
      "display_name": "Incident reporter",
      "description": "Summarizes incidents and sends chat updates.",
      "kind": "agent",
      "created_at": "2026-05-06T00:08:00Z",
      "updated_at": "2026-05-06T00:08:00Z",
      "active_attach_count": 0
    }
  ]
}
```

Do not store `private_root` as authoritative data. It is derived as:

```text
private_root = /private/<principal_id>
```

`active_attach_count` is derived from `active_attaches.length`. It is
best-effort runtime status, not a security boundary. If an agent crashes before
writing `detach_principal.act`, the lease may remain stale until a future
heartbeat/TTL cleanup pass removes it.

### `/_appfs/principals/<principal-id>.res.json`

Derived view example:

```json
{
  "principal_id": "incident-reporter",
  "display_name": "Incident reporter",
  "description": "Summarizes incidents and sends chat updates.",
  "kind": "agent",
  "private_root": "private/incident-reporter",
  "profiles": {
    "tinode": "tinode:incident-reporter"
  }
}
```

## Principal Creation Methods

There should be several ways to create or select a `principal_id`.

### Method 1. Automatic Default Principal

When no principal exists:

1. create `default`;
2. set display name to `Default agent`;
3. set description to `The default project agent`;
4. launch current appfs-agent as `default`.

This covers normal single-agent usage.

Default creation must be idempotent. If two agents race to create `default`, one creation should win and the other should treat the existing `default` as success, not as a fatal duplicate error.

### Method 2. Explicit Launch Principal

Future launcher surface:

```powershell
agentfs appfs launch `
  .agentfs/runtime.db `
  C:\mnt\appfs `
  --principal-id incident-reporter `
  --principal-name "Incident reporter" `
  --principal-description "Summarizes incidents and sends chat updates." `
  --agent-role reporter `
  --agent-bin C:\path\to\claw.exe `
  -- --dangerously-skip-permissions
```

Behavior:

1. create the principal if missing;
2. reuse it if already present;
3. create a fresh `attach_id` for this process;
4. inject the chosen `principal_id` into the launched process.

`last_seen_at` and `active_attach_count` should not be required in P1/P2. In the later principal-aware launcher/fork phase, the launcher may update them as best-effort runtime status.

### Method 3. AppFS Control Action

Expose a project-level action:

```text
/_appfs/principals/create_principal.act
```

Example:

```json
{
  "principal_id": "incident-reporter",
  "display_name": "Incident reporter",
  "description": "Summarizes incidents and sends chat updates.",
  "kind": "agent",
  "client_token": "create-incident-reporter"
}
```

This action is useful for tools, tests, and manual debugging.

Consumer:

1. AppFS runtime supervisor consumes this action;
2. no app connector is involved;
3. successful handling updates `principals.registry.json`;
4. the supervisor regenerates `principals/<principal-id>.res.json`;
5. the supervisor auto-instantiates private app instances from `app-policies.registry.json`.

Create semantics should be idempotent when the existing principal has the same desired metadata. Conflicting metadata should produce a clear `principal.create.failed` event unless the caller uses `update_principal.act`.

### Method 4. appfs-agent Tool-Driven Principal Fork

Add or adapt an appfs-agent tool/command that lets the current agent create a semantic child principal.

Example user intent:

```text
创建一个 agent 专门负责事故通知
```

Expected model/tool behavior:

1. choose semantic `principal_id`, such as `incident-notifier`;
2. write `/_appfs/principals/create_principal.act` or call a dedicated tool that does the same;
3. fork or compact the current conversation if needed;
4. start or schedule a child agent with `APPFS_PRINCIPAL_ID=incident-notifier`;
5. give the child a task-specific system/user prompt.

Method 4 should build on Method 3. The tool-driven principal fork should create the principal through the same `create_principal.act` path before starting a child process, so registry state and process lifecycle use one consistent control-plane entry.

### Method 5. Manual Admin Creation

An operator may append to `create_principal.act` directly.

This is useful for testing, but not the normal UX.

### Method 6. Principal Deletion

Expose a project-level action:

```text
/_appfs/principals/delete_principal.act
```

Example:

```json
{
  "principal_id": "incident-reporter",
  "client_token": "delete-incident-reporter"
}
```

Current v0 deletion policy:

1. `delete_principal.act` removes the principal from `principals.registry.json`;
2. it deletes the derived `principals/<principal-id>.res.json` view;
3. it does not remove `/private/<principal-id>` filesystem data in v0;
4. it best-effort asks affected private app instances to clean connector credentials through their app-specific cleanup action;
5. deleting an actively attached principal is allowed by the current low-level action and is expected to be guarded by future interactive tooling.

Not implemented in v0:

1. `delete_private_data` flag;
2. archived/tombstoned principal records;
3. force checks for active principals;
4. physical deletion of private app data.

## Fork Types

The word "fork" must be split into two different concepts.

### Work Fork

A work fork is task delegation under the same principal.

Examples:

1. current appfs-agent `Agent` tool;
2. current forked skill execution;
3. background subagent used for investigation or verification;
4. `/session fork` when it only copies conversation state.

Rules:

1. inherits the current `principal_id`;
2. should not create a new private app identity;
3. may reuse the same private app credentials;
4. is appropriate for helper work under the same agent persona.

### Principal Fork

A principal fork creates a new semantic agent identity.

Rules:

1. creates a new `principal_id`;
2. creates a new ephemeral `attach_id` for the child process;
3. creates a new private root at `/private/<principal-id>`;
4. auto-instantiates private apps for that principal;
5. private account-backed apps create or bind new per-principal accounts/profiles;
6. public apps remain shared.

If the user does not provide a name, the default agent should choose a semantic id from the task.

Examples:

1. `incident-notifier`
2. `meeting-scheduler`
3. `code-reviewer`
4. `research-assistant`

Current code note:

1. `/session fork` already copies session state and lineage, but it does not create a new process or principal.
2. subagent tools can spawn background work, but they do not yet attach a new AppFS principal.
3. principal-aware fork should be new integration work on top of these existing primitives.

## System Prompt Requirements

appfs-agent should inject an AppFS identity section when running inside an AppFS project.

It should include:

1. current `principal_id`;
2. current display name and description;
3. current `attach_id`;
4. derived private root path;
5. known principals summary;
6. explanation of `/public` and `/private`;
7. warning to use only the current principal's private app paths unless explicitly asked to inspect another principal.

Example:

```text
You are attached to an AppFS project.

Current agent identity:
- principal_id: default
- display_name: Default agent
- description: The default project agent.
- attach_id: attach-20260506-001
- private_root: /private/default

Known project principals:
- default: Default agent. The default project agent.
- incident-reporter: Incident reporter. Summarizes incidents and sends chat updates.

AppFS app layout:
- /public contains apps shared by all principals.
- /private/<principal_id> contains private app instances for each principal.
- Your private app root is /private/default.
- Do not operate on another principal's private app unless the user explicitly asks.
```

Compaction requirement:

1. AppFS identity context must be reintroduced after conversation compaction;
2. post-compaction context should include the same current identity summary or trigger the normal prompt injection path before the next model call;
3. losing principal identity after compaction is a correctness bug for private app usage.

## AppFS-Agent Skill Discovery

Skill listing should use:

1. all public apps under `/public`;
2. private apps under `/private/<current-principal-id>`;
3. legacy `/aiim` compatibility path while tests still depend on it.

Skill listing should not generate normal-use skills for another principal's private apps.

It may mention other principals as project context, but it should not advertise their private app actions as available skills.

Deduplication rule:

1. if `/public/aiim` exists, prefer it as canonical;
2. if root `/aiim` also exists, treat it as a compatibility alias;
3. do not emit two `appfs-aiim` skills for the same app.

Implementation note:

`AppfsRegisteredApp` and related appfs-agent app discovery structures should grow beyond `{ app_id, active_scope }`.

Recommended future fields:

1. `instance_id`
2. `app_id`
3. `visibility`
4. `principal_id`
5. `path`
6. `profile_id`
7. `active_scope`

## Event Reminder Filtering

appfs-agent should not subscribe to every registered app event stream once private app instances exist.

Rules:

1. subscribe to platform events that are safe for all principals;
2. subscribe to public app event streams;
3. subscribe to private app event streams only when `principal_id == current_principal_id`;
4. do not inject another principal's private app events into the current agent's `<system-reminder>`.

This requires appfs-agent to understand app instance `visibility`, `principal_id`, and `path`.

### appfs-agent Event Routing Status

The earlier broad AppFS event loop MVP has been disabled. In practice it woke the
model on every current-principal event, including self-generated receipts such
as `message.sent` and `action.completed`, which could create duplicate turns
after the model had already received the same receipt at a normal model-call
boundary.

The baseline behavior is still **model-call boundary injection**:

1. connector inbound work brings upstream events into AppFS, for example Tinode
   new messages become `_stream/events.evt.jsonl` records;
2. before each model call, appfs-agent reads current principal-visible AppFS
   event streams;
3. new records are injected as normal AppFS event reminders in the current turn;
4. context-only events such as action receipts are visible to the active turn
   without automatically starting another turn while the agent is idle.

The disabled command surfaces are:

```text
claw appfs-events watch ...
claw --watch-appfs-events
```

Safe idle wake is reintroduced through an explicit opt-in surface:

```text
claw --appfs-idle-wake
```

This mode uses a separate wake cursor from the normal model-facing
`appfs_event_cursors`. On first scan it baselines existing events so historical
backlog does not wake a newly started agent. Subsequent scans classify events
before delivery:

1. `message.received` with `content.requires_attention=true` may wake an idle
   agent;
2. `message.sent`, `action.completed`, `action.failed`, and credential status
   events remain context-only and are delivered at normal model-call boundaries;
3. noisy bookkeeping events such as `inbox.updated` are skipped while still
   advancing cursors;
4. private app events remain filtered to the current `principal_id`.

The v0 idle wake implementation runs at safe REPL idle boundaries. It is not a
fully non-blocking terminal editor and does not interrupt an in-progress
model/tool turn. It also does not replace connector inbound polling; Tinode still
uses connector-side inbound polling to bring upstream messages into AppFS before
appfs-agent can classify them.

Current non-goals:

1. appfs-agent still does not read Tinode directly;
2. `_stream/events.evt.jsonl` remains an event stream, not a read-through resource;
3. `apps.<app-id>.inbound_poll_ms` remains connector-side inbound polling;
4. appfs-agent does not subscribe to another principal's private events;
5. idle wake does not wake for self-generated receipts or every filesystem event;
6. fully asynchronous user-input interruption is deferred to a later router phase.

Follow-up design:

1. `docs/plans/2026-05-09-appfs-agent-event-boundary-and-idle-wake.md`
2. `docs/plans/2026-05-09-appfs-agent-unified-input-router-implementation.md`

## Connector Authentication Binding

Connectors should bind credentials in this order:

```text
profile_id if present else principal_id
```

Never bind long-lived connector credentials to `attach_id`.

Private connector state should be keyed by `profile_id`.

Private connector state may store:

1. `profile_id -> upstream account id`;
2. `profile_id -> token or password`;
3. `profile_id -> cursors`;
4. `profile_id + client_token -> idempotent action result`.

App resources may expose safe summaries:

1. upstream user id;
2. display name;
3. account status;
4. profile id.

App resources must not expose secrets:

1. passwords;
2. auth tokens;
3. API keys;
4. cookies.

### Credential Storage

Connector credentials must not live in the AppFS tree visible to models.

The AppFS filesystem may expose safe account summaries, but it must not expose token material.

Allowed storage choices:

1. connector-owned SQLite, local file, or OS secret store;
2. AppFS runtime DB secret key-value store, if the connector runs in-process or the runtime provides an explicit secret access API;
3. in-memory connector state for tests only.

Recommended v0 default:

1. external HTTP or gRPC bridge connectors should own their credential store;
2. in-process connectors may use AppFS runtime DB as an implementation detail;
3. AppFS registries should store `profile_id` and safe summaries, not secrets.

Possible secret key shape when AppFS runtime DB is used:

```text
connector:<connector-id>:profile:<profile-id>:credentials
```

Possible value shape:

```json
{
  "profile_id": "tinode:default",
  "upstream_user_id": "usrAbCdEf",
  "access_token": "<secret>",
  "refresh_token": "<secret>",
  "expires_at": "2026-05-07T12:00:00Z",
  "created_at": "2026-05-06T12:00:00Z",
  "updated_at": "2026-05-06T12:00:00Z"
}
```

This value is secret data. It must not be rendered into `*.res.json`, `*.evt.jsonl`, skill text, system prompt, or normal debug logs.

### Profile Context In Connector Calls

Future connector calls should receive profile context from AppFS, not from model-authored action payloads.

Recommended future `ConnectorContext` additions:

1. `instance_id`;
2. `visibility`;
3. `principal_id`;
4. `profile_id`;
5. `profile_state` or safe account status, if needed.

The action path still belongs in `SubmitActionRequest`. The identity and profile should come from the app instance registry and call context.

This prevents a model from writing an action payload that claims another principal's `profile_id`.

### Ensure Credentials Action

Private account-backed apps expose a standard app-level action:

```text
/private/<principal-id>/<app-id>/_app/ensure_credentials.act
```

Whether this action is **required** or **optional** depends on the app's credential policy, which must be declared in the app policy and reflected in the generated skill.

#### Two Credential Policies

**Policy A: auto-create (e.g. Tinode)**

The connector can create upstream accounts on its own, without user-provided secrets.

- `ensure_credentials.act` is **optional**.
- First business action transparently triggers credential creation if none exist.
- The skill says: "Directly write business actions. Credentials are created automatically on first use."

**Policy B: external-credential (e.g. email via SMTP, calendar via OAuth)**

The connector needs user-provided credentials (API key, OAuth token, SMTP password) and cannot create accounts on its own.

- `ensure_credentials.act` is **required** before business actions.
- The skill says: "Before using this app, call ensure_credentials.act with `credential_ref` pointing to your credentials."
- If the agent calls a business action before credentials exist, the connector returns an error with a clear message directing the agent to call `ensure_credentials.act` first.

The policy is declared in the app's compose entry or `app-policies.registry.json`:

```json
{
  "app_id": "tinode",
  "visibility": "private",
  "credential_policy": "auto-create",
  "profile_template": "tinode:{principal_id}"
}
```

```json
{
  "app_id": "email-sender",
  "visibility": "private",
  "credential_policy": "external-credential",
  "profile_template": "email:{principal_id}",
  "credential_help": "Provide an SMTP username and password via env var APPFS_EMAIL_CREDENTIALS"
}
```

#### Payload

The connector must derive the effective `profile_id` from the app instance context. If the payload includes an `expected_profile_id`, it is only a guardrail and must match the context-derived `profile_id`.

For `auto-create` apps:

```json
{
  "expected_profile_id": "tinode:default",
  "client_token": "ensure-tinode-default"
}
```

For `external-credential` apps:

```json
{
  "expected_profile_id": "email:default",
  "credential_ref": "env:APPFS_EMAIL_CREDENTIALS",
  "client_token": "ensure-email-default"
}
```

#### Rules

1. raw passwords, raw tokens, and refresh tokens should not be written into `.act` payloads;
2. use `credential_ref`, connector configuration, OS secret store, or automatic upstream account creation instead;
3. `auto-create` apps may omit `credential_ref`;
4. `external-credential` apps require `credential_ref` and should document the expected env var or secret path in the skill and `credential_help`;
5. ensure actions should be idempotent by `profile_id` and `client_token`;
6. ensure actions should not require the agent to know connector transport details.

#### Skill Representation

The generated skill for each app must reflect the credential policy:

**Auto-create app skill excerpt**:

```markdown
## Authentication
Credentials are created automatically on first use. You do not need to call ensure_credentials.act before using this app.
```

**External-credential app skill excerpt**:

```markdown
## Authentication
This app requires user-provided credentials before first use.

Before any business action, call:
```
printf '%s\n' '{"credential_ref":"env:APPFS_EMAIL_CREDENTIALS","client_token":"..."}' \
  >> /private/default/email-sender/_app/ensure_credentials.act
```

The credential should contain: SMTP username and password.
Set via environment variable: APPFS_EMAIL_CREDENTIALS
```

The Tinode-specific idea `ensure_agent.act` should be treated as an app-specific name for this generic concept. Prefer `_app/ensure_credentials.act` for new account-backed private apps.

### Credential Lifecycle

Profile lifecycle:

1. derive `profile_id` from app policy when a private app instance is materialized;
2. store the materialized `profile_id` in `apps.registry.json`;
3. **first credential-requiring business action triggers credential creation for `auto-create` apps**: when a connector receives a business action for a `profile_id` that has no stored credentials, the connector transparently creates or binds upstream credentials before executing the original action;
4. use credentials by looking up connector private state with `profile_id` on every connector call;
5. refresh credentials before upstream calls when close to expiry;
6. emit a safe event when credentials become ready, expire, or fail;
7. clean up or archive credential state when `delete_principal.act` requests principal deletion.

Credential creation must not be triggered by passive instance materialization, skill discovery, or app tree bootstrap unless the app policy explicitly marks that read as credential-required. In particular, `get_app_structure` during private app auto-instantiation should be able to return a safe skeleton without creating upstream accounts. This preserves the expected flow where a newly materialized private app may exist before any upstream credentials exist.

Typical credential-requiring business operations:

1. `submit_action` for actions that need an upstream account;
2. identity-scoped `fetch_snapshot_chunk` or `fetch_live_page` calls that truly require upstream auth.

**First-use auto-creation** is the primary credential bootstrapping path. An agent does not need to call a separate `ensure_credentials.act` before using a private app. When connector receives the first business action and finds no credentials for the call's `profile_id`, it creates credentials, stores them, and then proceeds with the original action. From the agent's perspective, the first message sent just works.

`_app/ensure_credentials.act` remains available as an optional pre-check: when an agent wants to verify or pre-create credentials without performing a business action (for early error detection or status display), it may call ensure_credentials. But it is not required for normal use.

**Agent experience (no separate credential step):**

```
用户: "给张三说明天开会"                  ← 第一个 Tinode 操作

模型: bash write send_message.act        ← 直接发业务 action
  ↓
connector: submit_action(profile_id="tinode:default", path="contacts/zhangsan/send_message.act")
  → 检查私有存储: "tinode:default" 无凭据
  → 自动创建 Tinode 账号, 获取 token, 保存
  → 用新 token 执行 send_message
  → 返回成功
  → 发射 profile.credentials.ready event

模型看到 event → 知道账号已就绪 → 回复用户 "已发送"
```

This eliminates the two-step "先注册再发消息" ceremony. The first business action may be slightly slower due to credential creation, but subsequent calls are fast.

Credential creation failure is reported as action failure with `AUTH_EXPIRED` or a connector-specific error. The agent may retry the same business action later; if the upstream account creation was a one-time failure (network, rate limit), retries are safe because credential creation is idempotent by `profile_id`.

Refresh behavior:

1. connector may return or emit `AUTH_EXPIRED` when upstream authentication is rejected;
2. automatic silent token refresh is not implemented in the Tinode connector yet;
3. appfs-agent may surface the failure and, if appropriate, call `_app/ensure_credentials.act` again or retry the original business action later;
4. a future connector revision may add silent refresh before `submit_action`, snapshot fetch, live fetch, and structure refresh without changing the AppFS tree contract.

Safe event examples:

```json
{"type":"profile.credentials.ready","principal_id":"default","profile_id":"tinode:default","upstream_user_id":"usrXXX"}
{"type":"profile.credentials.failed","principal_id":"default","profile_id":"tinode:default","error":"upstream auth rejected"}
{"type":"profile.credentials.expired","principal_id":"default","profile_id":"tinode:default","recoverable":true}
```

Events must not include access tokens, refresh tokens, passwords, cookies, or API keys.

### Credential Cleanup

`delete_principal.act` defines best-effort connector credential cleanup.

Current v0 behavior:

1. AppFS removes the principal record from `principals.registry.json`;
2. AppFS keeps `/private/<principal-id>` data in the mounted tree;
3. AppFS finds affected private app runtime instances for that principal;
4. AppFS invokes the app-specific cleanup action when available;
5. cleanup failures are reported in the action result but do not roll back principal deletion.

For Tinode v0, the concrete cleanup action is `/_app/forget_credentials.act`, which removes connector-private state keyed by the current `profile_id` and clears the shared principal-to-profile mapping.

Future cleanup policy may add archived principal records, a `delete_private_data`
flag, force checks for active principals, and physical private-data deletion.

## Compatibility With Current `aiim`

Migration approach:

1. canonical public path should become `/public/aiim`;
2. root `/aiim` remains as a compatibility path for current tests;
3. no Tinode behavior should be added to `aiim`;
4. new real chat connector should register as `tinode`.

v0 alias implementation:

1. do not require Windows symlinks;
2. do not require an AppFS mount-layer alias at first;
3. appfs-agent should keep a compatibility scan for root-level `/aiim` when generating skills;
4. once `/public/aiim` is stable and tests migrate, the root-level `/aiim` compatibility path can be removed or downgraded.

Longer term:

1. migrate tests to `/public/aiim`;
2. remove or downgrade `/aiim` alias after the public namespace is stable.

## Phased Implementation Plan

### Current Implementation Status

As of the current workspace state, P2 is not a fresh starting point anymore:

1. P1 principal metadata is implemented: `APPFS_PRINCIPAL_ID`, default principal fallback, status output, `principals.registry.json`, and per-principal record views all exist.
2. P2 principal management is implemented: `create_principal.act`, `update_principal.act`, `delete_principal.act`, `attach_principal.act`, `detach_principal.act`, supervisor handlers, idempotent create, and best-effort attach leases are present.
3. P2.5 app policies and auto-instantiation are implemented: compose writes app policies, public app instances are materialized at compose startup, and private app instances are materialized when a principal is created or attached.
4. P3 and P4 are mostly implemented in the current appfs-agent path: public/private app metadata is visible to appfs-agent, skill discovery filters to public apps plus the current principal's private apps, and event reminders filter private streams by current principal.
5. P5 principal-aware fork exists as a CLI/session workflow, but process launching is still manual-guided rather than a fully managed launcher.
6. P6 Tinode/private-account app support is the active area for product polish: the core direct-message, credential, inbound, inbox, and multi-agent smoke paths work. `AUTH_EXPIRED` exists as an error surface, but automatic token refresh is not yet implemented.
7. `delete_principal.act` currently deletes the registry entry immediately and best-effort invokes connector credential cleanup. It does not yet implement archived principal records, `delete_private_data`, force checks, or physical private-data deletion.

### P0. Keep Existing Behavior Stable

1. Keep `aiim` tests green.
2. Keep existing attach contract green.
3. Do not introduce Tinode behavior into `aiim`.

### P1. Add Principal Metadata

1. Add `APPFS_PRINCIPAL_ID`.
2. Default missing principal to `default_principal_id`, initially `default`.
3. Add status output for current principal.
4. Add `/_appfs/principals.registry.json`.
5. Add derived `/_appfs/principals/<principal-id>.res.json`.
6. Do not add an authoritative global `current_identity.res.json`.
7. Treat `principals.registry.json` as principal metadata source of truth.

### P2. Add Principal Management

1. Add `/_appfs/principals/create_principal.act`.
2. Add `/_appfs/principals/update_principal.act`.
3. Add `/_appfs/principals/delete_principal.act`.
4. Add AppFS supervisor handlers for those principal actions.
5. Make principal creation idempotent.
6. Materialize `/private/<principal-id>`.
7. Add `/_appfs/principals/attach_principal.act`.
8. Add `/_appfs/principals/detach_principal.act`.
9. Keep `active_attach_count` derived from best-effort attach leases.

### P2.5. Add App Policies And Auto-Instantiation

1. Add `/_appfs/app-policies.registry.json`.
2. Extend compose schema with `visibility`, `path`, and `path_template`.
3. Compose startup writes app policies.
4. Compose startup materializes public app instances.
5. `create_principal.act` auto-instantiates private app instances from policies.
6. Private app instances inherit connector/transport from app policies.
7. Do not require agents to provide transport for private apps.

### P3. Add Public/Private App Namespace

1. Add `/public/<app-id>`.
2. Add `/private/<principal-id>/<app-id>`.
3. Keep `/aiim` as compatibility path.
4. Deduplicate `/aiim` and `/public/aiim` in skill listing.
5. Make `apps.registry.json` an app instance registry.

### P4. Update appfs-agent Prompt, Skills, And Events

1. Inject current identity into system prompt.
2. Reinject identity context after compaction.
3. Inject known principals summary.
4. Generate skills from `/public` and current `/private/<principal-id>`.
5. Avoid generating normal-use skills for other principals' private apps.
6. Extend `AppfsRegisteredApp` with visibility/path/principal metadata.
7. Filter event reminder streams by current principal.

### P5. Add Principal-Aware Fork/Spawn

1. Keep work forks as same-principal delegation.
2. Build principal forks on existing `/session fork` and subagent spawn primitives.
3. Create the principal through `create_principal.act` before starting the child process.
4. Optionally compact inherited context.
5. Start child agent with `APPFS_PRINCIPAL_ID`.
6. Let launcher/fork lifecycle update `last_seen_at` and `active_attach_count` as best-effort runtime status.

### P6. Add Private Account-Backed Apps

1. Extend connector call context with `instance_id`, `principal_id`, and `profile_id`.
2. Implement transparent first-use credential creation: when a connector receives a business action for a `profile_id` with no stored credentials, create or bind credentials before executing the original action.
3. Implement optional `_app/ensure_credentials.act` for pre-check without business action.
4. Implement `tinode` as a new private app.
5. Bind Tinode credentials to `tinode:<principal-id>`.
6. Store Tinode tokens in connector private state, not in the AppFS tree.
7. Surface `AUTH_EXPIRED`; silent token refresh is a future connector enhancement.
8. Design the Tinode app tree separately.

**Acceptance test (Tinode first-message flow):**

```text
Given: AppFS compose running with tinode as private app.
       appfs-agent has attached as principal "default".
       private/default/tinode has been materialized by create/attach.
       No Tinode credentials exist yet.
       Model skill listing includes appfs-tinode.

When:  User says "给张三说明天开会"
Then:
 1. Model writes the current Tinode send-message action
    such as private/default/tinode/contacts/zhangsan/send_message.act
    with payload {"text":"明天十点开会"}.
    Model does NOT write a separate ensure_credentials.act first.
 2. Connector receives submit_action with profile_id="tinode:default".
    Connector finds no stored credentials for tinode:default.
    Connector transparently creates Tinode account, stores token.
 3. Connector proceeds with send_message using the new token.
 4. Connector returns success.
 5. Adapter emits profile.credentials.ready and action completion events.
 6. Model sees event in next <system-reminder>, replies "已发送".
```

## Key Risks

1. v0 private directories are semantic boundaries, not hard OS security boundaries.
2. Agents can technically inspect `/private/<other-principal>` unless stronger filtering is added later.
3. Poorly chosen principal ids may create confusing long-lived identities.
4. Fork-heavy workflows may create too many private app accounts; v0 should provide `delete_principal.act` for manual cleanup, with connector credential cleanup handled per connector.
5. Prompt and skill listing must be consistent with registry files, or the model will get confused.
6. Compose currently assumes one connector per app; private per-principal instances require treating compose entries as policies/templates rather than one static runtime app only.
7. Event reminder filtering must be implemented before private apps carry sensitive user data.
8. Writing raw credentials into `.act` files would leak secrets into model-visible and persistent filesystem state; account-backed apps must use secret references or connector-owned automatic account creation.
9. If credential storage is connector-owned, backup, migration, and revocation become connector responsibilities and must be tested per connector.

## Recommendation

Adopt this v0 model:

```text
attach_id = per-run process instance
principal_id = stable semantic agent identity, default "default"
public app definitions = compose/app-policies
private app definitions = compose/app-policies
public app instances = /public/<app-id>
private app instances = /private/<principal-id>/<app-id>
```

Do not introduce `team` or `/_views` in v0.

Keep appfs-agent at the project root, inject identity into the prompt, and generate skills from public apps plus the current principal's private apps.
