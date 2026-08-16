# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

appfs-platform is an **integration monorepo** that brings together two separate layers plus tooling:

- **appfs** (source repo: `agentfs`) — a filesystem protocol, runtime supervisor, connectors (Tinode, gRPC, HTTP bridges), and SDKs for AI agent app state
- **appfs-agent** (source repo: `claw-code`) — an interactive agent runtime (Claude-style) that attaches to an AppFS mount, reads app state, writes actions, and reacts to event streams
- **dashboard/** — debug dashboard: Fastify server + React frontend for inspecting agent sessions, timelines, and debug dumps
- **desktop/** — Electron shell that packages the dashboard with AppFS and agent binaries
- **integration/** — cross-project scripts, fixtures, end-to-end smoke tests, and contracts

The standalone repositories are the source of truth for component internals. This monorepo owns integration assets, end-to-end scenarios, and cross-project documentation.

## Build & Verify

### appfs-agent (Rust workspace)

```bash
cd appfs-agent/rust
cargo check --workspace
cargo test --workspace --no-fail-fast
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

Run a single test: `cargo test --package <crate> -- <test_name>`

Workspace lints: `unsafe_code = "forbid"`, clippy `all` + `pedantic` at warn.

9 crates under `appfs-agent/rust/crates/`: `runtime`, `api`, `tools`, `commands`, `rusty-claude-cli`, `plugins`, `telemetry`, `mock-anthropic-service`, `compat-harness`.

### appfs (Rust CLI + SDK)

```bash
cd appfs/cli
cargo build
cargo test
cargo test --package agentfs
```

Run a single test: `cargo test -- <test_name>`

### appfs TypeScript SDK

```bash
cd appfs/sdk/typescript
npm install && npm run build && npm test
```

### appfs Python SDK

```bash
cd appfs/sdk/python
uv sync && uv run pytest
```

### appfs-agent Python layer

```bash
cd appfs-agent
python -m unittest discover -s tests -v
```

### Dashboard

```bash
cd dashboard
npm install
(cd server && npm install)

# Type-check server
cd dashboard/server && npx tsc --noEmit

# Build frontend
cd dashboard && npx vite build

# Dev mode (server + client concurrently)
cd dashboard && npm run dev
```

Server runs on `http://localhost:3100` by default. SSE events at `/api/events`. REST endpoints: `/api/agents`, `/api/timeline`, `/api/messages`, `/api/principals`.

### Desktop

```bash
cd desktop
npm install
npm run build       # TypeScript compile
npm run start       # Build + launch Electron
npm test            # Run tests via tsx
npm run dist        # Production package via electron-builder
```

The desktop shell embeds the dashboard server and ships `agentfs` and `claw` binaries from `desktop/bin/`.

## Integration Smoke Tests

All smoke scripts are in `integration/scripts/`. They exercise both AppFS and appfs-agent together and require local infrastructure (WinFsp on Windows, FUSE/NFS on Linux/macOS).

| Script | Checkpoint | What it covers |
|--------|-----------|----------------|
| `test-windows-appfs-agent-smoke.ps1` | IC-0 | Basic mount + agent attach + status |
| `test-windows-appfs-agent-http-demo.ps1` | IC-1 | HTTP bridge + real agent prompt + action roundtrip |
| `test-windows-appfs-agent-multi-attach.ps1` | IC-2 | Two agents on same mount, distinct attach IDs |
| `test-windows-appfs-agent-launcher.ps1` | IC-3 | Joint AppFS + agent startup via launcher |
| `test-windows-appfs-tinode-multi-agent-smoke.ps1` | Tinode v0 | Multi-agent principals, private apps, credential warmup, cross-principal messaging |
| `test-unix-appfs-agent-smoke.sh` | IC-0 (Unix) | Linux FUSE / macOS NFS variant of basic smoke |

## Architecture

### Two-Layer Separation

```
appfs (filesystem layer)          appfs-agent (agent layer)
─────────────────────────         ─────────────────────────
compose / app policies            attach / detect AppFS
runtime supervisor                system prompt + skill listing
mount runtime + bridges           input router / event reminders
mounted app tree                  interactive turn loop (REPL)
_stream/events.evt.jsonl          shell + file tools
```

Agent reads/writes the mounted app tree as regular files. AppFS processes actions written to `*.act` files and emits events to `_stream/events.evt.jsonl`. The agent's input router consumes those events and injects them as pending input into the turn loop.

### appfs Key Components

- **CLI** (`appfs/cli/`) — `agentfs` binary; init, mount (FUSE/NFS/WinFsp), compose up, and the `appfs` subcommands
- **Runtime Supervisor** (`appfs/cli/src/cmd/appfs/`) — the live control plane that runs while a mount is active. A `tokio::select!` loop consumes `.act` action files via `action_consumer.rs` → `action_dispatcher.rs`, dispatches to `runtime_supervisor.rs` handlers, and emits events. Subsystems: `registry.rs`/`registry_manager.rs` (principal & app registries), `runtime_manifest.rs` (publishes `runtime.json`), `mount_runtime.rs`, `compose.rs`, plus per-connector adapters (`grpc_bridge_adapter.rs`, `http_bridge_adapter.rs`)
- **SDK** (`appfs/sdk/rust/src/`) — `AgentFS` struct, filesystem/KV/tool-call layers, connectors
- **Connectors** — `appfs_connector.rs` (SDK abstraction), `tinode_connector.rs` (WebSocket chat bridge), `grpc_bridge_adapter.rs` (tonic), `http_bridge_adapter.rs`
- **Compose** (`appfs/appfs-compose.*.yaml`) — per-app deployment configs (aiim, huoyan, tinode; `.local.yaml` variants point at local infra)
- **Tree Sync** (`appfs/cli/src/cmd/appfs/tree_sync.rs`) — initializes and refreshes mounted app structure

### appfs-agent Key Components

Rust workspace at `appfs-agent/rust/crates/`:

- **runtime** — the core crate; the largest subsystem. Beyond the obvious entry points it groups into several cross-cutting areas:
  - **AppFS integration** — `appfs.rs` (detect env, create/attach/heartbeat/detach principal, event polling, private-app warmup), `input_router.rs` (input source routing)
  - **Session & model loop** — `session.rs`, `conversation.rs`, `prompt.rs`, `compact.rs` + `summary_compression.rs` + `context.rs` (auto-compaction / context accounting)
  - **Task collaboration** — `task_board.rs`, `task_registry.rs`, `task_packet.rs`, `team_cron_registry.rs`, `worker_boot.rs` (multi-worker headless agents, shared task queues)
  - **Recovery & policy** — `recovery_recipes.rs`, `policy_engine.rs`, `sandbox.rs`, `permission_enforcer.rs`, `permissions.rs`
  - **MCP** — `mcp_client.rs`, `mcp_server.rs`, `mcp_stdio.rs`, `mcp_lifecycle_hardened.rs`, `mcp_tool_bridge.rs` (acts as both MCP client and server)
  - **Git integrity** — `branch_lock.rs`, `stale_base.rs`, `stale_branch.rs`, `green_contract.rs`, `lane_events.rs` (lane/branch freshness policy)
  - **Shell tools** — `bash.rs`, `windows_shell.rs`, `shell_cwd.rs`, `file_ops.rs`
- **api** — LLM client abstraction (Anthropic and others), streaming, prompt cache
- **tools** — file operations, shell, lane completion
- **commands** — slash commands, skills dispatch
- **rusty-claude-cli** — CLI entry point; hosts the interactive REPL and the `--headless` mode used by the dashboard to spawn agents
- **plugins**, **telemetry** — plugin lifecycle, usage tracking

Python layer at `appfs-agent/src/` — bootstrap, hooks, skills, tools, bridge — coexists with Rust.

### Dashboard Architecture

The dashboard is two things at once: a debug viewer **and** a control plane for headless agents.

- **Server** (`dashboard/server/src/`): Fastify app with JSONL parser, file watcher, agent registry (discovers sessions from 3 paths), and SSE event bus. Beyond debug endpoints, it owns **principal lifecycle** (`principal-lifecycle.ts` + `process-manager.ts`): `POST /api/projects/:id/principals` (create → `create_principal.act`), `…/:pid/start` (spawn `claw --headless` child), `…/:pid/stop` (terminate + `detach_principal.act`), and `DELETE …/:pid` (archive + `delete_principal.act`). Other route groups: `projects.ts`, `agents.ts`, `internal-external-agents.ts`, `model-configs.ts`, `mounted-apps.ts`, `events.ts`, `messages.ts`, `timeline.ts`, `principals.ts`, `process.ts`
- **Client** (`dashboard/src/`): React + Vite SPA with agent sidebar, timeline panel (single-agent list + multi-agent swimlane), message bubbles, debug dump viewer, compaction archive viewer
- Agent sessions are stored as JSONL files under `.claw/sessions/`. The server watches these files and normalizes them into a unified timeline with k-way chronological merge.

### AppFS Mount Contract

The agent discovers AppFS via `APPFS_ATTACH_SCHEMA` env or `runtime.json` manifest at `.well-known/appfs/runtime.json`. Key paths on the mount:

```
/_appfs/                    — control plane (register/unregister apps, principals)
/_appfs/register_app.act    — append JSON action to register an app
/_appfs/principals/{create,attach,detach,update,delete}_principal.act  — principal lifecycle actions
/_appfs/principals/<id>.res.json   — per-principal record view (read result)
/_appfs/principals.registry.json   — authoritative principal + attach registry (supervisor-owned)
/_stream/events.evt.jsonl   — append-only event stream (agent reads via cursor)
/_stream/cursor.res.json    — agent-side cursor tracking
/public/<app>/              — shared app tree visible to all principals
/private/<principal>/<app>/ — per-principal private app tree (materialized on principal create/attach)
```

Each `.act` file is an append-only action log. The supervisor's `action_consumer` watches them and dispatches to the matching `handle_*` handler in `runtime_supervisor.rs`, which writes back `.res.json` result views and emits `principal.*` / `action.completed` / `action.failed` / `message` events to the stream. App-specific actions live under the app tree (e.g. `<app>/contacts/<id>/send_message.act`).

### Input Router

`input_router.rs` classifies input into four sources: `UserTerminal`, `AppfsEvent`, `AgentMessage`, `System`. Each input is an `InputEnvelope` with optional `principal_id`, `app_id`, `stream_id`, `seq`. Delivery is either `InjectAtNextBoundary` (interrupt current turn) or `QueueAfterTurn`.

### Principal & Agent Lifecycle

A **principal** is an agent's identity in AppFS; an **attach lease** is a live process binding to that principal. The lifecycle spans all three layers and is the core of multi-agent operation (full design in `docs/agent-lifecycle-architecture.md`):

1. **Create** — dashboard (or agent auto-create) appends `create_principal.act`; supervisor registers it in `principals.registry.json`, materializes each `visibility: Private` app into `private/<principal>/<app>/` with `instance_id = "<app>--<principal>"`, and emits `principal.created`.
2. **Attach** — agent process writes `attach_principal.act`; supervisor creates an `AppfsAttachLease`. Rules: one principal may have many attach IDs, but only **one non-stale attach at a time** unless `takeover` is set; a new attach auto-takes-over stale ones and rejects fresh conflicts.
3. **Warmup** — for private apps the agent writes `ensure_credentials.act` and waits for `profile.credentials.ready` before entering the turn loop.
4. **Heartbeat** — headless agents spawn a background thread writing `update_principal.act` every **30s** (no `agent_status` field → supervisor only refreshes `last_seen_at`).
5. **Sweep** — supervisor runs `sweep_stale_attaches_once()` every **30s**; attaches unseen for **90s** (3 missed heartbeats) are dropped, collapsing to Offline.
6. **Stop / Detach** — `detach_principal.act` (best-effort, used on stop and as pre-delete cleanup for stale attaches).
7. **Delete / Archive** — `delete_principal.act` (with `force: true` if stale attaches remain) tears down private app runtime + credentials; sessions are archived and filtered out of resume.

Agent status updates (`update_principal.act` *with* `agent_status`) set the principal's `agent_status.state`. The `APPFS_MULTI_AGENT_MODE_SHARED` constant controls whether multiple agents share one principal context.

## Common Environment Variables

- `ANTHROPIC_API_KEY` — required for real model calls in smoke tests and agent sessions
- `ANTHROPIC_BASE_URL` — override Anthropic API endpoint (e.g., `https://open.bigmodel.cn/api/anthropic`)
- `APPFS_TINODE_ENDPOINT` — Tinode server URL for compose configs
- `APPFS_TINODE_API_KEY` — Tinode API key
- `APPFS_TINODE_LOGIN_PREFIX` — unique prefix per smoke test run to avoid credential collisions
- `APPFS_TINODE_CREDENTIAL_POLICY` — set to `auto-create` for automated credential warmup
- `APPFS_PRINCIPAL_ID` — override principal identity for agent sessions
- `APPFS_ATTACH_ID` — unique attach lease id for this process (the dashboard sets `dashboard-<principal>`); must be distinct per concurrent attach on the same principal
- `APPFS_MOUNT_ROOT` — explicit mount root path; highest-priority source for AppFS env detection (falls back to `runtime.json`, then CWD-up `.appfs/` heuristic)
- `APPFS_ATTACH_SCHEMA` — tells the agent how to discover the AppFS mount

## Sync Workflow

Standalone repos sync into this monorepo via git subtree:

```powershell
# Pull latest from standalone repos
./integration/scripts/sync-appfs.ps1
./integration/scripts/sync-appfs-agent.ps1
```

Sync order: `claw-code-parity` -> standalone `appfs-agent` -> this monorepo's `appfs-agent/`. Never sync `claw-code-parity` directly here.

## Working Conventions

- Component-internal changes go to the standalone repo first, then sync here
- Integration-only changes (scripts, fixtures, contracts, cross-project docs) land directly here
- ADRs go in `docs/adr/`
- Integration contracts live in `integration/` (not in subproject dirs)
- Tinode secrets and API keys stay in environment variables or runner secrets — never in compose files, events, skills, or session logs
