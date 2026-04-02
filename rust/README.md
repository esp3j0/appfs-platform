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

## Model and provider configuration

The runtime loads settings from these files, in precedence order:

1. `%HOME%\\.claw\\settings.json`
2. `.claw.json`
3. `.claw\\settings.json`
4. `.claw\\settings.local.json`

Shared repo defaults should usually live in `.claw.json`. Machine-local secrets and developer overrides should usually live in `.claw/settings.local.json`.

The current provider config shape is:

```json
{
  "model": "your-model-name",
  "provider": {
    "type": "openai",
    "baseUrl": "https://gateway.example/v1",
    "apiKeyEnv": "YOUR_API_KEY_ENV"
  }
}
```

Supported fields:

- `model`: any model identifier string
- `provider.type`: `anthropic`, `openai`, or `xai`
- `provider.baseUrl`: optional override for a proxy or custom gateway
- `provider.apiKeyEnv`: optional API key environment variable name
- `provider.authTokenEnv`: optional extra auth token env var for `anthropic`

Common examples:

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

Behavior notes:

- if `provider` is present, the runtime uses it directly instead of inferring from the model name
- if `provider` is absent, the runtime still supports model-based provider detection
- the configured `model` becomes the default model used by the CLI and agent runtime
- `authTokenEnv` is rejected for non-`anthropic` providers

To inspect the merged provider view:

```bash
cargo run --bin claw -- config provider
```

To inspect the full merged config:

```bash
cargo run --bin claw -- config
```

## Migration notes

Older setups could rely on model-name inference alone. That still works for known model families such as `claude-*`, `gpt-*`, `o1`/`o3`/`o4`, and `grok-*`.

Use explicit `provider` config when:

- you want to use a custom model name behind an OpenAI-compatible gateway
- you need to force a provider family even when the model name is ambiguous
- you want to override the default API base URL or credential env var names

This makes it possible to point `appfs-agent` at arbitrary gateways without having to rename models to match built-in detection rules.

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
