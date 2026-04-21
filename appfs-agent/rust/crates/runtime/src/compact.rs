use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::session::{
    AttachmentKind, CompactBoundaryMetadata, CompactPreservedSegment, CompactTrigger, ContentBlock,
    ConversationMessage, MessageRole, Session,
};

const NO_TOOLS_PREAMBLE: &str = r"CRITICAL: Respond with TEXT ONLY. Do NOT call any tools.

- Do NOT use Read, Bash, Grep, Glob, Edit, Write, or any other tool.
- You already have all the context you need in the conversation above.
- Tool calls will be rejected and will waste your only turn.
- Your entire response must be plain text: an <analysis> block followed by a <summary> block.

";
const NO_TOOLS_TRAILER: &str = "\n\nREMINDER: Do NOT call any tools. Respond with plain text only - an <analysis> block followed by a <summary> block. Tool calls will be rejected and you will fail the task.";
const DETAILED_ANALYSIS_INSTRUCTION_BASE: &str = r"Before providing your final summary, wrap your analysis in <analysis> tags to organize your thoughts and ensure you've covered all necessary points. In your analysis process:

1. Chronologically analyze each message and section of the conversation. For each section thoroughly identify:
   - The user's explicit requests and intents
   - Your approach to addressing the user's requests
   - Key decisions, technical concepts, and code patterns
   - Specific details like:
     - file names
     - full code snippets
     - function signatures
     - file edits
   - Errors that you ran into and how you fixed them
   - Pay special attention to specific user feedback that you received, especially if the user told you to do something differently.
2. Double-check for technical accuracy and completeness, addressing each required element thoroughly.";
const BASE_COMPACT_PROMPT: &str = r"Your task is to create a detailed summary of the conversation so far, paying close attention to the user's explicit requests and your previous actions.
This summary should be thorough in capturing technical details, code patterns, and architectural decisions that would be essential for continuing development work without losing context.

{analysis_instruction}

Your summary should include the following sections:

1. Primary Request and Intent: Capture all of the user's explicit requests and intents in detail
2. Key Technical Concepts: List all important technical concepts, technologies, and frameworks discussed.
3. Files and Code Sections: Enumerate specific files and code sections examined, modified, or created. Pay special attention to the most recent messages and include full code snippets where applicable and a summary of why each file read or edit is important.
4. Errors and fixes: List all errors that you ran into, and how you fixed them. Pay special attention to specific user feedback that you received, especially if the user told you to do something differently.
5. Problem Solving: Document problems solved and any ongoing troubleshooting efforts.
6. All user messages: List all user messages that are not tool results. These are critical for understanding the user's feedback and changing intent.
7. Pending Tasks: Outline any pending tasks that you have explicitly been asked to work on.
8. Current Work: Describe in detail precisely what was being worked on immediately before this summary request, paying special attention to the most recent messages from both user and assistant. Include file names and code snippets where applicable.
9. Optional Next Step: List the next step that you will take that is related to the most recent work you were doing. Ensure that this step is directly in line with the user's most recent explicit requests and the task you were working on immediately before this summary request. If there is a next step, include direct quotes from the most recent conversation showing exactly what task you were working on and where you left off.

Use this exact output structure:

<analysis>
[your drafting analysis]
</analysis>

<summary>
1. Primary Request and Intent:
   [Detailed description]

2. Key Technical Concepts:
   - [Concept 1]
   - [Concept 2]

3. Files and Code Sections:
   - [File Name 1]
     - [Why it matters]
     - [Changes made]
     - [Important snippet]

4. Errors and fixes:
   - [Error description]
     - [How you fixed it]
     - [Relevant user feedback]

5. Problem Solving:
   [Description]

6. All user messages:
   - [Detailed non-tool user message]

7. Pending Tasks:
   - [Task 1]

8. Current Work:
   [Precise description of current work]

9. Optional Next Step:
   [Optional next step]
</summary>

Please provide your summary based on the conversation so far, following this structure and ensuring precision and thoroughness in your response.

There may be additional summarization instructions provided in the included context. If so, remember to follow them when creating the summary.";

const COMPACT_CONTINUATION_PREAMBLE: &str =
    "This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation.\n\n";
const COMPACT_RECENT_MESSAGES_NOTE: &str = "Recent messages are preserved verbatim.";
const COMPACT_BOUNDARY_TEXT: &str = "Conversation compacted";
const COMPACT_DIRECT_RESUME_INSTRUCTION: &str = "Continue the conversation from where it left off without asking the user any further questions. Resume directly — do not acknowledge the summary, do not recap what was happening, and do not preface with continuation text.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionConfig {
    pub preserve_recent_messages: usize,
    pub max_estimated_tokens: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            preserve_recent_messages: 4,
            max_estimated_tokens: 10_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionResult {
    pub summary: String,
    pub formatted_summary: String,
    pub compacted_session: Session,
    pub removed_message_count: usize,
    pub user_display_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompactionLayout {
    LegacyResumableSummary,
    TsBoundaryAndSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreservedSegmentAnchor {
    LastSummaryMessage,
}

struct CompactedMessageOptions<'a> {
    summary: &'a str,
    post_compact_context_messages: Vec<ConversationMessage>,
    hook_result_messages: Vec<ConversationMessage>,
    preserved_messages: Vec<ConversationMessage>,
    preserved_segment_anchor: Option<PreservedSegmentAnchor>,
    suppress_follow_up_questions: bool,
    layout: CompactionLayout,
    boundary_trigger: Option<CompactTrigger>,
    pre_tokens: usize,
    removed_message_count: usize,
    source_messages: &'a [ConversationMessage],
}

pub(crate) struct BuildCompactionResultOptions {
    pub preserved_messages: Vec<ConversationMessage>,
    pub hook_result_messages: Vec<ConversationMessage>,
    pub preserved_segment_anchor: Option<PreservedSegmentAnchor>,
    pub removed_message_count: usize,
    pub suppress_follow_up_questions: bool,
    pub layout: CompactionLayout,
    pub boundary_trigger: Option<CompactTrigger>,
    pub user_display_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FullCompactSelection {
    pub messages_to_summarize: Vec<ConversationMessage>,
    pub preserved_messages: Vec<ConversationMessage>,
}

#[derive(Debug, Deserialize)]
struct CompactTodoItem {
    content: String,
    #[serde(rename = "activeForm")]
    active_form: String,
    status: CompactTodoStatus,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CompactTodoStatus {
    Pending,
    InProgress,
    Completed,
}

impl CompactTodoStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Deserialize)]
struct CompactSkillOutput {
    skill: String,
    path: String,
    args: Option<String>,
    description: Option<String>,
    prompt: String,
}

#[derive(Debug, Deserialize)]
struct CompactPlanModeOutput {
    active: bool,
    managed: bool,
    message: String,
    #[serde(rename = "settingsPath")]
    settings_path: String,
    #[serde(rename = "statePath")]
    state_path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CompactAgentOutput {
    #[serde(rename = "agentId")]
    agent_id: String,
    name: String,
    description: String,
    status: String,
    #[serde(rename = "outputFile")]
    output_file: String,
    #[serde(rename = "manifestFile")]
    manifest_file: String,
    #[serde(rename = "derivedState")]
    derived_state: String,
}

#[must_use]
pub fn estimate_session_tokens(session: &Session) -> usize {
    session.messages.iter().map(estimate_message_tokens).sum()
}

#[must_use]
pub fn should_compact(session: &Session, config: CompactionConfig) -> bool {
    let start = compacted_summary_prefix_len(session);
    let compactable = &session.messages[start..];

    compactable.len() > config.preserve_recent_messages
        && compactable
            .iter()
            .map(estimate_message_tokens)
            .sum::<usize>()
            >= config.max_estimated_tokens
}

#[must_use]
pub fn format_compact_summary(summary: &str) -> String {
    let without_analysis = strip_tag_block(summary, "analysis");
    let formatted = if let Some(content) = extract_tag_block(&without_analysis, "summary") {
        without_analysis.replace(
            &format!("<summary>{content}</summary>"),
            &format!("Summary:\n{}", content.trim()),
        )
    } else {
        without_analysis
    };

    collapse_blank_lines(&formatted).trim().to_string()
}

#[must_use]
/// Builds the TS-style full compact prompt for a text-only summarization turn.
pub fn get_compact_prompt(custom_instructions: Option<&str>) -> String {
    let mut prompt = NO_TOOLS_PREAMBLE.to_string();
    prompt.push_str(
        &BASE_COMPACT_PROMPT.replace("{analysis_instruction}", DETAILED_ANALYSIS_INSTRUCTION_BASE),
    );

    if let Some(instructions) = custom_instructions
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        prompt.push_str("\n\nAdditional Instructions:\n");
        prompt.push_str(instructions);
    }

    prompt.push_str(NO_TOOLS_TRAILER);
    prompt
}

#[must_use]
pub fn get_compact_continuation_message(
    summary: &str,
    suppress_follow_up_questions: bool,
    recent_messages_preserved: bool,
) -> String {
    let mut base = format!(
        "{COMPACT_CONTINUATION_PREAMBLE}{}",
        format_compact_summary(summary)
    );

    if recent_messages_preserved {
        base.push_str("\n\n");
        base.push_str(COMPACT_RECENT_MESSAGES_NOTE);
    }

    if suppress_follow_up_questions {
        base.push('\n');
        base.push_str(COMPACT_DIRECT_RESUME_INSTRUCTION);
    }

    base
}

#[must_use]
pub fn get_compact_user_summary_message(
    summary: &str,
    suppress_follow_up_questions: bool,
    recent_messages_preserved: bool,
) -> String {
    let mut base = format!(
        "{COMPACT_CONTINUATION_PREAMBLE}{}",
        format_compact_summary(summary)
    );

    if recent_messages_preserved {
        base.push_str("\n\n");
        base.push_str(COMPACT_RECENT_MESSAGES_NOTE);
    }

    if suppress_follow_up_questions {
        base.push('\n');
        base.push_str(COMPACT_DIRECT_RESUME_INSTRUCTION);
    }

    base
}

fn build_compacted_messages(options: CompactedMessageOptions<'_>) -> Vec<ConversationMessage> {
    let CompactedMessageOptions {
        summary,
        post_compact_context_messages,
        hook_result_messages,
        preserved_messages,
        preserved_segment_anchor,
        suppress_follow_up_questions,
        layout,
        boundary_trigger,
        pre_tokens,
        removed_message_count,
        source_messages,
    } = options;

    let recent_messages_preserved = !preserved_messages.is_empty();
    let mut compacted_messages = match layout {
        CompactionLayout::LegacyResumableSummary => vec![ConversationMessage::system_text(
            get_compact_continuation_message(
                summary,
                suppress_follow_up_questions,
                recent_messages_preserved,
            ),
        )],
        CompactionLayout::TsBoundaryAndSummary => {
            let summary_message = ConversationMessage::compact_summary_user_text(
                get_compact_user_summary_message(
                    summary,
                    suppress_follow_up_questions,
                    recent_messages_preserved,
                ),
                !recent_messages_preserved,
            );
            let boundary_message = build_ts_compact_boundary_message(
                boundary_trigger.unwrap_or(CompactTrigger::Manual),
                pre_tokens,
                removed_message_count,
                source_messages,
                &preserved_messages,
                preserved_segment_anchor,
                Some(summary_message.uuid.as_str()),
            );
            vec![boundary_message, summary_message]
        }
    };
    compacted_messages.extend(preserved_messages);
    compacted_messages.extend(post_compact_context_messages);
    compacted_messages.extend(hook_result_messages);
    compacted_messages
}

fn build_ts_compact_boundary_message(
    trigger: CompactTrigger,
    pre_tokens: usize,
    removed_message_count: usize,
    source_messages: &[ConversationMessage],
    preserved_messages: &[ConversationMessage],
    preserved_segment_anchor: Option<PreservedSegmentAnchor>,
    summary_message_uuid: Option<&str>,
) -> ConversationMessage {
    ConversationMessage::compact_boundary(CompactBoundaryMetadata {
        trigger,
        pre_tokens,
        user_context: None,
        messages_summarized: Some(removed_message_count),
        pre_compact_discovered_tools: collect_discovered_tools(source_messages),
        preserved_segment: build_preserved_segment_metadata(
            preserved_messages,
            preserved_segment_anchor,
            summary_message_uuid,
        ),
    })
}

fn build_preserved_segment_metadata(
    preserved_messages: &[ConversationMessage],
    anchor: Option<PreservedSegmentAnchor>,
    summary_message_uuid: Option<&str>,
) -> Option<CompactPreservedSegment> {
    if preserved_messages.is_empty() {
        return None;
    }
    let anchor_uuid = match anchor.unwrap_or(PreservedSegmentAnchor::LastSummaryMessage) {
        PreservedSegmentAnchor::LastSummaryMessage => summary_message_uuid?,
    };
    Some(CompactPreservedSegment {
        head: preserved_messages.first()?.uuid.clone(),
        anchor: anchor_uuid.to_string(),
        tail: preserved_messages.last()?.uuid.clone(),
    })
}

/// Rewrites a session into its post-compaction form.
pub(crate) fn build_compaction_result(
    session: &Session,
    summary: String,
    options: BuildCompactionResultOptions,
) -> CompactionResult {
    let BuildCompactionResultOptions {
        preserved_messages,
        hook_result_messages,
        preserved_segment_anchor,
        removed_message_count,
        suppress_follow_up_questions,
        layout,
        boundary_trigger,
        user_display_message,
    } = options;
    let formatted_summary = format_compact_summary(&summary);
    let pre_tokens = estimate_session_tokens(session);
    let post_compact_context_messages = match layout {
        CompactionLayout::LegacyResumableSummary => Vec::new(),
        CompactionLayout::TsBoundaryAndSummary => collect_post_compact_context_messages(session),
    };
    let compacted_messages = build_compacted_messages(CompactedMessageOptions {
        summary: &summary,
        post_compact_context_messages,
        hook_result_messages,
        preserved_messages,
        preserved_segment_anchor,
        suppress_follow_up_questions,
        layout,
        boundary_trigger,
        pre_tokens,
        removed_message_count,
        source_messages: &session.messages,
    });

    let mut compacted_session = session.clone();
    compacted_session.messages = compacted_messages;
    compacted_session.record_compaction(summary.clone(), removed_message_count);

    CompactionResult {
        summary,
        formatted_summary,
        compacted_session,
        removed_message_count,
        user_display_message,
    }
}

#[must_use]
pub(crate) fn select_full_compact_messages(
    session: &Session,
    preserve_recent_messages: usize,
) -> FullCompactSelection {
    if preserve_recent_messages == 0 {
        return FullCompactSelection {
            messages_to_summarize: session.messages.clone(),
            preserved_messages: Vec::new(),
        };
    }

    let compacted_prefix_len = compacted_summary_prefix_len(session);
    let keep_from = session
        .messages
        .len()
        .saturating_sub(preserve_recent_messages)
        .max(compacted_prefix_len);

    FullCompactSelection {
        messages_to_summarize: session.messages[..keep_from].to_vec(),
        preserved_messages: session.messages[keep_from..].to_vec(),
    }
}

fn collect_post_compact_context_messages(session: &Session) -> Vec<ConversationMessage> {
    let mut messages = Vec::new();

    if let Some(running_agents) = create_running_agents_context_message(&session.messages) {
        messages.push(running_agents);
    }
    if let Some(todo_list) = create_todo_context_message(session) {
        messages.push(todo_list);
    }
    if let Some(plan_mode) = create_plan_mode_context_message(&session.messages) {
        messages.push(plan_mode);
    }
    if let Some(skills) = create_invoked_skills_context_message(&session.messages) {
        messages.push(skills);
    }

    messages
}

fn create_running_agents_context_message(
    messages: &[ConversationMessage],
) -> Option<ConversationMessage> {
    let mut latest_agents = BTreeMap::new();
    for output in successful_tool_outputs(messages, "Agent") {
        let Some(parsed) = parse_tool_result_json_prefix::<CompactAgentOutput>(output) else {
            continue;
        };
        let refreshed = refresh_agent_manifest(&parsed).unwrap_or(parsed);
        latest_agents.insert(refreshed.agent_id.clone(), refreshed);
    }

    let running_agents = latest_agents
        .into_values()
        .filter(|agent| agent.status.eq_ignore_ascii_case("running"))
        .collect::<Vec<_>>();
    if running_agents.is_empty() {
        return None;
    }

    let mut content = String::from(
        "Background agents are still active after compaction. Avoid spawning duplicate agents unless the user explicitly asks.",
    );
    for agent in running_agents {
        let _ = write!(
            content,
            "\n- `{}` (`{}`): {}",
            agent.name,
            agent.agent_id,
            agent.description.trim()
        );
        let _ = write!(content, "\n  Status: {}", agent.status);
        if !agent.derived_state.trim().is_empty() {
            let _ = write!(content, "\n  Derived state: {}", agent.derived_state.trim());
        }
        let _ = write!(content, "\n  Output file: {}", agent.output_file);
    }

    Some(ConversationMessage::attachment_user_text(
        content,
        AttachmentKind::RunningAgents,
    ))
}

fn refresh_agent_manifest(agent: &CompactAgentOutput) -> Option<CompactAgentOutput> {
    let manifest = fs::read_to_string(&agent.manifest_file).ok()?;
    serde_json::from_str(&manifest).ok()
}

fn create_todo_context_message(session: &Session) -> Option<ConversationMessage> {
    let todo_store_path = todo_store_path_for_session(session)?;
    let contents = fs::read_to_string(&todo_store_path).ok()?;
    let todos = serde_json::from_str::<Vec<CompactTodoItem>>(&contents).ok()?;
    if todos.is_empty() {
        return None;
    }

    let mut content = format!(
        "Current TodoWrite task list remains active after compaction.\nSource: {}",
        todo_store_path.display()
    );
    for todo in todos {
        let _ = write!(
            content,
            "\n- [{}] {} (active form: {})",
            todo.status.label(),
            todo.content.trim(),
            todo.active_form.trim()
        );
    }

    Some(ConversationMessage::attachment_user_text(
        content,
        AttachmentKind::TodoList,
    ))
}

fn todo_store_path_for_session(session: &Session) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CLAWD_TODO_STORE") {
        return Some(PathBuf::from(path));
    }

    if let Some(workspace_root) = session.workspace_root() {
        return Some(workspace_root.join(".clawd-todos.json"));
    }

    session
        .persistence_path()
        .and_then(|_| std::env::current_dir().ok())
        .map(|cwd| cwd.join(".clawd-todos.json"))
}

fn create_plan_mode_context_message(
    messages: &[ConversationMessage],
) -> Option<ConversationMessage> {
    let latest_state = messages
        .iter()
        .rev()
        .flat_map(|message| message.blocks.iter().rev())
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_name,
                output,
                is_error,
                ..
            } if !*is_error && matches!(tool_name.as_str(), "EnterPlanMode" | "ExitPlanMode") => {
                parse_tool_result_json_prefix::<CompactPlanModeOutput>(output)
            }
            ContentBlock::Text { .. }
            | ContentBlock::ToolUse { .. }
            | ContentBlock::ToolResult { .. } => None,
        })?;

    if !latest_state.active {
        return None;
    }

    let managed_text = if latest_state.managed {
        "The override is still managed by EnterPlanMode."
    } else {
        "The worktree is still in local plan mode."
    };
    Some(ConversationMessage::attachment_user_text(
        format!(
            "Plan mode is still active for this worktree after compaction.\n{managed_text}\nLast plan-mode message: {}\nSettings path: {}\nState path: {}\nContinue exploring and planning until the user explicitly approves implementation.",
            latest_state.message.trim(),
            latest_state.settings_path,
            latest_state.state_path
        ),
        AttachmentKind::PlanMode,
    ))
}

fn create_invoked_skills_context_message(
    messages: &[ConversationMessage],
) -> Option<ConversationMessage> {
    let mut skills = BTreeMap::new();
    for output in successful_tool_outputs(messages, "Skill") {
        let Some(parsed) = parse_tool_result_json_prefix::<CompactSkillOutput>(output) else {
            continue;
        };
        skills.insert(parsed.skill.clone(), parsed);
    }

    if skills.is_empty() {
        return None;
    }

    let mut content = String::from("Previously invoked skills remain available after compaction.");
    for skill in skills.into_values() {
        let _ = write!(content, "\n\nSkill `{}`", skill.skill);
        let _ = write!(content, "\nPath: {}", skill.path);
        if let Some(description) = skill.description.as_deref().map(str::trim) {
            if !description.is_empty() {
                let _ = write!(content, "\nDescription: {description}");
            }
        }
        if let Some(args) = skill.args.as_deref().map(str::trim) {
            if !args.is_empty() {
                let _ = write!(content, "\nArgs: {args}");
            }
        }
        content.push_str("\nInstructions:\n");
        content.push_str(skill.prompt.trim());
    }

    Some(ConversationMessage::attachment_user_text(
        content,
        AttachmentKind::InvokedSkills,
    ))
}

fn successful_tool_outputs<'a>(
    messages: &'a [ConversationMessage],
    tool_name: &'a str,
) -> impl Iterator<Item = &'a str> + 'a {
    messages
        .iter()
        .flat_map(|message| message.blocks.iter())
        .filter_map(move |block| match block {
            ContentBlock::ToolResult {
                tool_name: block_tool_name,
                output,
                is_error,
                ..
            } if !*is_error && block_tool_name == tool_name => Some(output.as_str()),
            ContentBlock::Text { .. }
            | ContentBlock::ToolUse { .. }
            | ContentBlock::ToolResult { .. } => None,
        })
}

fn parse_tool_result_json_prefix<T: DeserializeOwned>(output: &str) -> Option<T> {
    let mut deserializer = serde_json::Deserializer::from_str(output.trim_start());
    T::deserialize(&mut deserializer).ok()
}

#[must_use]
/// Local heuristic compaction fallback used when a model-driven full compact
/// is unavailable.
pub fn compact_session(session: &Session, config: CompactionConfig) -> CompactionResult {
    if !should_compact(session, config) {
        return CompactionResult {
            summary: String::new(),
            formatted_summary: String::new(),
            compacted_session: session.clone(),
            removed_message_count: 0,
            user_display_message: None,
        };
    }

    let existing_summary = extract_existing_compacted_summary(&session.messages);
    let compacted_prefix_len = existing_summary
        .as_ref()
        .map_or(0, |(prefix_len, _)| *prefix_len);
    let keep_from = session
        .messages
        .len()
        .saturating_sub(config.preserve_recent_messages);
    let removed = &session.messages[compacted_prefix_len..keep_from];
    let preserved = session.messages[keep_from..].to_vec();
    let summary = merge_compact_summaries(
        existing_summary
            .as_ref()
            .map(|(_, summary)| summary.as_str()),
        &summarize_messages(removed),
    );
    build_compaction_result(
        session,
        summary,
        BuildCompactionResultOptions {
            preserved_messages: preserved,
            hook_result_messages: Vec::new(),
            preserved_segment_anchor: None,
            removed_message_count: removed.len(),
            suppress_follow_up_questions: true,
            layout: CompactionLayout::LegacyResumableSummary,
            boundary_trigger: None,
            user_display_message: None,
        },
    )
}

fn compacted_summary_prefix_len(session: &Session) -> usize {
    extract_existing_compacted_summary(&session.messages).map_or(0, |(prefix_len, _)| prefix_len)
}

fn collect_discovered_tools(messages: &[ConversationMessage]) -> Vec<String> {
    let mut tool_names = messages
        .iter()
        .flat_map(|message| message.blocks.iter())
        .filter_map(|block| match block {
            ContentBlock::ToolUse { name, .. } => Some(name.as_str()),
            ContentBlock::ToolResult { tool_name, .. } => Some(tool_name.as_str()),
            ContentBlock::Text { .. } => None,
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    tool_names.sort_unstable();
    tool_names.dedup();
    tool_names
}

fn summarize_messages(messages: &[ConversationMessage]) -> String {
    let user_messages = messages
        .iter()
        .filter(|message| message.role == MessageRole::User)
        .count();
    let assistant_messages = messages
        .iter()
        .filter(|message| message.role == MessageRole::Assistant)
        .count();
    let tool_messages = messages
        .iter()
        .filter(|message| message.role == MessageRole::Tool)
        .count();

    let tool_names = collect_discovered_tools(messages);

    let mut lines = vec![
        "<summary>".to_string(),
        "Conversation summary:".to_string(),
        format!(
            "- Scope: {} earlier messages compacted (user={}, assistant={}, tool={}).",
            messages.len(),
            user_messages,
            assistant_messages,
            tool_messages
        ),
    ];

    if !tool_names.is_empty() {
        lines.push(format!("- Tools mentioned: {}.", tool_names.join(", ")));
    }

    let recent_user_requests = collect_recent_role_summaries(messages, MessageRole::User, 3);
    if !recent_user_requests.is_empty() {
        lines.push("- Recent user requests:".to_string());
        lines.extend(
            recent_user_requests
                .into_iter()
                .map(|request| format!("  - {request}")),
        );
    }

    let pending_work = infer_pending_work(messages);
    if !pending_work.is_empty() {
        lines.push("- Pending work:".to_string());
        lines.extend(pending_work.into_iter().map(|item| format!("  - {item}")));
    }

    let key_files = collect_key_files(messages);
    if !key_files.is_empty() {
        lines.push(format!("- Key files referenced: {}.", key_files.join(", ")));
    }

    if let Some(current_work) = infer_current_work(messages) {
        lines.push(format!("- Current work: {current_work}"));
    }

    lines.push("- Key timeline:".to_string());
    for message in messages {
        let role = match message.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
        };
        let content = message
            .blocks
            .iter()
            .map(summarize_block)
            .collect::<Vec<_>>()
            .join(" | ");
        lines.push(format!("  - {role}: {content}"));
    }
    lines.push("</summary>".to_string());
    lines.join("\n")
}

fn merge_compact_summaries(existing_summary: Option<&str>, new_summary: &str) -> String {
    let Some(existing_summary) = existing_summary else {
        return new_summary.to_string();
    };

    let previous_highlights = extract_summary_highlights(existing_summary);
    let new_formatted_summary = format_compact_summary(new_summary);
    let new_highlights = extract_summary_highlights(&new_formatted_summary);
    let new_timeline = extract_summary_timeline(&new_formatted_summary);

    let mut lines = vec!["<summary>".to_string(), "Conversation summary:".to_string()];

    if !previous_highlights.is_empty() {
        lines.push("- Previously compacted context:".to_string());
        lines.extend(
            previous_highlights
                .into_iter()
                .map(|line| format!("  {line}")),
        );
    }

    if !new_highlights.is_empty() {
        lines.push("- Newly compacted context:".to_string());
        lines.extend(new_highlights.into_iter().map(|line| format!("  {line}")));
    }

    if !new_timeline.is_empty() {
        lines.push("- Key timeline:".to_string());
        lines.extend(new_timeline.into_iter().map(|line| format!("  {line}")));
    }

    lines.push("</summary>".to_string());
    lines.join("\n")
}

fn summarize_block(block: &ContentBlock) -> String {
    let raw = match block {
        ContentBlock::Text { text } => text.clone(),
        ContentBlock::ToolUse { name, input, .. } => format!("tool_use {name}({input})"),
        ContentBlock::ToolResult {
            tool_name,
            output,
            is_error,
            ..
        } => format!(
            "tool_result {tool_name}: {}{output}",
            if *is_error { "error " } else { "" }
        ),
    };
    truncate_summary(&raw, 160)
}

fn collect_recent_role_summaries(
    messages: &[ConversationMessage],
    role: MessageRole,
    limit: usize,
) -> Vec<String> {
    messages
        .iter()
        .filter(|message| message.role == role)
        .rev()
        .filter_map(|message| first_text_block(message))
        .take(limit)
        .map(|text| truncate_summary(text, 160))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn infer_pending_work(messages: &[ConversationMessage]) -> Vec<String> {
    messages
        .iter()
        .rev()
        .filter_map(first_text_block)
        .filter(|text| {
            let lowered = text.to_ascii_lowercase();
            lowered.contains("todo")
                || lowered.contains("next")
                || lowered.contains("pending")
                || lowered.contains("follow up")
                || lowered.contains("remaining")
        })
        .take(3)
        .map(|text| truncate_summary(text, 160))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn collect_key_files(messages: &[ConversationMessage]) -> Vec<String> {
    let mut files = messages
        .iter()
        .flat_map(|message| message.blocks.iter())
        .map(|block| match block {
            ContentBlock::Text { text } => text.as_str(),
            ContentBlock::ToolUse { input, .. } => input.as_str(),
            ContentBlock::ToolResult { output, .. } => output.as_str(),
        })
        .flat_map(extract_file_candidates)
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    files.into_iter().take(8).collect()
}

fn infer_current_work(messages: &[ConversationMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter_map(first_text_block)
        .find(|text| !text.trim().is_empty())
        .map(|text| truncate_summary(text, 200))
}

fn first_text_block(message: &ConversationMessage) -> Option<&str> {
    message.blocks.iter().find_map(|block| match block {
        ContentBlock::Text { text } if !text.trim().is_empty() => Some(text.as_str()),
        ContentBlock::ToolUse { .. }
        | ContentBlock::ToolResult { .. }
        | ContentBlock::Text { .. } => None,
    })
}

fn has_interesting_extension(candidate: &str) -> bool {
    std::path::Path::new(candidate)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["rs", "ts", "tsx", "js", "json", "md"]
                .iter()
                .any(|expected| extension.eq_ignore_ascii_case(expected))
        })
}

fn extract_file_candidates(content: &str) -> Vec<String> {
    content
        .split_whitespace()
        .filter_map(|token| {
            let candidate = token.trim_matches(|char: char| {
                matches!(char, ',' | '.' | ':' | ';' | ')' | '(' | '"' | '\'' | '`')
            });
            if candidate.contains('/') && has_interesting_extension(candidate) {
                Some(candidate.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn truncate_summary(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        return content.to_string();
    }
    let mut truncated = content.chars().take(max_chars).collect::<String>();
    truncated.push('…');
    truncated
}

fn estimate_message_tokens(message: &ConversationMessage) -> usize {
    message
        .blocks
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } => text.len() / 4 + 1,
            ContentBlock::ToolUse { name, input, .. } => (name.len() + input.len()) / 4 + 1,
            ContentBlock::ToolResult {
                tool_name, output, ..
            } => (tool_name.len() + output.len()) / 4 + 1,
        })
        .sum()
}

fn extract_tag_block(content: &str, tag: &str) -> Option<String> {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    let start_index = content.find(&start)? + start.len();
    let end_index = content[start_index..].find(&end)? + start_index;
    Some(content[start_index..end_index].to_string())
}

fn strip_tag_block(content: &str, tag: &str) -> String {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    if let (Some(start_index), Some(end_index_rel)) = (content.find(&start), content.find(&end)) {
        let end_index = end_index_rel + end.len();
        let mut stripped = String::new();
        stripped.push_str(&content[..start_index]);
        stripped.push_str(&content[end_index..]);
        stripped
    } else {
        content.to_string()
    }
}

fn collapse_blank_lines(content: &str) -> String {
    let mut result = String::new();
    let mut last_blank = false;
    for line in content.lines() {
        let is_blank = line.trim().is_empty();
        if is_blank && last_blank {
            continue;
        }
        result.push_str(line);
        result.push('\n');
        last_blank = is_blank;
    }
    result
}

fn extract_existing_compacted_summary(messages: &[ConversationMessage]) -> Option<(usize, String)> {
    if let Some(summary) = messages.first().and_then(extract_legacy_compacted_summary) {
        return Some((1, summary));
    }

    match (messages.first(), messages.get(1)) {
        (Some(boundary), Some(summary_message)) if is_compact_boundary_message(boundary) => {
            extract_full_compact_summary(summary_message).map(|summary| (2, summary))
        }
        _ => None,
    }
}

fn extract_legacy_compacted_summary(message: &ConversationMessage) -> Option<String> {
    if message.role != MessageRole::System {
        return None;
    }

    let text = first_text_block(message)?;
    let summary = text.strip_prefix(COMPACT_CONTINUATION_PREAMBLE)?;
    let summary = summary
        .split_once(&format!("\n\n{COMPACT_RECENT_MESSAGES_NOTE}"))
        .map_or(summary, |(value, _)| value);
    let summary = summary
        .split_once(&format!("\n{COMPACT_DIRECT_RESUME_INSTRUCTION}"))
        .map_or(summary, |(value, _)| value);
    Some(summary.trim().to_string())
}

fn extract_full_compact_summary(message: &ConversationMessage) -> Option<String> {
    if message.role != MessageRole::User {
        return None;
    }

    let text = first_text_block(message)?;
    if !message.is_compact_summary && !text.starts_with(COMPACT_CONTINUATION_PREAMBLE) {
        return None;
    }
    let summary = text.strip_prefix(COMPACT_CONTINUATION_PREAMBLE)?;
    let summary = summary
        .split_once(&format!("\n\n{COMPACT_RECENT_MESSAGES_NOTE}"))
        .map_or(summary, |(value, _)| value);
    let summary = summary
        .split_once(&format!("\n{COMPACT_DIRECT_RESUME_INSTRUCTION}"))
        .map_or(summary, |(value, _)| value);
    Some(summary.trim().to_string())
}

fn is_compact_boundary_message(message: &ConversationMessage) -> bool {
    message.role == MessageRole::System
        && (message.compact_metadata.is_some()
            || first_text_block(message).is_some_and(|text| text.trim() == COMPACT_BOUNDARY_TEXT))
}

fn extract_summary_highlights(summary: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_timeline = false;

    for line in format_compact_summary(summary).lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() || trimmed == "Summary:" || trimmed == "Conversation summary:" {
            continue;
        }
        if trimmed == "- Key timeline:" {
            in_timeline = true;
            continue;
        }
        if in_timeline {
            continue;
        }
        lines.push(trimmed.to_string());
    }

    lines
}

fn extract_summary_timeline(summary: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_timeline = false;

    for line in format_compact_summary(summary).lines() {
        let trimmed = line.trim_end();
        if trimmed == "- Key timeline:" {
            in_timeline = true;
            continue;
        }
        if !in_timeline {
            continue;
        }
        if trimmed.is_empty() {
            break;
        }
        lines.push(trimmed.to_string());
    }

    lines
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::{
        build_compaction_result, collect_key_files, collect_post_compact_context_messages,
        compact_session, estimate_session_tokens, format_compact_summary,
        get_compact_continuation_message, get_compact_prompt, get_compact_user_summary_message,
        infer_pending_work, select_full_compact_messages, should_compact,
        BuildCompactionResultOptions, CompactionConfig, CompactionLayout, PreservedSegmentAnchor,
    };
    use crate::session::{
        AttachmentKind, CompactTrigger, ContentBlock, ConversationMessage, MessageRole, Session,
    };

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be valid")
            .as_nanos();
        std::env::temp_dir().join(format!("runtime-compact-{label}-{nanos}"))
    }

    fn write_post_compact_todo_fixture(workspace: &std::path::Path) {
        let todo_path = workspace.join(".clawd-todos.json");
        fs::write(
            &todo_path,
            serde_json::to_string_pretty(&json!([
                {
                    "content": "Implement TS full compact parity",
                    "activeForm": "Implementing TS full compact parity",
                    "status": "in_progress"
                },
                {
                    "content": "Run cargo test --workspace",
                    "activeForm": "Running cargo test --workspace",
                    "status": "pending"
                }
            ]))
            .expect("todo json should serialize"),
        )
        .expect("todo file should write");
    }

    fn write_post_compact_agent_fixture(workspace: &std::path::Path) -> PathBuf {
        let manifest_dir = workspace.join(".claw").join("agents");
        fs::create_dir_all(&manifest_dir).expect("agent dir should exist");
        let manifest_path = manifest_dir.join("agent-compact.json");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&json!({
                "agentId": "agent-compact",
                "name": "planner",
                "description": "Finish compact parity",
                "status": "running",
                "outputFile": workspace.join(".claw").join("agents").join("agent-compact.md").display().to_string(),
                "manifestFile": manifest_path.display().to_string(),
                "derivedState": "working"
            }))
            .expect("agent json should serialize"),
        )
        .expect("agent manifest should write");
        manifest_path
    }

    fn build_post_compact_context_session(
        workspace: &std::path::Path,
        manifest_path: &std::path::Path,
    ) -> Session {
        let mut session = Session::new().with_workspace_root(workspace);
        session.messages = vec![
            ConversationMessage::tool_result(
                "plan-1",
                "EnterPlanMode",
                serde_json::to_string_pretty(&json!({
                    "success": true,
                    "operation": "enter",
                    "changed": true,
                    "active": true,
                    "managed": true,
                    "message": "Enabled worktree-local plan mode override.",
                    "settingsPath": workspace.join(".codex").join("settings.local.json").display().to_string(),
                    "statePath": workspace.join(".claw").join("plan-mode.json").display().to_string()
                }))
                .expect("plan json should serialize"),
                false,
            ),
            ConversationMessage::tool_result(
                "skill-1",
                "Skill",
                serde_json::to_string_pretty(&json!({
                    "skill": "$compact",
                    "path": workspace.join(".codex").join("skills").join("compact").join("SKILL.md").display().to_string(),
                    "args": "full",
                    "description": "Compact guidance",
                    "prompt": "Prefer TS full compact parity over local summaries."
                }))
                .expect("skill json should serialize"),
                false,
            ),
            ConversationMessage::tool_result(
                "agent-1",
                "Agent",
                serde_json::to_string_pretty(&json!({
                    "agentId": "agent-compact",
                    "name": "planner",
                    "description": "Finish compact parity",
                    "status": "running",
                    "outputFile": workspace.join(".claw").join("agents").join("agent-compact.md").display().to_string(),
                    "manifestFile": manifest_path.display().to_string(),
                    "derivedState": "working"
                }))
                .expect("agent json should serialize"),
                false,
            ),
        ];
        session
    }

    #[test]
    fn formats_compact_summary_like_upstream() {
        let summary = "<analysis>scratch</analysis>\n<summary>Kept work</summary>";
        assert_eq!(format_compact_summary(summary), "Summary:\nKept work");
    }

    #[test]
    fn builds_full_compact_prompt_with_no_tools_guard() {
        let prompt = get_compact_prompt(Some("Focus on Rust code changes."));
        assert!(prompt.contains("CRITICAL: Respond with TEXT ONLY."));
        assert!(prompt.contains("Do NOT call any tools."));
        assert!(prompt.contains("Primary Request and Intent"));
        assert!(prompt.contains("Additional Instructions:\nFocus on Rust code changes."));
    }

    #[test]
    fn builds_user_facing_summary_message_like_ts_full_compact() {
        let message =
            get_compact_user_summary_message("<summary>Carry over context</summary>", true, false);
        assert!(message.starts_with("This session is being continued from a previous conversation"));
        assert!(message.contains("Summary:\nCarry over context"));
        assert!(message.contains("Continue the conversation from where it left off"));
    }

    #[test]
    fn leaves_small_sessions_unchanged() {
        let mut session = Session::new();
        session.messages = vec![ConversationMessage::user_text("hello")];

        let result = compact_session(&session, CompactionConfig::default());
        assert_eq!(result.removed_message_count, 0);
        assert_eq!(result.compacted_session, session);
        assert!(result.summary.is_empty());
        assert!(result.formatted_summary.is_empty());
    }

    #[test]
    fn compacts_older_messages_into_a_system_summary() {
        let mut session = Session::new();
        session.messages = vec![
            ConversationMessage::user_text("one ".repeat(200)),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "two ".repeat(200),
            }]),
            ConversationMessage::tool_result("1", "bash", "ok ".repeat(200), false),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "recent".to_string(),
            }]),
        ];

        let result = compact_session(
            &session,
            CompactionConfig {
                preserve_recent_messages: 2,
                max_estimated_tokens: 1,
            },
        );

        assert_eq!(result.removed_message_count, 2);
        assert_eq!(
            result.compacted_session.messages[0].role,
            MessageRole::System
        );
        assert!(matches!(
            &result.compacted_session.messages[0].blocks[0],
            ContentBlock::Text { text } if text.contains("Summary:")
        ));
        assert!(result.formatted_summary.contains("Scope:"));
        assert!(result.formatted_summary.contains("Key timeline:"));
        assert!(should_compact(
            &session,
            CompactionConfig {
                preserve_recent_messages: 2,
                max_estimated_tokens: 1,
            }
        ));
        assert!(
            estimate_session_tokens(&result.compacted_session) < estimate_session_tokens(&session)
        );
    }

    #[test]
    fn keeps_previous_compacted_context_when_compacting_again() {
        let mut initial_session = Session::new();
        initial_session.messages = vec![
            ConversationMessage::user_text("Investigate rust/crates/runtime/src/compact.rs"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "I will inspect the compact flow.".to_string(),
            }]),
            ConversationMessage::user_text("Also update rust/crates/runtime/src/conversation.rs"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "Next: preserve prior summary context during auto compact.".to_string(),
            }]),
        ];
        let config = CompactionConfig {
            preserve_recent_messages: 2,
            max_estimated_tokens: 1,
        };

        let first = compact_session(&initial_session, config);
        let mut follow_up_messages = first.compacted_session.messages.clone();
        follow_up_messages.extend([
            ConversationMessage::user_text("Please add regression tests for compaction."),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "Working on regression coverage now.".to_string(),
            }]),
        ]);

        let mut second_session = Session::new();
        second_session.messages = follow_up_messages;
        let second = compact_session(&second_session, config);

        assert!(second
            .formatted_summary
            .contains("Previously compacted context:"));
        assert!(second
            .formatted_summary
            .contains("Scope: 2 earlier messages compacted"));
        assert!(second
            .formatted_summary
            .contains("Newly compacted context:"));
        assert!(second
            .formatted_summary
            .contains("Also update rust/crates/runtime/src/conversation.rs"));
        assert!(matches!(
            &second.compacted_session.messages[0].blocks[0],
            ContentBlock::Text { text }
                if text.contains("Previously compacted context:")
                    && text.contains("Newly compacted context:")
        ));
        assert!(matches!(
            &second.compacted_session.messages[1].blocks[0],
            ContentBlock::Text { text } if text.contains("Please add regression tests for compaction.")
        ));
    }

    #[test]
    fn ignores_existing_compacted_summary_when_deciding_to_recompact() {
        let summary = "<summary>Conversation summary:\n- Scope: earlier work preserved.\n- Key timeline:\n  - user: large preserved context\n</summary>";
        let mut session = Session::new();
        session.messages = vec![
            ConversationMessage::system_text(get_compact_continuation_message(summary, true, true)),
            ConversationMessage::user_text("tiny"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "recent".to_string(),
            }]),
        ];

        assert!(!should_compact(
            &session,
            CompactionConfig {
                preserve_recent_messages: 2,
                max_estimated_tokens: 1,
            }
        ));
    }

    #[test]
    fn ignores_existing_full_compact_prefix_when_deciding_to_recompact() {
        let summary =
            "<summary>Conversation summary:\n- Scope: earlier work preserved.\n</summary>";
        let mut session = Session::new();
        session.messages = vec![
            ConversationMessage::system_text("Conversation compacted"),
            ConversationMessage::user_text(get_compact_user_summary_message(summary, true, false)),
            ConversationMessage::user_text("tiny"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "recent".to_string(),
            }]),
        ];

        assert!(!should_compact(
            &session,
            CompactionConfig {
                preserve_recent_messages: 2,
                max_estimated_tokens: 1,
            }
        ));
    }

    #[test]
    fn truncates_long_blocks_in_summary() {
        let summary = super::summarize_block(&ContentBlock::Text {
            text: "x".repeat(400),
        });
        assert!(summary.ends_with('…'));
        assert!(summary.chars().count() <= 161);
    }

    #[test]
    fn extracts_key_files_from_message_content() {
        let files = collect_key_files(&[ConversationMessage::user_text(
            "Update rust/crates/runtime/src/compact.rs and rust/crates/rusty-claude-cli/src/main.rs next.",
        )]);
        assert!(files.contains(&"rust/crates/runtime/src/compact.rs".to_string()));
        assert!(files.contains(&"rust/crates/rusty-claude-cli/src/main.rs".to_string()));
    }

    #[test]
    fn infers_pending_work_from_recent_messages() {
        let pending = infer_pending_work(&[
            ConversationMessage::user_text("done"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "Next: update tests and follow up on remaining CLI polish.".to_string(),
            }]),
        ]);
        assert_eq!(pending.len(), 1);
        assert!(pending[0].contains("Next: update tests"));
    }

    #[test]
    fn build_compaction_result_can_replace_entire_session_with_summary_only() {
        let mut session = Session::new();
        session.messages = vec![
            ConversationMessage::user_text("first"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "second".to_string(),
            }]),
        ];

        let result = build_compaction_result(
            &session,
            "<summary>Model-generated summary</summary>".to_string(),
            BuildCompactionResultOptions {
                preserved_messages: Vec::new(),
                hook_result_messages: Vec::new(),
                preserved_segment_anchor: None,
                removed_message_count: session.messages.len(),
                suppress_follow_up_questions: true,
                layout: CompactionLayout::TsBoundaryAndSummary,
                boundary_trigger: Some(CompactTrigger::Manual),
                user_display_message: None,
            },
        );

        assert_eq!(result.removed_message_count, 2);
        assert_eq!(result.compacted_session.messages.len(), 2);
        assert!(matches!(
            &result.compacted_session.messages[0].blocks[0],
            ContentBlock::Text { text } if text == "Conversation compacted"
        ));
        assert!(matches!(
            result.compacted_session.messages[0].compact_metadata.as_ref(),
            Some(metadata)
                if metadata.trigger == CompactTrigger::Manual
                    && metadata.messages_summarized == Some(2)
        ));
        assert!(matches!(
            &result.compacted_session.messages[1].blocks[0],
            ContentBlock::Text { text }
                if text.contains("Summary:\nModel-generated summary")
                    && !text.contains("Recent messages are preserved verbatim.")
        ));
        assert!(result.compacted_session.messages[1].is_compact_summary);
        assert!(result.compacted_session.messages[1].is_visible_in_transcript_only);
    }

    #[test]
    fn build_compaction_result_records_preserved_segment_for_suffix_preserving_layout() {
        let mut session = Session::new();
        session.messages = vec![
            ConversationMessage::user_text("first"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "second".to_string(),
            }]),
            ConversationMessage::user_text("keep me"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "and me".to_string(),
            }]),
        ];

        let expected_head_uuid = session.messages[2].uuid.clone();
        let expected_tail_uuid = session.messages[3].uuid.clone();
        let preserved_messages = session.messages[2..].to_vec();
        let result = build_compaction_result(
            &session,
            "<summary>Model-generated summary</summary>".to_string(),
            BuildCompactionResultOptions {
                preserved_messages,
                hook_result_messages: Vec::new(),
                preserved_segment_anchor: Some(PreservedSegmentAnchor::LastSummaryMessage),
                removed_message_count: 2,
                suppress_follow_up_questions: true,
                layout: CompactionLayout::TsBoundaryAndSummary,
                boundary_trigger: Some(CompactTrigger::Manual),
                user_display_message: None,
            },
        );

        assert_eq!(result.compacted_session.messages.len(), 4);
        let summary_uuid = result.compacted_session.messages[1].uuid.clone();
        assert!(matches!(
            result.compacted_session.messages[0].compact_metadata.as_ref(),
            Some(metadata)
                if metadata.preserved_segment.as_ref().is_some_and(|segment|
                    segment.head == expected_head_uuid
                        && segment.anchor == summary_uuid
                        && segment.tail == expected_tail_uuid
                )
        ));
        assert!(matches!(
            &result.compacted_session.messages[1].blocks[0],
            ContentBlock::Text { text } if text.contains("Recent messages are preserved verbatim.")
        ));
        assert!(result.compacted_session.messages[1].is_compact_summary);
        assert!(!result.compacted_session.messages[1].is_visible_in_transcript_only);
        assert!(matches!(
            &result.compacted_session.messages[2].blocks[0],
            ContentBlock::Text { text } if text == "keep me"
        ));
    }

    #[test]
    fn select_full_compact_messages_keeps_recent_tail_without_reusing_compact_prefix() {
        let mut session = Session::new();
        session.messages = vec![
            ConversationMessage::compact_boundary(crate::session::CompactBoundaryMetadata {
                trigger: CompactTrigger::Manual,
                pre_tokens: 42,
                user_context: None,
                messages_summarized: Some(2),
                pre_compact_discovered_tools: Vec::new(),
                preserved_segment: None,
            }),
            ConversationMessage::user_text(get_compact_user_summary_message(
                "<summary>Older work</summary>",
                true,
                false,
            )),
            ConversationMessage::user_text("recent one"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "recent two".to_string(),
            }]),
            ConversationMessage::user_text("recent three"),
        ];

        let selection = select_full_compact_messages(&session, 2);
        assert_eq!(selection.messages_to_summarize.len(), 3);
        assert_eq!(selection.preserved_messages.len(), 2);
        assert!(matches!(
            &selection.preserved_messages[0].blocks[0],
            ContentBlock::Text { text } if text == "recent two"
        ));
        assert!(matches!(
            &selection.preserved_messages[1].blocks[0],
            ContentBlock::Text { text } if text == "recent three"
        ));
    }

    #[test]
    fn collects_post_compact_context_messages_from_runtime_state() {
        let workspace = temp_dir("post-compact-context");
        fs::create_dir_all(&workspace).expect("workspace should exist");
        write_post_compact_todo_fixture(&workspace);
        let manifest_path = write_post_compact_agent_fixture(&workspace);
        let session = build_post_compact_context_session(&workspace, &manifest_path);

        let messages = collect_post_compact_context_messages(&session);
        assert_eq!(messages.len(), 4);

        let rendered = messages
            .iter()
            .filter_map(|message| match &message.blocks[0] {
                ContentBlock::Text { text } => Some(text.as_str()),
                ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            messages[0]
                .attachment_metadata
                .as_ref()
                .map(|metadata| metadata.kind),
            Some(AttachmentKind::RunningAgents)
        );
        assert_eq!(
            messages[1]
                .attachment_metadata
                .as_ref()
                .map(|metadata| metadata.kind),
            Some(AttachmentKind::TodoList)
        );
        assert_eq!(
            messages[2]
                .attachment_metadata
                .as_ref()
                .map(|metadata| metadata.kind),
            Some(AttachmentKind::PlanMode)
        );
        assert_eq!(
            messages[3]
                .attachment_metadata
                .as_ref()
                .map(|metadata| metadata.kind),
            Some(AttachmentKind::InvokedSkills)
        );
        assert!(rendered[0].contains("Background agents are still active after compaction"));
        assert!(rendered[0].contains("agent-compact"));
        assert!(rendered[1].contains("Current TodoWrite task list remains active"));
        assert!(rendered[1].contains("Implement TS full compact parity"));
        assert!(rendered[2].contains("Plan mode is still active for this worktree"));
        assert!(rendered[3].contains("Previously invoked skills remain available"));
        assert!(rendered[3].contains("Prefer TS full compact parity over local summaries."));

        fs::remove_dir_all(&workspace).expect("workspace cleanup should succeed");
    }
}
