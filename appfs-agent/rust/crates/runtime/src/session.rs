use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::json::{JsonError, JsonValue};
use crate::usage::TokenUsage;
use getrandom::getrandom;

const SESSION_VERSION: u32 = 1;
const ROTATE_AFTER_BYTES: u64 = 256 * 1024;
const MAX_ROTATED_FILES: usize = 3;
static SESSION_ID_COUNTER: AtomicU64 = AtomicU64::new(0);
static MESSAGE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);
static LAST_TIMESTAMP_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
    ToolResult {
        tool_use_id: String,
        tool_name: String,
        output: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemMessageSubtype {
    CompactBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    RunningAgents,
    TodoList,
    PlanMode,
    InvokedSkills,
    HookAdditionalContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactTrigger {
    Manual,
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactPreservedSegment {
    pub head: String,
    pub anchor: String,
    pub tail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactBoundaryMetadata {
    pub trigger: CompactTrigger,
    pub pre_tokens: usize,
    pub user_context: Option<String>,
    pub messages_summarized: Option<usize>,
    pub pre_compact_discovered_tools: Vec<String>,
    pub preserved_segment: Option<CompactPreservedSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentMetadata {
    pub kind: AttachmentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookResultEvent {
    SessionStart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookResultMetadata {
    pub event: HookResultEvent,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMessage {
    pub uuid: String,
    pub role: MessageRole,
    pub blocks: Vec<ContentBlock>,
    pub usage: Option<TokenUsage>,
    pub subtype: Option<SystemMessageSubtype>,
    pub compact_metadata: Option<CompactBoundaryMetadata>,
    pub attachment_metadata: Option<AttachmentMetadata>,
    pub hook_result_metadata: Option<HookResultMetadata>,
    pub is_compact_summary: bool,
    pub is_visible_in_transcript_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCompaction {
    pub count: u32,
    pub removed_message_count: usize,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFork {
    pub parent_session_id: String,
    pub branch_name: Option<String>,
}

/// A single user prompt recorded with a timestamp for history tracking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPromptEntry {
    pub timestamp_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvokedSkill {
    pub skill: String,
    pub resolved_name: Option<String>,
    pub path: Option<String>,
    pub description: Option<String>,
    pub args: Option<String>,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionPersistence {
    path: PathBuf,
}

/// Persisted conversational state for the runtime and CLI session manager.
///
/// `workspace_root` binds the session to the worktree it was created in. The
/// global session store under `~/.local/share/opencode` is shared across every
/// `opencode serve` instance, so without an explicit workspace root parallel
/// lanes can race and report success while writes land in the wrong CWD. See
/// ROADMAP.md item 41 (Phantom completions root cause) for the full
/// background.
#[derive(Debug, Clone)]
pub struct Session {
    pub version: u32,
    pub session_id: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub messages: Vec<ConversationMessage>,
    pub compaction: Option<SessionCompaction>,
    pub fork: Option<SessionFork>,
    pub workspace_root: Option<PathBuf>,
    pub prompt_history: Vec<SessionPromptEntry>,
    pub invoked_skills: Vec<InvokedSkill>,
    persistence: Option<SessionPersistence>,
}

impl PartialEq for Session {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
            && self.session_id == other.session_id
            && self.created_at_ms == other.created_at_ms
            && self.updated_at_ms == other.updated_at_ms
            && self.messages == other.messages
            && self.compaction == other.compaction
            && self.fork == other.fork
            && self.workspace_root == other.workspace_root
            && self.prompt_history == other.prompt_history
            && self.invoked_skills == other.invoked_skills
    }
}

impl Eq for Session {}

#[derive(Debug)]
pub enum SessionError {
    Io(std::io::Error),
    Json(JsonError),
    Format(String),
}

impl Display for SessionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::Format(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<std::io::Error> for SessionError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<JsonError> for SessionError {
    fn from(value: JsonError) -> Self {
        Self::Json(value)
    }
}

impl Session {
    #[must_use]
    pub fn new() -> Self {
        let now = current_time_millis();
        Self {
            version: SESSION_VERSION,
            session_id: generate_session_id(),
            created_at_ms: now,
            updated_at_ms: now,
            messages: Vec::new(),
            compaction: None,
            fork: None,
            workspace_root: None,
            prompt_history: Vec::new(),
            invoked_skills: Vec::new(),
            persistence: None,
        }
    }

    #[must_use]
    pub fn with_persistence_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.persistence = Some(SessionPersistence { path: path.into() });
        self
    }

    /// Bind this session to the workspace root it was created in.
    ///
    /// This is the per-worktree counterpart to the global session store and
    /// lets downstream tooling reject writes that drift to the wrong CWD when
    /// multiple `opencode serve` instances share `~/.local/share/opencode`.
    #[must_use]
    pub fn with_workspace_root(mut self, workspace_root: impl Into<PathBuf>) -> Self {
        self.workspace_root = Some(workspace_root.into());
        self
    }

    #[must_use]
    pub fn workspace_root(&self) -> Option<&Path> {
        self.workspace_root.as_deref()
    }

    #[must_use]
    pub fn persistence_path(&self) -> Option<&Path> {
        self.persistence.as_ref().map(|value| value.path.as_path())
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), SessionError> {
        let path = path.as_ref();
        let snapshot = self.render_jsonl_snapshot()?;
        rotate_session_file_if_needed(path)?;
        write_atomic(path, &snapshot)?;
        cleanup_rotated_logs(path)?;
        Ok(())
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, SessionError> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)?;
        let session = match JsonValue::parse(&contents) {
            Ok(value)
                if value
                    .as_object()
                    .is_some_and(|object| object.contains_key("messages")) =>
            {
                Self::from_json(&value)?
            }
            Err(_) | Ok(_) => Self::from_jsonl(&contents)?,
        };
        Ok(session.with_persistence_path(path.to_path_buf()))
    }

    pub fn push_message(&mut self, message: ConversationMessage) -> Result<(), SessionError> {
        self.touch();
        self.messages.push(message);
        let persist_result = {
            let message_ref = self.messages.last().ok_or_else(|| {
                SessionError::Format("message was just pushed but missing".to_string())
            })?;
            self.append_persisted_message(message_ref)
        };
        if let Err(error) = persist_result {
            self.messages.pop();
            return Err(error);
        }
        Ok(())
    }

    pub fn push_user_text(&mut self, text: impl Into<String>) -> Result<(), SessionError> {
        self.push_message(ConversationMessage::user_text(text))
    }

    pub fn record_compaction(&mut self, summary: impl Into<String>, removed_message_count: usize) {
        self.touch();
        let count = self.compaction.as_ref().map_or(1, |value| value.count + 1);
        self.compaction = Some(SessionCompaction {
            count,
            removed_message_count,
            summary: summary.into(),
        });
    }

    pub fn upsert_invoked_skill(
        &mut self,
        invoked_skill: InvokedSkill,
    ) -> Result<(), SessionError> {
        let previous_updated_at_ms = self.updated_at_ms;
        let previous_invoked_skills = self.invoked_skills.clone();
        self.touch();
        upsert_invoked_skill_entry(&mut self.invoked_skills, invoked_skill);

        let persistence_path = self.persistence_path().map(Path::to_path_buf);
        if let Some(path) = persistence_path {
            if let Err(error) = self.save_to_path(path) {
                self.updated_at_ms = previous_updated_at_ms;
                self.invoked_skills = previous_invoked_skills;
                return Err(error);
            }
        }

        Ok(())
    }

    #[must_use]
    pub fn fork(&self, branch_name: Option<String>) -> Self {
        let now = current_time_millis();
        Self {
            version: self.version,
            session_id: generate_session_id(),
            created_at_ms: now,
            updated_at_ms: now,
            messages: self.messages.clone(),
            compaction: self.compaction.clone(),
            fork: Some(SessionFork {
                parent_session_id: self.session_id.clone(),
                branch_name: normalize_optional_string(branch_name),
            }),
            workspace_root: self.workspace_root.clone(),
            prompt_history: self.prompt_history.clone(),
            invoked_skills: self.invoked_skills.clone(),
            persistence: None,
        }
    }

    pub fn to_json(&self) -> Result<JsonValue, SessionError> {
        let mut object = BTreeMap::new();
        object.insert(
            "version".to_string(),
            JsonValue::Number(i64::from(self.version)),
        );
        object.insert(
            "session_id".to_string(),
            JsonValue::String(self.session_id.clone()),
        );
        object.insert(
            "created_at_ms".to_string(),
            JsonValue::Number(i64_from_u64(self.created_at_ms, "created_at_ms")?),
        );
        object.insert(
            "updated_at_ms".to_string(),
            JsonValue::Number(i64_from_u64(self.updated_at_ms, "updated_at_ms")?),
        );
        object.insert(
            "messages".to_string(),
            JsonValue::Array(
                self.messages
                    .iter()
                    .map(ConversationMessage::to_json)
                    .collect(),
            ),
        );
        if let Some(compaction) = &self.compaction {
            object.insert("compaction".to_string(), compaction.to_json()?);
        }
        if let Some(fork) = &self.fork {
            object.insert("fork".to_string(), fork.to_json());
        }
        if let Some(workspace_root) = &self.workspace_root {
            object.insert(
                "workspace_root".to_string(),
                JsonValue::String(workspace_root_to_string(workspace_root)?),
            );
        }
        if !self.prompt_history.is_empty() {
            object.insert(
                "prompt_history".to_string(),
                JsonValue::Array(
                    self.prompt_history
                        .iter()
                        .map(SessionPromptEntry::to_jsonl_record)
                        .collect(),
                ),
            );
        }
        if !self.invoked_skills.is_empty() {
            object.insert(
                "invoked_skills".to_string(),
                JsonValue::Array(
                    self.invoked_skills
                        .iter()
                        .map(InvokedSkill::to_json)
                        .collect(),
                ),
            );
        }
        Ok(JsonValue::Object(object))
    }

    pub fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value
            .as_object()
            .ok_or_else(|| SessionError::Format("session must be an object".to_string()))?;
        let version = object
            .get("version")
            .and_then(JsonValue::as_i64)
            .ok_or_else(|| SessionError::Format("missing version".to_string()))?;
        let version = u32::try_from(version)
            .map_err(|_| SessionError::Format("version out of range".to_string()))?;
        let messages = object
            .get("messages")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| SessionError::Format("missing messages".to_string()))?
            .iter()
            .map(ConversationMessage::from_json)
            .collect::<Result<Vec<_>, _>>()?;
        let now = current_time_millis();
        let session_id = object
            .get("session_id")
            .and_then(JsonValue::as_str)
            .map_or_else(generate_session_id, ToOwned::to_owned);
        let created_at_ms = object
            .get("created_at_ms")
            .map(|value| required_u64_from_value(value, "created_at_ms"))
            .transpose()?
            .unwrap_or(now);
        let updated_at_ms = object
            .get("updated_at_ms")
            .map(|value| required_u64_from_value(value, "updated_at_ms"))
            .transpose()?
            .unwrap_or(created_at_ms);
        let compaction = object
            .get("compaction")
            .map(SessionCompaction::from_json)
            .transpose()?;
        let fork = object.get("fork").map(SessionFork::from_json).transpose()?;
        let workspace_root = object
            .get("workspace_root")
            .and_then(JsonValue::as_str)
            .map(PathBuf::from);
        let prompt_history = object
            .get("prompt_history")
            .and_then(JsonValue::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(SessionPromptEntry::from_json_opt)
                    .collect()
            })
            .unwrap_or_default();
        let invoked_skills = object
            .get("invoked_skills")
            .map(invoked_skills_from_json)
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            version,
            session_id,
            created_at_ms,
            updated_at_ms,
            messages,
            compaction,
            fork,
            workspace_root,
            prompt_history,
            invoked_skills,
            persistence: None,
        })
    }

    fn from_jsonl(contents: &str) -> Result<Self, SessionError> {
        let mut version = SESSION_VERSION;
        let mut session_id = None;
        let mut created_at_ms = None;
        let mut updated_at_ms = None;
        let mut messages = Vec::new();
        let mut compaction = None;
        let mut fork = None;
        let mut workspace_root = None;
        let mut prompt_history = Vec::new();
        let mut invoked_skills = Vec::new();

        for (line_number, raw_line) in contents.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            let value = JsonValue::parse(line).map_err(|error| {
                SessionError::Format(format!(
                    "invalid JSONL record at line {}: {}",
                    line_number + 1,
                    error
                ))
            })?;
            let object = value.as_object().ok_or_else(|| {
                SessionError::Format(format!(
                    "JSONL record at line {} must be an object",
                    line_number + 1
                ))
            })?;
            match object
                .get("type")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    SessionError::Format(format!(
                        "JSONL record at line {} missing type",
                        line_number + 1
                    ))
                })? {
                "session_meta" => {
                    version = required_u32(object, "version")?;
                    session_id = Some(required_string(object, "session_id")?);
                    created_at_ms = Some(required_u64(object, "created_at_ms")?);
                    updated_at_ms = Some(required_u64(object, "updated_at_ms")?);
                    fork = object.get("fork").map(SessionFork::from_json).transpose()?;
                    workspace_root = object
                        .get("workspace_root")
                        .and_then(JsonValue::as_str)
                        .map(PathBuf::from);
                    if let Some(value) = object.get("invoked_skills") {
                        invoked_skills = invoked_skills_from_json(value)?;
                    }
                }
                "message" => {
                    let message_value = object.get("message").ok_or_else(|| {
                        SessionError::Format(format!(
                            "JSONL record at line {} missing message",
                            line_number + 1
                        ))
                    })?;
                    messages.push(ConversationMessage::from_json(message_value)?);
                }
                "compaction" => {
                    compaction = Some(SessionCompaction::from_json(&JsonValue::Object(
                        object.clone(),
                    ))?);
                }
                "prompt_history" => {
                    if let Some(entry) =
                        SessionPromptEntry::from_json_opt(&JsonValue::Object(object.clone()))
                    {
                        prompt_history.push(entry);
                    }
                }
                other => {
                    return Err(SessionError::Format(format!(
                        "unsupported JSONL record type at line {}: {other}",
                        line_number + 1
                    )))
                }
            }
        }

        let now = current_time_millis();
        Ok(Self {
            version,
            session_id: session_id.unwrap_or_else(generate_session_id),
            created_at_ms: created_at_ms.unwrap_or(now),
            updated_at_ms: updated_at_ms.unwrap_or(created_at_ms.unwrap_or(now)),
            messages,
            compaction,
            fork,
            workspace_root,
            prompt_history,
            invoked_skills,
            persistence: None,
        })
    }

    /// Record a user prompt with the current wall-clock timestamp.
    ///
    /// The entry is appended to the in-memory history and, when a persistence
    /// path is configured, incrementally written to the JSONL session file.
    pub fn push_prompt_entry(&mut self, text: impl Into<String>) -> Result<(), SessionError> {
        let timestamp_ms = current_time_millis();
        let entry = SessionPromptEntry {
            timestamp_ms,
            text: text.into(),
        };
        self.prompt_history.push(entry);
        let entry_ref = self.prompt_history.last().expect("entry was just pushed");
        self.append_persisted_prompt_entry(entry_ref)
    }

    fn render_jsonl_snapshot(&self) -> Result<String, SessionError> {
        let mut lines = vec![self.meta_record()?.render()];
        if let Some(compaction) = &self.compaction {
            lines.push(compaction.to_jsonl_record()?.render());
        }
        lines.extend(
            self.prompt_history
                .iter()
                .map(|entry| entry.to_jsonl_record().render()),
        );
        lines.extend(
            self.messages
                .iter()
                .map(|message| message_record(message).render()),
        );
        let mut rendered = lines.join("\n");
        rendered.push('\n');
        Ok(rendered)
    }

    fn append_persisted_message(&self, message: &ConversationMessage) -> Result<(), SessionError> {
        let Some(path) = self.persistence_path() else {
            return Ok(());
        };

        let needs_bootstrap = !path.exists() || fs::metadata(path)?.len() == 0;
        if needs_bootstrap {
            self.save_to_path(path)?;
            return Ok(());
        }

        let mut file = OpenOptions::new().append(true).open(path)?;
        writeln!(file, "{}", message_record(message).render())?;
        Ok(())
    }

    fn append_persisted_prompt_entry(
        &self,
        entry: &SessionPromptEntry,
    ) -> Result<(), SessionError> {
        let Some(path) = self.persistence_path() else {
            return Ok(());
        };

        let needs_bootstrap = !path.exists() || fs::metadata(path)?.len() == 0;
        if needs_bootstrap {
            self.save_to_path(path)?;
            return Ok(());
        }

        let mut file = OpenOptions::new().append(true).open(path)?;
        writeln!(file, "{}", entry.to_jsonl_record().render())?;
        Ok(())
    }

    fn meta_record(&self) -> Result<JsonValue, SessionError> {
        let mut object = BTreeMap::new();
        object.insert(
            "type".to_string(),
            JsonValue::String("session_meta".to_string()),
        );
        object.insert(
            "version".to_string(),
            JsonValue::Number(i64::from(self.version)),
        );
        object.insert(
            "session_id".to_string(),
            JsonValue::String(self.session_id.clone()),
        );
        object.insert(
            "created_at_ms".to_string(),
            JsonValue::Number(i64_from_u64(self.created_at_ms, "created_at_ms")?),
        );
        object.insert(
            "updated_at_ms".to_string(),
            JsonValue::Number(i64_from_u64(self.updated_at_ms, "updated_at_ms")?),
        );
        if let Some(fork) = &self.fork {
            object.insert("fork".to_string(), fork.to_json());
        }
        if let Some(workspace_root) = &self.workspace_root {
            object.insert(
                "workspace_root".to_string(),
                JsonValue::String(workspace_root_to_string(workspace_root)?),
            );
        }
        if !self.invoked_skills.is_empty() {
            object.insert(
                "invoked_skills".to_string(),
                JsonValue::Array(
                    self.invoked_skills
                        .iter()
                        .map(InvokedSkill::to_json)
                        .collect(),
                ),
            );
        }
        Ok(JsonValue::Object(object))
    }

    fn touch(&mut self) {
        self.updated_at_ms = current_time_millis();
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl ConversationMessage {
    #[allow(clippy::too_many_arguments)]
    fn new(
        role: MessageRole,
        blocks: Vec<ContentBlock>,
        usage: Option<TokenUsage>,
        subtype: Option<SystemMessageSubtype>,
        compact_metadata: Option<CompactBoundaryMetadata>,
        attachment_metadata: Option<AttachmentMetadata>,
        hook_result_metadata: Option<HookResultMetadata>,
        is_compact_summary: bool,
        is_visible_in_transcript_only: bool,
    ) -> Self {
        Self {
            uuid: generate_message_uuid(),
            role,
            blocks,
            usage,
            subtype,
            compact_metadata,
            attachment_metadata,
            hook_result_metadata,
            is_compact_summary,
            is_visible_in_transcript_only,
        }
    }

    #[must_use]
    pub fn system_text(text: impl Into<String>) -> Self {
        Self::new(
            MessageRole::System,
            vec![ContentBlock::Text { text: text.into() }],
            None,
            None,
            None,
            None,
            None,
            false,
            false,
        )
    }

    #[must_use]
    pub fn compact_boundary(metadata: CompactBoundaryMetadata) -> Self {
        Self::new(
            MessageRole::System,
            vec![ContentBlock::Text {
                text: "Conversation compacted".to_string(),
            }],
            None,
            Some(SystemMessageSubtype::CompactBoundary),
            Some(metadata),
            None,
            None,
            false,
            false,
        )
    }

    #[must_use]
    pub fn user_text(text: impl Into<String>) -> Self {
        Self::user_text_with_metadata(text, None, None, false, false)
    }

    #[must_use]
    pub fn attachment_user_text(text: impl Into<String>, kind: AttachmentKind) -> Self {
        Self::user_text_with_metadata(text, Some(AttachmentMetadata { kind }), None, false, false)
    }

    #[must_use]
    pub fn hook_result_user_text(
        text: impl Into<String>,
        attachment_kind: Option<AttachmentKind>,
        event: HookResultEvent,
        source: impl Into<String>,
    ) -> Self {
        Self::user_text_with_metadata(
            text,
            attachment_kind.map(|kind| AttachmentMetadata { kind }),
            Some(HookResultMetadata {
                event,
                source: source.into(),
            }),
            false,
            false,
        )
    }

    fn user_text_with_metadata(
        text: impl Into<String>,
        attachment_metadata: Option<AttachmentMetadata>,
        hook_result_metadata: Option<HookResultMetadata>,
        is_compact_summary: bool,
        is_visible_in_transcript_only: bool,
    ) -> Self {
        Self::new(
            MessageRole::User,
            vec![ContentBlock::Text { text: text.into() }],
            None,
            None,
            None,
            attachment_metadata,
            hook_result_metadata,
            is_compact_summary,
            is_visible_in_transcript_only,
        )
    }

    #[must_use]
    pub fn compact_summary_user_text(
        text: impl Into<String>,
        is_visible_in_transcript_only: bool,
    ) -> Self {
        Self::user_text_with_metadata(text, None, None, true, is_visible_in_transcript_only)
    }

    #[must_use]
    pub fn assistant(blocks: Vec<ContentBlock>) -> Self {
        Self::new(
            MessageRole::Assistant,
            blocks,
            None,
            None,
            None,
            None,
            None,
            false,
            false,
        )
    }

    #[must_use]
    pub fn assistant_with_usage(blocks: Vec<ContentBlock>, usage: Option<TokenUsage>) -> Self {
        Self::new(
            MessageRole::Assistant,
            blocks,
            usage,
            None,
            None,
            None,
            None,
            false,
            false,
        )
    }

    #[must_use]
    pub fn tool_result(
        tool_use_id: impl Into<String>,
        tool_name: impl Into<String>,
        output: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self::new(
            MessageRole::Tool,
            vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.into(),
                tool_name: tool_name.into(),
                output: output.into(),
                is_error,
            }],
            None,
            None,
            None,
            None,
            None,
            false,
            false,
        )
    }

    #[must_use]
    pub fn to_json(&self) -> JsonValue {
        let mut object = BTreeMap::new();
        object.insert("uuid".to_string(), JsonValue::String(self.uuid.clone()));
        object.insert(
            "role".to_string(),
            JsonValue::String(
                match self.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                }
                .to_string(),
            ),
        );
        object.insert(
            "blocks".to_string(),
            JsonValue::Array(self.blocks.iter().map(ContentBlock::to_json).collect()),
        );
        if let Some(usage) = self.usage {
            object.insert("usage".to_string(), usage_to_json(usage));
        }
        if let Some(subtype) = &self.subtype {
            object.insert(
                "subtype".to_string(),
                JsonValue::String(subtype.as_str().to_string()),
            );
        }
        if let Some(compact_metadata) = &self.compact_metadata {
            object.insert(
                "compact_metadata".to_string(),
                compact_metadata
                    .to_json()
                    .expect("compact metadata to serialize"),
            );
        }
        if let Some(attachment_metadata) = &self.attachment_metadata {
            object.insert(
                "attachment_metadata".to_string(),
                attachment_metadata.to_json(),
            );
        }
        if let Some(hook_result_metadata) = &self.hook_result_metadata {
            object.insert(
                "hook_result_metadata".to_string(),
                hook_result_metadata.to_json(),
            );
        }
        if self.is_compact_summary {
            object.insert("is_compact_summary".to_string(), JsonValue::Bool(true));
        }
        if self.is_visible_in_transcript_only {
            object.insert(
                "is_visible_in_transcript_only".to_string(),
                JsonValue::Bool(true),
            );
        }
        JsonValue::Object(object)
    }

    #[allow(clippy::too_many_lines)]
    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value
            .as_object()
            .ok_or_else(|| SessionError::Format("message must be an object".to_string()))?;
        let uuid = match object.get("uuid") {
            Some(value) => {
                let uuid = value.as_str().ok_or_else(|| {
                    SessionError::Format("message uuid must be a string".to_string())
                })?;
                if uuid.trim().is_empty() {
                    return Err(SessionError::Format(
                        "message uuid cannot be empty".to_string(),
                    ));
                }
                uuid.to_string()
            }
            None => generate_message_uuid(),
        };
        let role = match object
            .get("role")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| SessionError::Format("missing role".to_string()))?
        {
            "system" => MessageRole::System,
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            "tool" => MessageRole::Tool,
            other => {
                return Err(SessionError::Format(format!(
                    "unsupported message role: {other}"
                )))
            }
        };
        let blocks = object
            .get("blocks")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| SessionError::Format("missing blocks".to_string()))?
            .iter()
            .map(ContentBlock::from_json)
            .collect::<Result<Vec<_>, _>>()?;
        let usage = object.get("usage").map(usage_from_json).transpose()?;
        let subtype = object
            .get("subtype")
            .map(SystemMessageSubtype::from_json)
            .transpose()?;
        let compact_metadata = object
            .get("compact_metadata")
            .map(CompactBoundaryMetadata::from_json)
            .transpose()?;
        let attachment_metadata = object
            .get("attachment_metadata")
            .map(AttachmentMetadata::from_json)
            .transpose()?;
        let hook_result_metadata = object
            .get("hook_result_metadata")
            .map(HookResultMetadata::from_json)
            .transpose()?;
        let is_compact_summary = object
            .get("is_compact_summary")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let is_visible_in_transcript_only = object
            .get("is_visible_in_transcript_only")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        if subtype.is_some() && role != MessageRole::System {
            return Err(SessionError::Format(
                "message subtype is only supported on system messages".to_string(),
            ));
        }
        if compact_metadata.is_some() && subtype != Some(SystemMessageSubtype::CompactBoundary) {
            return Err(SessionError::Format(
                "compact_metadata requires subtype=compact_boundary".to_string(),
            ));
        }
        if attachment_metadata.is_some() && role != MessageRole::User {
            return Err(SessionError::Format(
                "attachment_metadata is only supported on user messages".to_string(),
            ));
        }
        if hook_result_metadata.is_some() && role != MessageRole::User {
            return Err(SessionError::Format(
                "hook_result_metadata is only supported on user messages".to_string(),
            ));
        }
        if is_compact_summary && role != MessageRole::User {
            return Err(SessionError::Format(
                "is_compact_summary is only supported on user messages".to_string(),
            ));
        }
        if is_compact_summary && (attachment_metadata.is_some() || hook_result_metadata.is_some()) {
            return Err(SessionError::Format(
                "compact summary messages cannot also be attachments or hook results".to_string(),
            ));
        }
        Ok(Self {
            uuid,
            role,
            blocks,
            usage,
            subtype,
            compact_metadata,
            attachment_metadata,
            hook_result_metadata,
            is_compact_summary,
            is_visible_in_transcript_only,
        })
    }
}

impl ContentBlock {
    #[must_use]
    pub fn to_json(&self) -> JsonValue {
        let mut object = BTreeMap::new();
        match self {
            Self::Text { text } => {
                object.insert("type".to_string(), JsonValue::String("text".to_string()));
                object.insert("text".to_string(), JsonValue::String(text.clone()));
            }
            Self::ToolUse { id, name, input } => {
                object.insert(
                    "type".to_string(),
                    JsonValue::String("tool_use".to_string()),
                );
                object.insert("id".to_string(), JsonValue::String(id.clone()));
                object.insert("name".to_string(), JsonValue::String(name.clone()));
                object.insert("input".to_string(), JsonValue::String(input.clone()));
            }
            Self::ToolResult {
                tool_use_id,
                tool_name,
                output,
                is_error,
            } => {
                object.insert(
                    "type".to_string(),
                    JsonValue::String("tool_result".to_string()),
                );
                object.insert(
                    "tool_use_id".to_string(),
                    JsonValue::String(tool_use_id.clone()),
                );
                object.insert(
                    "tool_name".to_string(),
                    JsonValue::String(tool_name.clone()),
                );
                object.insert("output".to_string(), JsonValue::String(output.clone()));
                object.insert("is_error".to_string(), JsonValue::Bool(*is_error));
            }
        }
        JsonValue::Object(object)
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value
            .as_object()
            .ok_or_else(|| SessionError::Format("block must be an object".to_string()))?;
        match object
            .get("type")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| SessionError::Format("missing block type".to_string()))?
        {
            "text" => Ok(Self::Text {
                text: required_string(object, "text")?,
            }),
            "tool_use" => Ok(Self::ToolUse {
                id: required_string(object, "id")?,
                name: required_string(object, "name")?,
                input: required_string(object, "input")?,
            }),
            "tool_result" => Ok(Self::ToolResult {
                tool_use_id: required_string(object, "tool_use_id")?,
                tool_name: required_string(object, "tool_name")?,
                output: required_string(object, "output")?,
                is_error: object
                    .get("is_error")
                    .and_then(JsonValue::as_bool)
                    .ok_or_else(|| SessionError::Format("missing is_error".to_string()))?,
            }),
            other => Err(SessionError::Format(format!(
                "unsupported block type: {other}"
            ))),
        }
    }
}

impl SystemMessageSubtype {
    #[must_use]
    fn as_str(self) -> &'static str {
        match self {
            Self::CompactBoundary => "compact_boundary",
        }
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        match value
            .as_str()
            .ok_or_else(|| SessionError::Format("subtype must be a string".to_string()))?
        {
            "compact_boundary" => Ok(Self::CompactBoundary),
            other => Err(SessionError::Format(format!(
                "unsupported message subtype: {other}"
            ))),
        }
    }
}

impl AttachmentKind {
    #[must_use]
    fn as_str(self) -> &'static str {
        match self {
            Self::RunningAgents => "running_agents",
            Self::TodoList => "todo_list",
            Self::PlanMode => "plan_mode",
            Self::InvokedSkills => "invoked_skills",
            Self::HookAdditionalContext => "hook_additional_context",
        }
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        match value
            .as_str()
            .ok_or_else(|| SessionError::Format("attachment kind must be a string".to_string()))?
        {
            "running_agents" => Ok(Self::RunningAgents),
            "todo_list" => Ok(Self::TodoList),
            "plan_mode" => Ok(Self::PlanMode),
            "invoked_skills" => Ok(Self::InvokedSkills),
            "hook_additional_context" => Ok(Self::HookAdditionalContext),
            other => Err(SessionError::Format(format!(
                "unsupported attachment kind: {other}"
            ))),
        }
    }
}

impl CompactTrigger {
    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Auto => "auto",
        }
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        match value
            .as_str()
            .ok_or_else(|| SessionError::Format("compact trigger must be a string".to_string()))?
        {
            "manual" => Ok(Self::Manual),
            "auto" => Ok(Self::Auto),
            other => Err(SessionError::Format(format!(
                "unsupported compact trigger: {other}"
            ))),
        }
    }
}

impl AttachmentMetadata {
    fn to_json(&self) -> JsonValue {
        let mut object = BTreeMap::new();
        object.insert(
            "kind".to_string(),
            JsonValue::String(self.kind.as_str().to_string()),
        );
        JsonValue::Object(object)
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value.as_object().ok_or_else(|| {
            SessionError::Format("attachment_metadata must be an object".to_string())
        })?;
        Ok(Self {
            kind: AttachmentKind::from_json(
                object
                    .get("kind")
                    .ok_or_else(|| SessionError::Format("missing kind".to_string()))?,
            )?,
        })
    }
}

impl HookResultEvent {
    #[must_use]
    fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "session_start",
        }
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        match value
            .as_str()
            .ok_or_else(|| SessionError::Format("hook event must be a string".to_string()))?
        {
            "session_start" => Ok(Self::SessionStart),
            other => Err(SessionError::Format(format!(
                "unsupported hook event: {other}"
            ))),
        }
    }
}

impl HookResultMetadata {
    fn to_json(&self) -> JsonValue {
        let mut object = BTreeMap::new();
        object.insert(
            "event".to_string(),
            JsonValue::String(self.event.as_str().to_string()),
        );
        object.insert("source".to_string(), JsonValue::String(self.source.clone()));
        JsonValue::Object(object)
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value.as_object().ok_or_else(|| {
            SessionError::Format("hook_result_metadata must be an object".to_string())
        })?;
        Ok(Self {
            event: HookResultEvent::from_json(
                object
                    .get("event")
                    .ok_or_else(|| SessionError::Format("missing event".to_string()))?,
            )?,
            source: required_string(object, "source")?,
        })
    }
}

impl CompactPreservedSegment {
    fn to_json(&self) -> JsonValue {
        let mut object = BTreeMap::new();
        object.insert(
            "head_uuid".to_string(),
            JsonValue::String(self.head.clone()),
        );
        object.insert(
            "anchor_uuid".to_string(),
            JsonValue::String(self.anchor.clone()),
        );
        object.insert(
            "tail_uuid".to_string(),
            JsonValue::String(self.tail.clone()),
        );
        JsonValue::Object(object)
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value.as_object().ok_or_else(|| {
            SessionError::Format("preserved_segment must be an object".to_string())
        })?;
        Ok(Self {
            head: required_string(object, "head_uuid")?,
            anchor: required_string(object, "anchor_uuid")?,
            tail: required_string(object, "tail_uuid")?,
        })
    }
}

impl CompactBoundaryMetadata {
    fn to_json(&self) -> Result<JsonValue, SessionError> {
        let mut object = BTreeMap::new();
        object.insert(
            "trigger".to_string(),
            JsonValue::String(self.trigger.as_str().to_string()),
        );
        object.insert(
            "pre_tokens".to_string(),
            JsonValue::Number(i64_from_usize(self.pre_tokens, "pre_tokens")?),
        );
        if let Some(user_context) = &self.user_context {
            object.insert(
                "user_context".to_string(),
                JsonValue::String(user_context.clone()),
            );
        }
        if let Some(messages_summarized) = self.messages_summarized {
            object.insert(
                "messages_summarized".to_string(),
                JsonValue::Number(i64_from_usize(messages_summarized, "messages_summarized")?),
            );
        }
        if !self.pre_compact_discovered_tools.is_empty() {
            object.insert(
                "pre_compact_discovered_tools".to_string(),
                JsonValue::Array(
                    self.pre_compact_discovered_tools
                        .iter()
                        .cloned()
                        .map(JsonValue::String)
                        .collect(),
                ),
            );
        }
        if let Some(preserved_segment) = &self.preserved_segment {
            object.insert("preserved_segment".to_string(), preserved_segment.to_json());
        }
        Ok(JsonValue::Object(object))
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value.as_object().ok_or_else(|| {
            SessionError::Format("compact_metadata must be an object".to_string())
        })?;
        let pre_compact_discovered_tools = object
            .get("pre_compact_discovered_tools")
            .and_then(JsonValue::as_array)
            .map(|values| {
                values
                    .iter()
                    .map(|value| {
                        value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                            SessionError::Format(
                                "pre_compact_discovered_tools must be strings".to_string(),
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            trigger: CompactTrigger::from_json(
                object
                    .get("trigger")
                    .ok_or_else(|| SessionError::Format("missing trigger".to_string()))?,
            )?,
            pre_tokens: required_usize(object, "pre_tokens")?,
            user_context: object
                .get("user_context")
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned),
            messages_summarized: object
                .get("messages_summarized")
                .map(|_| required_usize(object, "messages_summarized"))
                .transpose()?,
            pre_compact_discovered_tools,
            preserved_segment: object
                .get("preserved_segment")
                .map(CompactPreservedSegment::from_json)
                .transpose()?,
        })
    }
}

impl SessionCompaction {
    pub fn to_json(&self) -> Result<JsonValue, SessionError> {
        let mut object = BTreeMap::new();
        object.insert(
            "count".to_string(),
            JsonValue::Number(i64::from(self.count)),
        );
        object.insert(
            "removed_message_count".to_string(),
            JsonValue::Number(i64_from_usize(
                self.removed_message_count,
                "removed_message_count",
            )?),
        );
        object.insert(
            "summary".to_string(),
            JsonValue::String(self.summary.clone()),
        );
        Ok(JsonValue::Object(object))
    }

    pub fn to_jsonl_record(&self) -> Result<JsonValue, SessionError> {
        let mut object = BTreeMap::new();
        object.insert(
            "type".to_string(),
            JsonValue::String("compaction".to_string()),
        );
        object.insert(
            "count".to_string(),
            JsonValue::Number(i64::from(self.count)),
        );
        object.insert(
            "removed_message_count".to_string(),
            JsonValue::Number(i64_from_usize(
                self.removed_message_count,
                "removed_message_count",
            )?),
        );
        object.insert(
            "summary".to_string(),
            JsonValue::String(self.summary.clone()),
        );
        Ok(JsonValue::Object(object))
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value
            .as_object()
            .ok_or_else(|| SessionError::Format("compaction must be an object".to_string()))?;
        Ok(Self {
            count: required_u32(object, "count")?,
            removed_message_count: required_usize(object, "removed_message_count")?,
            summary: required_string(object, "summary")?,
        })
    }
}

impl SessionFork {
    #[must_use]
    pub fn to_json(&self) -> JsonValue {
        let mut object = BTreeMap::new();
        object.insert(
            "parent_session_id".to_string(),
            JsonValue::String(self.parent_session_id.clone()),
        );
        if let Some(branch_name) = &self.branch_name {
            object.insert(
                "branch_name".to_string(),
                JsonValue::String(branch_name.clone()),
            );
        }
        JsonValue::Object(object)
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value
            .as_object()
            .ok_or_else(|| SessionError::Format("fork metadata must be an object".to_string()))?;
        Ok(Self {
            parent_session_id: required_string(object, "parent_session_id")?,
            branch_name: object
                .get("branch_name")
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned),
        })
    }
}

impl SessionPromptEntry {
    #[must_use]
    pub fn to_jsonl_record(&self) -> JsonValue {
        let mut object = BTreeMap::new();
        object.insert(
            "type".to_string(),
            JsonValue::String("prompt_history".to_string()),
        );
        object.insert(
            "timestamp_ms".to_string(),
            JsonValue::Number(i64::try_from(self.timestamp_ms).unwrap_or(i64::MAX)),
        );
        object.insert("text".to_string(), JsonValue::String(self.text.clone()));
        JsonValue::Object(object)
    }

    fn from_json_opt(value: &JsonValue) -> Option<Self> {
        let object = value.as_object()?;
        let timestamp_ms = object
            .get("timestamp_ms")
            .and_then(JsonValue::as_i64)
            .and_then(|value| u64::try_from(value).ok())?;
        let text = object.get("text").and_then(JsonValue::as_str)?.to_string();
        Some(Self { timestamp_ms, text })
    }
}

impl InvokedSkill {
    fn identity_key(&self) -> &str {
        self.resolved_name.as_deref().unwrap_or(self.skill.as_str())
    }

    fn to_json(&self) -> JsonValue {
        let mut object = BTreeMap::new();
        object.insert("skill".to_string(), JsonValue::String(self.skill.clone()));
        if let Some(resolved_name) = &self.resolved_name {
            object.insert(
                "resolved_name".to_string(),
                JsonValue::String(resolved_name.clone()),
            );
        }
        if let Some(path) = &self.path {
            object.insert("path".to_string(), JsonValue::String(path.clone()));
        }
        if let Some(description) = &self.description {
            object.insert(
                "description".to_string(),
                JsonValue::String(description.clone()),
            );
        }
        if let Some(args) = &self.args {
            object.insert("args".to_string(), JsonValue::String(args.clone()));
        }
        object.insert("prompt".to_string(), JsonValue::String(self.prompt.clone()));
        JsonValue::Object(object)
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value
            .as_object()
            .ok_or_else(|| SessionError::Format("invoked skill must be an object".to_string()))?;
        Ok(Self {
            skill: required_string(object, "skill")?,
            resolved_name: object
                .get("resolved_name")
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned)
                .and_then(|value| normalize_optional_string(Some(value))),
            path: object
                .get("path")
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned)
                .and_then(|value| normalize_optional_string(Some(value))),
            description: object
                .get("description")
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned)
                .and_then(|value| normalize_optional_string(Some(value))),
            args: object
                .get("args")
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned)
                .and_then(|value| normalize_optional_string(Some(value))),
            prompt: required_string(object, "prompt")?,
        })
    }
}

fn message_record(message: &ConversationMessage) -> JsonValue {
    let mut object = BTreeMap::new();
    object.insert("type".to_string(), JsonValue::String("message".to_string()));
    object.insert("message".to_string(), message.to_json());
    JsonValue::Object(object)
}

fn usage_to_json(usage: TokenUsage) -> JsonValue {
    let mut object = BTreeMap::new();
    object.insert(
        "input_tokens".to_string(),
        JsonValue::Number(i64::from(usage.input_tokens)),
    );
    object.insert(
        "output_tokens".to_string(),
        JsonValue::Number(i64::from(usage.output_tokens)),
    );
    object.insert(
        "cache_creation_input_tokens".to_string(),
        JsonValue::Number(i64::from(usage.cache_creation_input_tokens)),
    );
    object.insert(
        "cache_read_input_tokens".to_string(),
        JsonValue::Number(i64::from(usage.cache_read_input_tokens)),
    );
    JsonValue::Object(object)
}

fn usage_from_json(value: &JsonValue) -> Result<TokenUsage, SessionError> {
    let object = value
        .as_object()
        .ok_or_else(|| SessionError::Format("usage must be an object".to_string()))?;
    Ok(TokenUsage {
        input_tokens: required_u32(object, "input_tokens")?,
        output_tokens: required_u32(object, "output_tokens")?,
        cache_creation_input_tokens: required_u32(object, "cache_creation_input_tokens")?,
        cache_read_input_tokens: required_u32(object, "cache_read_input_tokens")?,
    })
}

fn required_string(
    object: &BTreeMap<String, JsonValue>,
    key: &str,
) -> Result<String, SessionError> {
    object
        .get(key)
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| SessionError::Format(format!("missing {key}")))
}

fn required_u32(object: &BTreeMap<String, JsonValue>, key: &str) -> Result<u32, SessionError> {
    let value = object
        .get(key)
        .and_then(JsonValue::as_i64)
        .ok_or_else(|| SessionError::Format(format!("missing {key}")))?;
    u32::try_from(value).map_err(|_| SessionError::Format(format!("{key} out of range")))
}

fn required_u64(object: &BTreeMap<String, JsonValue>, key: &str) -> Result<u64, SessionError> {
    let value = object
        .get(key)
        .ok_or_else(|| SessionError::Format(format!("missing {key}")))?;
    required_u64_from_value(value, key)
}

fn required_u64_from_value(value: &JsonValue, key: &str) -> Result<u64, SessionError> {
    let value = value
        .as_i64()
        .ok_or_else(|| SessionError::Format(format!("missing {key}")))?;
    u64::try_from(value).map_err(|_| SessionError::Format(format!("{key} out of range")))
}

fn required_usize(object: &BTreeMap<String, JsonValue>, key: &str) -> Result<usize, SessionError> {
    let value = object
        .get(key)
        .and_then(JsonValue::as_i64)
        .ok_or_else(|| SessionError::Format(format!("missing {key}")))?;
    usize::try_from(value).map_err(|_| SessionError::Format(format!("{key} out of range")))
}

fn i64_from_u64(value: u64, key: &str) -> Result<i64, SessionError> {
    i64::try_from(value)
        .map_err(|_| SessionError::Format(format!("{key} out of range for JSON number")))
}

fn i64_from_usize(value: usize, key: &str) -> Result<i64, SessionError> {
    i64::try_from(value)
        .map_err(|_| SessionError::Format(format!("{key} out of range for JSON number")))
}

fn workspace_root_to_string(path: &Path) -> Result<String, SessionError> {
    path.to_str().map(ToOwned::to_owned).ok_or_else(|| {
        SessionError::Format(format!(
            "workspace_root is not valid UTF-8: {}",
            path.display()
        ))
    })
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn invoked_skills_from_json(value: &JsonValue) -> Result<Vec<InvokedSkill>, SessionError> {
    value
        .as_array()
        .ok_or_else(|| SessionError::Format("invoked_skills must be an array".to_string()))?
        .iter()
        .map(InvokedSkill::from_json)
        .collect()
}

fn upsert_invoked_skill_entry(invoked_skills: &mut Vec<InvokedSkill>, invoked_skill: InvokedSkill) {
    if let Some(existing) = invoked_skills.iter_mut().find(|existing| {
        existing
            .identity_key()
            .eq_ignore_ascii_case(invoked_skill.identity_key())
    }) {
        *existing = invoked_skill;
        return;
    }

    invoked_skills.push(invoked_skill);
}

fn current_time_millis() -> u64 {
    let wall_clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default();

    let mut candidate = wall_clock;
    loop {
        let previous = LAST_TIMESTAMP_MS.load(Ordering::Relaxed);
        if candidate <= previous {
            candidate = previous.saturating_add(1);
        }
        match LAST_TIMESTAMP_MS.compare_exchange(
            previous,
            candidate,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => return candidate,
            Err(actual) => candidate = actual.saturating_add(1),
        }
    }
}

fn generate_session_id() -> String {
    let millis = current_time_millis();
    let counter = SESSION_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("session-{millis}-{counter}")
}

fn generate_message_uuid() -> String {
    let mut bytes = [0_u8; 16];
    if getrandom(&mut bytes).is_err() {
        let millis = current_time_millis();
        let counter = MESSAGE_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        bytes[..8].copy_from_slice(&millis.to_be_bytes());
        bytes[8..].copy_from_slice(&counter.to_be_bytes());
    }
    // RFC 4122 variant + version 4 bits.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), SessionError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = temporary_path_for(path);
    fs::write(&temp_path, contents)?;
    fs::rename(temp_path, path)?;
    Ok(())
}

fn temporary_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("session");
    path.with_file_name(format!(
        "{file_name}.tmp-{}-{}",
        current_time_millis(),
        SESSION_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn rotate_session_file_if_needed(path: &Path) -> Result<(), SessionError> {
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(());
    };
    if metadata.len() < ROTATE_AFTER_BYTES {
        return Ok(());
    }
    let rotated_path = rotated_log_path(path);
    fs::rename(path, rotated_path)?;
    Ok(())
}

fn rotated_log_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("session");
    path.with_file_name(format!("{stem}.rot-{}.jsonl", current_time_millis()))
}

fn cleanup_rotated_logs(path: &Path) -> Result<(), SessionError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("session");
    let prefix = format!("{stem}.rot-");
    let mut rotated_paths = fs::read_dir(parent)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|entry_path| {
            entry_path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| {
                    name.starts_with(&prefix)
                        && Path::new(name)
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
                })
        })
        .collect::<Vec<_>>();

    rotated_paths.sort_by_key(|entry_path| {
        fs::metadata(entry_path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH)
    });

    let remove_count = rotated_paths.len().saturating_sub(MAX_ROTATED_FILES);
    for stale_path in rotated_paths.into_iter().take(remove_count) {
        fs::remove_file(stale_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        cleanup_rotated_logs, current_time_millis, rotate_session_file_if_needed, AttachmentKind,
        CompactBoundaryMetadata, CompactPreservedSegment, CompactTrigger, ContentBlock,
        ConversationMessage, HookResultEvent, InvokedSkill, MessageRole, Session, SessionFork,
        SystemMessageSubtype,
    };
    use crate::json::JsonValue;
    use crate::usage::TokenUsage;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn session_timestamps_are_monotonic_under_tight_loops() {
        let first = current_time_millis();
        let second = current_time_millis();
        let third = current_time_millis();

        assert!(first < second);
        assert!(second < third);
    }

    #[test]
    fn persists_and_restores_session_jsonl() {
        let mut session = Session::new();
        session
            .push_user_text("hello")
            .expect("user message should append");
        session
            .push_message(ConversationMessage::assistant_with_usage(
                vec![
                    ContentBlock::Text {
                        text: "thinking".to_string(),
                    },
                    ContentBlock::ToolUse {
                        id: "tool-1".to_string(),
                        name: "bash".to_string(),
                        input: "echo hi".to_string(),
                    },
                ],
                Some(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 4,
                    cache_creation_input_tokens: 1,
                    cache_read_input_tokens: 2,
                }),
            ))
            .expect("assistant message should append");
        session
            .push_message(ConversationMessage::tool_result(
                "tool-1", "bash", "hi", false,
            ))
            .expect("tool result should append");

        let path = temp_session_path("jsonl");
        session.save_to_path(&path).expect("session should save");
        let restored = Session::load_from_path(&path).expect("session should load");
        fs::remove_file(&path).expect("temp file should be removable");

        assert_eq!(restored, session);
        assert_eq!(restored.messages[2].role, MessageRole::Tool);
        assert_eq!(
            restored.messages[1].usage.expect("usage").total_tokens(),
            17
        );
        assert_eq!(restored.session_id, session.session_id);
    }

    #[test]
    fn loads_legacy_session_json_object() {
        let path = temp_session_path("legacy");
        let legacy = JsonValue::Object(
            [
                ("version".to_string(), JsonValue::Number(1)),
                (
                    "messages".to_string(),
                    JsonValue::Array(vec![JsonValue::Object(
                        [
                            ("role".to_string(), JsonValue::String("user".to_string())),
                            (
                                "blocks".to_string(),
                                JsonValue::Array(vec![JsonValue::Object(
                                    [
                                        ("type".to_string(), JsonValue::String("text".to_string())),
                                        (
                                            "text".to_string(),
                                            JsonValue::String("legacy".to_string()),
                                        ),
                                    ]
                                    .into_iter()
                                    .collect(),
                                )]),
                            ),
                        ]
                        .into_iter()
                        .collect(),
                    )]),
                ),
            ]
            .into_iter()
            .collect(),
        );
        fs::write(&path, legacy.render()).expect("legacy file should write");

        let restored = Session::load_from_path(&path).expect("legacy session should load");
        fs::remove_file(&path).expect("temp file should be removable");

        assert_eq!(restored.messages.len(), 1);
        assert_eq!(restored.messages[0].role, MessageRole::User);
        assert!(matches!(
            restored.messages[0].blocks.as_slice(),
            [ContentBlock::Text { text }] if text == "legacy"
        ));
        assert!(!restored.messages[0].uuid.is_empty());
        assert!(!restored.session_id.is_empty());
    }

    #[test]
    fn loads_legacy_session_jsonl_without_message_uuids() {
        let path = temp_session_path("legacy-jsonl");
        let legacy = [
            r#"{"type":"session_meta","version":1,"session_id":"legacy-jsonl","created_at_ms":1,"updated_at_ms":2}"#,
            r#"{"type":"message","message":{"role":"user","blocks":[{"type":"text","text":"legacy jsonl"}]}}"#,
        ]
        .join("\n");
        fs::write(&path, format!("{legacy}\n")).expect("legacy jsonl should write");

        let restored = Session::load_from_path(&path).expect("legacy jsonl should load");
        fs::remove_file(&path).expect("temp file should be removable");

        assert_eq!(restored.messages.len(), 1);
        assert_eq!(restored.messages[0].role, MessageRole::User);
        assert!(matches!(
            restored.messages[0].blocks.as_slice(),
            [ContentBlock::Text { text }] if text == "legacy jsonl"
        ));
        assert!(!restored.messages[0].uuid.is_empty());
        assert_eq!(restored.session_id, "legacy-jsonl");
    }

    #[test]
    fn appends_messages_to_persisted_jsonl_session() {
        let path = temp_session_path("append");
        let mut session = Session::new().with_persistence_path(path.clone());
        session
            .save_to_path(&path)
            .expect("initial save should succeed");
        session
            .push_user_text("hi")
            .expect("user append should succeed");
        session
            .push_message(ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "hello".to_string(),
            }]))
            .expect("assistant append should succeed");

        let restored = Session::load_from_path(&path).expect("session should replay from jsonl");
        fs::remove_file(&path).expect("temp file should be removable");

        assert_eq!(restored.messages.len(), 2);
        assert_eq!(restored.messages[0].role, MessageRole::User);
        assert!(matches!(
            restored.messages[0].blocks.as_slice(),
            [ContentBlock::Text { text }] if text == "hi"
        ));
        assert!(!restored.messages[0].uuid.is_empty());
    }

    #[test]
    fn persists_invoked_skills_in_session_metadata() {
        let path = temp_session_path("invoked-skills");
        let mut session = Session::new().with_persistence_path(path.clone());
        session
            .push_user_text("run trace")
            .expect("message should append");
        session
            .upsert_invoked_skill(InvokedSkill {
                skill: "trace".to_string(),
                resolved_name: Some("trace".to_string()),
                path: Some("C:\\repo\\.agents\\skills\\trace\\SKILL.md".to_string()),
                description: Some("Trace helper".to_string()),
                args: Some("full".to_string()),
                prompt: "# trace\nFollow the traces\n".to_string(),
            })
            .expect("invoked skill should persist");

        let restored = Session::load_from_path(&path).expect("session should load");
        fs::remove_file(&path).expect("temp file should be removable");

        assert_eq!(restored.invoked_skills.len(), 1);
        assert_eq!(restored.invoked_skills[0].skill, "trace");
        assert_eq!(
            restored.invoked_skills[0].resolved_name.as_deref(),
            Some("trace")
        );
        assert_eq!(restored.invoked_skills[0].args.as_deref(), Some("full"));
        assert!(restored.invoked_skills[0]
            .prompt
            .contains("Follow the traces"));
        assert_eq!(restored.messages.len(), 1);
    }

    #[test]
    fn persists_compaction_metadata() {
        let path = temp_session_path("compaction");
        let mut session = Session::new();
        session
            .push_user_text("before")
            .expect("message should append");
        session.record_compaction("summarized earlier work", 4);
        session.save_to_path(&path).expect("session should save");

        let restored = Session::load_from_path(&path).expect("session should load");
        fs::remove_file(&path).expect("temp file should be removable");

        let compaction = restored.compaction.expect("compaction metadata");
        assert_eq!(compaction.count, 1);
        assert_eq!(compaction.removed_message_count, 4);
        assert!(compaction.summary.contains("summarized"));
    }

    #[test]
    fn persists_compact_boundary_message_metadata() {
        let path = temp_session_path("compact-boundary");
        let mut session = Session::new();
        session.messages = vec![
            ConversationMessage::compact_boundary(CompactBoundaryMetadata {
                trigger: CompactTrigger::Auto,
                pre_tokens: 1234,
                user_context: Some("preserve recent diagnostics".to_string()),
                messages_summarized: Some(8),
                pre_compact_discovered_tools: vec!["bash".to_string(), "read_file".to_string()],
                preserved_segment: Some(CompactPreservedSegment {
                    head: "message-head-uuid".to_string(),
                    anchor: "summary-message-uuid".to_string(),
                    tail: "message-tail-uuid".to_string(),
                }),
            }),
            ConversationMessage::user_text("Summary:\ncarried work"),
        ];
        session.save_to_path(&path).expect("session should save");

        let restored = Session::load_from_path(&path).expect("session should load");
        fs::remove_file(&path).expect("temp file should be removable");

        assert_eq!(
            restored.messages[0].subtype,
            Some(SystemMessageSubtype::CompactBoundary)
        );
        let metadata = restored.messages[0]
            .compact_metadata
            .as_ref()
            .expect("compact boundary metadata");
        assert_eq!(metadata.trigger, CompactTrigger::Auto);
        assert_eq!(metadata.pre_tokens, 1234);
        assert_eq!(metadata.messages_summarized, Some(8));
        assert_eq!(
            metadata.pre_compact_discovered_tools,
            vec!["bash".to_string(), "read_file".to_string()]
        );
        assert_eq!(
            metadata.preserved_segment.as_ref().map(|segment| (
                segment.head.as_str(),
                segment.anchor.as_str(),
                segment.tail.as_str()
            )),
            Some((
                "message-head-uuid",
                "summary-message-uuid",
                "message-tail-uuid"
            ))
        );
    }

    #[test]
    fn persists_compact_summary_message_flags() {
        let path = temp_session_path("compact-summary");
        let mut session = Session::new();
        session.messages = vec![ConversationMessage::compact_summary_user_text(
            "Summary:\ncarry over context",
            true,
        )];
        session.save_to_path(&path).expect("session should save");

        let restored = Session::load_from_path(&path).expect("session should load");
        fs::remove_file(&path).expect("temp file should be removable");

        assert!(restored.messages[0].is_compact_summary);
        assert!(restored.messages[0].is_visible_in_transcript_only);
    }

    #[test]
    fn persists_attachment_and_hook_result_metadata() {
        let path = temp_session_path("attachment-hook");
        let mut session = Session::new();
        session.messages = vec![
            ConversationMessage::attachment_user_text(
                "Previously invoked skills remain available after compaction.",
                AttachmentKind::InvokedSkills,
            ),
            ConversationMessage::hook_result_user_text(
                "SessionStart hook (compact) output:\nPreserve diagnostics",
                Some(AttachmentKind::HookAdditionalContext),
                HookResultEvent::SessionStart,
                "compact",
            ),
        ];
        session.save_to_path(&path).expect("session should save");

        let restored = Session::load_from_path(&path).expect("session should load");
        fs::remove_file(&path).expect("temp file should be removable");

        assert_eq!(
            restored.messages[0]
                .attachment_metadata
                .as_ref()
                .map(|metadata| metadata.kind),
            Some(AttachmentKind::InvokedSkills)
        );
        assert_eq!(
            restored.messages[1]
                .attachment_metadata
                .as_ref()
                .map(|metadata| metadata.kind),
            Some(AttachmentKind::HookAdditionalContext)
        );
        let hook_metadata = restored.messages[1]
            .hook_result_metadata
            .as_ref()
            .expect("hook result metadata");
        assert_eq!(hook_metadata.event, HookResultEvent::SessionStart);
        assert_eq!(hook_metadata.source, "compact");
    }

    #[test]
    fn forks_sessions_with_branch_metadata_and_persists_it() {
        let path = temp_session_path("fork");
        let mut session = Session::new();
        session
            .push_user_text("before fork")
            .expect("message should append");

        let forked = session
            .fork(Some("investigation".to_string()))
            .with_persistence_path(path.clone());
        forked
            .save_to_path(&path)
            .expect("forked session should save");

        let restored = Session::load_from_path(&path).expect("forked session should load");
        fs::remove_file(&path).expect("temp file should be removable");

        assert_ne!(restored.session_id, session.session_id);
        assert_eq!(
            restored.fork,
            Some(SessionFork {
                parent_session_id: session.session_id,
                branch_name: Some("investigation".to_string()),
            })
        );
        assert_eq!(restored.messages, forked.messages);
    }

    #[test]
    fn rotates_and_cleans_up_large_session_logs() {
        // given
        let path = temp_session_path("rotation");
        let oversized_length =
            usize::try_from(super::ROTATE_AFTER_BYTES + 10).expect("rotate threshold should fit");
        fs::write(&path, "x".repeat(oversized_length)).expect("oversized file should write");

        // when
        rotate_session_file_if_needed(&path).expect("rotation should succeed");

        // then
        assert!(
            !path.exists(),
            "original path should be rotated away before rewrite"
        );

        for _ in 0..5 {
            let rotated = super::rotated_log_path(&path);
            fs::write(&rotated, "old").expect("rotated file should write");
        }
        cleanup_rotated_logs(&path).expect("cleanup should succeed");

        let rotated_count = rotation_files(&path).len();
        assert!(rotated_count <= super::MAX_ROTATED_FILES);
        for rotated in rotation_files(&path) {
            fs::remove_file(rotated).expect("rotated file should be removable");
        }
    }

    #[test]
    fn rejects_jsonl_record_without_type() {
        // given
        let path = write_temp_session_file(
            "missing-type",
            r#"{"message":{"role":"user","blocks":[{"type":"text","text":"hello"}]}}"#,
        );

        // when
        let error = Session::load_from_path(&path)
            .expect_err("session should reject JSONL records without a type");

        // then
        assert!(error.to_string().contains("missing type"));
        fs::remove_file(path).expect("temp file should be removable");
    }

    #[test]
    fn rejects_jsonl_message_record_without_message_payload() {
        // given
        let path = write_temp_session_file("missing-message", r#"{"type":"message"}"#);

        // when
        let error = Session::load_from_path(&path)
            .expect_err("session should reject JSONL message records without message payload");

        // then
        assert!(error.to_string().contains("missing message"));
        fs::remove_file(path).expect("temp file should be removable");
    }

    #[test]
    fn rejects_jsonl_record_with_unknown_type() {
        // given
        let path = write_temp_session_file("unknown-type", r#"{"type":"mystery"}"#);

        // when
        let error = Session::load_from_path(&path)
            .expect_err("session should reject unknown JSONL record types");

        // then
        assert!(error.to_string().contains("unsupported JSONL record type"));
        fs::remove_file(path).expect("temp file should be removable");
    }

    #[test]
    fn rejects_legacy_session_json_without_messages() {
        // given
        let session = JsonValue::Object(
            [("version".to_string(), JsonValue::Number(1))]
                .into_iter()
                .collect(),
        );

        // when
        let error = Session::from_json(&session)
            .expect_err("legacy session objects should require messages");

        // then
        assert!(error.to_string().contains("missing messages"));
    }

    #[test]
    fn normalizes_blank_fork_branch_name_to_none() {
        // given
        let session = Session::new();

        // when
        let forked = session.fork(Some("   ".to_string()));

        // then
        assert_eq!(forked.fork.expect("fork metadata").branch_name, None);
    }

    #[test]
    fn rejects_unknown_content_block_type() {
        // given
        let block = JsonValue::Object(
            [("type".to_string(), JsonValue::String("unknown".to_string()))]
                .into_iter()
                .collect(),
        );

        // when
        let error = ContentBlock::from_json(&block)
            .expect_err("content blocks should reject unknown types");

        // then
        assert!(error.to_string().contains("unsupported block type"));
    }

    #[test]
    fn rejects_attachment_metadata_on_non_user_messages() {
        let message = JsonValue::Object(
            [
                (
                    "role".to_string(),
                    JsonValue::String("assistant".to_string()),
                ),
                (
                    "blocks".to_string(),
                    JsonValue::Array(vec![JsonValue::Object(
                        [
                            ("type".to_string(), JsonValue::String("text".to_string())),
                            ("text".to_string(), JsonValue::String("hello".to_string())),
                        ]
                        .into_iter()
                        .collect(),
                    )]),
                ),
                (
                    "attachment_metadata".to_string(),
                    JsonValue::Object(
                        [(
                            "kind".to_string(),
                            JsonValue::String("invoked_skills".to_string()),
                        )]
                        .into_iter()
                        .collect(),
                    ),
                ),
            ]
            .into_iter()
            .collect(),
        );

        let error = ConversationMessage::from_json(&message)
            .expect_err("assistant messages should reject attachment metadata");

        assert!(error
            .to_string()
            .contains("attachment_metadata is only supported on user messages"));
    }

    #[test]
    fn persists_workspace_root_round_trip_and_forks_inherit_it() {
        // given
        let path = temp_session_path("workspace-root");
        let workspace_root = PathBuf::from("/tmp/b4-phantom-diag");
        let mut session = Session::new().with_workspace_root(workspace_root.clone());
        session
            .push_user_text("write to the right cwd")
            .expect("user message should append");

        // when
        session
            .save_to_path(&path)
            .expect("workspace-bound session should save");
        let restored = Session::load_from_path(&path).expect("session should load");
        let forked = restored.fork(Some("phantom-diag".to_string()));
        fs::remove_file(&path).expect("temp file should be removable");

        // then
        assert_eq!(restored.workspace_root(), Some(workspace_root.as_path()));
        assert_eq!(forked.workspace_root(), Some(workspace_root.as_path()));
    }

    fn temp_session_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("runtime-session-{label}-{nanos}.json"))
    }

    fn write_temp_session_file(label: &str, contents: &str) -> PathBuf {
        let path = temp_session_path(label);
        fs::write(&path, format!("{contents}\n")).expect("temp session file should write");
        path
    }

    fn rotation_files(path: &Path) -> Vec<PathBuf> {
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .expect("temp path should have file stem")
            .to_string();
        fs::read_dir(path.parent().expect("temp path should have parent"))
            .expect("temp dir should read")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|entry_path| {
                entry_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| {
                        name.starts_with(&format!("{stem}.rot-"))
                            && Path::new(name)
                                .extension()
                                .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
                    })
            })
            .collect()
    }
}

/// Per-worktree session isolation: returns a session directory namespaced
/// by the workspace fingerprint of the given working directory.
/// This prevents parallel `opencode serve` instances from colliding.
/// Called by external consumers (e.g. clawhip) to enumerate sessions for a CWD.
#[allow(dead_code)]
pub fn workspace_sessions_dir(cwd: &std::path::Path) -> Result<std::path::PathBuf, SessionError> {
    let store = crate::session_control::SessionStore::from_cwd(cwd)
        .map_err(|e| SessionError::Io(std::io::Error::other(e.to_string())))?;
    Ok(store.sessions_dir().to_path_buf())
}

#[cfg(test)]
mod workspace_sessions_dir_tests {
    use super::*;
    use std::fs;

    #[test]
    fn workspace_sessions_dir_returns_fingerprinted_path_for_valid_cwd() {
        let tmp = std::env::temp_dir().join("claw-session-dir-test");
        fs::create_dir_all(&tmp).expect("create temp dir");

        let result = workspace_sessions_dir(&tmp);
        assert!(
            result.is_ok(),
            "workspace_sessions_dir should succeed for a valid CWD, got: {result:?}"
        );
        let dir = result.unwrap();
        // The returned path should be non-empty and end with a hash component
        assert!(!dir.as_os_str().is_empty());
        // Two calls with the same CWD should produce identical paths (deterministic)
        let result2 = workspace_sessions_dir(&tmp).unwrap();
        assert_eq!(dir, result2, "workspace_sessions_dir must be deterministic");

        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn workspace_sessions_dir_differs_for_different_cwds() {
        let tmp_a = std::env::temp_dir().join("claw-session-dir-a");
        let tmp_b = std::env::temp_dir().join("claw-session-dir-b");
        fs::create_dir_all(&tmp_a).expect("create dir a");
        fs::create_dir_all(&tmp_b).expect("create dir b");

        let dir_a = workspace_sessions_dir(&tmp_a).expect("dir a");
        let dir_b = workspace_sessions_dir(&tmp_b).expect("dir b");
        assert_ne!(
            dir_a, dir_b,
            "different CWDs must produce different session dirs"
        );

        fs::remove_dir_all(&tmp_a).ok();
        fs::remove_dir_all(&tmp_b).ok();
    }
}
