# appfs-agent

`appfs-agent` is the agent runtime companion for [AppFS](C:/Users/esp3j/rep/agentfs/README.md): a local, tool-using coding/runtime harness that is being adapted to run inside the AppFS ecosystem.

The repository started from clean-room harness experimentation and parity work. Its current direction is narrower and more practical:

- provide a local agent runtime that fits naturally into AppFS workflows
- keep strong shell/file/tool ergonomics for agents
- support plugins, hooks, skills, and session state
- evolve toward an AppFS-native control plane and runtime contract

## Current status

This repository is now positioned as **AppFS's agent runtime workspace**.

Today:

- the primary implementation lives in `rust/`
- there is still historical parity and analysis material in `src/` and `PARITY.md`
- some internal crate and binary names still reflect the earlier migration stage
- the public repo name and product direction are now `appfs-agent`

In other words: the runtime direction has changed first; internal naming cleanup will follow incrementally.

## How this fits with AppFS

AppFS gives shell-first agents a filesystem-native way to work with apps.

`appfs-agent` is intended to be the agent-side runtime that can:

- execute local coding and automation tasks
- maintain conversations, sessions, and tool state
- load project instructions and local skills
- expose or consume AppFS-managed resources
- eventually participate as a first-class managed runtime inside `agentfs appfs`

Short version:

- `agentfs` / `appfs`: filesystem protocol, mounts, runtime control plane
- `appfs-agent`: the agent execution runtime that lives on top of that substrate

## Repository layout

```text
.
├── rust/                    # Active Rust runtime workspace
│   ├── crates/claw-cli/     # Current interactive CLI binary
│   ├── crates/runtime/      # Conversation loop, config, hooks, sessions
│   ├── crates/tools/        # Built-in tool registry and execution
│   ├── crates/commands/     # Slash commands and local discovery
│   ├── crates/plugins/      # Plugin loading, lifecycle, hook support
│   ├── crates/api/          # Model/provider clients and streaming
│   ├── crates/lsp/          # LSP support types/process helpers
│   └── crates/server/       # Supporting services
├── src/                     # Historical parity-analysis and porting workspace
├── tests/                   # Validation for the non-Rust workspace surfaces
├── PARITY.md                # TS snapshot vs Rust migration checkpoint
└── README.md
```

## Current capabilities

The Rust workspace already provides a usable local agent runtime core:

- interactive REPL and one-shot prompt execution
- session persistence and resume flows
- shell, file, search, web, todo, notebook, config, REPL, and PowerShell tools
- plugin discovery and plugin-provided tools
- hook execution around tool calls
- local skills and agent discovery
- OAuth support, MCP bootstrap/client support, and LSP support primitives

This is enough to treat the Rust workspace as the foundation of `appfs-agent`, even though the full AppFS integration story is not finished yet.

## Current gaps

The runtime is not fully AppFS-native yet.

Important gaps still include:

- internal naming still uses `claw` in several places
- no dedicated AppFS runtime entrypoint yet
- no explicit AppFS-managed lifecycle integration yet
- no final command/API contract for how AppFS should start, supervise, and communicate with the agent runtime
- historical TS parity is still incomplete in several advanced areas

See [PARITY.md](C:/Users/esp3j/rep/claw-code/PARITY.md) for the migration checkpoint.

## Build and run

The active implementation is the Rust workspace:

```bash
cd rust
cargo build --workspace
cargo run --bin claw -- --help
```

The current binary is still named `claw`. That is an implementation detail inherited from the earlier migration stage, not the final product name.

## Model and provider configuration

`appfs-agent` now supports two ways to choose a model backend:

- set only `model` and let the runtime infer the provider from the model family
- set both `model` and `provider` to force a specific provider family and gateway

Runtime settings are loaded from these files, in precedence order:

- `%HOME%\\.claw\\settings.json`
- `.claw.json`
- `.claw\\settings.json`
- `.claw\\settings.local.json`

The most useful pattern for AppFS deployments is to keep shared defaults in `.claw.json` and machine- or secret-specific overrides in `.claw/settings.local.json`.

Example: OpenAI-compatible gateway with a custom model name

```json
{
  "model": "qwen3-coder-plus",
  "provider": {
    "type": "openai",
    "baseUrl": "https://gateway.example/v1",
    "apiKeyEnv": "APPFS_GATEWAY_API_KEY"
  }
}
```

Example: official OpenAI API

```json
{
  "model": "gpt-4.1",
  "provider": {
    "type": "openai",
    "baseUrl": "https://api.openai.com/v1",
    "apiKeyEnv": "OPENAI_API_KEY"
  }
}
```

Example: Anthropic-compatible route

```json
{
  "model": "claude-sonnet-4-6",
  "provider": {
    "type": "anthropic",
    "baseUrl": "https://anthropic-proxy.example",
    "apiKeyEnv": "ANTHROPIC_API_KEY",
    "authTokenEnv": "ANTHROPIC_AUTH_TOKEN"
  }
}
```

Example: xAI

```json
{
  "model": "grok-3",
  "provider": {
    "type": "xai",
    "baseUrl": "https://api.x.ai/v1",
    "apiKeyEnv": "XAI_API_KEY"
  }
}
```

Notes:

- `model` can be any string; when `provider` is present, the runtime uses `provider.type` instead of guessing
- `provider.type` currently supports `anthropic`, `openai`, and `xai`
- `provider.baseUrl` lets you point at a proxy or any OpenAI-compatible gateway
- `provider.authTokenEnv` is only valid for `anthropic`
- if `provider` is omitted, the runtime falls back to model-based provider detection
- the configured `model` is now used as the default model for the CLI and agent runtime

You can inspect merged settings with:

```bash
cd rust
cargo run --bin claw -- config provider
```

This prints the merged `provider` section after config-file precedence has been applied.

## Near-term roadmap

The next phase for `appfs-agent` is to turn the current local runtime into an AppFS-native component:

1. keep tightening the Rust runtime core
2. define the AppFS-facing runtime contract
3. introduce an AppFS-oriented entrypoint and naming pass
4. decide which historical parity features are worth preserving
5. make Windows support and CI first-class if AppFS deployment needs it

## Relationship to the archived work

This repository still contains:

- historical migration artifacts
- parity analysis against the archived TypeScript snapshot
- an earlier Python-first porting surface

Those are now supporting materials. The main product direction is the Rust-based `appfs-agent` runtime.
