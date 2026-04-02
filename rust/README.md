# appfs-agent Rust Workspace

This Rust workspace is the active implementation core of `appfs-agent`, the agent runtime being built for AppFS.

The codebase still uses some earlier internal names such as `claw` for the current CLI binary. Those names are transitional. The product direction for this workspace is now:

- a local agent runtime for AppFS-oriented workflows
- strong tool execution, sessions, plugins, hooks, and workspace context
- a future AppFS-native runtime entrypoint and lifecycle model

## Current status

- **Version:** `0.1.0`
- **Stage:** active runtime foundation, not final AppFS integration
- **Primary implementation:** Rust workspace in this directory
- **Current binary name:** `claw` (transitional)

## What exists today

The workspace already contains the main runtime building blocks:

- `claw-cli` — current interactive CLI and one-shot prompt entrypoint
- `runtime` — sessions, config, hooks, prompts, permissions, MCP, remote state
- `tools` — built-in tool registry and tool execution
- `commands` — slash commands, local skill discovery, agent discovery
- `plugins` — plugin discovery, registry, lifecycle, hook, and tool support
- `api` — provider clients and streaming
- `lsp` — LSP support types and process helpers
- `server` and `compat-harness` — supporting integration surfaces

## Build and run

### Prerequisites

- Rust stable toolchain
- Cargo
- credentials for the model/provider you intend to use

### Build

```bash
cargo build --workspace
cargo build --release -p claw-cli
```

### Run

```bash
cargo run --bin claw -- --help
cargo run --bin claw --
cargo run --bin claw -- prompt "summarize this workspace"
```

## Current capabilities

- interactive REPL and one-shot prompt execution
- saved-session inspection and resume flows
- built-in tools for shell, file operations, search, web fetch/search, todos, notebooks, config, and REPL-like execution
- plugin and hook support
- local skill and agent discovery
- OAuth support
- MCP bootstrap/client support
- LSP support primitives

## Current limitations

- the runtime is not yet presented through a final AppFS-native entrypoint
- naming is still transitional in code and binaries
- Windows is not yet part of the CI matrix
- some higher-level parity features from the older TS snapshot are still missing
- release packaging/distribution is still immature

## Near-term direction

This workspace is expected to evolve in three layers:

1. strengthen the current runtime core
2. define how AppFS should launch, supervise, and communicate with the runtime
3. complete naming and packaging changes once the runtime contract stabilizes

## Verification notes

At the current checkpoint:

- `cargo build --workspace` passes
- Windows build works
- Windows test coverage is still incomplete because some tests remain Unix-oriented

## Related docs

- Root repo overview: [../README.md](../README.md)
- Migration checkpoint: [../PARITY.md](../PARITY.md)
- Draft release notes: [docs/releases/0.1.0.md](docs/releases/0.1.0.md)
