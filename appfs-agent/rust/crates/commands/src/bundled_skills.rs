use std::collections::BTreeMap;

use runtime::{
    current_tool_session_compaction_summary, current_tool_session_messages, ContentBlock,
    ConversationMessage, MessageRole, SystemMessageSubtype,
};

use crate::{parse_skill_document, SkillDocument};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundledSkillId {
    Verify,
    Remember,
    Stuck,
    Skillify,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledSkillFile {
    pub relative_path: &'static str,
    pub content: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BundledSkill {
    pub id: BundledSkillId,
    pub document: SkillDocument,
}

impl BundledSkillId {
    fn name(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Remember => "remember",
            Self::Stuck => "stuck",
            Self::Skillify => "skillify",
        }
    }

    fn markdown(self) -> &'static str {
        match self {
            Self::Verify => VERIFY_SKILL_MD,
            Self::Remember => REMEMBER_SKILL_MD,
            Self::Stuck => STUCK_SKILL_MD,
            Self::Skillify => SKILLIFY_SKILL_MD,
        }
    }

    fn render_prompt(self, document: &SkillDocument, args: Option<&str>) -> String {
        match self {
            Self::Verify => {
                let mut parts = vec![document.markdown_content.trim().to_string()];
                if let Some(args) = args.map(str::trim).filter(|value| !value.is_empty()) {
                    parts.push(format!("## User Request\n\n{args}"));
                }
                parts.join("\n\n")
            }
            Self::Remember => {
                let mut prompt = document.markdown_content.trim_start().to_string();
                if let Some(args) = args.map(str::trim).filter(|value| !value.is_empty()) {
                    prompt.push_str("\n\n## Additional Context From User\n\n");
                    prompt.push_str(args);
                }
                prompt
            }
            Self::Stuck => {
                let mut prompt = document.markdown_content.trim_start().to_string();
                if let Some(args) = args.map(str::trim).filter(|value| !value.is_empty()) {
                    prompt.push_str("\n\n## User-Provided Context\n\n");
                    prompt.push_str(args);
                }
                prompt
            }
            Self::Skillify => render_skillify_prompt(document, args),
        }
    }

    fn reference_files(self) -> &'static [BundledSkillFile] {
        match self {
            Self::Verify => &[
                BundledSkillFile {
                    relative_path: "examples/cli.md",
                    content: VERIFY_EXAMPLE_CLI_MD,
                },
                BundledSkillFile {
                    relative_path: "examples/server.md",
                    content: VERIFY_EXAMPLE_SERVER_MD,
                },
            ],
            Self::Remember | Self::Stuck | Self::Skillify => &[],
        }
    }

    fn is_enabled(self) -> bool {
        match self {
            Self::Verify | Self::Remember | Self::Stuck | Self::Skillify => true,
        }
    }
}

#[must_use]
pub fn bundled_skill_inventory() -> Vec<BundledSkill> {
    [
        BundledSkillId::Verify,
        BundledSkillId::Remember,
        BundledSkillId::Stuck,
        BundledSkillId::Skillify,
    ]
    .into_iter()
    .filter(|skill| skill.is_enabled())
    .map(|id| BundledSkill {
        id,
        document: parse_skill_document(id.markdown(), id.name().to_string(), "Skill"),
    })
    .collect()
}

#[must_use]
pub fn resolve_bundled_skill(requested: &str) -> Option<BundledSkill> {
    let requested = requested.trim();
    if requested.is_empty() {
        return None;
    }

    bundled_skill_inventory().into_iter().find(|skill| {
        skill.document.resolved_name.eq_ignore_ascii_case(requested)
            || skill
                .document
                .user_facing_name()
                .eq_ignore_ascii_case(requested)
    })
}

#[must_use]
pub fn render_bundled_skill_prompt(skill: &BundledSkill, args: Option<&str>) -> String {
    skill.id.render_prompt(&skill.document, args)
}

#[must_use]
pub fn bundled_skill_reference_files(skill: &BundledSkill) -> BTreeMap<String, String> {
    skill
        .id
        .reference_files()
        .iter()
        .map(|file| (file.relative_path.to_string(), file.content.to_string()))
        .collect()
}

fn render_skillify_prompt(document: &SkillDocument, args: Option<&str>) -> String {
    let session_memory = current_tool_session_compaction_summary()
        .unwrap_or_else(|| "No session memory available.".to_string());
    let user_messages = render_skillify_user_messages(
        current_tool_session_messages()
            .as_deref()
            .map(extract_skillify_user_messages),
    );
    let user_description_block = args
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("The user described this process as: \"{value}\"\n\n"))
        .unwrap_or_default();

    document
        .markdown_content
        .trim_start()
        .replace("{{sessionMemory}}", &session_memory)
        .replace("{{userMessages}}", &user_messages)
        .replace("{{userDescriptionBlock}}", &user_description_block)
}

fn render_skillify_user_messages(user_messages: Option<Vec<String>>) -> String {
    let user_messages = user_messages.unwrap_or_default();
    if user_messages.is_empty() {
        "No recent user messages available.".to_string()
    } else {
        user_messages.join("\n\n---\n\n")
    }
}

fn extract_skillify_user_messages(messages: &[ConversationMessage]) -> Vec<String> {
    messages_after_compact_boundary(messages)
        .iter()
        .filter_map(|message| {
            if message.role != MessageRole::User
                || message.is_compact_summary
                || message.is_visible_in_transcript_only
                || message.attachment_metadata.is_some()
                || message.hook_result_metadata.is_some()
            {
                return None;
            }

            let text = message
                .blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n");

            (!text.trim().is_empty()).then_some(text)
        })
        .collect()
}

fn messages_after_compact_boundary(messages: &[ConversationMessage]) -> &[ConversationMessage] {
    let boundary_index = messages
        .iter()
        .rposition(|message| message.subtype == Some(SystemMessageSubtype::CompactBoundary));
    boundary_index.map_or(messages, |index| &messages[index..])
}

const VERIFY_SKILL_MD: &str = r"---
name: verify
description: Verify a code change does what it should by running the app.
allowed-tools:
  - Bash
  - Read
  - Glob
  - Grep
when_to_use: Use when the user wants the change exercised end-to-end instead of only reasoning about it.
---
# Verify

You are validating that the current code change actually works in the running app or CLI.

## Goal
Produce a concrete verification report based on commands you ran and behavior you observed.

## Expectations
- Prefer exercising the real user-facing flow over only reading code.
- Reuse existing scripts, dev commands, or test harnesses when the repo already defines them.
- If you need examples, inspect files in this skill's base directory, especially the `examples/` notes.
- If the app needs setup, do the minimum setup required to verify the specific change.

## Output
Provide:
1. What you ran
2. What you observed
3. Whether the change is verified, partially verified, or blocked
4. Any follow-up gaps the user should know about
";

const VERIFY_EXAMPLE_CLI_MD: &str = r#"# CLI Verification Example

1. Build or start the CLI in the cheapest realistic way.
2. Run the exact command flow affected by the change.
3. Capture the visible output and any files written.
4. Report the concrete pass/fail evidence, not just "looks good".
"#;

const VERIFY_EXAMPLE_SERVER_MD: &str = r"# Server Verification Example

1. Start the app or server with the repo's documented command.
2. Hit the changed endpoint or UI flow with a real request.
3. Check both the response and any user-visible side effects.
4. Summarize the verification steps and observed result.
";

const REMEMBER_SKILL_MD: &str = r"---
name: remember
description: Review memory-related context and propose promotions or cleanup across memory layers.
when_to_use: Use when the user wants to review, organize, or promote memory entries, or clean up conflicts between CLAUDE.md, CLAUDE.local.md, and auto-memory style notes.
---
# Memory Review

## Goal
Review the user's memory landscape and produce a clear report of proposed changes, grouped by action type. Do not apply changes unless the user explicitly asks.

## Steps

### 1. Gather all memory layers
Read `CLAUDE.md` and `CLAUDE.local.md` from the project root if they exist. Review any already-loaded memory context in the conversation and note which layers are present.

### 2. Classify what belongs where
For each substantive memory item, propose whether it belongs in:
- `CLAUDE.md` for repo-wide instructions
- `CLAUDE.local.md` for user-specific preferences
- shared/team memory if the repo clearly uses it
- the current transient memory layer if it is temporary or uncertain

### 3. Look for cleanup opportunities
Call out duplicates, outdated instructions, and conflicts between layers. Say which source seems newer or more authoritative when that is clear.

### 4. Present a proposal, not an edit
Group the report into:
1. Promotions
2. Cleanup
3. Ambiguous items that need the user's choice
4. No-action items

## Rules
- Present proposals before making changes
- Ask rather than guess when the destination is ambiguous
- Keep the report actionable and easy to approve item by item
";

const STUCK_SKILL_MD: &str = r"---
name: stuck
description: Investigate a frozen, stuck, or unusually slow agent session on this machine and produce a diagnostic report.
allowed-tools:
  - Bash
  - Read
when_to_use: Use when the user says another agent session appears frozen, hung, or extremely slow.
---
# Diagnose Stuck Session

The user believes another agent session on this machine is frozen, stuck, or unusually slow. Investigate and produce a concise diagnostic report.

## What to look for
- Sustained high CPU that suggests a loop
- Stopped, zombie, or uninterruptible processes
- A hung child process such as `git`, `node`, or a shell command
- Very large memory usage compared with nearby sessions

## Investigation steps
1. List relevant agent processes on this machine.
2. For suspicious processes, inspect child processes and collect more detail.
3. If logs or session files are available, read the most relevant tail rather than dumping everything.
4. Do not kill processes unless the user explicitly asks.

## Report
Explain:
1. Which process or session looks unhealthy
2. The evidence you found
3. Your best diagnosis
4. Safe next actions for the user
";

const SKILLIFY_SKILL_MD: &str = r#"---
name: skillify
description: Capture this session's repeatable process into a reusable skill.
allowed-tools:
  - Read
  - Write
  - Edit
  - Glob
  - Grep
  - AskUserQuestion
  - Bash(mkdir:*)
argument-hint: "[description of the process you want to capture]"
when_to_use: Use when the user wants to turn the process from this session into a reusable skill, especially near the end of the workflow.
---
# Skillify

{{userDescriptionBlock}}You are capturing this session's repeatable process as a reusable skill.

## Your Session Context

Here is the session memory summary:
<session_memory>
{{sessionMemory}}
</session_memory>

Here are the user's messages during this session. Pay attention to how they steered the process so you can preserve their preferences in the skill:
<user_messages>
{{userMessages}}
</user_messages>

## Your Task

### Step 1: Analyze the session

Before asking any questions, analyze the session to identify:
- What repeatable process was performed
- What the inputs or parameters were
- The distinct steps, in order
- The success artifacts or criteria for each step
- Where the user corrected or steered you
- What tools and permissions were needed
- What agents were used
- What the goals and success artifacts were

### Step 2: Interview the user

Use `AskUserQuestion` for all questions. Never ask these questions in plain assistant text.

For each round:
- Iterate until the user is happy.
- Offer substantive choices only. The user already has a freeform option if they want to type edits.
- Do not over-interview simple workflows.

Round 1: High-level confirmation
- Suggest a name and description for the skill based on your analysis.
- Suggest the high-level goal and success criteria.

Round 2: Workflow structure
- Present the high-level steps you identified as a numbered list.
- If the skill needs arguments, suggest them based on what you observed.
- If it is not obvious, ask whether this skill should run inline or forked.
- Ask where the skill should be saved. Suggest a default based on context. Options:
  - This repo (`.claw/skills/<name>/SKILL.md`) for workflows specific to this project
  - Personal (`~/.claw/skills/<name>/SKILL.md`) for workflows that should follow the user across repos

Round 3: Break down each step
For each major step, ask whatever is needed to clarify:
- What this step produces that later steps need
- What proves the step succeeded
- Whether the user should confirm before proceeding
- Whether any steps can run in parallel
- Whether any steps should use a task agent or teammate
- What hard constraints or hard preferences must be preserved

Round 4: Final questions
- Confirm when the skill should be invoked and suggest trigger phrases
- Ask about any remaining gotchas only if they still matter

### Step 3: Write the SKILL.md

Create the skill directory and file at the location the user chose in Round 2.

Use this format:

```markdown
---
name: {{skill-name}}
description: {{one-line description}}
allowed-tools:
  {{list of tool permission patterns observed during session}}
when_to_use: {{detailed description of when the skill should be invoked, including trigger phrases and example user messages}}
argument-hint: "{{hint showing argument placeholders}}"
arguments:
  {{list of argument names}}
context: {{inline or fork -- omit for inline}}
---

# {{Skill Title}}
Description of skill

## Inputs
- `$arg_name`: Description of this input

## Goal
Clearly stated goal for this workflow. Prefer explicit artifacts or criteria for completion.

## Steps

### 1. Step Name
What to do in this step. Be specific and actionable.

**Success criteria**: Always include this. It should be clear when the step is done and the skill can move on.
```

Per-step annotations:
- `Success criteria` is required on every step.
- `Execution`: `Direct` by default, or `Task agent`, `Teammate`, or `[human]` when needed.
- `Artifacts`: Only include this when later steps depend on outputs from the current step.
- `Human checkpoint`: Use this for irreversible actions, judgment calls, or output review.
- `Rules`: Capture hard requirements, especially where the user corrected you during the session.

Step structure tips:
- Steps that can run concurrently can use sub-numbers such as `3a` and `3b`.
- Steps requiring the user to act should use `[human]` in the title.
- Keep simple skills simple.

Frontmatter rules:
- `allowed-tools`: Capture the minimum permissions needed. Prefer precise patterns such as `Bash(gh:*)`.
- `context`: Only set `context: fork` for self-contained skills that do not need mid-process user input.
- `when_to_use` is critical. Start with `Use when...` and include trigger phrases.
- `arguments` and `argument-hint`: Only include these when the skill takes parameters.

### Step 4: Confirm and save

Before writing the file:
- Output the complete `SKILL.md` content as a `yaml` fenced code block so the user can review it.
- Ask for confirmation using `AskUserQuestion` with a concise question such as `Does this SKILL.md look good to save?`

After writing, tell the user:
- Where the skill was saved
- How to invoke it: `/{{skill-name}} [arguments]`
- That they can edit the `SKILL.md` directly to refine it
"#;

#[cfg(test)]
mod tests {
    use super::{
        bundled_skill_inventory, bundled_skill_reference_files, render_bundled_skill_prompt,
        resolve_bundled_skill,
    };
    use runtime::{
        with_tool_session_snapshot, AttachmentKind, CompactBoundaryMetadata, CompactTrigger,
        ConversationMessage,
    };

    #[test]
    fn resolve_bundled_skill_matches_name_case_insensitively() {
        let skill = resolve_bundled_skill("VeRiFy").expect("verify skill should resolve");
        assert_eq!(skill.document.resolved_name, "verify");
        assert_eq!(
            render_bundled_skill_prompt(&skill, Some("check login flow")),
            concat!(
                "# Verify\n\n",
                "You are validating that the current code change actually works in the running app or CLI.\n\n",
                "## Goal\n",
                "Produce a concrete verification report based on commands you ran and behavior you observed.\n\n",
                "## Expectations\n",
                "- Prefer exercising the real user-facing flow over only reading code.\n",
                "- Reuse existing scripts, dev commands, or test harnesses when the repo already defines them.\n",
                "- If you need examples, inspect files in this skill's base directory, especially the `examples/` notes.\n",
                "- If the app needs setup, do the minimum setup required to verify the specific change.\n\n",
                "## Output\n",
                "Provide:\n",
                "1. What you ran\n",
                "2. What you observed\n",
                "3. Whether the change is verified, partially verified, or blocked\n",
                "4. Any follow-up gaps the user should know about\n\n",
                "## User Request\n\n",
                "check login flow"
            )
        );
    }

    #[test]
    fn bundled_skill_inventory_exposes_reference_files() {
        let verify = bundled_skill_inventory()
            .into_iter()
            .find(|skill| skill.document.resolved_name == "verify")
            .expect("verify should exist");
        let files = bundled_skill_reference_files(&verify);
        assert_eq!(files.len(), 2);
        assert!(files.contains_key("examples/cli.md"));
        assert!(files.contains_key("examples/server.md"));
    }

    #[test]
    fn skillify_prompt_uses_session_snapshot_and_recent_user_messages() {
        let skill = resolve_bundled_skill("skillify").expect("skillify skill should resolve");
        let messages = vec![
            ConversationMessage::user_text("before compact"),
            ConversationMessage::compact_boundary(CompactBoundaryMetadata {
                trigger: CompactTrigger::Manual,
                pre_tokens: 128,
                user_context: None,
                messages_summarized: Some(2),
                pre_compact_discovered_tools: Vec::new(),
                preserved_segment: None,
            }),
            ConversationMessage::compact_summary_user_text("summarized history", false),
            ConversationMessage::user_text("please preserve my review workflow"),
            ConversationMessage::attachment_user_text("todo state", AttachmentKind::TodoList),
        ];

        let prompt = with_tool_session_snapshot(&messages, Some("Older session summary"), || {
            render_bundled_skill_prompt(&skill, Some("capture this workflow"))
        });

        assert!(prompt.contains("Older session summary"));
        assert!(prompt.contains("please preserve my review workflow"));
        assert!(prompt.contains("The user described this process as: \"capture this workflow\""));
        assert!(!prompt.contains("before compact"));
        assert!(!prompt.contains("summarized history"));
        assert!(!prompt.contains("todo state"));
    }
}
