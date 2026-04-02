# PARITY GAP ANALYSIS

Scope: read-only comparison between the archived TypeScript implementation at `archive/claw_code_ts_snapshot/src/` and the current Rust workspace under `rust/crates/`.

Method:
- compared user-visible command/tool surfaces
- checked runtime plumbing, plugin/hook support, and discovery paths
- validated the current Rust workspace with local build/test commands on Windows
- did not copy TypeScript source

## Executive summary

The Rust rewrite is no longer an MVP skeleton. It has already become the main product surface for the repository:
- interactive and one-shot CLI flows exist
- conversation/session runtime exists
- built-in tools exist for shell, file operations, search, web, todos, notebooks, skills, agents, config, REPL, and PowerShell
- plugin management exists
- hook execution exists
- local agent and skill discovery exists
- OAuth, MCP bootstrap/client support, and LSP support types exist

It is still **not feature-parity** with the old TypeScript CLI.

The current state is best described as:
- **core product path migrated**
- **historical TS feature surface only partially migrated**

The biggest remaining gaps are:
- much narrower CLI breadth than the TS snapshot
- much smaller tool surface than the TS snapshot
- no TS-equivalent structured/remote IO stack
- no TS-equivalent task/team/review/plan ecosystem
- skills are discoverable locally, but the TS bundled-skill and MCP-skill pipeline is still missing
- Windows maturity is incomplete even though the Rust workspace builds there

## Quantitative snapshot

Using repository-local snapshot data:

- archived TS-like files under `archive/claw_code_ts_snapshot/src/`: **1902**
- archived unique command names: **141**
- archived unique tool names: **94**

Current Rust workspace surface:

- Rust crates under `rust/crates/`: **9**
- registered slash commands in `rust/crates/commands/src/lib.rs`: **29**
- registered built-in tools in `rust/crates/tools/src/lib.rs`: **19**

Interpretation:
- Rust has already covered the most important product primitives
- Rust has not yet reproduced the long tail of the TS command/tool ecosystem

## Area-by-area assessment

## tools/

### TS exists

The TS snapshot contains a large tool family under `archive/claw_code_ts_snapshot/src/tools/`, including:
- `AgentTool`
- `AskUserQuestionTool`
- `BashTool`
- `ConfigTool`
- `FileReadTool`
- `FileWriteTool`
- `GlobTool`
- `GrepTool`
- `LSPTool`
- `ListMcpResourcesTool`
- `MCPTool`
- `McpAuthTool`
- `NotebookEditTool`
- `PowerShellTool`
- `ReadMcpResourceTool`
- `RemoteTriggerTool`
- `ScheduleCronTool`
- `SkillTool`
- `Task*`
- `Team*`
- `TodoWriteTool`
- `ToolSearchTool`
- `WebFetchTool`
- `WebSearchTool`

TS tool execution/orchestration is spread across:
- `archive/claw_code_ts_snapshot/src/services/tools/StreamingToolExecutor.ts`
- `archive/claw_code_ts_snapshot/src/services/tools/toolExecution.ts`
- `archive/claw_code_ts_snapshot/src/services/tools/toolHooks.ts`
- `archive/claw_code_ts_snapshot/src/services/tools/toolOrchestration.ts`

### Rust exists

Rust tool registration is centralized in `rust/crates/tools/src/lib.rs` via `mvp_tool_specs()`.

Current built-ins are:
- `bash`
- `read_file`
- `write_file`
- `edit_file`
- `glob_search`
- `grep_search`
- `WebFetch`
- `WebSearch`
- `TodoWrite`
- `Skill`
- `Agent`
- `ToolSearch`
- `NotebookEdit`
- `Sleep`
- `SendUserMessage`
- `Config`
- `StructuredOutput`
- `REPL`
- `PowerShell`

Execution is wired into the runtime loop in:
- `rust/crates/tools/src/lib.rs`
- `rust/crates/runtime/src/conversation.rs`

### Missing or incomplete in Rust

- no Rust equivalents yet for the TS MCP tool family (`MCPTool`, `ListMcpResourcesTool`, `ReadMcpResourceTool`, `McpAuthTool`) as user-facing tools
- no Rust equivalent yet for `AskUserQuestionTool`
- no Rust equivalent yet for `ScheduleCronTool`
- no Rust equivalent yet for the TS `Task*` and `Team*` tool families
- no Rust equivalent yet for `RemoteTriggerTool`
- `lsp` support exists as a crate, but there is still no user-facing Rust `LSPTool` parity
- the registry is still intentionally compact rather than parity-complete

**Status:** solid core tool surface, but far from TS tool parity.

## hooks/

### TS exists

TS hook command/runtime support exists under:
- `archive/claw_code_ts_snapshot/src/commands/hooks/`
- `archive/claw_code_ts_snapshot/src/services/tools/toolHooks.ts`
- `archive/claw_code_ts_snapshot/src/services/tools/toolExecution.ts`

### Rust exists

Rust hook support is real, not config-only:
- hook config parsing and merging in `rust/crates/runtime/src/config.rs`
- hook execution in `rust/crates/runtime/src/hooks.rs`
- runtime invocation around tool execution in `rust/crates/runtime/src/conversation.rs`

Rust currently supports:
- `PreToolUse`
- `PostToolUse`
- allow/deny semantics based on hook exit codes
- hook feedback appended to tool results

### Missing or incomplete in Rust

- no `/hooks` parity command surface
- hook behavior is simpler than the TS ecosystem
- no evidence yet of TS-style broader session/event hook coverage beyond tool use

**Status:** implemented for tool execution, but not full TS command/config UX parity.

## plugins/

### TS exists

TS plugin support spans:
- `archive/claw_code_ts_snapshot/src/plugins/builtinPlugins.ts`
- `archive/claw_code_ts_snapshot/src/plugins/bundled/index.ts`
- `archive/claw_code_ts_snapshot/src/services/plugins/PluginInstallationManager.ts`
- `archive/claw_code_ts_snapshot/src/services/plugins/pluginOperations.ts`
- `archive/claw_code_ts_snapshot/src/commands/plugin/`
- `archive/claw_code_ts_snapshot/src/commands/reload-plugins/`

### Rust exists

Rust now has a dedicated plugin subsystem:
- `rust/crates/plugins/src/lib.rs`
- `rust/crates/plugins/src/hooks.rs`

Implemented capabilities include:
- plugin manifest loading
- bundled and external plugin discovery
- install / enable / disable / uninstall / update flows
- plugin lifecycle validation
- plugin-provided hooks
- plugin-provided tools aggregated into the runtime tool registry
- `/plugin` slash command support

The CLI wires plugin state into runtime/tool loading in:
- `rust/crates/claw-cli/src/main.rs`
- `rust/crates/commands/src/lib.rs`

### Missing or incomplete in Rust

- no `/reload-plugins` parity command
- plugin feature set is still slimmer than the historical TS plugin ecosystem
- no evidence yet of TS-level plugin UI/marketplace richness

**Status:** implemented and usable, but not full TS parity.

## skills/ and CLAW.md discovery

### TS exists

TS skill loading includes:
- `archive/claw_code_ts_snapshot/src/skills/loadSkillsDir.ts`
- `archive/claw_code_ts_snapshot/src/skills/bundledSkills.ts`
- `archive/claw_code_ts_snapshot/src/skills/mcpSkillBuilders.ts`
- bundled skills under `archive/claw_code_ts_snapshot/src/skills/bundled/`
- `/skills` command surface under `archive/claw_code_ts_snapshot/src/commands/skills/`

### Rust exists

Rust has:
- local `Skill` tool support in `rust/crates/tools/src/lib.rs`
- `CLAW.md` discovery in `rust/crates/runtime/src/prompt.rs`
- `/memory` and `/init`
- `claw skills` and `/skills`
- `claw agents` and `/agents`
- project/user skill and agent discovery in `rust/crates/commands/src/lib.rs`

Discovered locations include:
- `.codex/skills`
- `.claw/skills`
- legacy `.codex/commands`
- legacy `.claw/commands`
- corresponding user-level homes

### Missing or incomplete in Rust

- no TS-equivalent bundled skill registry
- no TS-equivalent MCP skill-builder pipeline
- no evidence yet of TS-style bundled skill extraction/runtime packaging behavior
- no comparable TS team-memory/session-memory ecosystem around skills

**Status:** local discovery and listing are implemented; bundled/registry parity is still missing.

## cli/

### TS exists

The TS snapshot exposes a very broad command surface under `archive/claw_code_ts_snapshot/src/commands/`, including major families such as:
- `agents`
- `hooks`
- `mcp`
- `memory`
- `model`
- `permissions`
- `plan`
- `plugin`
- `resume`
- `review`
- `skills`
- `tasks`
- plus many platform/product commands (`chrome`, `desktop`, `doctor`, `mobile`, `voice`, etc.)

TS also has structured/remote transport plumbing in:
- `archive/claw_code_ts_snapshot/src/cli/structuredIO.ts`
- `archive/claw_code_ts_snapshot/src/cli/remoteIO.ts`
- `archive/claw_code_ts_snapshot/src/cli/transports/*`

### Rust exists

Rust slash command registration lives in `rust/crates/commands/src/lib.rs`.

Current Rust command surface includes:
- `help`
- `status`
- `compact`
- `model`
- `permissions`
- `clear`
- `cost`
- `resume`
- `config`
- `memory`
- `init`
- `diff`
- `version`
- `bughunter`
- `branch`
- `worktree`
- `commit`
- `commit-push-pr`
- `pr`
- `issue`
- `ultraplan`
- `teleport`
- `debug-tool-call`
- `export`
- `session`
- `plugin`
- `agents`
- `skills`

The direct CLI also supports:
- `claw agents`
- `claw skills`
- `claw login`
- `claw logout`

### Missing or incomplete in Rust

- command breadth remains far narrower than the TS snapshot
- no `/hooks` parity command
- no `/mcp` parity command family
- no `/plan`, `/review`, or `/tasks` parity command families
- no parity for many TS platform/product commands (`chrome`, `desktop`, `doctor`, `mobile`, `voice`, etc.)
- direct slash invocation is intentionally limited outside interactive/resume-safe flows

**Status:** mature local CLI core, but much narrower than TS.

## assistant/ runtime and orchestration

### TS exists

TS assistant/runtime behavior is spread across:
- `archive/claw_code_ts_snapshot/src/assistant/`
- `archive/claw_code_ts_snapshot/src/query.ts`
- `archive/claw_code_ts_snapshot/src/services/tools/StreamingToolExecutor.ts`
- `archive/claw_code_ts_snapshot/src/services/tools/toolOrchestration.ts`
- `archive/claw_code_ts_snapshot/src/cli/structuredIO.ts`
- `archive/claw_code_ts_snapshot/src/cli/remoteIO.ts`

### Rust exists

Rust has a strong local runtime core:
- conversation loop in `rust/crates/runtime/src/conversation.rs`
- session persistence in `rust/crates/runtime/src/session.rs`
- prompt building in `rust/crates/runtime/src/prompt.rs`
- CLI event rendering in `rust/crates/claw-cli/src/main.rs`

### Missing or incomplete in Rust

- no TS-equivalent structured IO stack
- no TS-equivalent remote IO stack
- no evidence yet of TS-level background task/session orchestration
- JSON output mode still lags the TS structured transport path in cleanliness/shape guarantees

**Status:** strong local assistant loop, missing the TS transport/orchestration layers.

## services/

### TS exists

The TS snapshot includes large service families under `archive/claw_code_ts_snapshot/src/services/`, including:
- API
- OAuth
- MCP
- plugin operations
- analytics
- session memory
- settings sync
- policy limits
- notifications
- voice-related services

### Rust exists

Rust core services are well established:
- provider client/streaming in `rust/crates/api/`
- runtime config/permissions/session/prompt in `rust/crates/runtime/`
- OAuth in `rust/crates/runtime/src/oauth.rs`
- MCP config/bootstrap/client/stdio in `rust/crates/runtime/src/{config,mcp,mcp_client,mcp_stdio}.rs`
- upstream proxy / remote support in `rust/crates/runtime/src/remote.rs`
- LSP support types/process helpers in `rust/crates/lsp/`
- plugin services in `rust/crates/plugins/`

### Missing or incomplete in Rust

- broader TS service ecosystem is still much larger
- no evidence yet of TS-equivalent analytics/settings-sync/policy-limit subsystems
- no TS-style MCP management UI layer
- no user-facing LSP parity even though the crate exists

**Status:** core runtime/service foundation exists; long-tail service parity does not.

## Validation status in this worktree

Local validation performed from Windows:

- `cargo build --workspace` in `rust/`: **passes**
- `cargo run --bin claw -- --help`: **passes**
- `cargo test --workspace` in `rust/`: **fails on Windows**

Windows test failure cause:
- `rust/crates/runtime/src/mcp_stdio.rs` test code uses `std::os::unix::fs::PermissionsExt`
- those tests are not currently guarded for Windows

CI currently reflects this maturity level:
- `rust/.github/workflows/ci.yml` runs on Ubuntu and macOS
- Windows is not in the current CI matrix

Interpretation:
- the Rust workspace is buildable on Windows
- Windows test/release readiness is still incomplete

## Overall assessment

Current migration status:

- **Core rewrite:** mostly complete
- **Primary local product path:** complete enough for active use and continued development
- **Historical TS parity:** incomplete
- **Windows maturity:** partial

If the goal is "replace the TS snapshot as the active implementation," the project is already there.

If the goal is "reach broad feature parity with the TS snapshot," the project still has substantial work remaining.

## Recommended next parity targets

Highest-value next steps appear to be:

1. Add user-facing MCP parity
   - `/mcp`
   - MCP tool/resource auth/list/read surfaces

2. Add the missing workflow command/tool families
   - `/plan`
   - `/review`
   - `/tasks`
   - `Task*`
   - `Team*`
   - `AskUserQuestionTool`

3. Decide whether to rebuild or intentionally drop TS structured/remote transport parity
   - `structuredIO`
   - `remoteIO`
   - transport adapters

4. Decide whether skills should stay local-first or regain bundled/MCP-registry behavior

5. Make Windows an explicit target
   - fix Unix-only test code
   - add Windows CI
   - verify runtime subprocess/MCP behavior on Windows
