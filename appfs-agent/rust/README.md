# 🦞 Claw Code — Rust Implementation

A high-performance Rust rewrite of the Claw Code CLI agent harness. Built for speed, safety, and native tool execution.

## Quick Start

```bash
# Build
cd rust/
cargo build --release

# Run interactive REPL
./target/release/claw

# One-shot prompt
./target/release/claw prompt "explain this codebase"

# With specific model
./target/release/claw --model sonnet prompt "fix the bug in main.rs"

# JSON output for automation
cargo run -p rusty-claude-cli -- --output-format json prompt "summarize src/main.rs"

# Inspect registered hooks and whether they are enabled
cargo run -p rusty-claude-cli -- hook list
```

## Configuration

Set your API credentials:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
# Or use a proxy
export ANTHROPIC_BASE_URL="https://your-proxy.com"
```

Or authenticate via OAuth:

```bash
claw login
```

## AppFS Event Boundary Injection

When `claw` runs from an AppFS mount, it automatically syncs current
principal-visible AppFS event streams before each model call and injects fresh
records as `<system-reminder>` context. This keeps same-turn action receipts
such as `action.completed`, `action.failed`, and Tinode `message.received`
available while the model is actively working.

The earlier broad idle watcher commands remain disabled because they woke the
model on every event, including self-generated receipts:

- `cargo run -p rusty-claude-cli -- appfs-events watch ...`
- `cargo run -p rusty-claude-cli -- --watch-appfs-events`

For safe idle wake, use the new opt-in flag:

```bash
cargo run -p rusty-claude-cli -- --appfs-idle-wake
```

`--appfs-idle-wake` starts the normal interactive REPL and scans AppFS events at
safe idle boundaries. It uses a separate wake cursor and only wakes the agent for
attention-worthy events, such as Tinode `message.received` records with
`requires_attention=true`. Status events, self-generated receipts such as
`message.sent`, and action receipts such as `action.completed` remain available
through normal model-call boundary injection but do not start a new idle turn by
themselves.

This is intentionally not a fully asynchronous terminal editor yet. The CLI
checks for idle wake before and after safe REPL operations; it does not interrupt
an in-progress model/tool turn or force a wake for every filesystem event.

For experimental running-turn user guidance, combine idle wake with
`--running-input`:

```bash
cargo run -p rusty-claude-cli -- --appfs-idle-wake --running-input
```

`--running-input` switches the interactive REPL to a minimal single-stdin-owner
terminal controller. While the agent is running, submitted lines become
`user.guidance` and are injected before the next safe model-call boundary. Use
`/queue <text>` to defer a note until after the current turn. The default
`rustyline` REPL remains unchanged unless this flag is set.

Design notes:

- `docs/plans/2026-05-09-appfs-agent-event-boundary-and-idle-wake.md`
- `docs/plans/2026-05-09-appfs-agent-unified-input-router-implementation.md`
- `docs/plans/2026-05-10-appfs-agent-pr7-running-guidance-input.md`

## AppFS Attach Identity

When the interactive CLI starts inside an AppFS mount, it now ensures the current
AppFS principal exists before building the system prompt and skill listing. The
principal comes from `APPFS_PRINCIPAL_ID`; when unset it defaults to `default`.
This is attach-driven: starting `claw` inside the mounted AppFS tree is what
creates the attach lease, principal ensure, and private-app warmup. AppFS
startup alone does not create the identity or the private apps.

If the principal is missing, `claw` appends to
`/_appfs/principals/create_principal.act` and waits for AppFS to materialize that
principal's private app instances, such as `private/<principal-id>/tinode`.

After the principal is ready, `claw` appends to
`/_appfs/principals/attach_principal.act` with its process `attach_id`. On normal
process exit it appends to `/_appfs/principals/detach_principal.act`, allowing
AppFS to keep `active_attach_count` and `active_attaches` as best-effort live
status. Detach does not delete the principal, private app data, or credentials.

This attach step prepares AppFS identity and private app visibility. After
attach, `claw` also best-effort warms private apps that expose
`_app/ensure_credentials.act` by appending a standard ensure request for the
current principal. Long-lived credentials still belong to the connector; the
agent only triggers the app-specific bootstrap path. If the attach is killed
ungracefully, the detach lease may lag until the next lifecycle update.

## Mock parity harness

The workspace now includes a deterministic Anthropic-compatible mock service and a clean-environment CLI harness for end-to-end parity checks.

```bash
cd rust/

# Run the scripted clean-environment harness
./scripts/run_mock_parity_harness.sh

# Or start the mock service manually for ad hoc CLI runs
cargo run -p mock-anthropic-service -- --bind 127.0.0.1:0
```

Harness coverage:

- `streaming_text`
- `read_file_roundtrip`
- `grep_chunk_assembly`
- `write_file_allowed`
- `write_file_denied`
- `multi_tool_turn_roundtrip`
- `bash_stdout_roundtrip`
- `bash_permission_prompt_approved`
- `bash_permission_prompt_denied`
- `plugin_tool_roundtrip`

Primary artifacts:

- `crates/mock-anthropic-service/` — reusable mock Anthropic-compatible service
- `crates/rusty-claude-cli/tests/mock_parity_harness.rs` — clean-env CLI harness
- `scripts/run_mock_parity_harness.sh` — reproducible wrapper
- `scripts/run_mock_parity_diff.py` — scenario checklist + PARITY mapping runner
- `mock_parity_scenarios.json` — scenario-to-PARITY manifest

## Features

| Feature | Status |
|---------|--------|
| Anthropic API + streaming | ✅ |
| OAuth login/logout | ✅ |
| Interactive REPL (rustyline) | ✅ |
| Tool system (bash, read, write, edit, grep, glob) | ✅ |
| Web tools (search, fetch) | ✅ |
| Sub-agent orchestration | ✅ |
| Todo tracking | ✅ |
| Notebook editing | ✅ |
| CLAUDE.md / project memory | ✅ |
| Config file hierarchy (.claude.json) | ✅ |
| Permission system | ✅ |
| MCP server lifecycle | ✅ |
| Session persistence + resume | ✅ |
| Extended thinking (thinking blocks) | ✅ |
| Cost tracking + usage display | ✅ |
| Git integration | ✅ |
| Markdown terminal rendering (ANSI) | ✅ |
| Model aliases (opus/sonnet/haiku) | ✅ |
| Slash commands (/status, /compact, /clear, etc.) | ✅ |
| Hooks (PreToolUse/PostToolUse) | 🔧 Config only |
| Plugin system | 📋 Planned |
| Skills registry | 📋 Planned |

## Model Aliases

Short names resolve to the latest model versions:

| Alias | Resolves To |
|-------|------------|
| `opus` | `claude-opus-4-6` |
| `sonnet` | `claude-sonnet-4-6` |
| `haiku` | `claude-haiku-4-5-20251213` |

## CLI Flags

```
claw [OPTIONS] [COMMAND]

Options:
  --model MODEL                    Set the model (alias or full name)
  --dangerously-skip-permissions   Skip all permission checks
  --permission-mode MODE           Set read-only, workspace-write, or danger-full-access
  --allowedTools TOOLS             Restrict enabled tools
  --output-format FORMAT           Output format (text or json)
  --appfs-idle-wake                Wake idle REPL turns for attention-worthy AppFS events
  --running-input                  Experimental: accept guidance while a turn is running
  --version, -V                    Print version info

Commands:
  prompt <text>      One-shot prompt (non-interactive)
  login              Authenticate via OAuth
  logout             Clear stored credentials
  init               Initialize project config
  doctor             Check environment health
  self-update        Update to latest version
```

## Slash Commands (REPL)

Tab completion now expands not just slash command names, but also common workflow arguments like model aliases, permission modes, and recent session IDs.

| Command | Description |
|---------|-------------|
| `/help` | Show help |
| `/status` | Show session status (model, tokens, cost) |
| `/cost` | Show cost breakdown |
| `/compact` | Compact conversation history |
| `/clear` | Clear conversation |
| `/model [name]` | Show or switch model |
| `/permissions` | Show or switch permission mode |
| `/config [section]` | Show config (env, hooks, model) |
| `/memory` | Show CLAUDE.md contents |
| `/diff` | Show git diff |
| `/export [path]` | Export conversation |
| `/session [id]` | Resume a previous session |
| `/version` | Show version |

## Workspace Layout

```
rust/
├── Cargo.toml              # Workspace root
├── Cargo.lock
└── crates/
    ├── api/                # Anthropic API client + SSE streaming
    ├── commands/           # Shared slash-command registry
    ├── compat-harness/     # TS manifest extraction harness
    ├── mock-anthropic-service/ # Deterministic local Anthropic-compatible mock
    ├── runtime/            # Session, config, permissions, MCP, prompts
    ├── rusty-claude-cli/   # Main CLI binary (`claw`)
    └── tools/              # Built-in tool implementations
```

### Crate Responsibilities

- **api** — HTTP client, SSE stream parser, request/response types, auth (API key + OAuth bearer)
- **commands** — Slash command definitions and help text generation
- **compat-harness** — Extracts tool/prompt manifests from upstream TS source
- **mock-anthropic-service** — Deterministic `/v1/messages` mock for CLI parity tests and local harness runs
- **runtime** — `ConversationRuntime` agentic loop, `ConfigLoader` hierarchy, `Session` persistence, permission policy, MCP client, system prompt assembly, usage tracking
- **rusty-claude-cli** — REPL, one-shot prompt, streaming display, tool call rendering, CLI argument parsing
- **tools** — Tool specs + execution: Bash, ReadFile, WriteFile, EditFile, GlobSearch, GrepSearch, WebSearch, WebFetch, Agent, TodoWrite, NotebookEdit, Skill, ToolSearch, REPL runtimes

## Stats

- **~20K lines** of Rust
- **7 crates** in workspace
- **Binary name:** `claw`
- **Default model:** `claude-opus-4-6`
- **Default permissions:** `danger-full-access`

## License

See repository root.
