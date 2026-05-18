# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

appfs-platform is an **integration monorepo** that brings together two separate layers:

- **appfs** (source repo: `agentfs`) — a filesystem protocol, runtime supervisor, connectors (Tinode, gRPC, HTTP bridges), and SDKs for AI agent app state
- **appfs-agent** (source repo: `claw-code`) — an interactive agent runtime (Claude-style) that attaches to an AppFS mount, reads app state, writes actions, and reacts to event streams
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

Workspace lints: `unsafe_code = "forbid"`, clippy `all` + `pedantic` at warn.

### appfs (Rust CLI + SDK)

```bash
cd appfs/cli
cargo build
cargo test
cargo test --package agentfs
```

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

Tests run from repo root via CI (`appfs-agent/python`):

```bash
cd appfs-agent
uv run pytest
```

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

- **CLI** (`appfs/cli/`) — `agentfs` binary; init, mount (FUSE/NFS/WinFsp), compose up, appfs subcommands
- **SDK** (`appfs/sdk/rust/src/`) — `AgentFS` struct, filesystem/KV/tool-call layers, connectors
- **Connectors** — `appfs_connector.rs` (SDK abstraction), `tinode_connector.rs` (WebSocket chat bridge), `grpc_bridge_adapter.rs` (tonic), `http_bridge_adapter.rs`
- **Compose** (`appfs/appfs-compose.*.yaml`) — per-app deployment configs (tinode, aiim, huoyan)
- **Tree Sync** (`appfs/cli/src/cmd/appfs/tree_sync.rs`) — initializes and refreshes mounted app structure

### appfs-agent Key Components

Rust workspace at `appfs-agent/rust/crates/`:

- **runtime** — core: `appfs.rs` (attach, principal, registry, event polling), `input_router.rs` (input source routing), `session.rs`, `hooks.rs`, `permissions.rs`
- **api** — LLM client abstraction (Anthropic and others), streaming, prompt cache
- **tools** — file operations, shell, lane completion
- **commands** — slash commands, skills dispatch
- **rusty-claude-cli** — CLI entry point, main REPL loop
- **plugins**, **telemetry** — plugin lifecycle, usage tracking

Python layer at `appfs-agent/src/` — bootstrap, hooks, skills, tools, bridge — coexists with Rust.

### AppFS Mount Contract

The agent discovers AppFS via `APPFS_ATTACH_SCHEMA` env or `runtime.json` manifest at `.well-known/appfs/runtime.json`. Key paths on the mount:

```
/_appfs/                    — control plane (register/unregister apps, principals)
/_appfs/register_app.act    — append JSON action to register an app
/_appfs/principals/          — principal attach/detach actions
/_stream/events.evt.jsonl   — append-only event stream (agent reads via cursor)
/_stream/cursor.res.json    — agent-side cursor tracking
/public/<app>/              — shared app tree visible to all principals
/private/<principal>/<app>/ — per-principal private app tree
```

### Input Router

`input_router.rs` classifies input into four sources: `UserTerminal`, `AppfsEvent`, `AgentMessage`, `System`. Each input is an `InputEnvelope` with optional `principal_id`, `app_id`, `stream_id`, `seq`. Delivery is either `InjectAtNextBoundary` (interrupt current turn) or `QueueAfterTurn`.

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
