# TS -> Rust Migration Backlog

Last updated: 2026-04-21

## Purpose

This document is the working checklist for migrating important `claw_code_ts_snapshot`
behavior into the Rust implementation.

Use this file as the execution backlog:

- Mark an item `[x]` only when the model-visible behavior is implemented in Rust.
- Prefer checking off vertical slices that include runtime wiring and regression tests.
- If a large item needs to be split, add child checklist items beneath it instead of
  rewriting the section from scratch.

Reference roots:

- TS source: [`archive/claw_code_ts_snapshot/src`](/C:/Users/esp3j/rep/appfs-agent/archive/claw_code_ts_snapshot/src)
- Rust workspace: [`rust/`](/C:/Users/esp3j/rep/appfs-agent/rust)

## Already migrated

- [x] Bash / PowerShell model-visible wrapper parity for large output persistence and
  session-scoped output paths
- [x] Glob / Grep wrapper parity, including persisted output handling
- [x] Read / Write / Edit text-file wrapper parity for core JSON shape
- [x] Session-scoped tool output storage under `.claw/sessions/<session-id>/...`
- [x] File read-state tracking for stale write / stale edit protection
- [x] Full compact core path for Rust `/compact` and `--resume` flows
- [x] Compact transcript support for preserved summary / attachment-style context messages

## Recommended migration order

1. Skill system foundation
2. Skill execution semantics
3. Agent runtime orchestration
4. High-value tool deep parity
5. Security / analytics / polish parity

## Tools backlog

### P0 - High-value tool parity

- [x] `Skill` tool execution parity foundation
  TS reference: `src/tools/SkillTool/SkillTool.ts`, `src/skills/loadSkillsDir.ts`
  Rust landing zone: `rust/crates/tools/src/lib.rs`, `rust/crates/commands/src/lib.rs`
  Done when: Rust no longer treats `Skill` as a plain file reader and instead returns
  normalized skill metadata plus execution-ready prompt content that matches TS rules.

- [ ] `LSP` real operation parity
  TS reference: `src/tools/LSPTool/LSPTool.ts`
  Rust landing zone: `rust/crates/runtime/src/lsp_client.rs`, `rust/crates/tools/src/lib.rs`
  Done when: definition / references / hover / symbols are backed by real LSP calls
  rather than dispatch placeholders, and tool output shape is stable under tests.

- [ ] `ReadMcpResource` content parity
  TS reference: `src/tools/ReadMcpResourceTool/ReadMcpResourceTool.ts`
  Rust landing zone: `rust/crates/runtime/src/mcp_tool_bridge.rs`,
  `rust/crates/tools/src/lib.rs`
  Done when: Rust returns actual resource contents, persists binary blobs to disk when
  needed, and exposes model-visible output comparable to TS.

- [ ] `McpAuth` OAuth flow parity
  TS reference: `src/tools/McpAuthTool/McpAuthTool.ts`
  Rust landing zone: `rust/crates/runtime/src/mcp_tool_bridge.rs`,
  `rust/crates/runtime/src/mcp_stdio.rs`, `rust/crates/tools/src/lib.rs`
  Done when: Rust can surface an auth URL or equivalent actionable auth state instead
  of only reporting current connection status.

- [ ] `SendMessage` tool parity
  TS reference: `src/tools/SendMessageTool/SendMessageTool.ts`
  Rust landing zone: `rust/crates/tools/src/lib.rs`, `rust/crates/runtime/src/worker_boot.rs`
  Done when: a running agent can be continued through a model-visible tool call with
  delivery semantics comparable to TS.

- [ ] `EnterWorktree` / `ExitWorktree` parity
  TS reference: `src/tools/EnterWorktreeTool/EnterWorktreeTool.ts`,
  `src/tools/ExitWorktreeTool`
  Rust landing zone: `rust/crates/tools/src/lib.rs`, `rust/crates/runtime/src/session_control.rs`
  Done when: Rust can create a session-linked isolated worktree and switch session
  context into and out of it with persisted state.

- [ ] `Brief` attachment bridge parity
  TS reference: `src/tools/BriefTool/BriefTool.ts`, `src/tools/BriefTool/upload.ts`
  Rust landing zone: `rust/crates/tools/src/lib.rs`
  Done when: attachment metadata supports the same cross-surface behavior expected by TS,
  including uploaded/bridged attachment identifiers where applicable.

- [ ] `AskUserQuestion` interactive parity
  TS reference: `src/tools/AskUserQuestionTool`
  Rust landing zone: `rust/crates/tools/src/lib.rs`
  Done when: Rust stops relying on terminal stdin prompting and instead supports the
  same structured user-question flow expected by the higher-level runtime/UI.

### P1 - Important but not first

- [ ] Cron tool durability parity
  TS reference: `src/tools/ScheduleCronTool/CronCreateTool.ts`,
  `src/tools/ScheduleCronTool/CronDeleteTool.ts`,
  `src/tools/ScheduleCronTool/CronListTool.ts`
  Rust landing zone: `rust/crates/runtime/src/team_cron_registry.rs`,
  `rust/crates/tools/src/lib.rs`
  Done when: cron entries are not only in-memory records, but support durable storage
  and scheduler semantics comparable to TS.

- [ ] Team / Task orchestration parity
  TS reference: `src/tools/Task*`, `src/tools/Team*`
  Rust landing zone: `rust/crates/runtime/src/task_registry.rs`,
  `rust/crates/runtime/src/team_cron_registry.rs`, `rust/crates/tools/src/lib.rs`
  Done when: tool semantics reflect actual orchestrated execution rather than only
  registry bookkeeping.

- [ ] `RemoteTrigger` semantics parity
  TS reference: `src/tools/RemoteTriggerTool`
  Rust landing zone: `rust/crates/tools/src/lib.rs`
  Done when: input validation, result shape, and failure surface match TS expectations.

- [ ] Deep bash security parity
  TS reference: `src/tools/BashTool/bashSecurity.ts`
  Rust landing zone: `rust/crates/runtime/src/bash_validation.rs`
  Done when: the highest-value shell parsing / escaping / redirection / substitution
  defenses from TS are explicitly ported or intentionally rejected with rationale.

## Agent runtime backlog

### P0 - High-value runtime parity

- [ ] Coordinator mode parity
  TS reference: `src/coordinator/coordinatorMode.ts`
  Rust landing zone: `rust/crates/runtime/src/conversation.rs`,
  `rust/crates/tools/src/lib.rs`, prompt-building/runtime wiring
  Done when: Rust can run with a coordinator-style system prompt and worker orchestration
  model rather than only a simple local sub-agent model.

- [ ] Worker notification protocol parity
  TS reference: `src/coordinator/coordinatorMode.ts`, `src/tools/AgentTool/*`
  Rust landing zone: `rust/crates/runtime/src/worker_boot.rs`,
  `rust/crates/runtime/src/session.rs`
  Done when: worker completions are surfaced back into the conversation in a structured,
  resumable way comparable to TS task notifications.

- [ ] Background / resumable agent lifecycle parity
  TS reference: `src/tools/AgentTool/AgentTool.tsx`,
  `src/tools/AgentTool/resumeAgent.ts`
  Rust landing zone: `rust/crates/tools/src/lib.rs`,
  `rust/crates/runtime/src/worker_boot.rs`
  Done when: agent launch, backgrounding, resume, retry, kill, and continuation are
  all first-class runtime behaviors.

- [ ] Worktree-isolated sub-agent execution parity
  TS reference: `src/tools/AgentTool/forkSubagent.ts`,
  `src/utils/worktree.js`
  Rust landing zone: runtime/session/worktree management modules
  Done when: sub-agents can be launched into isolated worktrees rather than always
  sharing the current workspace.

### P1 - Important follow-up runtime parity

- [ ] Session/project-root semantics parity for throwaway worktrees
  TS reference: `src/bootstrap/state.ts`
  Rust landing zone: `rust/crates/runtime/src/session_control.rs`,
  prompt/context discovery
  Done when: Rust distinguishes project identity from current cwd the way TS does for
  skills, history, and session storage.

- [ ] Cost / token tracking parity
  TS reference: `src/cost-tracker.ts`, `src/bootstrap/state.ts`
  Rust landing zone: `rust/crates/runtime/src/usage.rs`, conversation/runtime telemetry
  Done when: session-visible cost and token accounting are comparable to TS.

- [ ] Post-compaction analytics parity
  TS reference: `src/bootstrap/state.ts`
  Rust landing zone: `rust/crates/runtime/src/conversation.rs`,
  `rust/crates/runtime/src/compact.rs`
  Done when: Rust preserves the same useful post-compact latches / follow-up metadata
  TS uses for prompt-cache and cache-miss attribution.

- [ ] Remote / CCR agent execution parity
  TS reference: remote agent and CCR transport code under `src/tools/AgentTool` and `src/cli/transports`
  Rust landing zone: future runtime integration
  Done when: remote isolated agents are a real runtime capability in Rust, or this item
  is explicitly dropped by product decision.

## Skills backlog

### P0 - Foundation

- [x] Shared skill frontmatter parser parity
  TS reference: `src/skills/loadSkillsDir.ts`
  Rust landing zone: likely new shared module under `rust/crates/tools/src/` or
  `rust/crates/commands/src/`
  Done when: Rust parses at least `name`, `description`, `arguments`, `argument-hint`,
  `allowed-tools`, `model`, `effort`, `hooks`, `paths`, `context`, and `agent`.

- [x] Skill argument substitution parity
  TS reference: `src/utils/argumentSubstitution.js`, `src/skills/loadSkillsDir.ts`
  Rust landing zone: tool/command skill loading path
  Done when: Rust can apply skill arguments into prompt content using TS-compatible
  substitution rules.

- [x] Conditional skill activation by `paths` parity
  TS reference: `src/skills/loadSkillsDir.ts`
  Rust landing zone: skill discovery and prompt-building layers
  Done when: path-scoped skills activate only when the current workspace context matches
  their path selectors.

- [x] Skill execution-context parity
  TS reference: `src/tools/SkillTool/SkillTool.ts`, `src/skills/loadSkillsDir.ts`
  Rust landing zone: `Skill` tool plus agent runtime wiring
  Done when: Rust honors whether a skill should run directly or in a forked agent context.

### P1 - Important follow-up skill parity

- [ ] Bundled skill registry parity
  TS reference: `src/skills/bundled`, `src/skills/bundledSkills.ts`
  Rust landing zone: new bundled-skill registry plus command/tool discovery wiring
  Done when: high-value built-in skills such as `verify`, `stuck`, `remember`, `skillify`,
  and `batch` are available in Rust with stable discovery semantics.
  - [x] Shared bundled-skill registry and `/skills` discovery wiring
  - [x] `Skill` tool execution path for bundled skills, including prompt wrapper and
    session-scoped reference-file extraction
  - [x] Prompt-based bundled skills: `verify`, `remember`, `stuck`
- [x] Context-heavy bundled skill: `skillify`
  - [ ] Worktree/coordinator-heavy bundled skill: `batch`

- [ ] MCP-generated skill parity
  TS reference: `src/skills/mcpSkillBuilders.ts`
  Rust landing zone: MCP + skill integration layer
  Done when: MCP-provided prompts/skills can participate in the Rust skill system the
  way they do in TS.

- [x] Invoked-skill preservation registry parity
  TS reference: `src/bootstrap/state.ts`, `src/tools/SkillTool/SkillTool.ts`
  Rust landing zone: `rust/crates/runtime/src/compact.rs`, runtime session state
  Done when: compaction preserves invoked skills from explicit state, not only by
  scanning historical tool results.

- [ ] Skill metadata surfaces parity
  TS reference: `src/tools/SkillTool/prompt.ts`, `src/skills/loadSkillsDir.ts`
  Rust landing zone: `rust/crates/commands/src/lib.rs`, `rust/crates/tools/src/lib.rs`
  Done when: listing/discovery surfaces expose the same important metadata the model and
  user rely on in TS.

## First implementation slice

Start here unless a more urgent product need appears:

- [x] Build a shared Rust skill loader that parses TS-compatible frontmatter fields and
  returns structured metadata for both `/skills` inventory and the `Skill` tool.

Why this first:

- It unlocks most of the remaining skill backlog.
- It is lower-risk than coordinator/worktree migration.
- It creates a clean seam for later `Skill` execution parity and bundled skills.
