use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use api::{
    max_tokens_for_model, resolve_model_alias, InputContentBlock, InputMessage, MessageRequest,
    MessageResponse, OutputContentBlock, PromptCache, ProviderClient, ProviderKind,
    ProviderOverride, ToolResultContentBlock,
};
use glob::Pattern;
use plugins::{
    PluginError, PluginHooks, PluginManager, PluginManagerConfig, PluginRegistry, PluginSummary,
};
use runtime::{
    detect_appfs_environment, load_system_prompt, tool_output_root, AssistantEvent,
    CompactionConfig, ConfigLoader, ConfigSource, ConversationMessage, ConversationRuntime,
    McpOAuthConfig, McpServerConfig, MessageRole, PermissionMode, PermissionPolicy, RuntimeConfig,
    RuntimeFeatureConfig, RuntimeHookConfig, RuntimeProviderConfig, RuntimeProviderKind,
    ScopedMcpServerConfig, Session, StaticToolExecutor,
};
use serde_json::{json, Value};

mod bundled_skills;
mod skill_docs;

pub use bundled_skills::{
    bundled_skill_reference_files, render_bundled_skill_prompt, resolve_bundled_skill,
    BundledSkill, BundledSkillId,
};
pub use skill_docs::{
    extract_skill_frontmatter_name, load_skill_document, parse_skill_document, SkillDocument,
    SkillExecutionContext,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSkill {
    pub document: SkillDocument,
    pub source: ResolvedSkillSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedSkillSource {
    Filesystem {
        path: PathBuf,
    },
    Bundled {
        id: BundledSkillId,
    },
    Generated {
        id: String,
        base_dir: Option<PathBuf>,
    },
}

#[must_use]
pub fn render_resolved_skill_prompt(skill: &ResolvedSkill, args: Option<&str>) -> String {
    match &skill.source {
        ResolvedSkillSource::Filesystem { .. } => {
            skill.document.render_markdown_with_arguments(args)
        }
        ResolvedSkillSource::Bundled { id } => render_bundled_skill_prompt(
            &BundledSkill {
                id: *id,
                document: skill.document.clone(),
            },
            args,
        ),
        ResolvedSkillSource::Generated { .. } => {
            skill.document.render_markdown_with_arguments(args)
        }
    }
}

#[must_use]
pub fn resolved_skill_reference_files(skill: &ResolvedSkill) -> BTreeMap<String, String> {
    match &skill.source {
        ResolvedSkillSource::Filesystem { .. } => BTreeMap::new(),
        ResolvedSkillSource::Bundled { id } => bundled_skill_reference_files(&BundledSkill {
            id: *id,
            document: skill.document.clone(),
        }),
        ResolvedSkillSource::Generated { .. } => BTreeMap::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandManifestEntry {
    pub name: String,
    pub source: CommandSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSource {
    Builtin,
    InternalOnly,
    FeatureGated,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandRegistry {
    entries: Vec<CommandManifestEntry>,
}

impl CommandRegistry {
    #[must_use]
    pub fn new(entries: Vec<CommandManifestEntry>) -> Self {
        Self { entries }
    }

    #[must_use]
    pub fn entries(&self) -> &[CommandManifestEntry] {
        &self.entries
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub summary: &'static str,
    pub argument_hint: Option<&'static str>,
    pub resume_supported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillSlashDispatch {
    Local,
    Invoke(String),
}

const DEFAULT_DATE: &str = "2026-03-31";
const DEFAULT_MODEL: &str = "claude-opus-4-6";

const SLASH_COMMAND_SPECS: &[SlashCommandSpec] = &[
    SlashCommandSpec {
        name: "help",
        aliases: &[],
        summary: "Show available slash commands",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "status",
        aliases: &[],
        summary: "Show current session status with branch freshness, worktrees, and recent commits",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "sandbox",
        aliases: &[],
        summary: "Show sandbox isolation status",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "compact",
        aliases: &[],
        summary: "Compact local session history",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "model",
        aliases: &[],
        summary: "Show or switch the active model",
        argument_hint: Some("[model]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "permissions",
        aliases: &[],
        summary: "Show or switch the active permission mode",
        argument_hint: Some("[read-only|workspace-write|danger-full-access]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "clear",
        aliases: &[],
        summary: "Start a fresh local session",
        argument_hint: Some("[--confirm]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "cost",
        aliases: &[],
        summary: "Show cumulative token usage for this session",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "resume",
        aliases: &[],
        summary: "Load a saved session into the REPL",
        argument_hint: Some("<session-path>"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "config",
        aliases: &[],
        summary: "Inspect Claw config files or merged sections",
        argument_hint: Some("[env|hooks|model|provider|plugins]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "mcp",
        aliases: &[],
        summary: "Inspect configured MCP servers",
        argument_hint: Some("[list|show <server>|help]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "memory",
        aliases: &[],
        summary: "Inspect loaded Claude instruction memory files",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "init",
        aliases: &[],
        summary: "Create a starter CLAUDE.md for this repo",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "diff",
        aliases: &[],
        summary: "Show git diff for current workspace changes",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "version",
        aliases: &[],
        summary: "Show CLI version and build information",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "bughunter",
        aliases: &[],
        summary: "Inspect the codebase for likely bugs",
        argument_hint: Some("[scope]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "commit",
        aliases: &[],
        summary: "Generate a commit message and create a git commit",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "pr",
        aliases: &[],
        summary: "Draft or create a pull request from the conversation",
        argument_hint: Some("[context]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "issue",
        aliases: &[],
        summary: "Draft or create a GitHub issue from the conversation",
        argument_hint: Some("[context]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "ultraplan",
        aliases: &[],
        summary: "Run a deep planning prompt with multi-step reasoning",
        argument_hint: Some("[task]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "teleport",
        aliases: &[],
        summary: "Jump to a file or symbol by searching the workspace",
        argument_hint: Some("<symbol-or-path>"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "debug-tool-call",
        aliases: &[],
        summary: "Replay the last tool call with debug details",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "export",
        aliases: &[],
        summary: "Export the current conversation to a file",
        argument_hint: Some("[file]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "session",
        aliases: &[],
        summary: "List, switch, fork, or delete managed local sessions",
        argument_hint: Some(
            "[list|switch <session-id>|fork [branch-name]|delete <session-id> [--force]]",
        ),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "principal",
        aliases: &[],
        summary: "List or create AppFS semantic principals",
        argument_hint: Some("[list|create <principal-id> [description]]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "plugin",
        aliases: &["plugins", "marketplace"],
        summary: "Manage Claw Code plugins",
        argument_hint: Some(
            "[list|install <path>|enable <name>|disable <name>|uninstall <id>|update <id>]",
        ),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "agents",
        aliases: &[],
        summary: "List configured agents",
        argument_hint: Some("[list|help]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "skills",
        aliases: &[],
        summary: "List or install available skills",
        argument_hint: Some("[list|install <path>|help]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "doctor",
        aliases: &[],
        summary: "Diagnose setup issues and environment health",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "login",
        aliases: &[],
        summary: "Log in to the service",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "logout",
        aliases: &[],
        summary: "Log out of the current session",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "plan",
        aliases: &[],
        summary: "Toggle or inspect planning mode",
        argument_hint: Some("[on|off]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "review",
        aliases: &[],
        summary: "Run a code review on current changes",
        argument_hint: Some("[scope]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "tasks",
        aliases: &[],
        summary: "List and manage background tasks",
        argument_hint: Some("[list|get <id>|stop <id>]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "theme",
        aliases: &[],
        summary: "Switch the terminal color theme",
        argument_hint: Some("[theme-name]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "vim",
        aliases: &[],
        summary: "Toggle vim keybinding mode",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "voice",
        aliases: &[],
        summary: "Toggle voice input mode",
        argument_hint: Some("[on|off]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "upgrade",
        aliases: &[],
        summary: "Check for and install CLI updates",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "usage",
        aliases: &[],
        summary: "Show detailed API usage statistics",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "stats",
        aliases: &[],
        summary: "Show workspace and session statistics",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "rename",
        aliases: &[],
        summary: "Rename the current session",
        argument_hint: Some("<name>"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "copy",
        aliases: &[],
        summary: "Copy conversation or output to clipboard",
        argument_hint: Some("[last|all]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "share",
        aliases: &[],
        summary: "Share the current conversation",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "feedback",
        aliases: &[],
        summary: "Submit feedback about the current session",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "hooks",
        aliases: &[],
        summary: "List and manage lifecycle hooks",
        argument_hint: Some("[list|run <hook>]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "files",
        aliases: &[],
        summary: "List files in the current context window",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "context",
        aliases: &[],
        summary: "Inspect or manage the conversation context",
        argument_hint: Some("[show|clear]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "color",
        aliases: &[],
        summary: "Configure terminal color settings",
        argument_hint: Some("[scheme]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "effort",
        aliases: &[],
        summary: "Set the effort level for responses",
        argument_hint: Some("[low|medium|high]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "fast",
        aliases: &[],
        summary: "Toggle fast/concise response mode",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "exit",
        aliases: &[],
        summary: "Exit the REPL session",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "branch",
        aliases: &[],
        summary: "Create or switch git branches",
        argument_hint: Some("[name]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "rewind",
        aliases: &[],
        summary: "Rewind the conversation to a previous state",
        argument_hint: Some("[steps]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "summary",
        aliases: &[],
        summary: "Generate a summary of the conversation",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "desktop",
        aliases: &[],
        summary: "Open or manage the desktop app integration",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "ide",
        aliases: &[],
        summary: "Open or configure IDE integration",
        argument_hint: Some("[vscode|cursor]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "tag",
        aliases: &[],
        summary: "Tag the current conversation point",
        argument_hint: Some("[label]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "brief",
        aliases: &[],
        summary: "Toggle brief output mode",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "advisor",
        aliases: &[],
        summary: "Toggle advisor mode for guidance-only responses",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "stickers",
        aliases: &[],
        summary: "Browse and manage sticker packs",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "insights",
        aliases: &[],
        summary: "Show AI-generated insights about the session",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "thinkback",
        aliases: &[],
        summary: "Replay the thinking process of the last response",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "release-notes",
        aliases: &[],
        summary: "Generate release notes from recent changes",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "security-review",
        aliases: &[],
        summary: "Run a security review on the codebase",
        argument_hint: Some("[scope]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "keybindings",
        aliases: &[],
        summary: "Show or configure keyboard shortcuts",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "privacy-settings",
        aliases: &[],
        summary: "View or modify privacy settings",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "output-style",
        aliases: &[],
        summary: "Switch output formatting style",
        argument_hint: Some("[style]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "add-dir",
        aliases: &[],
        summary: "Add an additional directory to the context",
        argument_hint: Some("<path>"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "allowed-tools",
        aliases: &[],
        summary: "Show or modify the allowed tools list",
        argument_hint: Some("[add|remove|list] [tool]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "api-key",
        aliases: &[],
        summary: "Show or set the Anthropic API key",
        argument_hint: Some("[key]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "approve",
        aliases: &["yes", "y"],
        summary: "Approve a pending tool execution",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "deny",
        aliases: &["no", "n"],
        summary: "Deny a pending tool execution",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "undo",
        aliases: &[],
        summary: "Undo the last file write or edit",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "stop",
        aliases: &[],
        summary: "Stop the current generation",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "retry",
        aliases: &[],
        summary: "Retry the last failed message",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "paste",
        aliases: &[],
        summary: "Paste clipboard content as input",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "screenshot",
        aliases: &[],
        summary: "Take a screenshot and add to conversation",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "image",
        aliases: &[],
        summary: "Add an image file to the conversation",
        argument_hint: Some("<path>"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "terminal-setup",
        aliases: &[],
        summary: "Configure terminal integration settings",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "search",
        aliases: &[],
        summary: "Search files in the workspace",
        argument_hint: Some("<query>"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "listen",
        aliases: &[],
        summary: "Listen for voice input",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "speak",
        aliases: &[],
        summary: "Read the last response aloud",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "language",
        aliases: &[],
        summary: "Set the interface language",
        argument_hint: Some("[language]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "profile",
        aliases: &[],
        summary: "Show or switch user profile",
        argument_hint: Some("[name]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "max-tokens",
        aliases: &[],
        summary: "Show or set the max output tokens",
        argument_hint: Some("[count]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "temperature",
        aliases: &[],
        summary: "Show or set the sampling temperature",
        argument_hint: Some("[value]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "system-prompt",
        aliases: &[],
        summary: "Show the active system prompt",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "tool-details",
        aliases: &[],
        summary: "Show detailed info about a specific tool",
        argument_hint: Some("<tool-name>"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "format",
        aliases: &[],
        summary: "Format the last response in a different style",
        argument_hint: Some("[markdown|plain|json]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "pin",
        aliases: &[],
        summary: "Pin a message to persist across compaction",
        argument_hint: Some("[message-index]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "unpin",
        aliases: &[],
        summary: "Unpin a previously pinned message",
        argument_hint: Some("[message-index]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "bookmarks",
        aliases: &[],
        summary: "List or manage conversation bookmarks",
        argument_hint: Some("[add|remove|list]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "workspace",
        aliases: &["cwd"],
        summary: "Show or change the working directory",
        argument_hint: Some("[path]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "history",
        aliases: &[],
        summary: "Show conversation history summary",
        argument_hint: Some("[count]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "tokens",
        aliases: &[],
        summary: "Show token count for the current conversation",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "cache",
        aliases: &[],
        summary: "Show prompt cache statistics",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "providers",
        aliases: &[],
        summary: "List available model providers",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "notifications",
        aliases: &[],
        summary: "Show or configure notification settings",
        argument_hint: Some("[on|off|status]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "changelog",
        aliases: &[],
        summary: "Show recent changes to the codebase",
        argument_hint: Some("[count]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "test",
        aliases: &[],
        summary: "Run tests for the current project",
        argument_hint: Some("[filter]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "lint",
        aliases: &[],
        summary: "Run linting for the current project",
        argument_hint: Some("[filter]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "build",
        aliases: &[],
        summary: "Build the current project",
        argument_hint: Some("[target]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "run",
        aliases: &[],
        summary: "Run a command in the project context",
        argument_hint: Some("<command>"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "git",
        aliases: &[],
        summary: "Run a git command in the workspace",
        argument_hint: Some("<subcommand>"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "stash",
        aliases: &[],
        summary: "Stash or unstash workspace changes",
        argument_hint: Some("[pop|list|apply]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "blame",
        aliases: &[],
        summary: "Show git blame for a file",
        argument_hint: Some("<file> [line]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "log",
        aliases: &[],
        summary: "Show git log for the workspace",
        argument_hint: Some("[count]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "cron",
        aliases: &[],
        summary: "Manage scheduled tasks",
        argument_hint: Some("[list|add|remove]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "team",
        aliases: &[],
        summary: "Manage agent teams",
        argument_hint: Some("[list|create|delete]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "benchmark",
        aliases: &[],
        summary: "Run performance benchmarks",
        argument_hint: Some("[suite]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "migrate",
        aliases: &[],
        summary: "Run pending data migrations",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "reset",
        aliases: &[],
        summary: "Reset configuration to defaults",
        argument_hint: Some("[section]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "telemetry",
        aliases: &[],
        summary: "Show or configure telemetry settings",
        argument_hint: Some("[on|off|status]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "env",
        aliases: &[],
        summary: "Show environment variables visible to tools",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "project",
        aliases: &[],
        summary: "Show project detection info",
        argument_hint: None,
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "templates",
        aliases: &[],
        summary: "List or apply prompt templates",
        argument_hint: Some("[list|apply <name>]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "explain",
        aliases: &[],
        summary: "Explain a file or code snippet",
        argument_hint: Some("<path> [line-range]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "refactor",
        aliases: &[],
        summary: "Suggest refactoring for a file or function",
        argument_hint: Some("<path> [scope]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "docs",
        aliases: &[],
        summary: "Generate or show documentation",
        argument_hint: Some("[path]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "fix",
        aliases: &[],
        summary: "Fix errors in a file or project",
        argument_hint: Some("[path]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "perf",
        aliases: &[],
        summary: "Analyze performance of a function or file",
        argument_hint: Some("<path>"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "chat",
        aliases: &[],
        summary: "Switch to free-form chat mode",
        argument_hint: None,
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "focus",
        aliases: &[],
        summary: "Focus context on specific files or directories",
        argument_hint: Some("<path> [path...]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "unfocus",
        aliases: &[],
        summary: "Remove focus from files or directories",
        argument_hint: Some("[path...]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "web",
        aliases: &[],
        summary: "Fetch and summarize a web page",
        argument_hint: Some("<url>"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "map",
        aliases: &[],
        summary: "Show a visual map of the codebase structure",
        argument_hint: Some("[depth]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "symbols",
        aliases: &[],
        summary: "List symbols (functions, classes, etc.) in a file",
        argument_hint: Some("<path>"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "references",
        aliases: &[],
        summary: "Find all references to a symbol",
        argument_hint: Some("<symbol>"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "definition",
        aliases: &[],
        summary: "Go to the definition of a symbol",
        argument_hint: Some("<symbol>"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "hover",
        aliases: &[],
        summary: "Show hover information for a symbol",
        argument_hint: Some("<symbol>"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "diagnostics",
        aliases: &[],
        summary: "Show LSP diagnostics for a file",
        argument_hint: Some("[path]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "autofix",
        aliases: &[],
        summary: "Auto-fix all fixable diagnostics",
        argument_hint: Some("[path]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "multi",
        aliases: &[],
        summary: "Execute multiple slash commands in sequence",
        argument_hint: Some("<commands>"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "macro",
        aliases: &[],
        summary: "Record or replay command macros",
        argument_hint: Some("[record|stop|play <name>]"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "alias",
        aliases: &[],
        summary: "Create a command alias",
        argument_hint: Some("<name> <command>"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "parallel",
        aliases: &[],
        summary: "Run commands in parallel subagents",
        argument_hint: Some("<count> <prompt>"),
        resume_supported: false,
    },
    SlashCommandSpec {
        name: "agent",
        aliases: &[],
        summary: "Manage sub-agents and spawned sessions",
        argument_hint: Some("[list|spawn|kill]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "subagent",
        aliases: &[],
        summary: "Control active subagent execution",
        argument_hint: Some("[list|steer <target> <msg>|kill <id>]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "reasoning",
        aliases: &[],
        summary: "Toggle extended reasoning mode",
        argument_hint: Some("[on|off|stream]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "budget",
        aliases: &[],
        summary: "Show or set token budget limits",
        argument_hint: Some("[show|set <limit>]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "rate-limit",
        aliases: &[],
        summary: "Configure API rate limiting",
        argument_hint: Some("[status|set <rpm>]"),
        resume_supported: true,
    },
    SlashCommandSpec {
        name: "metrics",
        aliases: &[],
        summary: "Show performance and usage metrics",
        argument_hint: None,
        resume_supported: true,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    Help,
    Status,
    Sandbox,
    Compact,
    Bughunter {
        scope: Option<String>,
    },
    Commit,
    Pr {
        context: Option<String>,
    },
    Issue {
        context: Option<String>,
    },
    Ultraplan {
        task: Option<String>,
    },
    Teleport {
        target: Option<String>,
    },
    DebugToolCall,
    Model {
        model: Option<String>,
    },
    Permissions {
        mode: Option<String>,
    },
    Clear {
        confirm: bool,
    },
    Cost,
    Resume {
        session_path: Option<String>,
    },
    Config {
        section: Option<String>,
    },
    Mcp {
        action: Option<String>,
        target: Option<String>,
    },
    Memory,
    Init,
    Diff,
    Version,
    Export {
        path: Option<String>,
    },
    Session {
        action: Option<String>,
        target: Option<String>,
    },
    Principal {
        action: Option<String>,
        target: Option<String>,
        description: Option<String>,
    },
    Plugins {
        action: Option<String>,
        target: Option<String>,
    },
    Agents {
        args: Option<String>,
    },
    Skills {
        args: Option<String>,
    },
    Doctor,
    Login,
    Logout,
    Vim,
    Upgrade,
    Stats,
    Share,
    Feedback,
    Files,
    Fast,
    Exit,
    Summary,
    Desktop,
    Brief,
    Advisor,
    Stickers,
    Insights,
    Thinkback,
    ReleaseNotes,
    SecurityReview,
    Keybindings,
    PrivacySettings,
    Plan {
        mode: Option<String>,
    },
    Review {
        scope: Option<String>,
    },
    Tasks {
        args: Option<String>,
    },
    Theme {
        name: Option<String>,
    },
    Voice {
        mode: Option<String>,
    },
    Usage {
        scope: Option<String>,
    },
    Rename {
        name: Option<String>,
    },
    Copy {
        target: Option<String>,
    },
    Hooks {
        args: Option<String>,
    },
    Context {
        action: Option<String>,
    },
    Color {
        scheme: Option<String>,
    },
    Effort {
        level: Option<String>,
    },
    Branch {
        name: Option<String>,
    },
    Rewind {
        steps: Option<String>,
    },
    Ide {
        target: Option<String>,
    },
    Tag {
        label: Option<String>,
    },
    OutputStyle {
        style: Option<String>,
    },
    AddDir {
        path: Option<String>,
    },
    History {
        count: Option<String>,
    },
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommandParseError {
    message: String,
}

impl SlashCommandParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SlashCommandParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SlashCommandParseError {}

impl SlashCommand {
    pub fn parse(input: &str) -> Result<Option<Self>, SlashCommandParseError> {
        validate_slash_command_input(input)
    }
}

#[allow(clippy::too_many_lines)]
pub fn validate_slash_command_input(
    input: &str,
) -> Result<Option<SlashCommand>, SlashCommandParseError> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return Ok(None);
    }

    let mut parts = trimmed.trim_start_matches('/').split_whitespace();
    let command = parts.next().unwrap_or_default();
    if command.is_empty() {
        return Err(SlashCommandParseError::new(
            "Slash command name is missing. Use /help to list available slash commands.",
        ));
    }

    let args = parts.collect::<Vec<_>>();
    let remainder = remainder_after_command(trimmed, command);

    Ok(Some(match command {
        "help" => {
            validate_no_args(command, &args)?;
            SlashCommand::Help
        }
        "status" => {
            validate_no_args(command, &args)?;
            SlashCommand::Status
        }
        "sandbox" => {
            validate_no_args(command, &args)?;
            SlashCommand::Sandbox
        }
        "compact" => {
            validate_no_args(command, &args)?;
            SlashCommand::Compact
        }
        "bughunter" => SlashCommand::Bughunter { scope: remainder },
        "commit" => {
            validate_no_args(command, &args)?;
            SlashCommand::Commit
        }
        "pr" => SlashCommand::Pr { context: remainder },
        "issue" => SlashCommand::Issue { context: remainder },
        "ultraplan" => SlashCommand::Ultraplan { task: remainder },
        "teleport" => SlashCommand::Teleport {
            target: Some(require_remainder(command, remainder, "<symbol-or-path>")?),
        },
        "debug-tool-call" => {
            validate_no_args(command, &args)?;
            SlashCommand::DebugToolCall
        }
        "model" => SlashCommand::Model {
            model: optional_single_arg(command, &args, "[model]")?,
        },
        "permissions" => SlashCommand::Permissions {
            mode: parse_permissions_mode(&args)?,
        },
        "clear" => SlashCommand::Clear {
            confirm: parse_clear_args(&args)?,
        },
        "cost" => {
            validate_no_args(command, &args)?;
            SlashCommand::Cost
        }
        "resume" => SlashCommand::Resume {
            session_path: Some(require_remainder(command, remainder, "<session-path>")?),
        },
        "config" => SlashCommand::Config {
            section: parse_config_section(&args)?,
        },
        "mcp" => parse_mcp_command(&args)?,
        "memory" => {
            validate_no_args(command, &args)?;
            SlashCommand::Memory
        }
        "init" => {
            validate_no_args(command, &args)?;
            SlashCommand::Init
        }
        "diff" => {
            validate_no_args(command, &args)?;
            SlashCommand::Diff
        }
        "version" => {
            validate_no_args(command, &args)?;
            SlashCommand::Version
        }
        "export" => SlashCommand::Export { path: remainder },
        "session" => parse_session_command(&args)?,
        "principal" => parse_principal_command(&args)?,
        "plugin" | "plugins" | "marketplace" => parse_plugin_command(&args)?,
        "agents" => SlashCommand::Agents {
            args: parse_list_or_help_args(command, remainder)?,
        },
        "skills" => SlashCommand::Skills {
            args: parse_skills_args(remainder.as_deref())?,
        },
        "doctor" => {
            validate_no_args(command, &args)?;
            SlashCommand::Doctor
        }
        "login" => {
            validate_no_args(command, &args)?;
            SlashCommand::Login
        }
        "logout" => {
            validate_no_args(command, &args)?;
            SlashCommand::Logout
        }
        "vim" => {
            validate_no_args(command, &args)?;
            SlashCommand::Vim
        }
        "upgrade" => {
            validate_no_args(command, &args)?;
            SlashCommand::Upgrade
        }
        "stats" => {
            validate_no_args(command, &args)?;
            SlashCommand::Stats
        }
        "share" => {
            validate_no_args(command, &args)?;
            SlashCommand::Share
        }
        "feedback" => {
            validate_no_args(command, &args)?;
            SlashCommand::Feedback
        }
        "files" => {
            validate_no_args(command, &args)?;
            SlashCommand::Files
        }
        "fast" => {
            validate_no_args(command, &args)?;
            SlashCommand::Fast
        }
        "exit" => {
            validate_no_args(command, &args)?;
            SlashCommand::Exit
        }
        "summary" => {
            validate_no_args(command, &args)?;
            SlashCommand::Summary
        }
        "desktop" => {
            validate_no_args(command, &args)?;
            SlashCommand::Desktop
        }
        "brief" => {
            validate_no_args(command, &args)?;
            SlashCommand::Brief
        }
        "advisor" => {
            validate_no_args(command, &args)?;
            SlashCommand::Advisor
        }
        "stickers" => {
            validate_no_args(command, &args)?;
            SlashCommand::Stickers
        }
        "insights" => {
            validate_no_args(command, &args)?;
            SlashCommand::Insights
        }
        "thinkback" => {
            validate_no_args(command, &args)?;
            SlashCommand::Thinkback
        }
        "release-notes" => {
            validate_no_args(command, &args)?;
            SlashCommand::ReleaseNotes
        }
        "security-review" => {
            validate_no_args(command, &args)?;
            SlashCommand::SecurityReview
        }
        "keybindings" => {
            validate_no_args(command, &args)?;
            SlashCommand::Keybindings
        }
        "privacy-settings" => {
            validate_no_args(command, &args)?;
            SlashCommand::PrivacySettings
        }
        "plan" => SlashCommand::Plan { mode: remainder },
        "review" => SlashCommand::Review { scope: remainder },
        "tasks" => SlashCommand::Tasks { args: remainder },
        "theme" => SlashCommand::Theme { name: remainder },
        "voice" => SlashCommand::Voice { mode: remainder },
        "usage" => SlashCommand::Usage { scope: remainder },
        "rename" => SlashCommand::Rename { name: remainder },
        "copy" => SlashCommand::Copy { target: remainder },
        "hooks" => SlashCommand::Hooks { args: remainder },
        "context" => SlashCommand::Context { action: remainder },
        "color" => SlashCommand::Color { scheme: remainder },
        "effort" => SlashCommand::Effort { level: remainder },
        "branch" => SlashCommand::Branch { name: remainder },
        "rewind" => SlashCommand::Rewind { steps: remainder },
        "ide" => SlashCommand::Ide { target: remainder },
        "tag" => SlashCommand::Tag { label: remainder },
        "output-style" => SlashCommand::OutputStyle { style: remainder },
        "add-dir" => SlashCommand::AddDir { path: remainder },
        "history" => SlashCommand::History {
            count: optional_single_arg(command, &args, "[count]")?,
        },
        other => SlashCommand::Unknown(other.to_string()),
    }))
}
fn validate_no_args(command: &str, args: &[&str]) -> Result<(), SlashCommandParseError> {
    if args.is_empty() {
        return Ok(());
    }

    Err(command_error(
        &format!("Unexpected arguments for /{command}."),
        command,
        &format!("/{command}"),
    ))
}

fn optional_single_arg(
    command: &str,
    args: &[&str],
    argument_hint: &str,
) -> Result<Option<String>, SlashCommandParseError> {
    match args {
        [] => Ok(None),
        [value] => Ok(Some((*value).to_string())),
        _ => Err(usage_error(command, argument_hint)),
    }
}

fn require_remainder(
    command: &str,
    remainder: Option<String>,
    argument_hint: &str,
) -> Result<String, SlashCommandParseError> {
    remainder.ok_or_else(|| usage_error(command, argument_hint))
}

fn parse_permissions_mode(args: &[&str]) -> Result<Option<String>, SlashCommandParseError> {
    let mode = optional_single_arg(
        "permissions",
        args,
        "[read-only|workspace-write|danger-full-access]",
    )?;
    if let Some(mode) = mode {
        if matches!(
            mode.as_str(),
            "read-only" | "workspace-write" | "danger-full-access"
        ) {
            return Ok(Some(mode));
        }
        return Err(command_error(
            &format!(
                "Unsupported /permissions mode '{mode}'. Use read-only, workspace-write, or danger-full-access."
            ),
            "permissions",
            "/permissions [read-only|workspace-write|danger-full-access]",
        ));
    }

    Ok(None)
}

fn parse_clear_args(args: &[&str]) -> Result<bool, SlashCommandParseError> {
    match args {
        [] => Ok(false),
        ["--confirm"] => Ok(true),
        [unexpected] => Err(command_error(
            &format!("Unsupported /clear argument '{unexpected}'. Use /clear or /clear --confirm."),
            "clear",
            "/clear [--confirm]",
        )),
        _ => Err(usage_error("clear", "[--confirm]")),
    }
}

fn parse_config_section(args: &[&str]) -> Result<Option<String>, SlashCommandParseError> {
    let section = optional_single_arg("config", args, "[env|hooks|model|plugins]")?;
    if let Some(section) = section {
        if matches!(section.as_str(), "env" | "hooks" | "model" | "plugins") {
            return Ok(Some(section));
        }
        return Err(command_error(
            &format!("Unsupported /config section '{section}'. Use env, hooks, model, or plugins."),
            "config",
            "/config [env|hooks|model|plugins]",
        ));
    }

    Ok(None)
}

fn parse_session_command(args: &[&str]) -> Result<SlashCommand, SlashCommandParseError> {
    match args {
        [] => Ok(SlashCommand::Session {
            action: None,
            target: None,
        }),
        ["list"] => Ok(SlashCommand::Session {
            action: Some("list".to_string()),
            target: None,
        }),
        ["list", ..] => Err(usage_error("session", "[list|switch <session-id>|fork [branch-name]|delete <session-id> [--force]]")),
        ["switch"] => Err(usage_error("session switch", "<session-id>")),
        ["switch", target] => Ok(SlashCommand::Session {
            action: Some("switch".to_string()),
            target: Some((*target).to_string()),
        }),
        ["switch", ..] => Err(command_error(
            "Unexpected arguments for /session switch.",
            "session",
            "/session switch <session-id>",
        )),
        ["fork"] => Ok(SlashCommand::Session {
            action: Some("fork".to_string()),
            target: None,
        }),
        ["fork", target] => Ok(SlashCommand::Session {
            action: Some("fork".to_string()),
            target: Some((*target).to_string()),
        }),
        ["fork", ..] => Err(command_error(
            "Unexpected arguments for /session fork.",
            "session",
            "/session fork [branch-name]",
        )),
        ["delete"] => Err(usage_error("session delete", "<session-id> [--force]")),
        ["delete", target] => Ok(SlashCommand::Session {
            action: Some("delete".to_string()),
            target: Some((*target).to_string()),
        }),
        ["delete", target, "--force"] => Ok(SlashCommand::Session {
            action: Some("delete-force".to_string()),
            target: Some((*target).to_string()),
        }),
        ["delete", _target, unexpected] => Err(command_error(
            &format!(
                "Unsupported /session delete flag '{unexpected}'. Use --force to skip confirmation."
            ),
            "session",
            "/session delete <session-id> [--force]",
        )),
        ["delete", ..] => Err(command_error(
            "Unexpected arguments for /session delete.",
            "session",
            "/session delete <session-id> [--force]",
        )),
        [action, ..] => Err(command_error(
            &format!(
                "Unknown /session action '{action}'. Use list, switch <session-id>, fork [branch-name], or delete <session-id> [--force]."
            ),
            "session",
            "/session [list|switch <session-id>|fork [branch-name]|delete <session-id> [--force]]",
        )),
    }
}

fn parse_principal_command(args: &[&str]) -> Result<SlashCommand, SlashCommandParseError> {
    match args {
        [] | ["list"] => Ok(SlashCommand::Principal {
            action: Some("list".to_string()),
            target: None,
            description: None,
        }),
        ["list", ..] => Err(usage_error(
            "principal",
            "[list|create <principal-id> [description]]",
        )),
        ["create"] => Err(usage_error(
            "principal create",
            "<principal-id> [description]",
        )),
        ["create", target] => Ok(SlashCommand::Principal {
            action: Some("create".to_string()),
            target: Some((*target).to_string()),
            description: None,
        }),
        ["create", target, description @ ..] => Ok(SlashCommand::Principal {
            action: Some("create".to_string()),
            target: Some((*target).to_string()),
            description: Some(description.join(" ")),
        }),
        [action, ..] => Err(command_error(
            &format!(
                "Unknown /principal action '{action}'. Use list or create <principal-id> [description]."
            ),
            "principal",
            "/principal [list|create <principal-id> [description]]",
        )),
    }
}

fn parse_mcp_command(args: &[&str]) -> Result<SlashCommand, SlashCommandParseError> {
    match args {
        [] => Ok(SlashCommand::Mcp {
            action: None,
            target: None,
        }),
        ["list"] => Ok(SlashCommand::Mcp {
            action: Some("list".to_string()),
            target: None,
        }),
        ["list", ..] => Err(usage_error("mcp list", "")),
        ["show"] => Err(usage_error("mcp show", "<server>")),
        ["show", target] => Ok(SlashCommand::Mcp {
            action: Some("show".to_string()),
            target: Some((*target).to_string()),
        }),
        ["show", ..] => Err(command_error(
            "Unexpected arguments for /mcp show.",
            "mcp",
            "/mcp show <server>",
        )),
        ["help" | "-h" | "--help"] => Ok(SlashCommand::Mcp {
            action: Some("help".to_string()),
            target: None,
        }),
        [action, ..] => Err(command_error(
            &format!("Unknown /mcp action '{action}'. Use list, show <server>, or help."),
            "mcp",
            "/mcp [list|show <server>|help]",
        )),
    }
}

fn parse_plugin_command(args: &[&str]) -> Result<SlashCommand, SlashCommandParseError> {
    match args {
        [] => Ok(SlashCommand::Plugins {
            action: None,
            target: None,
        }),
        ["list"] => Ok(SlashCommand::Plugins {
            action: Some("list".to_string()),
            target: None,
        }),
        ["list", ..] => Err(usage_error("plugin list", "")),
        ["install"] => Err(usage_error("plugin install", "<path>")),
        ["install", target @ ..] => Ok(SlashCommand::Plugins {
            action: Some("install".to_string()),
            target: Some(target.join(" ")),
        }),
        ["enable"] => Err(usage_error("plugin enable", "<name>")),
        ["enable", target] => Ok(SlashCommand::Plugins {
            action: Some("enable".to_string()),
            target: Some((*target).to_string()),
        }),
        ["enable", ..] => Err(command_error(
            "Unexpected arguments for /plugin enable.",
            "plugin",
            "/plugin enable <name>",
        )),
        ["disable"] => Err(usage_error("plugin disable", "<name>")),
        ["disable", target] => Ok(SlashCommand::Plugins {
            action: Some("disable".to_string()),
            target: Some((*target).to_string()),
        }),
        ["disable", ..] => Err(command_error(
            "Unexpected arguments for /plugin disable.",
            "plugin",
            "/plugin disable <name>",
        )),
        ["uninstall"] => Err(usage_error("plugin uninstall", "<id>")),
        ["uninstall", target] => Ok(SlashCommand::Plugins {
            action: Some("uninstall".to_string()),
            target: Some((*target).to_string()),
        }),
        ["uninstall", ..] => Err(command_error(
            "Unexpected arguments for /plugin uninstall.",
            "plugin",
            "/plugin uninstall <id>",
        )),
        ["update"] => Err(usage_error("plugin update", "<id>")),
        ["update", target] => Ok(SlashCommand::Plugins {
            action: Some("update".to_string()),
            target: Some((*target).to_string()),
        }),
        ["update", ..] => Err(command_error(
            "Unexpected arguments for /plugin update.",
            "plugin",
            "/plugin update <id>",
        )),
        [action, ..] => Err(command_error(
            &format!(
                "Unknown /plugin action '{action}'. Use list, install <path>, enable <name>, disable <name>, uninstall <id>, or update <id>."
            ),
            "plugin",
            "/plugin [list|install <path>|enable <name>|disable <name>|uninstall <id>|update <id>]",
        )),
    }
}

fn parse_list_or_help_args(
    command: &str,
    args: Option<String>,
) -> Result<Option<String>, SlashCommandParseError> {
    match normalize_optional_args(args.as_deref()) {
        None | Some("list" | "help" | "-h" | "--help") => Ok(args),
        Some(unexpected) => Err(command_error(
            &format!(
                "Unexpected arguments for /{command}: {unexpected}. Use /{command}, /{command} list, or /{command} help."
            ),
            command,
            &format!("/{command} [list|help]"),
        )),
    }
}

fn parse_skills_args(args: Option<&str>) -> Result<Option<String>, SlashCommandParseError> {
    let Some(args) = normalize_optional_args(args) else {
        return Ok(None);
    };

    if matches!(args, "list" | "help" | "-h" | "--help") {
        return Ok(Some(args.to_string()));
    }

    if args == "install" {
        return Err(command_error(
            "Usage: /skills install <path>",
            "skills",
            "/skills install <path>",
        ));
    }

    if let Some(target) = args.strip_prefix("install").map(str::trim) {
        if !target.is_empty() {
            return Ok(Some(format!("install {target}")));
        }
    }

    Ok(Some(args.to_string()))
}

fn usage_error(command: &str, argument_hint: &str) -> SlashCommandParseError {
    let usage = format!("/{command} {argument_hint}");
    let usage = usage.trim_end().to_string();
    command_error(
        &format!("Usage: {usage}"),
        command_root_name(command),
        &usage,
    )
}

fn command_error(message: &str, command: &str, usage: &str) -> SlashCommandParseError {
    let detail = render_slash_command_help_detail(command)
        .map(|detail| format!("\n\n{detail}"))
        .unwrap_or_default();
    SlashCommandParseError::new(format!("{message}\n  Usage            {usage}{detail}"))
}

fn remainder_after_command(input: &str, command: &str) -> Option<String> {
    input
        .trim()
        .strip_prefix(&format!("/{command}"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn find_slash_command_spec(name: &str) -> Option<&'static SlashCommandSpec> {
    slash_command_specs().iter().find(|spec| {
        spec.name.eq_ignore_ascii_case(name)
            || spec
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(name))
    })
}

fn command_root_name(command: &str) -> &str {
    command.split_whitespace().next().unwrap_or(command)
}

fn slash_command_usage(spec: &SlashCommandSpec) -> String {
    match spec.argument_hint {
        Some(argument_hint) => format!("/{} {argument_hint}", spec.name),
        None => format!("/{}", spec.name),
    }
}

fn slash_command_detail_lines(spec: &SlashCommandSpec) -> Vec<String> {
    let mut lines = vec![format!("/{}", spec.name)];
    lines.push(format!("  Summary          {}", spec.summary));
    lines.push(format!("  Usage            {}", slash_command_usage(spec)));
    lines.push(format!(
        "  Category         {}",
        slash_command_category(spec.name)
    ));
    if !spec.aliases.is_empty() {
        lines.push(format!(
            "  Aliases          {}",
            spec.aliases
                .iter()
                .map(|alias| format!("/{alias}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if spec.resume_supported {
        lines.push("  Resume           Supported with --resume SESSION.jsonl".to_string());
    }
    lines
}

#[must_use]
pub fn render_slash_command_help_detail(name: &str) -> Option<String> {
    find_slash_command_spec(name).map(|spec| slash_command_detail_lines(spec).join("\n"))
}

#[must_use]
pub fn slash_command_specs() -> &'static [SlashCommandSpec] {
    SLASH_COMMAND_SPECS
}

#[must_use]
pub fn resume_supported_slash_commands() -> Vec<&'static SlashCommandSpec> {
    slash_command_specs()
        .iter()
        .filter(|spec| spec.resume_supported)
        .collect()
}

fn slash_command_category(name: &str) -> &'static str {
    match name {
        "help" | "status" | "cost" | "resume" | "session" | "version" | "login" | "logout"
        | "usage" | "stats" | "rename" | "clear" | "compact" | "history" | "tokens" | "cache"
        | "exit" | "summary" | "tag" | "thinkback" | "copy" | "share" | "feedback" | "rewind"
        | "pin" | "unpin" | "bookmarks" | "context" | "files" | "focus" | "unfocus" | "retry"
        | "stop" | "undo" => "Session",
        "model" | "permissions" | "config" | "memory" | "theme" | "vim" | "voice" | "color"
        | "effort" | "fast" | "brief" | "output-style" | "keybindings" | "privacy-settings"
        | "stickers" | "language" | "profile" | "max-tokens" | "temperature" | "system-prompt"
        | "api-key" | "terminal-setup" | "notifications" | "telemetry" | "providers" | "env"
        | "project" | "reasoning" | "budget" | "rate-limit" | "workspace" | "reset" | "ide"
        | "desktop" | "upgrade" => "Config",
        "debug-tool-call" | "doctor" | "sandbox" | "diagnostics" | "tool-details" | "changelog"
        | "metrics" => "Debug",
        _ => "Tools",
    }
}

fn format_slash_command_help_line(spec: &SlashCommandSpec) -> String {
    let name = slash_command_usage(spec);
    let alias_suffix = if spec.aliases.is_empty() {
        String::new()
    } else {
        format!(
            " (aliases: {})",
            spec.aliases
                .iter()
                .map(|alias| format!("/{alias}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let resume = if spec.resume_supported {
        " [resume]"
    } else {
        ""
    };
    format!("  {name:<66} {}{alias_suffix}{resume}", spec.summary)
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    if left == right {
        return 0;
    }
    if left.is_empty() {
        return right.chars().count();
    }
    if right.is_empty() {
        return left.chars().count();
    }

    let right_chars = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut current = vec![0; right_chars.len() + 1];

    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let substitution_cost = usize::from(left_char != *right_char);
            current[right_index + 1] = (current[right_index] + 1)
                .min(previous[right_index + 1] + 1)
                .min(previous[right_index] + substitution_cost);
        }
        previous.clone_from(&current);
    }

    previous[right_chars.len()]
}

#[must_use]
pub fn suggest_slash_commands(input: &str, limit: usize) -> Vec<String> {
    let query = input.trim().trim_start_matches('/').to_ascii_lowercase();
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }

    let mut suggestions = slash_command_specs()
        .iter()
        .filter_map(|spec| {
            let best = std::iter::once(spec.name)
                .chain(spec.aliases.iter().copied())
                .map(str::to_ascii_lowercase)
                .map(|candidate| {
                    let prefix_rank =
                        if candidate.starts_with(&query) || query.starts_with(&candidate) {
                            0
                        } else if candidate.contains(&query) || query.contains(&candidate) {
                            1
                        } else {
                            2
                        };
                    let distance = levenshtein_distance(&candidate, &query);
                    (prefix_rank, distance)
                })
                .min();

            best.and_then(|(prefix_rank, distance)| {
                if prefix_rank <= 1 || distance <= 2 {
                    Some((prefix_rank, distance, spec.name.len(), spec.name))
                } else {
                    None
                }
            })
        })
        .collect::<Vec<_>>();

    suggestions.sort_unstable();
    suggestions
        .into_iter()
        .map(|(_, _, _, name)| format!("/{name}"))
        .take(limit)
        .collect()
}

#[must_use]
pub fn render_slash_command_help() -> String {
    let mut lines = vec![
        "Slash commands".to_string(),
        "  Start here        /status, /diff, /agents, /skills, /commit".to_string(),
        "  [resume]          also works with --resume SESSION.jsonl".to_string(),
        String::new(),
    ];

    let categories = ["Session", "Tools", "Config", "Debug"];

    for category in categories {
        lines.push(category.to_string());
        for spec in slash_command_specs()
            .iter()
            .filter(|spec| slash_command_category(spec.name) == category)
        {
            lines.push(format_slash_command_help_line(spec));
        }
        lines.push(String::new());
    }

    lines.push("Keyboard shortcuts".to_string());
    lines.push("  Up/Down              Navigate prompt history".to_string());
    lines.push("  Tab                  Complete commands, modes, and recent sessions".to_string());
    lines.push("  Ctrl-C               Clear input (or exit on empty prompt)".to_string());
    lines.push("  Shift+Enter/Ctrl+J   Insert a newline".to_string());

    lines
        .into_iter()
        .rev()
        .skip_while(String::is_empty)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommandResult {
    pub message: String,
    pub session: Session,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginsCommandResult {
    pub message: String,
    pub reload_runtime: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DefinitionSource {
    Bundled,
    ProjectClaw,
    ProjectCodex,
    ProjectClaude,
    UserClawConfigHome,
    UserCodexHome,
    UserClaw,
    UserCodex,
    UserClaude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DefinitionScope {
    Bundled,
    Project,
    UserConfigHome,
    UserHome,
}

impl DefinitionScope {
    fn label(self) -> &'static str {
        match self {
            Self::Bundled => "Bundled",
            Self::Project => "Project (.claw)",
            Self::UserConfigHome => "User ($CLAW_CONFIG_HOME)",
            Self::UserHome => "User (~/.claw)",
        }
    }
}

impl DefinitionSource {
    fn report_scope(self) -> DefinitionScope {
        match self {
            Self::Bundled => DefinitionScope::Bundled,
            Self::ProjectClaw | Self::ProjectCodex | Self::ProjectClaude => {
                DefinitionScope::Project
            }
            Self::UserClawConfigHome | Self::UserCodexHome => DefinitionScope::UserConfigHome,
            Self::UserClaw | Self::UserCodex | Self::UserClaude => DefinitionScope::UserHome,
        }
    }

    fn label(self) -> &'static str {
        self.report_scope().label()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentSummary {
    name: String,
    description: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
    source: DefinitionSource,
    shadowed_by: Option<DefinitionSource>,
}

#[derive(Debug, Clone, PartialEq)]
struct SkillSummary {
    name: String,
    description: Option<String>,
    location: SkillLocation,
    document: SkillDocument,
    source: DefinitionSource,
    shadowed_by: Option<DefinitionSource>,
    origin: SkillOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SkillLocation {
    Filesystem(PathBuf),
    Bundled(BundledSkillId),
    Generated(GeneratedSkill),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedSkill {
    id: String,
    base_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillOrigin {
    Bundled,
    SkillsDir,
    LegacyCommandsDir,
}

impl SkillOrigin {
    fn detail_label(self) -> Option<&'static str> {
        match self {
            Self::Bundled => Some("bundled"),
            Self::SkillsDir => None,
            Self::LegacyCommandsDir => Some("legacy /commands"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillRoot {
    source: DefinitionSource,
    path: PathBuf,
    origin: SkillOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstalledSkill {
    invocation_name: String,
    display_name: Option<String>,
    source: PathBuf,
    registry_root: PathBuf,
    installed_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SkillInstallSource {
    Directory { root: PathBuf, prompt_path: PathBuf },
    MarkdownFile { path: PathBuf },
}

#[allow(clippy::too_many_lines)]
pub fn handle_plugins_slash_command(
    action: Option<&str>,
    target: Option<&str>,
    manager: &mut PluginManager,
) -> Result<PluginsCommandResult, PluginError> {
    match action {
        None | Some("list") => Ok(PluginsCommandResult {
            message: render_plugins_report(&manager.list_installed_plugins()?),
            reload_runtime: false,
        }),
        Some("install") => {
            let Some(target) = target else {
                return Ok(PluginsCommandResult {
                    message: "Usage: /plugins install <path>".to_string(),
                    reload_runtime: false,
                });
            };
            let install = manager.install(target)?;
            let plugin = manager
                .list_installed_plugins()?
                .into_iter()
                .find(|plugin| plugin.metadata.id == install.plugin_id);
            Ok(PluginsCommandResult {
                message: render_plugin_install_report(&install.plugin_id, plugin.as_ref()),
                reload_runtime: true,
            })
        }
        Some("enable") => {
            let Some(target) = target else {
                return Ok(PluginsCommandResult {
                    message: "Usage: /plugins enable <name>".to_string(),
                    reload_runtime: false,
                });
            };
            let plugin = resolve_plugin_target(manager, target)?;
            manager.enable(&plugin.metadata.id)?;
            Ok(PluginsCommandResult {
                message: format!(
                    "Plugins\n  Result           enabled {}\n  Name             {}\n  Version          {}\n  Status           enabled",
                    plugin.metadata.id, plugin.metadata.name, plugin.metadata.version
                ),
                reload_runtime: true,
            })
        }
        Some("disable") => {
            let Some(target) = target else {
                return Ok(PluginsCommandResult {
                    message: "Usage: /plugins disable <name>".to_string(),
                    reload_runtime: false,
                });
            };
            let plugin = resolve_plugin_target(manager, target)?;
            manager.disable(&plugin.metadata.id)?;
            Ok(PluginsCommandResult {
                message: format!(
                    "Plugins\n  Result           disabled {}\n  Name             {}\n  Version          {}\n  Status           disabled",
                    plugin.metadata.id, plugin.metadata.name, plugin.metadata.version
                ),
                reload_runtime: true,
            })
        }
        Some("uninstall") => {
            let Some(target) = target else {
                return Ok(PluginsCommandResult {
                    message: "Usage: /plugins uninstall <plugin-id>".to_string(),
                    reload_runtime: false,
                });
            };
            manager.uninstall(target)?;
            Ok(PluginsCommandResult {
                message: format!("Plugins\n  Result           uninstalled {target}"),
                reload_runtime: true,
            })
        }
        Some("update") => {
            let Some(target) = target else {
                return Ok(PluginsCommandResult {
                    message: "Usage: /plugins update <plugin-id>".to_string(),
                    reload_runtime: false,
                });
            };
            let update = manager.update(target)?;
            let plugin = manager
                .list_installed_plugins()?
                .into_iter()
                .find(|plugin| plugin.metadata.id == update.plugin_id);
            Ok(PluginsCommandResult {
                message: format!(
                    "Plugins\n  Result           updated {}\n  Name             {}\n  Old version      {}\n  New version      {}\n  Status           {}",
                    update.plugin_id,
                    plugin
                        .as_ref()
                        .map_or_else(|| update.plugin_id.clone(), |plugin| plugin.metadata.name.clone()),
                    update.old_version,
                    update.new_version,
                    plugin
                        .as_ref()
                        .map_or("unknown", |plugin| if plugin.enabled { "enabled" } else { "disabled" }),
                ),
                reload_runtime: true,
            })
        }
        Some(other) => Ok(PluginsCommandResult {
            message: format!(
                "Unknown /plugins action '{other}'. Use list, install, enable, disable, uninstall, or update."
            ),
            reload_runtime: false,
        }),
    }
}

pub fn handle_agents_slash_command(args: Option<&str>, cwd: &Path) -> std::io::Result<String> {
    match normalize_optional_args(args) {
        None | Some("list") => {
            let roots = discover_definition_roots(cwd, "agents");
            let agents = load_agents_from_roots(&roots)?;
            Ok(render_agents_report(&agents))
        }
        Some("-h" | "--help" | "help") => Ok(render_agents_usage(None)),
        Some(args) => Ok(render_agents_usage(Some(args))),
    }
}

pub fn handle_agents_slash_command_json(args: Option<&str>, cwd: &Path) -> std::io::Result<Value> {
    if let Some(args) = normalize_optional_args(args) {
        if let Some(help_path) = help_path_from_args(args) {
            return Ok(match help_path.as_slice() {
                [] => render_agents_usage_json(None),
                _ => render_agents_usage_json(Some(&help_path.join(" "))),
            });
        }
    }

    match normalize_optional_args(args) {
        None | Some("list") => {
            let roots = discover_definition_roots(cwd, "agents");
            let agents = load_agents_from_roots(&roots)?;
            Ok(render_agents_report_json(cwd, &agents))
        }
        Some(args) if is_help_arg(args) => Ok(render_agents_usage_json(None)),
        Some(args) => Ok(render_agents_usage_json(Some(args))),
    }
}

pub fn handle_mcp_slash_command(
    args: Option<&str>,
    cwd: &Path,
) -> Result<String, runtime::ConfigError> {
    let loader = ConfigLoader::default_for(cwd);
    render_mcp_report_for(&loader, cwd, args)
}

pub fn handle_mcp_slash_command_json(
    args: Option<&str>,
    cwd: &Path,
) -> Result<Value, runtime::ConfigError> {
    let loader = ConfigLoader::default_for(cwd);
    render_mcp_report_json_for(&loader, cwd, args)
}

pub fn handle_skills_slash_command(args: Option<&str>, cwd: &Path) -> std::io::Result<String> {
    if let Some(args) = normalize_optional_args(args) {
        if let Some(help_path) = help_path_from_args(args) {
            return Ok(match help_path.as_slice() {
                [] => render_skills_usage(None),
                ["install", ..] => render_skills_usage(Some("install")),
                _ => render_skills_usage(Some(&help_path.join(" "))),
            });
        }
    }

    match normalize_optional_args(args) {
        None | Some("list") => {
            let roots = discover_skill_roots(cwd);
            let skills = load_skills_from_roots_for_context(&roots, cwd)?;
            Ok(render_skills_report(&skills))
        }
        Some("install") => Ok(render_skills_usage(Some("install"))),
        Some(args) if args.starts_with("install ") => {
            let target = args["install ".len()..].trim();
            if target.is_empty() {
                return Ok(render_skills_usage(Some("install")));
            }
            let install = install_skill(target, cwd)?;
            Ok(render_skill_install_report(&install))
        }
        Some("-h" | "--help" | "help") => Ok(render_skills_usage(None)),
        Some(args) => Ok(render_skills_usage(Some(args))),
    }
}

pub fn handle_skills_slash_command_json(args: Option<&str>, cwd: &Path) -> std::io::Result<Value> {
    if let Some(args) = normalize_optional_args(args) {
        if let Some(help_path) = help_path_from_args(args) {
            return Ok(match help_path.as_slice() {
                [] => render_skills_usage_json(None),
                ["install", ..] => render_skills_usage_json(Some("install")),
                _ => render_skills_usage_json(Some(&help_path.join(" "))),
            });
        }
    }

    match normalize_optional_args(args) {
        None | Some("list") => {
            let roots = discover_skill_roots(cwd);
            let skills = load_skills_from_roots_for_context(&roots, cwd)?;
            Ok(render_skills_report_json(&skills))
        }
        Some("install") => Ok(render_skills_usage_json(Some("install"))),
        Some(args) if args.starts_with("install ") => {
            let target = args["install ".len()..].trim();
            if target.is_empty() {
                return Ok(render_skills_usage_json(Some("install")));
            }
            let install = install_skill(target, cwd)?;
            Ok(render_skill_install_report_json(&install))
        }
        Some(args) if is_help_arg(args) => Ok(render_skills_usage_json(None)),
        Some(args) => Ok(render_skills_usage_json(Some(args))),
    }
}

#[must_use]
pub fn classify_skills_slash_command(args: Option<&str>) -> SkillSlashDispatch {
    match normalize_optional_args(args) {
        None | Some("list" | "help" | "-h" | "--help") => SkillSlashDispatch::Local,
        Some(args) if args == "install" || args.starts_with("install ") => {
            SkillSlashDispatch::Local
        }
        Some(args) => SkillSlashDispatch::Invoke(format!("${}", args.trim_start_matches('/'))),
    }
}

/// Resolve a skill invocation by validating the skill exists on disk before
/// returning the dispatch.  When the skill is not found, returns `Err` with a
/// human-readable message that lists nearby skill names.
pub fn resolve_skill_invocation(
    cwd: &Path,
    args: Option<&str>,
) -> Result<SkillSlashDispatch, String> {
    let dispatch = classify_skills_slash_command(args);
    if let SkillSlashDispatch::Invoke(ref prompt) = dispatch {
        // Extract the skill name from the "$skill [args]" prompt.
        let skill_token = prompt
            .trim_start_matches('$')
            .split_whitespace()
            .next()
            .unwrap_or_default();
        if !skill_token.is_empty() {
            if let Err(error) = resolve_skill(cwd, skill_token) {
                let mut message = format!("Unknown skill: {skill_token} ({error})");
                let roots = discover_skill_roots(cwd);
                if let Ok(available) = load_skills_from_roots_for_context(&roots, cwd) {
                    let names: Vec<String> = available
                        .iter()
                        .filter(|s| s.shadowed_by.is_none())
                        .map(|s| s.name.clone())
                        .collect();
                    if !names.is_empty() {
                        let _ = write!(message, "\n  Available skills: {}", names.join(", "));
                    }
                }
                message.push_str("\n  Usage: /skills [list|install <path>|help|<skill> [args]]");
                return Err(message);
            }
        }
    }
    Ok(dispatch)
}

fn requested_skill_name(skill: &str) -> std::io::Result<String> {
    let requested = skill
        .trim()
        .trim_start_matches('/')
        .trim_start_matches('$')
        .to_string();
    if requested.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "skill must not be empty",
        ));
    }
    Ok(requested)
}

pub fn resolve_skill(cwd: &Path, skill: &str) -> std::io::Result<ResolvedSkill> {
    let requested = requested_skill_name(skill)?;
    let roots = discover_skill_roots(cwd);

    for skill in load_skills_from_roots_for_context(&roots, cwd)? {
        if skill.name.eq_ignore_ascii_case(requested.as_str())
            || skill
                .document
                .resolved_name
                .eq_ignore_ascii_case(requested.as_str())
        {
            let source = match skill.location {
                SkillLocation::Filesystem(path) => ResolvedSkillSource::Filesystem { path },
                SkillLocation::Bundled(id) => ResolvedSkillSource::Bundled { id },
                SkillLocation::Generated(generated) => ResolvedSkillSource::Generated {
                    id: generated.id,
                    base_dir: generated.base_dir,
                },
            };
            return Ok(ResolvedSkill {
                document: skill.document,
                source,
            });
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("unknown skill: {requested}"),
    ))
}

pub fn resolve_skill_path(cwd: &Path, skill: &str) -> std::io::Result<PathBuf> {
    let requested = requested_skill_name(skill)?;

    let roots = discover_skill_roots(cwd);
    for skill in load_skills_from_roots_for_context(&roots, cwd)? {
        if skill.name.eq_ignore_ascii_case(&requested)
            || skill
                .document
                .resolved_name
                .eq_ignore_ascii_case(&requested)
        {
            return match skill.location {
                SkillLocation::Filesystem(path) => Ok(path),
                SkillLocation::Bundled(_) => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("skill `{requested}` is bundled and has no filesystem path"),
                )),
                SkillLocation::Generated(generated) => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("skill `{}` is generated for the current AppFS app and has no filesystem path", generated.id),
                )),
            };
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("unknown skill: {requested}"),
    ))
}

fn render_mcp_report_for(
    loader: &ConfigLoader,
    cwd: &Path,
    args: Option<&str>,
) -> Result<String, runtime::ConfigError> {
    if let Some(args) = normalize_optional_args(args) {
        if let Some(help_path) = help_path_from_args(args) {
            return Ok(match help_path.as_slice() {
                [] => render_mcp_usage(None),
                ["show", ..] => render_mcp_usage(Some("show")),
                _ => render_mcp_usage(Some(&help_path.join(" "))),
            });
        }
    }

    match normalize_optional_args(args) {
        None | Some("list") => {
            let runtime_config = loader.load()?;
            Ok(render_mcp_summary_report(
                cwd,
                runtime_config.mcp().servers(),
            ))
        }
        Some(args) if is_help_arg(args) => Ok(render_mcp_usage(None)),
        Some("show") => Ok(render_mcp_usage(Some("show"))),
        Some(args) if args.split_whitespace().next() == Some("show") => {
            let mut parts = args.split_whitespace();
            let _ = parts.next();
            let Some(server_name) = parts.next() else {
                return Ok(render_mcp_usage(Some("show")));
            };
            if parts.next().is_some() {
                return Ok(render_mcp_usage(Some(args)));
            }
            let runtime_config = loader.load()?;
            Ok(render_mcp_server_report(
                cwd,
                server_name,
                runtime_config.mcp().get(server_name),
            ))
        }
        Some(args) => Ok(render_mcp_usage(Some(args))),
    }
}

fn render_mcp_report_json_for(
    loader: &ConfigLoader,
    cwd: &Path,
    args: Option<&str>,
) -> Result<Value, runtime::ConfigError> {
    if let Some(args) = normalize_optional_args(args) {
        if let Some(help_path) = help_path_from_args(args) {
            return Ok(match help_path.as_slice() {
                [] => render_mcp_usage_json(None),
                ["show", ..] => render_mcp_usage_json(Some("show")),
                _ => render_mcp_usage_json(Some(&help_path.join(" "))),
            });
        }
    }

    match normalize_optional_args(args) {
        None | Some("list") => {
            let runtime_config = loader.load()?;
            Ok(render_mcp_summary_report_json(
                cwd,
                runtime_config.mcp().servers(),
            ))
        }
        Some(args) if is_help_arg(args) => Ok(render_mcp_usage_json(None)),
        Some("show") => Ok(render_mcp_usage_json(Some("show"))),
        Some(args) if args.split_whitespace().next() == Some("show") => {
            let mut parts = args.split_whitespace();
            let _ = parts.next();
            let Some(server_name) = parts.next() else {
                return Ok(render_mcp_usage_json(Some("show")));
            };
            if parts.next().is_some() {
                return Ok(render_mcp_usage_json(Some(args)));
            }
            let runtime_config = loader.load()?;
            Ok(render_mcp_server_report_json(
                cwd,
                server_name,
                runtime_config.mcp().get(server_name),
            ))
        }
        Some(args) => Ok(render_mcp_usage_json(Some(args))),
    }
}

#[must_use]
pub fn render_plugins_report(plugins: &[PluginSummary]) -> String {
    let mut lines = vec!["Plugins".to_string()];
    if plugins.is_empty() {
        lines.push("  No plugins installed.".to_string());
        return lines.join("\n");
    }
    for plugin in plugins {
        let enabled = if plugin.enabled {
            "enabled"
        } else {
            "disabled"
        };
        lines.push(format!(
            "  {name:<20} v{version:<10} {enabled}",
            name = plugin.metadata.name,
            version = plugin.metadata.version,
        ));
    }
    lines.join("\n")
}

fn render_plugin_install_report(plugin_id: &str, plugin: Option<&PluginSummary>) -> String {
    let name = plugin.map_or(plugin_id, |plugin| plugin.metadata.name.as_str());
    let version = plugin.map_or("unknown", |plugin| plugin.metadata.version.as_str());
    let enabled = plugin.is_some_and(|plugin| plugin.enabled);
    format!(
        "Plugins\n  Result           installed {plugin_id}\n  Name             {name}\n  Version          {version}\n  Status           {}",
        if enabled { "enabled" } else { "disabled" }
    )
}

fn resolve_plugin_target(
    manager: &PluginManager,
    target: &str,
) -> Result<PluginSummary, PluginError> {
    let mut matches = manager
        .list_installed_plugins()?
        .into_iter()
        .filter(|plugin| plugin.metadata.id == target || plugin.metadata.name == target)
        .collect::<Vec<_>>();
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(PluginError::NotFound(format!(
            "plugin `{target}` is not installed or discoverable"
        ))),
        _ => Err(PluginError::InvalidManifest(format!(
            "plugin name `{target}` is ambiguous; use the full plugin id"
        ))),
    }
}

fn discover_definition_roots(cwd: &Path, leaf: &str) -> Vec<(DefinitionSource, PathBuf)> {
    let mut roots = Vec::new();

    for ancestor in cwd.ancestors() {
        push_unique_root(
            &mut roots,
            DefinitionSource::ProjectClaw,
            ancestor.join(".claw").join(leaf),
        );
        push_unique_root(
            &mut roots,
            DefinitionSource::ProjectCodex,
            ancestor.join(".codex").join(leaf),
        );
        push_unique_root(
            &mut roots,
            DefinitionSource::ProjectClaude,
            ancestor.join(".claude").join(leaf),
        );
    }

    if let Ok(claw_config_home) = env::var("CLAW_CONFIG_HOME") {
        push_unique_root(
            &mut roots,
            DefinitionSource::UserClawConfigHome,
            PathBuf::from(claw_config_home).join(leaf),
        );
    }

    if let Ok(codex_home) = env::var("CODEX_HOME") {
        push_unique_root(
            &mut roots,
            DefinitionSource::UserCodexHome,
            PathBuf::from(codex_home).join(leaf),
        );
    }

    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        push_unique_root(
            &mut roots,
            DefinitionSource::UserClaw,
            home.join(".claw").join(leaf),
        );
        push_unique_root(
            &mut roots,
            DefinitionSource::UserCodex,
            home.join(".codex").join(leaf),
        );
        push_unique_root(
            &mut roots,
            DefinitionSource::UserClaude,
            home.join(".claude").join(leaf),
        );
    }

    roots
}

#[allow(clippy::too_many_lines)]
fn discover_skill_roots(cwd: &Path) -> Vec<SkillRoot> {
    let mut roots = Vec::new();

    for ancestor in cwd.ancestors() {
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::ProjectClaw,
            ancestor.join(".claw").join("skills"),
            SkillOrigin::SkillsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::ProjectCodex,
            ancestor.join(".codex").join("skills"),
            SkillOrigin::SkillsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::ProjectClaude,
            ancestor.join(".claude").join("skills"),
            SkillOrigin::SkillsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::ProjectClaw,
            ancestor.join(".claw").join("commands"),
            SkillOrigin::LegacyCommandsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::ProjectCodex,
            ancestor.join(".codex").join("commands"),
            SkillOrigin::LegacyCommandsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::ProjectClaude,
            ancestor.join(".claude").join("commands"),
            SkillOrigin::LegacyCommandsDir,
        );
    }

    if let Ok(claw_config_home) = env::var("CLAW_CONFIG_HOME") {
        let claw_config_home = PathBuf::from(claw_config_home);
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserClawConfigHome,
            claw_config_home.join("skills"),
            SkillOrigin::SkillsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserClawConfigHome,
            claw_config_home.join("commands"),
            SkillOrigin::LegacyCommandsDir,
        );
    }

    if let Ok(codex_home) = env::var("CODEX_HOME") {
        let codex_home = PathBuf::from(codex_home);
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserCodexHome,
            codex_home.join("skills"),
            SkillOrigin::SkillsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserCodexHome,
            codex_home.join("commands"),
            SkillOrigin::LegacyCommandsDir,
        );
    }

    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserClaw,
            home.join(".claw").join("skills"),
            SkillOrigin::SkillsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserClaw,
            home.join(".claw").join("commands"),
            SkillOrigin::LegacyCommandsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserCodex,
            home.join(".codex").join("skills"),
            SkillOrigin::SkillsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserCodex,
            home.join(".codex").join("commands"),
            SkillOrigin::LegacyCommandsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserClaude,
            home.join(".claude").join("skills"),
            SkillOrigin::SkillsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserClaude,
            home.join(".claude").join("commands"),
            SkillOrigin::LegacyCommandsDir,
        );
    }

    roots
}

type ConditionalSkillActivationMap = BTreeMap<PathBuf, BTreeSet<String>>;

fn conditional_skill_activations() -> &'static Mutex<ConditionalSkillActivationMap> {
    static ACTIVATIONS: OnceLock<Mutex<ConditionalSkillActivationMap>> = OnceLock::new();
    ACTIVATIONS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn conditional_skill_context_root(cwd: &Path) -> PathBuf {
    tool_output_root(cwd)
}

fn is_conditional_skill_activated(cwd: &Path, skill_name: &str) -> bool {
    let activations = conditional_skill_activations()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    activations
        .get(&conditional_skill_context_root(cwd))
        .is_some_and(|names| names.contains(skill_name))
}

fn record_conditional_skill_activation(cwd: &Path, skill_name: &str) {
    let mut activations = conditional_skill_activations()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    activations
        .entry(conditional_skill_context_root(cwd))
        .or_default()
        .insert(skill_name.to_string());
}

pub fn activate_conditional_skills_for_paths(
    file_paths: &[PathBuf],
    cwd: &Path,
) -> std::io::Result<Vec<String>> {
    if file_paths.is_empty() {
        return Ok(Vec::new());
    }

    let roots = discover_skill_roots(cwd);
    let skills = load_skills_from_roots(&roots)?;
    let mut activated = Vec::new();

    for skill in skills {
        let Some(patterns) = skill.document.paths.as_deref() else {
            continue;
        };
        if is_conditional_skill_activated(cwd, &skill.document.resolved_name) {
            continue;
        }

        let matches_path = file_paths.iter().any(|path| {
            normalize_relative_match_path(path, cwd)
                .is_some_and(|relative_path| skill_paths_match(patterns, &relative_path))
        });
        if !matches_path {
            continue;
        }

        record_conditional_skill_activation(cwd, &skill.document.resolved_name);
        activated.push(skill.document.resolved_name);
    }

    Ok(activated)
}

fn normalize_relative_match_path(path: &Path, cwd: &Path) -> Option<String> {
    let normalized_path = normalize_conditional_skill_match_path(path);
    let normalized_cwd = normalize_conditional_skill_match_path(cwd);
    let relative = normalized_path.strip_prefix(&normalized_cwd).ok()?;
    let relative = relative.to_string_lossy().replace('\\', "/");
    let relative = relative.trim_start_matches("./").trim_matches('/');
    if relative.is_empty() || relative.starts_with("..") {
        None
    } else {
        Some(relative.to_string())
    }
}

fn normalize_conditional_skill_match_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return clean_conditional_skill_match_path(canonical);
    }

    if let Some(parent) = path.parent() {
        let canonical_parent = parent.canonicalize().map_or_else(
            |_| clean_conditional_skill_match_path(parent.to_path_buf()),
            clean_conditional_skill_match_path,
        );
        if let Some(name) = path.file_name() {
            return clean_conditional_skill_match_path(canonical_parent.join(name));
        }
    }

    clean_conditional_skill_match_path(path.to_path_buf())
}

fn clean_conditional_skill_match_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(stripped) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
    }

    path
}

fn skill_paths_match(patterns: &[String], relative_path: &str) -> bool {
    patterns
        .iter()
        .any(|pattern| skill_path_pattern_matches(pattern, relative_path))
}

fn skill_path_pattern_matches(pattern: &str, relative_path: &str) -> bool {
    let normalized_pattern = pattern.replace('\\', "/").trim_matches('/').to_string();
    if normalized_pattern.is_empty() {
        return false;
    }

    if !pattern_has_glob_metacharacters(&normalized_pattern) {
        return relative_path == normalized_pattern
            || relative_path.starts_with(&format!("{normalized_pattern}/"));
    }

    Pattern::new(&normalized_pattern)
        .ok()
        .is_some_and(|compiled| compiled.matches(relative_path))
}

fn pattern_has_glob_metacharacters(pattern: &str) -> bool {
    pattern
        .chars()
        .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}' | '!'))
}

fn install_skill(source: &str, cwd: &Path) -> std::io::Result<InstalledSkill> {
    let registry_root = default_skill_install_root()?;
    install_skill_into(source, cwd, &registry_root)
}

fn install_skill_into(
    source: &str,
    cwd: &Path,
    registry_root: &Path,
) -> std::io::Result<InstalledSkill> {
    let source = resolve_skill_install_source(source, cwd)?;
    let prompt_path = source.prompt_path();
    let display_name = load_skill_document(prompt_path, "Skill")?.display_name;
    let invocation_name = derive_skill_install_name(&source, display_name.as_deref())?;
    let installed_path = registry_root.join(&invocation_name);

    if installed_path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "skill '{invocation_name}' is already installed at {}",
                installed_path.display()
            ),
        ));
    }

    fs::create_dir_all(&installed_path)?;
    let install_result = match &source {
        SkillInstallSource::Directory { root, .. } => {
            copy_directory_contents(root, &installed_path)
        }
        SkillInstallSource::MarkdownFile { path } => {
            fs::copy(path, installed_path.join("SKILL.md")).map(|_| ())
        }
    };
    if let Err(error) = install_result {
        let _ = fs::remove_dir_all(&installed_path);
        return Err(error);
    }

    Ok(InstalledSkill {
        invocation_name,
        display_name,
        source: source.report_path().to_path_buf(),
        registry_root: registry_root.to_path_buf(),
        installed_path,
    })
}

fn default_skill_install_root() -> std::io::Result<PathBuf> {
    if let Ok(claw_config_home) = env::var("CLAW_CONFIG_HOME") {
        return Ok(PathBuf::from(claw_config_home).join("skills"));
    }
    if let Ok(codex_home) = env::var("CODEX_HOME") {
        return Ok(PathBuf::from(codex_home).join("skills"));
    }
    if let Some(home) = env::var_os("HOME") {
        return Ok(PathBuf::from(home).join(".claw").join("skills"));
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "unable to resolve a skills install root; set CLAW_CONFIG_HOME or HOME",
    ))
}

fn resolve_skill_install_source(source: &str, cwd: &Path) -> std::io::Result<SkillInstallSource> {
    let candidate = PathBuf::from(source);
    let source = if candidate.is_absolute() {
        candidate
    } else {
        cwd.join(candidate)
    };
    let source = fs::canonicalize(&source)?;

    if source.is_dir() {
        let prompt_path = source.join("SKILL.md");
        if !prompt_path.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "skill directory '{}' must contain SKILL.md",
                    source.display()
                ),
            ));
        }
        return Ok(SkillInstallSource::Directory {
            root: source,
            prompt_path,
        });
    }

    if source
        .extension()
        .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("md"))
    {
        return Ok(SkillInstallSource::MarkdownFile { path: source });
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!(
            "skill source '{}' must be a directory with SKILL.md or a markdown file",
            source.display()
        ),
    ))
}

fn derive_skill_install_name(
    source: &SkillInstallSource,
    declared_name: Option<&str>,
) -> std::io::Result<String> {
    for candidate in [declared_name, source.fallback_name().as_deref()] {
        if let Some(candidate) = candidate.and_then(sanitize_skill_invocation_name) {
            return Ok(candidate);
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!(
            "unable to derive an installable invocation name from '{}'",
            source.report_path().display()
        ),
    ))
}

fn sanitize_skill_invocation_name(candidate: &str) -> Option<String> {
    let trimmed = candidate
        .trim()
        .trim_start_matches('/')
        .trim_start_matches('$');
    if trimmed.is_empty() {
        return None;
    }

    let mut sanitized = String::new();
    let mut last_was_separator = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            sanitized.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if (ch.is_whitespace() || matches!(ch, '/' | '\\'))
            && !last_was_separator
            && !sanitized.is_empty()
        {
            sanitized.push('-');
            last_was_separator = true;
        }
    }

    let sanitized = sanitized
        .trim_matches(|ch| matches!(ch, '-' | '_' | '.'))
        .to_string();
    (!sanitized.is_empty()).then_some(sanitized)
}

fn copy_directory_contents(source: &Path, destination: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let entry_type = entry.file_type()?;
        let destination_path = destination.join(entry.file_name());
        if entry_type.is_dir() {
            fs::create_dir_all(&destination_path)?;
            copy_directory_contents(&entry.path(), &destination_path)?;
        } else {
            fs::copy(entry.path(), destination_path)?;
        }
    }
    Ok(())
}

impl SkillInstallSource {
    fn prompt_path(&self) -> &Path {
        match self {
            Self::Directory { prompt_path, .. } => prompt_path,
            Self::MarkdownFile { path } => path,
        }
    }

    fn fallback_name(&self) -> Option<String> {
        match self {
            Self::Directory { root, .. } => root
                .file_name()
                .map(|name| name.to_string_lossy().to_string()),
            Self::MarkdownFile { path } => path
                .file_stem()
                .map(|name| name.to_string_lossy().to_string()),
        }
    }

    fn report_path(&self) -> &Path {
        match self {
            Self::Directory { root, .. } => root,
            Self::MarkdownFile { path } => path,
        }
    }
}

fn push_unique_root(
    roots: &mut Vec<(DefinitionSource, PathBuf)>,
    source: DefinitionSource,
    path: PathBuf,
) {
    if path.is_dir() && !roots.iter().any(|(_, existing)| existing == &path) {
        roots.push((source, path));
    }
}

fn push_unique_skill_root(
    roots: &mut Vec<SkillRoot>,
    source: DefinitionSource,
    path: PathBuf,
    origin: SkillOrigin,
) {
    if path.is_dir() && !roots.iter().any(|existing| existing.path == path) {
        roots.push(SkillRoot {
            source,
            path,
            origin,
        });
    }
}

fn load_agents_from_roots(
    roots: &[(DefinitionSource, PathBuf)],
) -> std::io::Result<Vec<AgentSummary>> {
    let mut agents = Vec::new();
    let mut active_sources = BTreeMap::<String, DefinitionSource>::new();

    for (source, root) in roots {
        let mut root_agents = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if entry.path().extension().is_none_or(|ext| ext != "toml") {
                continue;
            }
            let contents = fs::read_to_string(entry.path())?;
            let fallback_name = entry.path().file_stem().map_or_else(
                || entry.file_name().to_string_lossy().to_string(),
                |stem| stem.to_string_lossy().to_string(),
            );
            root_agents.push(AgentSummary {
                name: parse_toml_string(&contents, "name").unwrap_or(fallback_name),
                description: parse_toml_string(&contents, "description"),
                model: parse_toml_string(&contents, "model"),
                reasoning_effort: parse_toml_string(&contents, "model_reasoning_effort"),
                source: *source,
                shadowed_by: None,
            });
        }
        root_agents.sort_by(|left, right| left.name.cmp(&right.name));

        for mut agent in root_agents {
            let key = agent.name.to_ascii_lowercase();
            if let Some(existing) = active_sources.get(&key) {
                agent.shadowed_by = Some(*existing);
            } else {
                active_sources.insert(key, agent.source);
            }
            agents.push(agent);
        }
    }

    Ok(agents)
}

fn load_skills_from_roots(roots: &[SkillRoot]) -> std::io::Result<Vec<SkillSummary>> {
    load_skills_from_roots_impl(roots, None)
}

fn load_skills_from_roots_for_context(
    roots: &[SkillRoot],
    cwd: &Path,
) -> std::io::Result<Vec<SkillSummary>> {
    load_skills_from_roots_impl(roots, Some(cwd))
}

fn load_skills_from_roots_impl(
    roots: &[SkillRoot],
    activation_cwd: Option<&Path>,
) -> std::io::Result<Vec<SkillSummary>> {
    let mut skills = Vec::new();
    let mut active_sources = BTreeMap::<String, DefinitionSource>::new();

    merge_skill_summaries(&mut skills, &mut active_sources, bundled_skill_summaries());

    if let Some(cwd) = activation_cwd {
        merge_skill_summaries(
            &mut skills,
            &mut active_sources,
            generated_appfs_skill_summaries(cwd),
        );
    }

    for root in roots {
        merge_skill_summaries(
            &mut skills,
            &mut active_sources,
            load_skills_from_root(root, activation_cwd)?,
        );
    }

    Ok(skills)
}

fn bundled_skill_summaries() -> Vec<SkillSummary> {
    let mut bundled_skills = bundled_skills::bundled_skill_inventory()
        .into_iter()
        .map(|skill| SkillSummary {
            name: skill.document.user_facing_name().to_string(),
            description: Some(skill.document.description.clone()),
            location: SkillLocation::Bundled(skill.id),
            document: skill.document,
            source: DefinitionSource::Bundled,
            shadowed_by: None,
            origin: SkillOrigin::Bundled,
        })
        .collect::<Vec<_>>();
    bundled_skills.sort_by(|left, right| left.name.cmp(&right.name));
    bundled_skills
}

fn generated_appfs_skill_summaries(cwd: &Path) -> Vec<SkillSummary> {
    let Some(environment) = detect_appfs_environment(cwd) else {
        return Vec::new();
    };

    let mut candidates = BTreeMap::<String, PathBuf>::new();
    for app in &environment.registered_apps {
        candidates.entry(app.app_id.clone()).or_insert_with(|| {
            let trimmed = app.path.trim().trim_start_matches(['/', '\\']);
            if trimmed.is_empty() {
                environment.mount_root.clone()
            } else {
                environment.mount_root.join(trimmed)
            }
        });
    }

    if let (Some(app_id), Some(app_root)) = (
        environment.current_app_id.as_ref(),
        environment.current_app_root.as_ref(),
    ) {
        candidates
            .entry(app_id.clone())
            .or_insert_with(|| app_root.clone());
    }

    if candidates.is_empty() {
        for child in discover_app_roots_under_mount(&environment.mount_root) {
            if let Some(app_id) = child
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .filter(|name| !name.is_empty())
            {
                candidates.entry(app_id).or_insert(child);
            }
        }
    }

    let mut summaries = candidates
        .into_iter()
        .filter_map(|(app_id, app_root)| synthesize_appfs_skill_for_app(&app_id, &app_root))
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| left.name.cmp(&right.name));
    summaries
}

fn discover_app_roots_under_mount(mount_root: &Path) -> Vec<PathBuf> {
    fs::read_dir(mount_root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let file_type = entry.file_type().ok()?;
            if !file_type.is_dir() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if matches!(name.as_str(), "_appfs" | ".well-known") {
                return None;
            }
            let looks_like_app = path.join("_app").is_dir() || path.join("_stream").is_dir();
            looks_like_app.then_some(path)
        })
        .collect()
}

fn synthesize_appfs_skill_for_app(fallback_app_id: &str, app_root: &Path) -> Option<SkillSummary> {
    let skill_doc = read_json_value(&app_root.join("_app").join("skill.res.json"));
    let control_doc = read_json_value(&app_root.join("_app").join("control.res.json"));
    let actions_doc = read_json_value(&app_root.join("_app").join("actions.res.json"));
    let current_scope_doc = read_json_value(&app_root.join("_app").join("current_scope.res.json"));
    let available_scopes_doc =
        read_json_value(&app_root.join("_app").join("available_scopes.res.json"));

    let app_id = skill_doc
        .as_ref()
        .and_then(|doc| doc.get("app_id").and_then(Value::as_str))
        .or_else(|| {
            actions_doc
                .as_ref()
                .and_then(|doc| doc.get("app_id").and_then(Value::as_str))
        })
        .or_else(|| {
            control_doc
                .as_ref()
                .and_then(|doc| doc.get("app_id").and_then(Value::as_str))
        })
        .unwrap_or(&fallback_app_id)
        .to_string();

    if skill_doc.is_none()
        && control_doc.is_none()
        && actions_doc.is_none()
        && current_scope_doc.is_none()
        && available_scopes_doc.is_none()
    {
        return None;
    }

    let skill_name = format!("appfs-{app_id}");
    let description = appfs_skill_override_description(skill_doc.as_ref()).unwrap_or_else(|| {
        format!("Operate the current {app_id} AppFS app through append-only action files and event streams.")
    });
    let when_to_use = appfs_skill_override_when_to_use(skill_doc.as_ref()).unwrap_or_else(|| {
        build_appfs_skill_when_to_use(&app_id, control_doc.as_ref(), actions_doc.as_ref())
    });
    let markdown_content = build_appfs_skill_markdown(
        &app_id,
        app_root,
        skill_doc.as_ref(),
        control_doc.as_ref(),
        actions_doc.as_ref(),
        current_scope_doc.as_ref(),
        available_scopes_doc.as_ref(),
    );
    let allowed_tools = appfs_skill_allowed_tools(skill_doc.as_ref())
        .unwrap_or_else(|| "bash, read_file, glob_search".to_string());
    let contents = format!(
        "---\nname: {}\ndescription: {}\nwhen_to_use: {}\nallowed-tools: {}\n---\n\n{markdown_content}\n",
        yaml_quote_scalar(&skill_name),
        yaml_quote_scalar(&description),
        yaml_quote_scalar(&when_to_use),
        yaml_quote_scalar(&allowed_tools),
    );
    let document = parse_skill_document(&contents, skill_name.clone(), "Skill");

    Some(SkillSummary {
        name: document.user_facing_name().to_string(),
        description: Some(document.description.clone()),
        location: SkillLocation::Generated(GeneratedSkill {
            id: skill_name.clone(),
            base_dir: Some(app_root.to_path_buf()),
        }),
        document,
        source: DefinitionSource::ProjectClaw,
        shadowed_by: None,
        origin: SkillOrigin::SkillsDir,
    })
}

fn appfs_skill_override_description(skill_doc: Option<&Value>) -> Option<String> {
    skill_doc
        .and_then(|doc| doc.get("description"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(ToOwned::to_owned)
}

fn appfs_skill_override_when_to_use(skill_doc: Option<&Value>) -> Option<String> {
    let doc = skill_doc?;
    if let Some(text) = doc
        .get("when_to_use")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_string());
    }

    let clauses = doc
        .get("when_to_use")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    if clauses.is_empty() {
        None
    } else {
        Some(clauses.join(" "))
    }
}

fn appfs_skill_allowed_tools(skill_doc: Option<&Value>) -> Option<String> {
    let tools = skill_doc
        .and_then(|doc| doc.get("allowed_tools"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|tool| !tool.is_empty())
        .collect::<Vec<_>>();
    if tools.is_empty() {
        None
    } else {
        Some(tools.join(", "))
    }
}

fn appfs_skill_overview_markdown(skill_doc: Option<&Value>, app_id: &str) -> (String, bool) {
    if let Some(overview) = skill_doc
        .and_then(|doc| doc.get("overview_markdown"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|overview| !overview.is_empty())
    {
        return (overview.to_string(), true);
    }

    if let Some(description) = appfs_skill_override_description(skill_doc) {
        return (description, true);
    }

    (
        format!(
            "Operate the mounted `{app_id}` AppFS app by reading resource files and appending one JSON object line to `*.act` sinks."
        ),
        false,
    )
}

fn appfs_skill_generated_section_enabled(skill_doc: Option<&Value>, section: &str) -> bool {
    skill_doc
        .and_then(|doc| doc.get("include_generated_sections"))
        .and_then(|sections| sections.get(section))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn build_appfs_skill_when_to_use(
    app_id: &str,
    control_doc: Option<&Value>,
    actions_doc: Option<&Value>,
) -> String {
    let mut clauses = vec![format!(
        "Use when the user wants to work with the current {app_id} AppFS app."
    )];
    clauses.push(
        "Load it before performing app-specific control or action-file operations.".to_string(),
    );

    if let Some(description) = control_doc
        .and_then(|doc| doc.get("description"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|description| !description.is_empty())
    {
        clauses.push(format!("App context: {description}"));
    }

    let mentions = actions_doc
        .and_then(|doc| doc.get("contact_routes"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|route| route.get("mention_tokens").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .take(4)
        .collect::<Vec<_>>();
    if !mentions.is_empty() {
        clauses.push(format!(
            "Especially use it when the user asks to message {}.",
            mentions.join(" / ")
        ));
    }

    let actions = actions_doc
        .and_then(|doc| doc.get("recommended_actions"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|action| action.get("use_when").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .take(2)
        .collect::<Vec<_>>();
    if !actions.is_empty() {
        clauses.push(actions.join(" "));
    }

    clauses.join(" ")
}

fn build_appfs_skill_markdown(
    app_id: &str,
    app_root: &Path,
    skill_doc: Option<&Value>,
    control_doc: Option<&Value>,
    actions_doc: Option<&Value>,
    current_scope_doc: Option<&Value>,
    available_scopes_doc: Option<&Value>,
) -> String {
    let (overview_markdown, uses_skill_narrative) =
        appfs_skill_overview_markdown(skill_doc, app_id);
    let mut lines = vec![
        format!("# appfs-{app_id}"),
        String::new(),
        overview_markdown,
        String::new(),
        "## AppFS action rules".to_string(),
        "- Every `*.act` file is an append-only JSONL sink.".to_string(),
        "- Never use `write_file` or `edit_file` on `*.act` paths because those tools overwrite the sink.".to_string(),
        "- Use `bash` to append exactly one JSON object plus a trailing newline.".to_string(),
        "- After appending to an action, prefer the AppFS event reminder that is injected into the next model call; use `action.completed` or `action.failed` there to decide whether the action succeeded.".to_string(),
        "- Only inspect `_stream/events.evt.jsonl` manually when debugging or when no AppFS event reminder appears.".to_string(),
    ];

    if !uses_skill_narrative {
        if let Some(description) = control_doc
            .and_then(|doc| doc.get("description"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|description| !description.is_empty())
        {
            lines.push(String::new());
            lines.push("## App overview".to_string());
            lines.push(format!("- {description}"));
        }
    }

    if appfs_skill_generated_section_enabled(skill_doc, "scope_summary") {
        if let Some(scope_doc) = current_scope_doc {
            if let Some(active_scope) = scope_doc.get("active_scope").and_then(Value::as_str) {
                lines.push(String::new());
                lines.push("## Current scope".to_string());
                lines.push(format!("- Active scope: `{active_scope}`."));
            }
            if let Some(resource) = scope_doc.get("primary_resource").and_then(Value::as_str) {
                lines.push(format!("- Primary resource: `{resource}`."));
            }
        }

        if let Some(scopes_doc) = available_scopes_doc {
            let scopes = scopes_doc
                .get("scopes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|scope| scope.get("scope_id").and_then(Value::as_str))
                .map(|scope| format!("`{scope}`"))
                .collect::<Vec<_>>();
            if !scopes.is_empty() {
                lines.push(format!("- Known scopes: {}.", scopes.join(", ")));
            }
        }
    }

    if appfs_skill_generated_section_enabled(skill_doc, "control_actions") {
        if let Some(actions) = control_doc
            .and_then(|doc| doc.get("actions"))
            .and_then(Value::as_array)
        {
            append_appfs_skill_action_section(
                &mut lines,
                "## App control actions",
                actions,
                app_root,
            );
        }
    }

    if appfs_skill_generated_section_enabled(skill_doc, "recommended_actions") {
        if let Some(actions) = actions_doc
            .and_then(|doc| doc.get("recommended_actions"))
            .and_then(Value::as_array)
        {
            lines.push(String::new());
            lines.push("## Recommended actions".to_string());
            for action in actions {
                let path = action
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>");
                let summary = action
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("App action");
                let mut line = format!("- `{path}`: {summary}");
                if let Some(example) = action.get("example_payload") {
                    if let Some(command) = format_appfs_append_example(app_root, path, example) {
                        line.push_str(&format!(" Append with: `{command}`."));
                    }
                }
                lines.push(line);
                if let Some(use_when) = action.get("use_when").and_then(Value::as_array) {
                    let reasons = use_when
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>();
                    if !reasons.is_empty() {
                        lines.push(format!("  Use when: {}", reasons.join(" ")));
                    }
                }
            }
        }
    }

    if appfs_skill_generated_section_enabled(skill_doc, "contact_routing") {
        if let Some(routes) = actions_doc
            .and_then(|doc| doc.get("contact_routes"))
            .and_then(Value::as_array)
        {
            lines.push(String::new());
            lines.push("## Contact routing".to_string());
            for route in routes {
                let mention_tokens = route
                    .get("mention_tokens")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|token| format!("`{token}`"))
                    .collect::<Vec<_>>();
                let path = route
                    .get("send_message_path")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>");
                let mentions = if mention_tokens.is_empty() {
                    "<none>".to_string()
                } else {
                    mention_tokens.join(", ")
                };
                lines.push(format!("- {mentions} -> `{path}`."));
            }
        }
    }

    lines.push(String::new());
    lines.push("When the task matches this app, load this skill first, then follow the action rules above.".to_string());
    lines.join("\n")
}

fn append_appfs_skill_action_section(
    lines: &mut Vec<String>,
    title: &str,
    actions: &[Value],
    app_root: &Path,
) {
    if actions.is_empty() {
        return;
    }

    lines.push(String::new());
    lines.push(title.to_string());
    for action in actions {
        let path = action
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let summary = action
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("App action");
        let mut line = format!("- `{path}`: {summary}");
        if let Some(example) = action.get("example_payload") {
            if let Some(command) = format_appfs_append_example(app_root, path, example) {
                line.push_str(&format!(" Append with: `{command}`."));
            }
        }
        lines.push(line);
        if let Some(use_when) = action.get("use_when").and_then(Value::as_array) {
            let reasons = use_when
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            if !reasons.is_empty() {
                lines.push(format!("  Use when: {}", reasons.join(" ")));
            }
        }
    }
}

fn format_appfs_append_example(
    app_root: &Path,
    action_path: &str,
    example: &Value,
) -> Option<String> {
    let example_text = serde_json::to_string(example).ok()?;
    let target_path = appfs_action_target_path(app_root, action_path);
    Some(format!(
        "printf '%s\\n' {} >> {}",
        bash_single_quote(&example_text),
        bash_single_quote(&path_to_bash_display(&target_path))
    ))
}

fn appfs_action_target_path(app_root: &Path, action_path: &str) -> PathBuf {
    let relative_action_path = action_path.trim_start_matches(['/', '\\']);
    app_root.join(relative_action_path)
}

fn path_to_bash_display(path: &Path) -> String {
    let raw = path.display().to_string().replace('\\', "/");
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let rest = raw[2..].trim_start_matches('/');
        if rest.is_empty() {
            format!("/{drive}")
        } else {
            format!("/{drive}/{rest}")
        }
    } else {
        raw
    }
}

fn bash_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn read_json_value(path: &Path) -> Option<Value> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn yaml_quote_scalar(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn load_skills_from_root(
    root: &SkillRoot,
    activation_cwd: Option<&Path>,
) -> std::io::Result<Vec<SkillSummary>> {
    let mut root_skills = Vec::new();
    for entry in fs::read_dir(&root.path)? {
        let entry = entry?;
        let Some((skill_path, description_fallback_label)) =
            resolve_skill_entry_path(root.origin, entry.path())
        else {
            continue;
        };
        let document = load_skill_document(&skill_path, description_fallback_label)?;
        if should_skip_conditional_skill(&document, activation_cwd) {
            continue;
        }
        root_skills.push(SkillSummary {
            name: document.user_facing_name().to_string(),
            description: Some(document.description.clone()),
            location: SkillLocation::Filesystem(skill_path),
            document,
            source: root.source,
            shadowed_by: None,
            origin: root.origin,
        });
    }
    root_skills.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(root_skills)
}

fn resolve_skill_entry_path(origin: SkillOrigin, path: PathBuf) -> Option<(PathBuf, &'static str)> {
    match origin {
        SkillOrigin::Bundled => unreachable!("bundled skills are synthesized in-memory"),
        SkillOrigin::SkillsDir => {
            if !path.is_dir() {
                return None;
            }
            let skill_path = path.join("SKILL.md");
            skill_path.is_file().then_some((skill_path, "Skill"))
        }
        SkillOrigin::LegacyCommandsDir => {
            if path.is_dir() {
                let skill_path = path.join("SKILL.md");
                return skill_path
                    .is_file()
                    .then_some((skill_path, "Custom command"));
            }
            path.extension()
                .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("md"))
                .then_some((path, "Custom command"))
        }
    }
}

fn should_skip_conditional_skill(document: &SkillDocument, activation_cwd: Option<&Path>) -> bool {
    activation_cwd.is_some_and(|cwd| {
        document.paths.is_some() && !is_conditional_skill_activated(cwd, &document.resolved_name)
    })
}

fn merge_skill_summaries(
    destination: &mut Vec<SkillSummary>,
    active_sources: &mut BTreeMap<String, DefinitionSource>,
    skill_summaries: Vec<SkillSummary>,
) {
    for mut skill in skill_summaries {
        let key = skill.name.to_ascii_lowercase();
        if let Some(existing) = active_sources.get(&key) {
            skill.shadowed_by = Some(*existing);
        } else {
            active_sources.insert(key, skill.source);
        }
        destination.push(skill);
    }
}

fn parse_toml_string(contents: &str, key: &str) -> Option<String> {
    let prefix = format!("{key} =");
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        let Some(value) = trimmed.strip_prefix(&prefix) else {
            continue;
        };
        let value = value.trim();
        let Some(value) = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
        else {
            continue;
        };
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn render_agents_report(agents: &[AgentSummary]) -> String {
    if agents.is_empty() {
        return "No agents found.".to_string();
    }

    let total_active = agents
        .iter()
        .filter(|agent| agent.shadowed_by.is_none())
        .count();
    let mut lines = vec![
        "Agents".to_string(),
        format!("  {total_active} active agents"),
        String::new(),
    ];

    for scope in [
        DefinitionScope::Bundled,
        DefinitionScope::Project,
        DefinitionScope::UserConfigHome,
        DefinitionScope::UserHome,
    ] {
        let group = agents
            .iter()
            .filter(|agent| agent.source.report_scope() == scope)
            .collect::<Vec<_>>();
        if group.is_empty() {
            continue;
        }

        lines.push(format!("{}:", scope.label()));
        for agent in group {
            let detail = agent_detail(agent);
            match agent.shadowed_by {
                Some(winner) => lines.push(format!("  (shadowed by {}) {detail}", winner.label())),
                None => lines.push(format!("  {detail}")),
            }
        }
        lines.push(String::new());
    }

    lines.join("\n").trim_end().to_string()
}

fn render_agents_report_json(cwd: &Path, agents: &[AgentSummary]) -> Value {
    let active = agents
        .iter()
        .filter(|agent| agent.shadowed_by.is_none())
        .count();
    json!({
        "kind": "agents",
        "action": "list",
        "working_directory": cwd.display().to_string(),
        "count": agents.len(),
        "summary": {
            "total": agents.len(),
            "active": active,
            "shadowed": agents.len().saturating_sub(active),
        },
        "agents": agents.iter().map(agent_summary_json).collect::<Vec<_>>(),
    })
}

fn agent_detail(agent: &AgentSummary) -> String {
    let mut parts = vec![agent.name.clone()];
    if let Some(description) = &agent.description {
        parts.push(description.clone());
    }
    if let Some(model) = &agent.model {
        parts.push(model.clone());
    }
    if let Some(reasoning) = &agent.reasoning_effort {
        parts.push(reasoning.clone());
    }
    parts.join(" · ")
}

fn render_skills_report(skills: &[SkillSummary]) -> String {
    if skills.is_empty() {
        return "No skills found.".to_string();
    }

    let total_active = skills
        .iter()
        .filter(|skill| skill.shadowed_by.is_none())
        .count();
    let mut lines = vec![
        "Skills".to_string(),
        format!("  {total_active} available skills"),
        String::new(),
    ];

    for scope in [
        DefinitionScope::Bundled,
        DefinitionScope::Project,
        DefinitionScope::UserConfigHome,
        DefinitionScope::UserHome,
    ] {
        let group = skills
            .iter()
            .filter(|skill| skill.source.report_scope() == scope)
            .collect::<Vec<_>>();
        if group.is_empty() {
            continue;
        }

        lines.push(format!("{}:", scope.label()));
        for skill in group {
            let mut parts = vec![skill.name.clone()];
            if let Some(description) = &skill.description {
                parts.push(description.clone());
            }
            if let Some(detail) = skill.origin.detail_label() {
                parts.push(detail.to_string());
            }
            let detail = parts.join(" · ");
            match skill.shadowed_by {
                Some(winner) => lines.push(format!("  (shadowed by {}) {detail}", winner.label())),
                None => lines.push(format!("  {detail}")),
            }
        }
        lines.push(String::new());
    }

    lines.join("\n").trim_end().to_string()
}

fn render_skills_report_json(skills: &[SkillSummary]) -> Value {
    let active = skills
        .iter()
        .filter(|skill| skill.shadowed_by.is_none())
        .count();
    json!({
        "kind": "skills",
        "action": "list",
        "summary": {
            "total": skills.len(),
            "active": active,
            "shadowed": skills.len().saturating_sub(active),
        },
        "skills": skills.iter().map(skill_summary_json).collect::<Vec<_>>(),
    })
}

fn render_skill_install_report(skill: &InstalledSkill) -> String {
    let mut lines = vec![
        "Skills".to_string(),
        format!("  Result           installed {}", skill.invocation_name),
        format!("  Invoke as        ${}", skill.invocation_name),
    ];
    if let Some(display_name) = &skill.display_name {
        lines.push(format!("  Display name     {display_name}"));
    }
    lines.push(format!("  Source           {}", skill.source.display()));
    lines.push(format!(
        "  Registry         {}",
        skill.registry_root.display()
    ));
    lines.push(format!(
        "  Installed path   {}",
        skill.installed_path.display()
    ));
    lines.join("\n")
}

fn render_skill_install_report_json(skill: &InstalledSkill) -> Value {
    json!({
        "kind": "skills",
        "action": "install",
        "result": "installed",
        "invocation_name": &skill.invocation_name,
        "invoke_as": format!("${}", skill.invocation_name),
        "display_name": &skill.display_name,
        "source": skill.source.display().to_string(),
        "registry_root": skill.registry_root.display().to_string(),
        "installed_path": skill.installed_path.display().to_string(),
    })
}

fn render_mcp_summary_report(
    cwd: &Path,
    servers: &BTreeMap<String, ScopedMcpServerConfig>,
) -> String {
    let mut lines = vec![
        "MCP".to_string(),
        format!("  Working directory {}", cwd.display()),
        format!("  Configured servers {}", servers.len()),
    ];
    if servers.is_empty() {
        lines.push("  No MCP servers configured.".to_string());
        return lines.join("\n");
    }

    lines.push(String::new());
    for (name, server) in servers {
        lines.push(format!(
            "  {name:<16} {transport:<13} {scope:<7} {summary}",
            transport = mcp_transport_label(&server.config),
            scope = config_source_label(server.scope),
            summary = mcp_server_summary(&server.config)
        ));
    }

    lines.join("\n")
}

fn render_mcp_summary_report_json(
    cwd: &Path,
    servers: &BTreeMap<String, ScopedMcpServerConfig>,
) -> Value {
    json!({
        "kind": "mcp",
        "action": "list",
        "working_directory": cwd.display().to_string(),
        "configured_servers": servers.len(),
        "servers": servers
            .iter()
            .map(|(name, server)| mcp_server_json(name, server))
            .collect::<Vec<_>>(),
    })
}

fn render_mcp_server_report(
    cwd: &Path,
    server_name: &str,
    server: Option<&ScopedMcpServerConfig>,
) -> String {
    let Some(server) = server else {
        return format!(
            "MCP\n  Working directory {}\n  Result            server `{server_name}` is not configured",
            cwd.display()
        );
    };

    let mut lines = vec![
        "MCP".to_string(),
        format!("  Working directory {}", cwd.display()),
        format!("  Name              {server_name}"),
        format!("  Scope             {}", config_source_label(server.scope)),
        format!(
            "  Transport         {}",
            mcp_transport_label(&server.config)
        ),
    ];

    match &server.config {
        McpServerConfig::Stdio(config) => {
            lines.push(format!("  Command           {}", config.command));
            lines.push(format!(
                "  Args              {}",
                format_optional_list(&config.args)
            ));
            lines.push(format!(
                "  Env keys          {}",
                format_optional_keys(config.env.keys().cloned().collect())
            ));
            lines.push(format!(
                "  Tool timeout      {}",
                config
                    .tool_call_timeout_ms
                    .map_or_else(|| "<default>".to_string(), |value| format!("{value} ms"))
            ));
        }
        McpServerConfig::Sse(config) | McpServerConfig::Http(config) => {
            lines.push(format!("  URL               {}", config.url));
            lines.push(format!(
                "  Header keys       {}",
                format_optional_keys(config.headers.keys().cloned().collect())
            ));
            lines.push(format!(
                "  Header helper     {}",
                config.headers_helper.as_deref().unwrap_or("<none>")
            ));
            lines.push(format!(
                "  OAuth             {}",
                format_mcp_oauth(config.oauth.as_ref())
            ));
        }
        McpServerConfig::Ws(config) => {
            lines.push(format!("  URL               {}", config.url));
            lines.push(format!(
                "  Header keys       {}",
                format_optional_keys(config.headers.keys().cloned().collect())
            ));
            lines.push(format!(
                "  Header helper     {}",
                config.headers_helper.as_deref().unwrap_or("<none>")
            ));
        }
        McpServerConfig::Sdk(config) => {
            lines.push(format!("  SDK name          {}", config.name));
        }
        McpServerConfig::ManagedProxy(config) => {
            lines.push(format!("  URL               {}", config.url));
            lines.push(format!("  Proxy id          {}", config.id));
        }
    }

    lines.join("\n")
}

fn render_mcp_server_report_json(
    cwd: &Path,
    server_name: &str,
    server: Option<&ScopedMcpServerConfig>,
) -> Value {
    match server {
        Some(server) => json!({
            "kind": "mcp",
            "action": "show",
            "working_directory": cwd.display().to_string(),
            "found": true,
            "server": mcp_server_json(server_name, server),
        }),
        None => json!({
            "kind": "mcp",
            "action": "show",
            "working_directory": cwd.display().to_string(),
            "found": false,
            "server_name": server_name,
            "message": format!("server `{server_name}` is not configured"),
        }),
    }
}
fn normalize_optional_args(args: Option<&str>) -> Option<&str> {
    args.map(str::trim).filter(|value| !value.is_empty())
}

fn is_help_arg(arg: &str) -> bool {
    matches!(arg, "help" | "-h" | "--help")
}

fn help_path_from_args(args: &str) -> Option<Vec<&str>> {
    let parts = args.split_whitespace().collect::<Vec<_>>();
    let help_index = parts.iter().position(|part| is_help_arg(part))?;
    Some(parts[..help_index].to_vec())
}

fn render_agents_usage(unexpected: Option<&str>) -> String {
    let mut lines = vec![
        "Agents".to_string(),
        "  Usage            /agents [list|help]".to_string(),
        "  Direct CLI       claw agents".to_string(),
        "  Sources          .claw/agents, ~/.claw/agents, $CLAW_CONFIG_HOME/agents".to_string(),
    ];
    if let Some(args) = unexpected {
        lines.push(format!("  Unexpected       {args}"));
    }
    lines.join("\n")
}

fn render_agents_usage_json(unexpected: Option<&str>) -> Value {
    json!({
        "kind": "agents",
        "action": "help",
        "usage": {
            "slash_command": "/agents [list|help]",
            "direct_cli": "claw agents [list|help]",
            "sources": [".claw/agents", "~/.claw/agents", "$CLAW_CONFIG_HOME/agents"],
        },
        "unexpected": unexpected,
    })
}

fn render_skills_usage(unexpected: Option<&str>) -> String {
    let mut lines = vec![
        "Skills".to_string(),
        "  Usage            /skills [list|install <path>|help|<skill> [args]]".to_string(),
        "  Direct CLI       claw skills [list|install <path>|help|<skill> [args]]".to_string(),
        "  Invoke           /skills help overview -> $help overview".to_string(),
        "  Install root     $CLAW_CONFIG_HOME/skills or ~/.claw/skills".to_string(),
        "  Sources          bundled built-ins, .claw/skills, ~/.claw/skills, legacy /commands"
            .to_string(),
    ];
    if let Some(args) = unexpected {
        lines.push(format!("  Unexpected       {args}"));
    }
    lines.join("\n")
}

fn render_skills_usage_json(unexpected: Option<&str>) -> Value {
    json!({
        "kind": "skills",
        "action": "help",
        "usage": {
            "slash_command": "/skills [list|install <path>|help|<skill> [args]]",
            "direct_cli": "claw skills [list|install <path>|help|<skill> [args]]",
            "invoke": "/skills help overview -> $help overview",
            "install_root": "$CLAW_CONFIG_HOME/skills or ~/.claw/skills",
            "sources": [
                "bundled built-ins",
                ".claw/skills",
                "~/.claw/skills",
                "legacy /commands",
                "legacy fallback dirs still load automatically",
            ],
        },
        "unexpected": unexpected,
    })
}

const DEFAULT_MODEL_SKILL_LISTING_CHAR_BUDGET: usize = 8_000;
const MAX_MODEL_SKILL_ENTRY_CHARS: usize = 320;

pub fn render_model_facing_skill_listing(cwd: &Path) -> std::io::Result<Option<String>> {
    render_model_facing_skill_listing_with_budget(cwd, DEFAULT_MODEL_SKILL_LISTING_CHAR_BUDGET)
}

pub fn render_model_facing_skill_listing_with_budget(
    cwd: &Path,
    budget_chars: usize,
) -> std::io::Result<Option<String>> {
    let roots = discover_skill_roots(cwd);
    let mut skills = load_skills_from_roots_for_context(&roots, cwd)?;
    skills.retain(|skill| skill.shadowed_by.is_none() && !skill.document.disable_model_invocation);
    if skills.is_empty() {
        return Ok(None);
    }

    skills.sort_by(|left, right| {
        skill_listing_sort_key(left)
            .cmp(&skill_listing_sort_key(right))
            .then_with(|| left.name.cmp(&right.name))
    });

    let mut lines = vec![
        "Available skills. When one matches the user's request, invoke the `Skill` tool before responding. Use only the exact skill names listed below.".to_string(),
        "If none of these fit, you may use `ToolSearch` to search for more specialized tools.".to_string(),
        String::new(),
    ];
    let mut used_chars = lines.iter().map(|line| line.len()).sum::<usize>() + (lines.len() - 1);
    let mut omitted = 0usize;

    for skill in skills {
        let entry = format_model_facing_skill_entry(&skill);
        let separator_chars = usize::from(!lines.is_empty());
        if used_chars + separator_chars + entry.len() > budget_chars {
            omitted += 1;
            continue;
        }
        used_chars += separator_chars + entry.len();
        lines.push(entry);
    }

    if omitted > 0 {
        lines.push(format!(
            "_{omitted} additional skills omitted to stay within the context budget._"
        ));
    }

    Ok(Some(lines.join("\n")))
}

fn skill_listing_sort_key(skill: &SkillSummary) -> (u8, String) {
    let priority = match skill.location {
        SkillLocation::Generated(_) => 0,
        SkillLocation::Bundled(_) => 1,
        SkillLocation::Filesystem(_) => 2,
    };
    (priority, skill.name.to_ascii_lowercase())
}

fn format_model_facing_skill_entry(skill: &SkillSummary) -> String {
    let mut description = skill.document.description.trim().to_string();
    if let Some(when_to_use) = skill
        .document
        .when_to_use
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        description.push_str(" Use when: ");
        description.push_str(when_to_use);
    }
    if description.chars().count() > MAX_MODEL_SKILL_ENTRY_CHARS {
        let truncated = description
            .chars()
            .take(MAX_MODEL_SKILL_ENTRY_CHARS - 1)
            .collect::<String>();
        description = format!("{truncated}…");
    }
    format!("- {}: {}", skill.name, description)
}

fn render_mcp_usage(unexpected: Option<&str>) -> String {
    let mut lines = vec![
        "MCP".to_string(),
        "  Usage            /mcp [list|show <server>|help]".to_string(),
        "  Direct CLI       claw mcp [list|show <server>|help]".to_string(),
        "  Sources          .claw/settings.json, .claw/settings.local.json".to_string(),
    ];
    if let Some(args) = unexpected {
        lines.push(format!("  Unexpected       {args}"));
    }
    lines.join("\n")
}

fn render_mcp_usage_json(unexpected: Option<&str>) -> Value {
    json!({
        "kind": "mcp",
        "action": "help",
        "usage": {
            "slash_command": "/mcp [list|show <server>|help]",
            "direct_cli": "claw mcp [list|show <server>|help]",
            "sources": [".claw/settings.json", ".claw/settings.local.json"],
        },
        "unexpected": unexpected,
    })
}

fn config_source_label(source: ConfigSource) -> &'static str {
    match source {
        ConfigSource::User => "user",
        ConfigSource::Project => "project",
        ConfigSource::Local => "local",
    }
}

fn mcp_transport_label(config: &McpServerConfig) -> &'static str {
    match config {
        McpServerConfig::Stdio(_) => "stdio",
        McpServerConfig::Sse(_) => "sse",
        McpServerConfig::Http(_) => "http",
        McpServerConfig::Ws(_) => "ws",
        McpServerConfig::Sdk(_) => "sdk",
        McpServerConfig::ManagedProxy(_) => "managed-proxy",
    }
}

fn mcp_server_summary(config: &McpServerConfig) -> String {
    match config {
        McpServerConfig::Stdio(config) => {
            if config.args.is_empty() {
                config.command.clone()
            } else {
                format!("{} {}", config.command, config.args.join(" "))
            }
        }
        McpServerConfig::Sse(config) | McpServerConfig::Http(config) => config.url.clone(),
        McpServerConfig::Ws(config) => config.url.clone(),
        McpServerConfig::Sdk(config) => config.name.clone(),
        McpServerConfig::ManagedProxy(config) => format!("{} ({})", config.id, config.url),
    }
}

fn format_optional_list(values: &[String]) -> String {
    if values.is_empty() {
        "<none>".to_string()
    } else {
        values.join(" ")
    }
}

fn format_optional_keys(mut keys: Vec<String>) -> String {
    if keys.is_empty() {
        return "<none>".to_string();
    }
    keys.sort();
    keys.join(", ")
}

fn format_mcp_oauth(oauth: Option<&McpOAuthConfig>) -> String {
    let Some(oauth) = oauth else {
        return "<none>".to_string();
    };

    let mut parts = Vec::new();
    if let Some(client_id) = &oauth.client_id {
        parts.push(format!("client_id={client_id}"));
    }
    if let Some(port) = oauth.callback_port {
        parts.push(format!("callback_port={port}"));
    }
    if let Some(url) = &oauth.auth_server_metadata_url {
        parts.push(format!("metadata_url={url}"));
    }
    if let Some(xaa) = oauth.xaa {
        parts.push(format!("xaa={xaa}"));
    }
    if parts.is_empty() {
        "enabled".to_string()
    } else {
        parts.join(", ")
    }
}

fn definition_source_id(source: DefinitionSource) -> &'static str {
    match source {
        DefinitionSource::Bundled => "bundled",
        DefinitionSource::ProjectClaw
        | DefinitionSource::ProjectCodex
        | DefinitionSource::ProjectClaude => "project_claw",
        DefinitionSource::UserClawConfigHome | DefinitionSource::UserCodexHome => {
            "user_claw_config_home"
        }
        DefinitionSource::UserClaw | DefinitionSource::UserCodex | DefinitionSource::UserClaude => {
            "user_claw"
        }
    }
}

fn definition_source_json(source: DefinitionSource) -> Value {
    json!({
        "id": definition_source_id(source),
        "label": source.label(),
    })
}

fn agent_summary_json(agent: &AgentSummary) -> Value {
    json!({
        "name": &agent.name,
        "description": &agent.description,
        "model": &agent.model,
        "reasoning_effort": &agent.reasoning_effort,
        "source": definition_source_json(agent.source),
        "active": agent.shadowed_by.is_none(),
        "shadowed_by": agent.shadowed_by.map(definition_source_json),
    })
}

fn skill_origin_id(origin: SkillOrigin) -> &'static str {
    match origin {
        SkillOrigin::Bundled => "bundled",
        SkillOrigin::SkillsDir => "skills_dir",
        SkillOrigin::LegacyCommandsDir => "legacy_commands_dir",
    }
}

fn skill_origin_json(origin: SkillOrigin) -> Value {
    json!({
        "id": skill_origin_id(origin),
        "detail_label": origin.detail_label(),
    })
}

fn skill_summary_json(skill: &SkillSummary) -> Value {
    json!({
        "name": &skill.name,
        "description": &skill.description,
        "metadata": skill_document_json(&skill.document),
        "source": definition_source_json(skill.source),
        "origin": skill_origin_json(skill.origin),
        "active": skill.shadowed_by.is_none(),
        "shadowed_by": skill.shadowed_by.map(definition_source_json),
    })
}

fn skill_document_json(document: &SkillDocument) -> Value {
    json!({
        "resolved_name": &document.resolved_name,
        "display_name": &document.display_name,
        "description": &document.description,
        "has_user_specified_description": document.has_user_specified_description,
        "allowed_tools": &document.allowed_tools,
        "argument_hint": &document.argument_hint,
        "argument_names": &document.argument_names,
        "when_to_use": &document.when_to_use,
        "version": &document.version,
        "model": &document.model,
        "disable_model_invocation": document.disable_model_invocation,
        "user_invocable": document.user_invocable,
        "hooks": &document.hooks,
        "execution_context": document.execution_context.map(SkillExecutionContext::as_str),
        "agent": &document.agent,
        "effort": &document.effort,
        "paths": &document.paths,
        "shell": &document.shell,
    })
}

fn config_source_id(source: ConfigSource) -> &'static str {
    match source {
        ConfigSource::User => "user",
        ConfigSource::Project => "project",
        ConfigSource::Local => "local",
    }
}

fn config_source_json(source: ConfigSource) -> Value {
    json!({
        "id": config_source_id(source),
        "label": config_source_label(source),
    })
}

fn mcp_transport_json(config: &McpServerConfig) -> Value {
    let label = mcp_transport_label(config);
    json!({
        "id": label,
        "label": label,
    })
}

fn mcp_oauth_json(oauth: Option<&McpOAuthConfig>) -> Value {
    let Some(oauth) = oauth else {
        return Value::Null;
    };
    json!({
        "client_id": &oauth.client_id,
        "callback_port": oauth.callback_port,
        "auth_server_metadata_url": &oauth.auth_server_metadata_url,
        "xaa": oauth.xaa,
    })
}

fn mcp_server_details_json(config: &McpServerConfig) -> Value {
    match config {
        McpServerConfig::Stdio(config) => json!({
            "command": &config.command,
            "args": &config.args,
            "env_keys": config.env.keys().cloned().collect::<Vec<_>>(),
            "tool_call_timeout_ms": config.tool_call_timeout_ms,
        }),
        McpServerConfig::Sse(config) | McpServerConfig::Http(config) => json!({
            "url": &config.url,
            "header_keys": config.headers.keys().cloned().collect::<Vec<_>>(),
            "headers_helper": &config.headers_helper,
            "oauth": mcp_oauth_json(config.oauth.as_ref()),
        }),
        McpServerConfig::Ws(config) => json!({
            "url": &config.url,
            "header_keys": config.headers.keys().cloned().collect::<Vec<_>>(),
            "headers_helper": &config.headers_helper,
        }),
        McpServerConfig::Sdk(config) => json!({
            "name": &config.name,
        }),
        McpServerConfig::ManagedProxy(config) => json!({
            "url": &config.url,
            "id": &config.id,
        }),
    }
}

fn mcp_server_json(name: &str, server: &ScopedMcpServerConfig) -> Value {
    json!({
        "name": name,
        "scope": config_source_json(server.scope),
        "transport": mcp_transport_json(&server.config),
        "summary": mcp_server_summary(&server.config),
        "details": mcp_server_details_json(&server.config),
    })
}

struct InitializedPluginRegistry {
    registry: PluginRegistry,
}

impl InitializedPluginRegistry {
    fn new(registry: PluginRegistry) -> Result<Self, String> {
        registry.initialize().map_err(|error| error.to_string())?;
        Ok(Self { registry })
    }

    fn aggregated_hooks(&self) -> Result<PluginHooks, String> {
        self.registry
            .aggregated_hooks()
            .map_err(|error| error.to_string())
    }
}

impl Drop for InitializedPluginRegistry {
    fn drop(&mut self) {
        let _ = self.registry.shutdown();
    }
}

struct CompactApiClient {
    runtime: tokio::runtime::Runtime,
    client: ProviderClient,
    model: String,
}

impl CompactApiClient {
    fn new(client: ProviderClient, model: String) -> Result<Self, String> {
        Ok(Self {
            runtime: tokio::runtime::Runtime::new().map_err(|error| error.to_string())?,
            client,
            model,
        })
    }
}

impl runtime::ApiClient for CompactApiClient {
    fn stream(
        &mut self,
        request: runtime::ApiRequest,
    ) -> Result<Vec<AssistantEvent>, runtime::RuntimeError> {
        let message_request = MessageRequest {
            model: self.model.clone(),
            max_tokens: max_tokens_for_model(&self.model),
            messages: convert_compact_messages(&request.messages),
            system: (!request.system_prompt.is_empty()).then(|| request.system_prompt.join("\n\n")),
            stream: false,
            ..Default::default()
        };
        let response = self
            .runtime
            .block_on(self.client.send_message(&message_request))
            .map_err(|error| runtime::RuntimeError::new(error.to_string()))?;
        Ok(message_response_to_assistant_events(response))
    }
}

fn message_response_to_assistant_events(response: MessageResponse) -> Vec<AssistantEvent> {
    let mut events = Vec::new();
    for block in response.content {
        match block {
            OutputContentBlock::Text { text } => events.push(AssistantEvent::TextDelta(text)),
            OutputContentBlock::ToolUse { id, name, input } => {
                events.push(AssistantEvent::ToolUse {
                    id,
                    name,
                    input: serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string()),
                });
            }
            OutputContentBlock::Thinking { .. } | OutputContentBlock::RedactedThinking { .. } => {}
        }
    }
    let usage = response.usage.token_usage();
    if usage.total_tokens() > 0 {
        events.push(AssistantEvent::Usage(usage));
    }
    events.push(AssistantEvent::MessageStop);
    events
}

fn convert_compact_messages(messages: &[ConversationMessage]) -> Vec<InputMessage> {
    messages
        .iter()
        .filter_map(|message| {
            let role = match message.role {
                MessageRole::System | MessageRole::User | MessageRole::Tool => "user",
                MessageRole::Assistant => "assistant",
            };
            let content = message
                .blocks
                .iter()
                .map(|block| match block {
                    runtime::ContentBlock::Text { text } => {
                        InputContentBlock::Text { text: text.clone() }
                    }
                    runtime::ContentBlock::ToolUse { id, name, input } => {
                        InputContentBlock::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: serde_json::from_str(input)
                                .unwrap_or_else(|_| serde_json::json!({ "raw": input })),
                        }
                    }
                    runtime::ContentBlock::ToolResult {
                        tool_use_id,
                        output,
                        is_error,
                        ..
                    } => InputContentBlock::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: vec![ToolResultContentBlock::Text {
                            text: output.clone(),
                        }],
                        is_error: *is_error,
                    },
                })
                .collect::<Vec<_>>();
            (!content.is_empty()).then(|| InputMessage {
                role: role.to_string(),
                content,
            })
        })
        .collect()
}

fn resolve_compact_model(config: &RuntimeConfig) -> String {
    let configured_model = env::var("ANTHROPIC_MODEL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| config.model().map(ToOwned::to_owned))
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let aliased = config
        .aliases()
        .get(configured_model.trim())
        .cloned()
        .unwrap_or(configured_model);
    resolve_model_alias(&aliased).clone()
}

fn provider_override_from_runtime_config(config: &RuntimeProviderConfig) -> ProviderOverride {
    ProviderOverride {
        provider: match config.provider() {
            RuntimeProviderKind::Anthropic => ProviderKind::Anthropic,
            RuntimeProviderKind::OpenAi => ProviderKind::OpenAi,
            RuntimeProviderKind::Xai => ProviderKind::Xai,
        },
        base_url: config.base_url().map(ToOwned::to_owned),
        api_key_env: config.api_key_env().map(ToOwned::to_owned),
        auth_token_env: config.auth_token_env().map(ToOwned::to_owned),
    }
}

fn runtime_hook_config_from_plugin_hooks(hooks: PluginHooks) -> RuntimeHookConfig {
    RuntimeHookConfig::new(
        hooks.pre_tool_use,
        hooks.post_tool_use,
        hooks.post_tool_use_failure,
    )
    .with_pre_compact(hooks.pre_compact)
    .with_post_compact(hooks.post_compact)
    .with_session_start(hooks.session_start)
}

fn resolve_plugin_path(cwd: &Path, config_home: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else if value.starts_with('.') {
        cwd.join(path)
    } else {
        config_home.join(path)
    }
}

fn build_plugin_manager(
    cwd: &Path,
    loader: &ConfigLoader,
    runtime_config: &RuntimeConfig,
) -> PluginManager {
    let plugin_settings = runtime_config.plugins();
    let mut plugin_config = PluginManagerConfig::new(loader.config_home().to_path_buf());
    plugin_config.enabled_plugins = plugin_settings.enabled_plugins().clone();
    plugin_config.external_dirs = plugin_settings
        .external_directories()
        .iter()
        .map(|path| resolve_plugin_path(cwd, loader.config_home(), path))
        .collect();
    plugin_config.install_root = plugin_settings
        .install_root()
        .map(|path| resolve_plugin_path(cwd, loader.config_home(), path));
    plugin_config.registry_path = plugin_settings
        .registry_path()
        .map(|path| resolve_plugin_path(cwd, loader.config_home(), path));
    plugin_config.bundled_root = plugin_settings
        .bundled_root()
        .map(|path| resolve_plugin_path(cwd, loader.config_home(), path));
    PluginManager::new(plugin_config)
}

fn build_full_compact_feature_config(
    cwd: &Path,
    loader: &ConfigLoader,
    runtime_config: &RuntimeConfig,
) -> Result<(RuntimeFeatureConfig, InitializedPluginRegistry), String> {
    let plugin_registry = InitializedPluginRegistry::new(
        build_plugin_manager(cwd, loader, runtime_config)
            .plugin_registry()
            .map_err(|error| error.to_string())?,
    )?;
    let plugin_hook_config =
        runtime_hook_config_from_plugin_hooks(plugin_registry.aggregated_hooks()?);
    let feature_config = runtime_config
        .feature_config()
        .clone()
        .with_hooks(runtime_config.hooks().merged(&plugin_hook_config));
    Ok((feature_config, plugin_registry))
}

fn run_full_compact(
    session: &Session,
    compaction: CompactionConfig,
) -> Result<runtime::CompactionResult, String> {
    let cwd = env::current_dir().map_err(|error| error.to_string())?;
    let loader = ConfigLoader::default_for(&cwd);
    let runtime_config = loader.load().map_err(|error| error.to_string())?;
    let system_prompt = load_system_prompt(cwd.clone(), DEFAULT_DATE, env::consts::OS, "unknown")
        .map_err(|error| error.to_string())?;
    let model = resolve_compact_model(&runtime_config);
    let provider_override = runtime_config
        .provider()
        .map(provider_override_from_runtime_config);
    let client =
        ProviderClient::from_model_with_provider_override(&model, provider_override.as_ref())
            .map_err(|error| error.to_string())?
            .with_prompt_cache(PromptCache::new(&session.session_id));
    let compact_client = CompactApiClient::new(client, model)?;
    let (feature_config, _plugin_registry) =
        build_full_compact_feature_config(&cwd, &loader, &runtime_config)?;
    let mut runtime = ConversationRuntime::new_with_features(
        session.clone(),
        compact_client,
        StaticToolExecutor::new(),
        PermissionPolicy::new(PermissionMode::DangerFullAccess),
        system_prompt,
        &feature_config,
    );
    runtime
        .compact(compaction)
        .map_err(|error| error.to_string())
}

fn format_compact_result_message(
    result: &runtime::CompactionResult,
    fallback_label: &str,
) -> String {
    let mut message = if result.removed_message_count == 0 {
        "Compaction skipped: session is below the compaction threshold.".to_string()
    } else {
        format!(
            "{fallback_label} {} messages.",
            result.removed_message_count
        )
    };
    if let Some(display) = result
        .user_display_message
        .as_deref()
        .map(str::trim)
        .filter(|display| !display.is_empty())
    {
        message.push('\n');
        message.push_str(display);
    }
    message
}

pub fn handle_slash_command_with_compactor<F>(
    input: &str,
    session: &Session,
    compaction: CompactionConfig,
    compact_with_runtime: &mut F,
) -> Option<SlashCommandResult>
where
    F: FnMut(&Session, CompactionConfig) -> Result<runtime::CompactionResult, String>,
{
    let command = match SlashCommand::parse(input) {
        Ok(Some(command)) => command,
        Ok(None) => return None,
        Err(error) => {
            return Some(SlashCommandResult {
                message: error.to_string(),
                session: session.clone(),
            });
        }
    };

    match command {
        SlashCommand::Compact => match compact_with_runtime(session, compaction) {
            Ok(result) => Some(SlashCommandResult {
                message: format_compact_result_message(
                    &result,
                    "Compacted session with full compact summary for",
                ),
                session: result.compacted_session,
            }),
            Err(error) => Some(SlashCommandResult {
                message: format!("Compaction failed: {error}"),
                session: session.clone(),
            }),
        },
        SlashCommand::Help => Some(SlashCommandResult {
            message: render_slash_command_help(),
            session: session.clone(),
        }),
        SlashCommand::Status
        | SlashCommand::Bughunter { .. }
        | SlashCommand::Commit
        | SlashCommand::Pr { .. }
        | SlashCommand::Issue { .. }
        | SlashCommand::Ultraplan { .. }
        | SlashCommand::Teleport { .. }
        | SlashCommand::DebugToolCall
        | SlashCommand::Sandbox
        | SlashCommand::Model { .. }
        | SlashCommand::Permissions { .. }
        | SlashCommand::Clear { .. }
        | SlashCommand::Cost
        | SlashCommand::Resume { .. }
        | SlashCommand::Config { .. }
        | SlashCommand::Mcp { .. }
        | SlashCommand::Memory
        | SlashCommand::Init
        | SlashCommand::Diff
        | SlashCommand::Version
        | SlashCommand::Export { .. }
        | SlashCommand::Session { .. }
        | SlashCommand::Principal { .. }
        | SlashCommand::Plugins { .. }
        | SlashCommand::Agents { .. }
        | SlashCommand::Skills { .. }
        | SlashCommand::Doctor
        | SlashCommand::Login
        | SlashCommand::Logout
        | SlashCommand::Vim
        | SlashCommand::Upgrade
        | SlashCommand::Stats
        | SlashCommand::Share
        | SlashCommand::Feedback
        | SlashCommand::Files
        | SlashCommand::Fast
        | SlashCommand::Exit
        | SlashCommand::Summary
        | SlashCommand::Desktop
        | SlashCommand::Brief
        | SlashCommand::Advisor
        | SlashCommand::Stickers
        | SlashCommand::Insights
        | SlashCommand::Thinkback
        | SlashCommand::ReleaseNotes
        | SlashCommand::SecurityReview
        | SlashCommand::Keybindings
        | SlashCommand::PrivacySettings
        | SlashCommand::Plan { .. }
        | SlashCommand::Review { .. }
        | SlashCommand::Tasks { .. }
        | SlashCommand::Theme { .. }
        | SlashCommand::Voice { .. }
        | SlashCommand::Usage { .. }
        | SlashCommand::Rename { .. }
        | SlashCommand::Copy { .. }
        | SlashCommand::Hooks { .. }
        | SlashCommand::Context { .. }
        | SlashCommand::Color { .. }
        | SlashCommand::Effort { .. }
        | SlashCommand::Branch { .. }
        | SlashCommand::Rewind { .. }
        | SlashCommand::Ide { .. }
        | SlashCommand::Tag { .. }
        | SlashCommand::OutputStyle { .. }
        | SlashCommand::AddDir { .. }
        | SlashCommand::History { .. }
        | SlashCommand::Unknown(_) => None,
    }
}

#[must_use]
pub fn handle_slash_command(
    input: &str,
    session: &Session,
    compaction: CompactionConfig,
) -> Option<SlashCommandResult> {
    handle_slash_command_with_compactor(input, session, compaction, &mut run_full_compact)
}

#[cfg(test)]
mod tests {
    use super::{
        activate_conditional_skills_for_paths, classify_skills_slash_command, discover_skill_roots,
        handle_agents_slash_command_json, handle_plugins_slash_command,
        handle_skills_slash_command_json, handle_slash_command,
        handle_slash_command_with_compactor, load_agents_from_roots, load_skills_from_roots,
        load_skills_from_roots_for_context, normalize_relative_match_path, render_agents_report,
        render_agents_report_json, render_mcp_report_json_for, render_plugins_report,
        render_resolved_skill_prompt, render_skills_report, render_slash_command_help,
        render_slash_command_help_detail, resolve_skill, resolve_skill_path,
        resume_supported_slash_commands, slash_command_specs, suggest_slash_commands,
        validate_slash_command_input, DefinitionSource, ResolvedSkillSource, SkillOrigin,
        SkillRoot, SkillSlashDispatch, SlashCommand,
    };
    use plugins::{PluginKind, PluginManager, PluginManagerConfig, PluginMetadata, PluginSummary};
    use runtime::{
        CompactBoundaryMetadata, CompactTrigger, CompactionConfig, CompactionResult, ConfigLoader,
        ContentBlock, ConversationMessage, MessageRole, Session,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("commands-plugin-{label}-{nanos}"))
    }

    fn write_external_plugin(root: &Path, name: &str, version: &str) {
        fs::create_dir_all(root.join(".claude-plugin")).expect("manifest dir");
        fs::write(
            root.join(".claude-plugin").join("plugin.json"),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\",\n  \"description\": \"commands plugin\"\n}}"
            ),
        )
        .expect("write manifest");
    }

    fn write_bundled_plugin(root: &Path, name: &str, version: &str, default_enabled: bool) {
        fs::create_dir_all(root.join(".claude-plugin")).expect("manifest dir");
        fs::write(
            root.join(".claude-plugin").join("plugin.json"),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\",\n  \"description\": \"bundled commands plugin\",\n  \"defaultEnabled\": {}\n}}",
                if default_enabled { "true" } else { "false" }
            ),
        )
        .expect("write bundled manifest");
    }

    fn write_agent(root: &Path, name: &str, description: &str, model: &str, reasoning: &str) {
        fs::create_dir_all(root).expect("agent root");
        fs::write(
            root.join(format!("{name}.toml")),
            format!(
                "name = \"{name}\"\ndescription = \"{description}\"\nmodel = \"{model}\"\nmodel_reasoning_effort = \"{reasoning}\"\n"
            ),
        )
        .expect("write agent");
    }

    fn write_skill(root: &Path, name: &str, description: &str) {
        let skill_root = root.join(name);
        fs::create_dir_all(&skill_root).expect("skill root");
        fs::write(
            skill_root.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n"),
        )
        .expect("write skill");
    }

    fn write_legacy_command(root: &Path, name: &str, description: &str) {
        fs::create_dir_all(root).expect("commands root");
        fs::write(
            root.join(format!("{name}.md")),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n"),
        )
        .expect("write command");
    }

    fn seed_appfs_skill_mount(root: &Path) -> PathBuf {
        let mount_root = root.join("mnt");
        let app_root = mount_root.join("aiim");
        fs::create_dir_all(mount_root.join("_appfs")).expect("control dir");
        fs::create_dir_all(app_root.join("_app")).expect("app control dir");
        fs::create_dir_all(app_root.join("_stream")).expect("stream dir");
        fs::write(mount_root.join("_appfs").join("register_app.act"), "").expect("register act");
        fs::write(
            app_root.join("_app").join("skill.res.json"),
            r#"{
  "app_id": "aiim",
  "description": "一个 AI 专用的聊天软件，用于收发消息。",
  "when_to_use": [
    "当用户想要向某人发送消息、查看聊天记录，或者处理其他与聊天相关的任务时使用。",
    "在当前挂载的 AIIM app 上执行专用 act 或控制操作之前，先加载这个 skill。"
  ],
  "overview_markdown": "AIIM 是当前挂载出来的聊天软件。你可以用它查看聊天记录、给联系人发送消息，并在需要时切换不同聊天 scope 后再继续读取或操作。",
  "allowed_tools": [
    "bash",
    "read_file",
    "glob_search"
  ],
  "include_generated_sections": {
    "scope_summary": true,
    "control_actions": true,
    "recommended_actions": true,
    "contact_routing": true
  }
}"#,
        )
        .expect("write skill doc");
        fs::write(
            app_root.join("_app").join("actions.res.json"),
            r#"{
  "app_id": "aiim",
  "recommended_actions": [
    {
      "name": "send_message",
      "path": "contacts/zhangsan/send_message.act",
      "summary": "Send a direct message to 张三 / zhangsan.",
      "use_when": [
        "User asks to tell 张三 / zhangsan / 老张 something."
      ],
      "example_payload": {
        "text": "明天上午十点开会",
        "priority": "normal"
      }
    }
  ],
  "contact_routes": [
    {
      "contact_id": "zhangsan",
      "send_message_path": "contacts/zhangsan/send_message.act",
      "mention_tokens": ["张三", "老张", "zhangsan"]
    }
  ]
}"#,
        )
        .expect("write actions doc");
        fs::write(
            app_root.join("_app").join("current_scope.res.json"),
            r#"{"app_id":"aiim","active_scope":"chat-001","primary_resource":"chats/chat-001/messages.res.jsonl"}"#,
        )
        .expect("write current scope");
        fs::write(
            app_root.join("_app").join("available_scopes.res.json"),
            r#"{"app_id":"aiim","active_scope":"chat-001","scopes":[{"scope_id":"chat-001"},{"scope_id":"chat-long"}]}"#,
        )
        .expect("write scopes");
        app_root
    }

    fn seed_appfs_private_skill_mount(root: &Path) -> PathBuf {
        let mount_root = root.join("mnt");
        let aiim_root = mount_root.join("aiim");
        let tinode_root = mount_root.join("private").join("default").join("tinode");
        let secret_root = mount_root
            .join("private")
            .join("incident-reporter")
            .join("secret");
        fs::create_dir_all(mount_root.join("_appfs")).expect("control dir");
        fs::create_dir_all(aiim_root.join("_app")).expect("aiim control dir");
        fs::create_dir_all(tinode_root.join("_app")).expect("tinode control dir");
        fs::create_dir_all(secret_root.join("_app")).expect("secret control dir");
        fs::write(mount_root.join("_appfs").join("register_app.act"), "").expect("register act");
        fs::write(
            mount_root.join("_appfs").join("apps.registry.json"),
            r#"{"version":1,"apps":[{"instance_id":"aiim","app_id":"aiim","visibility":"public","path":"aiim","session_id":"sess-aiim","registered_at":"2026-04-07T00:00:00Z","transport":{"kind":"in_process","http_timeout_ms":5000,"grpc_timeout_ms":5000,"bridge_max_retries":3,"bridge_initial_backoff_ms":50,"bridge_max_backoff_ms":500,"bridge_circuit_breaker_failures":5,"bridge_circuit_breaker_cooldown_ms":1000}},{"instance_id":"tinode--default","app_id":"tinode","visibility":"private_instance","parent_app_id":"tinode","principal_id":"default","profile_id":"tinode:default","path":"private/default/tinode","session_id":"sess-tinode-default","registered_at":"2026-04-07T00:00:00Z","transport":{"kind":"in_process","http_timeout_ms":5000,"grpc_timeout_ms":5000,"bridge_max_retries":3,"bridge_initial_backoff_ms":50,"bridge_max_backoff_ms":500,"bridge_circuit_breaker_failures":5,"bridge_circuit_breaker_cooldown_ms":1000}},{"instance_id":"secret--incident-reporter","app_id":"secret","visibility":"private_instance","parent_app_id":"secret","principal_id":"incident-reporter","profile_id":"secret:incident-reporter","path":"private/incident-reporter/secret","session_id":"sess-secret-incident","registered_at":"2026-04-07T00:00:00Z","transport":{"kind":"in_process","http_timeout_ms":5000,"grpc_timeout_ms":5000,"bridge_max_retries":3,"bridge_initial_backoff_ms":50,"bridge_max_backoff_ms":500,"bridge_circuit_breaker_failures":5,"bridge_circuit_breaker_cooldown_ms":1000}}]}"#,
        )
        .expect("write registry");
        fs::write(
            aiim_root.join("_app").join("skill.res.json"),
            r#"{"app_id":"aiim","description":"AIIM public chat app.","when_to_use":"Use for public AIIM chat."}"#,
        )
        .expect("write aiim skill");
        fs::write(
            tinode_root.join("_app").join("skill.res.json"),
            r#"{"app_id":"tinode","description":"Tinode private chat app.","when_to_use":"Use for the current principal's Tinode chat."}"#,
        )
        .expect("write tinode skill");
        fs::write(
            secret_root.join("_app").join("skill.res.json"),
            r#"{"app_id":"secret","description":"Incident-only private app.","when_to_use":"Use only for incident reporter."}"#,
        )
        .expect("write secret skill");
        mount_root
    }

    fn seed_control_only_appfs_skill_mount(root: &Path) -> PathBuf {
        let mount_root = root.join("mnt");
        let app_root = mount_root.join("scheduler");
        fs::create_dir_all(mount_root.join("_appfs")).expect("control dir");
        fs::create_dir_all(app_root.join("_app")).expect("app control dir");
        fs::create_dir_all(app_root.join("_stream")).expect("stream dir");
        fs::write(mount_root.join("_appfs").join("register_app.act"), "").expect("register act");
        fs::write(
            app_root.join("_app").join("control.res.json"),
            r#"{
  "app_id": "scheduler",
  "description": "Manage meeting scheduling flows inside the current scheduler app.",
  "events_path": "_stream/events.evt.jsonl",
  "current_scope_path": "_app/current_scope.res.json",
  "available_scopes_path": "_app/available_scopes.res.json",
  "actions": [
    {
      "name": "enter_scope",
      "path": "_app/enter_scope.act",
      "summary": "Switch to a named scheduling scope.",
      "example_payload": {
        "target_scope": "meeting-room-a"
      }
    },
    {
      "name": "schedule_meeting",
      "path": "meetings/create.act",
      "summary": "Create a new meeting with room and attendees.",
      "example_payload": {
        "topic": "Design review",
        "room": "A-301"
      }
    }
  ]
}"#,
        )
        .expect("write control doc");
        fs::write(
            app_root.join("_app").join("current_scope.res.json"),
            r#"{"app_id":"scheduler","active_scope":"meeting-room-a","primary_resource":"meetings/today.res.jsonl"}"#,
        )
        .expect("write current scope");
        fs::write(
            app_root.join("_app").join("available_scopes.res.json"),
            r#"{"app_id":"scheduler","active_scope":"meeting-room-a","scopes":[{"scope_id":"meeting-room-a"},{"scope_id":"meeting-room-b"}]}"#,
        )
        .expect("write scopes");
        app_root
    }

    fn parse_error_message(input: &str) -> String {
        SlashCommand::parse(input)
            .expect_err("slash command should be rejected")
            .to_string()
    }

    #[test]
    fn resolves_generated_appfs_skill_for_current_app() {
        let workspace = temp_dir("generated-appfs-skill");
        let app_root = seed_appfs_skill_mount(&workspace);
        let cwd = app_root.join("workspace");
        fs::create_dir_all(&cwd).expect("workspace dir");

        let resolved = resolve_skill(&cwd, "appfs-aiim").expect("generated appfs skill");
        let prompt = render_resolved_skill_prompt(&resolved, None);

        assert!(prompt.contains("AIIM 是当前挂载出来的聊天软件。"));
        assert!(prompt.contains("append-only JSONL"));
        assert!(prompt.contains("AppFS event reminder"));
        assert!(prompt.contains("contacts/zhangsan/send_message.act"));
        assert!(prompt.contains("printf '%s\\n'"));
        assert!(prompt.contains(">> '"));
        assert!(prompt.contains("张三"));
        assert!(!prompt.contains("tail -n"));
    }

    #[test]
    fn resolves_generated_appfs_skill_from_mount_root() {
        let workspace = temp_dir("generated-appfs-skill-root");
        let app_root = seed_appfs_skill_mount(&workspace);
        let mount_root = app_root
            .parent()
            .expect("app root should have mount root parent");

        let resolved = resolve_skill(mount_root, "appfs-aiim").expect("generated appfs skill");
        let prompt = render_resolved_skill_prompt(&resolved, None);

        assert!(prompt.contains("AIIM 是当前挂载出来的聊天软件。"));
        assert!(prompt.contains("contacts/zhangsan/send_message.act"));
    }

    #[test]
    fn resolves_generated_appfs_skill_from_control_doc_without_actions_doc() {
        let workspace = temp_dir("generated-appfs-skill-control-only");
        let app_root = seed_control_only_appfs_skill_mount(&workspace);
        let cwd = app_root.join("workspace");
        fs::create_dir_all(&cwd).expect("workspace dir");

        let resolved = resolve_skill(&cwd, "appfs-scheduler").expect("generated scheduler skill");
        let prompt = render_resolved_skill_prompt(&resolved, None);

        assert!(prompt.contains("Manage meeting scheduling flows"));
        assert!(prompt.contains("`_app/enter_scope.act`"));
        assert!(prompt.contains("`meetings/create.act`"));
        assert!(prompt.contains("AppFS event reminder"));
        assert!(prompt.contains("meeting-room-b"));
        assert!(!prompt.contains("contacts/zhangsan/send_message.act"));
    }

    #[test]
    fn appfs_append_examples_use_git_bash_drive_paths_on_windows_mounts() {
        let command = super::format_appfs_append_example(
            &PathBuf::from(r"C:\mnt\appfs-compose-aiim\aiim"),
            r"\contacts\zhangsan\send_message.act",
            &serde_json::json!({
                "text": "明天上午十点开会",
                "priority": "normal"
            }),
        )
        .expect("append example");

        assert!(command.starts_with("printf '%s\\n' '{"));
        assert!(command
            .contains("' >> '/c/mnt/appfs-compose-aiim/aiim/contacts/zhangsan/send_message.act'"));
        assert!(!command.contains(r"C:\mnt"));
    }

    #[test]
    fn render_model_facing_skill_listing_includes_regular_and_appfs_skills() {
        let workspace = temp_dir("model-facing-skill-listing");
        let app_root = seed_appfs_skill_mount(&workspace);
        let cwd = app_root.join("workspace");
        fs::create_dir_all(&cwd).expect("workspace dir");
        write_skill(
            &workspace.join(".claw").join("skills"),
            "trace",
            "Trace repository state",
        );

        let listing = super::render_model_facing_skill_listing_with_budget(&cwd, 8_000)
            .expect("listing should render")
            .expect("listing should exist");

        assert!(listing.contains("- appfs-aiim:"));
        assert!(listing.contains("一个 AI 专用的聊天软件，用于收发消息。"));
        assert!(listing.contains("当用户想要向某人发送消息、查看聊天记录"));
        assert!(listing.contains("- trace:"));
        assert!(listing.contains("invoke the `Skill` tool before responding"));
    }

    #[test]
    fn render_model_facing_skill_listing_includes_appfs_skills_from_mount_root() {
        let workspace = temp_dir("model-facing-skill-listing-root");
        let app_root = seed_appfs_skill_mount(&workspace);
        let mount_root = app_root
            .parent()
            .expect("app root should have mount root parent");

        let listing = super::render_model_facing_skill_listing_with_budget(mount_root, 8_000)
            .expect("listing should render")
            .expect("listing should exist");

        assert!(listing.contains("- appfs-aiim:"));
        assert!(listing.contains("一个 AI 专用的聊天软件，用于收发消息。"));
        assert!(listing.contains("当用户想要向某人发送消息、查看聊天记录"));
    }

    #[test]
    fn generated_appfs_skills_filter_private_apps_by_current_principal() {
        let workspace = temp_dir("model-facing-skill-listing-private");
        let mount_root = seed_appfs_private_skill_mount(&workspace);

        let listing = super::render_model_facing_skill_listing_with_budget(&mount_root, 8_000)
            .expect("listing should render")
            .expect("listing should exist");

        assert!(listing.contains("- appfs-aiim: AIIM public chat app."));
        assert!(listing.contains("- appfs-tinode: Tinode private chat app."));
        assert!(!listing.contains("appfs-secret"));

        let resolved = resolve_skill(&mount_root, "appfs-tinode").expect("generated tinode skill");
        assert_eq!(
            resolved.source,
            ResolvedSkillSource::Generated {
                id: "appfs-tinode".to_string(),
                base_dir: Some(mount_root.join("private").join("default").join("tinode")),
            }
        );
        let prompt = render_resolved_skill_prompt(&resolved, None);
        assert!(prompt.contains("Tinode private chat app."));
    }

    #[test]
    fn render_model_facing_skill_listing_skips_disable_model_invocation_skills() {
        let workspace = temp_dir("model-facing-skill-disable");
        let skills_root = workspace.join(".claw").join("skills");
        write_skill(&skills_root, "trace", "Trace repository state");
        let internal_root = skills_root.join("internal");
        fs::create_dir_all(&internal_root).expect("internal root");
        fs::write(
            internal_root.join("SKILL.md"),
            "---\nname: internal\ndescription: Internal only\ndisable-model-invocation: true\n---\n\n# internal\n",
        )
        .expect("write internal skill");

        let listing = super::render_model_facing_skill_listing_with_budget(&workspace, 8_000)
            .expect("listing should render")
            .expect("listing should exist");

        assert!(listing.contains("- trace:"));
        assert!(!listing.contains("- internal:"));
    }

    #[allow(clippy::too_many_lines)]
    #[test]
    fn parses_supported_slash_commands() {
        assert_eq!(SlashCommand::parse("/help"), Ok(Some(SlashCommand::Help)));
        assert_eq!(
            SlashCommand::parse(" /status "),
            Ok(Some(SlashCommand::Status))
        );
        assert_eq!(
            SlashCommand::parse("/sandbox"),
            Ok(Some(SlashCommand::Sandbox))
        );
        assert_eq!(
            SlashCommand::parse("/bughunter runtime"),
            Ok(Some(SlashCommand::Bughunter {
                scope: Some("runtime".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/commit"),
            Ok(Some(SlashCommand::Commit))
        );
        assert_eq!(
            SlashCommand::parse("/pr ready for review"),
            Ok(Some(SlashCommand::Pr {
                context: Some("ready for review".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/issue flaky test"),
            Ok(Some(SlashCommand::Issue {
                context: Some("flaky test".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/ultraplan ship both features"),
            Ok(Some(SlashCommand::Ultraplan {
                task: Some("ship both features".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/teleport conversation.rs"),
            Ok(Some(SlashCommand::Teleport {
                target: Some("conversation.rs".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/debug-tool-call"),
            Ok(Some(SlashCommand::DebugToolCall))
        );
        assert_eq!(
            SlashCommand::parse("/bughunter runtime"),
            Ok(Some(SlashCommand::Bughunter {
                scope: Some("runtime".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/commit"),
            Ok(Some(SlashCommand::Commit))
        );
        assert_eq!(
            SlashCommand::parse("/pr ready for review"),
            Ok(Some(SlashCommand::Pr {
                context: Some("ready for review".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/issue flaky test"),
            Ok(Some(SlashCommand::Issue {
                context: Some("flaky test".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/ultraplan ship both features"),
            Ok(Some(SlashCommand::Ultraplan {
                task: Some("ship both features".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/teleport conversation.rs"),
            Ok(Some(SlashCommand::Teleport {
                target: Some("conversation.rs".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/debug-tool-call"),
            Ok(Some(SlashCommand::DebugToolCall))
        );
        assert_eq!(
            SlashCommand::parse("/model claude-opus"),
            Ok(Some(SlashCommand::Model {
                model: Some("claude-opus".to_string()),
            }))
        );
        assert_eq!(
            SlashCommand::parse("/model"),
            Ok(Some(SlashCommand::Model { model: None }))
        );
        assert_eq!(
            SlashCommand::parse("/permissions read-only"),
            Ok(Some(SlashCommand::Permissions {
                mode: Some("read-only".to_string()),
            }))
        );
        assert_eq!(
            SlashCommand::parse("/clear"),
            Ok(Some(SlashCommand::Clear { confirm: false }))
        );
        assert_eq!(
            SlashCommand::parse("/clear --confirm"),
            Ok(Some(SlashCommand::Clear { confirm: true }))
        );
        assert_eq!(SlashCommand::parse("/cost"), Ok(Some(SlashCommand::Cost)));
        assert_eq!(
            SlashCommand::parse("/resume session.json"),
            Ok(Some(SlashCommand::Resume {
                session_path: Some("session.json".to_string()),
            }))
        );
        assert_eq!(
            SlashCommand::parse("/config"),
            Ok(Some(SlashCommand::Config { section: None }))
        );
        assert_eq!(
            SlashCommand::parse("/config env"),
            Ok(Some(SlashCommand::Config {
                section: Some("env".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/mcp"),
            Ok(Some(SlashCommand::Mcp {
                action: None,
                target: None
            }))
        );
        assert_eq!(
            SlashCommand::parse("/mcp show remote"),
            Ok(Some(SlashCommand::Mcp {
                action: Some("show".to_string()),
                target: Some("remote".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/memory"),
            Ok(Some(SlashCommand::Memory))
        );
        assert_eq!(SlashCommand::parse("/init"), Ok(Some(SlashCommand::Init)));
        assert_eq!(SlashCommand::parse("/diff"), Ok(Some(SlashCommand::Diff)));
        assert_eq!(
            SlashCommand::parse("/version"),
            Ok(Some(SlashCommand::Version))
        );
        assert_eq!(
            SlashCommand::parse("/export notes.txt"),
            Ok(Some(SlashCommand::Export {
                path: Some("notes.txt".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/session switch abc123"),
            Ok(Some(SlashCommand::Session {
                action: Some("switch".to_string()),
                target: Some("abc123".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/plugins install demo"),
            Ok(Some(SlashCommand::Plugins {
                action: Some("install".to_string()),
                target: Some("demo".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/plugins list"),
            Ok(Some(SlashCommand::Plugins {
                action: Some("list".to_string()),
                target: None
            }))
        );
        assert_eq!(
            SlashCommand::parse("/plugins enable demo"),
            Ok(Some(SlashCommand::Plugins {
                action: Some("enable".to_string()),
                target: Some("demo".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/skills install ./fixtures/help-skill"),
            Ok(Some(SlashCommand::Skills {
                args: Some("install ./fixtures/help-skill".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/plugins disable demo"),
            Ok(Some(SlashCommand::Plugins {
                action: Some("disable".to_string()),
                target: Some("demo".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/session fork incident-review"),
            Ok(Some(SlashCommand::Session {
                action: Some("fork".to_string()),
                target: Some("incident-review".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/principal list"),
            Ok(Some(SlashCommand::Principal {
                action: Some("list".to_string()),
                target: None,
                description: None,
            }))
        );
        assert_eq!(
            SlashCommand::parse("/principal create incident-reporter handles incident updates"),
            Ok(Some(SlashCommand::Principal {
                action: Some("create".to_string()),
                target: Some("incident-reporter".to_string()),
                description: Some("handles incident updates".to_string()),
            }))
        );
    }

    #[test]
    fn parses_history_command_without_count() {
        // given
        let input = "/history";

        // when
        let parsed = SlashCommand::parse(input);

        // then
        assert_eq!(parsed, Ok(Some(SlashCommand::History { count: None })));
    }

    #[test]
    fn parses_history_command_with_numeric_count() {
        // given
        let input = "/history 25";

        // when
        let parsed = SlashCommand::parse(input);

        // then
        assert_eq!(
            parsed,
            Ok(Some(SlashCommand::History {
                count: Some("25".to_string())
            }))
        );
    }

    #[test]
    fn rejects_history_with_extra_arguments() {
        // given
        let input = "/history 25 extra";

        // when
        let error = parse_error_message(input);

        // then
        assert!(error.contains("Usage: /history [count]"));
    }

    #[test]
    fn rejects_unexpected_arguments_for_no_arg_commands() {
        // given
        let input = "/compact now";

        // when
        let error = parse_error_message(input);

        // then
        assert!(error.contains("Unexpected arguments for /compact."));
        assert!(error.contains("  Usage            /compact"));
        assert!(error.contains("  Summary          Compact local session history"));
    }

    #[test]
    fn rejects_invalid_argument_values() {
        // given
        let input = "/permissions admin";

        // when
        let error = parse_error_message(input);

        // then
        assert!(error.contains(
            "Unsupported /permissions mode 'admin'. Use read-only, workspace-write, or danger-full-access."
        ));
        assert!(error.contains(
            "  Usage            /permissions [read-only|workspace-write|danger-full-access]"
        ));
    }

    #[test]
    fn rejects_missing_required_arguments() {
        // given
        let input = "/teleport";

        // when
        let error = parse_error_message(input);

        // then
        assert!(error.contains("Usage: /teleport <symbol-or-path>"));
        assert!(error.contains("  Category         Tools"));
    }

    #[test]
    fn rejects_invalid_session_and_plugin_shapes() {
        // given
        let session_input = "/session switch";
        let plugin_input = "/plugins list extra";

        // when
        let session_error = parse_error_message(session_input);
        let plugin_error = parse_error_message(plugin_input);

        // then
        assert!(session_error.contains("Usage: /session switch <session-id>"));
        assert!(session_error.contains("/session"));
        assert!(plugin_error.contains("Usage: /plugin list"));
        assert!(plugin_error.contains("Aliases          /plugins, /marketplace"));
    }

    #[test]
    fn rejects_invalid_agents_and_skills_arguments() {
        // given
        let agents_input = "/agents show planner";
        let skills_input = "/skills install";

        // when
        let agents_error = parse_error_message(agents_input);
        let skills_error = parse_error_message(skills_input);

        // then
        assert!(agents_error.contains(
            "Unexpected arguments for /agents: show planner. Use /agents, /agents list, or /agents help."
        ));
        assert!(agents_error.contains("  Usage            /agents [list|help]"));
        assert!(skills_error.contains("Usage: /skills install <path>"));
        assert!(skills_error.contains("  Usage            /skills install <path>"));
    }

    #[test]
    fn accepts_skills_invocation_arguments_for_prompt_dispatch() {
        assert_eq!(
            SlashCommand::parse("/skills help overview"),
            Ok(Some(SlashCommand::Skills {
                args: Some("help overview".to_string()),
            }))
        );
        assert_eq!(
            classify_skills_slash_command(Some("help overview")),
            SkillSlashDispatch::Invoke("$help overview".to_string())
        );
        assert_eq!(
            classify_skills_slash_command(Some("/test")),
            SkillSlashDispatch::Invoke("$test".to_string())
        );
        assert_eq!(
            classify_skills_slash_command(Some("install ./skill-pack")),
            SkillSlashDispatch::Local
        );
    }

    #[test]
    fn resolves_project_skills_and_legacy_commands_from_shared_registry() {
        let workspace = temp_dir("resolve-project-skills");
        let project_skills = workspace.join(".codex").join("skills");
        let legacy_commands = workspace.join(".claude").join("commands");

        write_skill(&project_skills, "plan", "Project planning guidance");
        write_legacy_command(&legacy_commands, "handoff", "Legacy handoff guidance");

        assert_eq!(
            resolve_skill_path(&workspace, "$plan").expect("project skill should resolve"),
            project_skills.join("plan").join("SKILL.md")
        );
        assert_eq!(
            resolve_skill_path(&workspace, "/handoff").expect("legacy command should resolve"),
            legacy_commands.join("handoff.md")
        );
        let bundled = resolve_skill(&workspace, "verify").expect("bundled verify should resolve");
        assert_eq!(bundled.document.resolved_name, "verify");
        assert!(matches!(
            bundled.source,
            super::ResolvedSkillSource::Bundled { .. }
        ));
    }

    #[test]
    fn renders_skills_reports_as_json() {
        let workspace = temp_dir("skills-json-workspace");
        let project_skills = workspace.join(".codex").join("skills");
        let project_commands = workspace.join(".claude").join("commands");
        let user_home = temp_dir("skills-json-home");
        let user_skills = user_home.join(".codex").join("skills");

        write_skill(&project_skills, "plan", "Project planning guidance");
        write_legacy_command(&project_commands, "deploy", "Legacy deployment guidance");
        write_skill(&user_skills, "plan", "User planning guidance");
        write_skill(&user_skills, "help", "Help guidance");

        let roots = vec![
            SkillRoot {
                source: DefinitionSource::ProjectCodex,
                path: project_skills,
                origin: SkillOrigin::SkillsDir,
            },
            SkillRoot {
                source: DefinitionSource::ProjectClaude,
                path: project_commands,
                origin: SkillOrigin::LegacyCommandsDir,
            },
            SkillRoot {
                source: DefinitionSource::UserCodex,
                path: user_skills,
                origin: SkillOrigin::SkillsDir,
            },
        ];
        let report = super::render_skills_report_json(
            &load_skills_from_roots(&roots).expect("skills should load"),
        );
        assert_eq!(report["kind"], "skills");
        assert_eq!(report["action"], "list");
        assert_eq!(report["summary"]["active"], 7);
        assert_eq!(report["summary"]["shadowed"], 1);
        let skills = report["skills"].as_array().expect("skills array");
        let verify = skills
            .iter()
            .find(|skill| skill["name"] == "verify")
            .expect("verify bundled skill");
        assert_eq!(verify["source"]["id"], "bundled");
        assert_eq!(verify["origin"]["id"], "bundled");
        let plan = skills
            .iter()
            .find(|skill| {
                skill["name"] == "plan"
                    && skill["source"]["id"] == "project_claw"
                    && skill["active"] == true
            })
            .expect("project plan");
        assert_eq!(plan["metadata"]["resolved_name"], "plan");
        assert_eq!(plan["metadata"]["description"], "Project planning guidance");
        let deploy = skills
            .iter()
            .find(|skill| skill["name"] == "deploy")
            .expect("deploy skill");
        assert_eq!(deploy["origin"]["id"], "legacy_commands_dir");
        let shadowed_plan = skills
            .iter()
            .find(|skill| {
                skill["name"] == "plan"
                    && skill["source"]["id"] == "user_claw"
                    && skill["active"] == false
            })
            .expect("shadowed user plan");
        assert_eq!(shadowed_plan["shadowed_by"]["id"], "project_claw");

        let help = handle_skills_slash_command_json(Some("help"), &workspace).expect("skills help");
        assert_eq!(help["kind"], "skills");
        assert_eq!(help["action"], "help");
        assert_eq!(
            help["usage"]["direct_cli"],
            "claw skills [list|install <path>|help|<skill> [args]]"
        );

        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(user_home);
    }

    #[test]
    fn conditional_skills_activate_only_after_matching_path_touch() {
        let workspace = temp_dir("conditional-skills-workspace");
        let project_skills = workspace.join(".codex").join("skills");
        let source_file = workspace.join("src").join("lib.rs");
        fs::create_dir_all(
            source_file
                .parent()
                .expect("source file should have parent"),
        )
        .expect("create source dir");
        fs::write(&source_file, "fn helper() {}\n").expect("write source file");
        write_skill(&project_skills, "help", "Help guidance");

        let conditional_root = project_skills.join("rustacean");
        fs::create_dir_all(&conditional_root).expect("conditional skill root");
        fs::write(
            conditional_root.join("SKILL.md"),
            "---\nname: rustacean\ndescription: Rust path guidance\npaths: src/**\n---\n\n# rustacean\n",
        )
        .expect("write conditional skill");

        let roots = discover_skill_roots(&workspace);
        let before = load_skills_from_roots_for_context(&roots, &workspace)
            .expect("skills should load before activation");
        assert_eq!(before.len(), 5);
        assert!(before
            .iter()
            .any(|skill| skill.document.resolved_name == "help" && skill.shadowed_by.is_none()));
        assert!(!before
            .iter()
            .any(|skill| skill.document.resolved_name == "rustacean"));
        assert!(resolve_skill_path(&workspace, "rustacean").is_err());

        let activated =
            activate_conditional_skills_for_paths(std::slice::from_ref(&source_file), &workspace)
                .expect("activation should succeed");
        assert_eq!(activated, vec!["rustacean".to_string()]);

        let after = load_skills_from_roots_for_context(&roots, &workspace)
            .expect("skills should load after activation");
        assert_eq!(after.len(), 6);
        assert!(after.iter().any(|skill| {
            skill.document.resolved_name == "rustacean" && skill.shadowed_by.is_none()
        }));
        assert_eq!(
            resolve_skill_path(&workspace, "rustacean").expect("conditional skill should resolve"),
            conditional_root.join("SKILL.md")
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn conditional_skill_matching_normalizes_noncanonical_workspace_roots() {
        let workspace = temp_dir("conditional-skills-noncanonical");
        let source_dir = workspace.join("src");
        let source_file = source_dir.join("lib.rs");
        fs::create_dir_all(&source_dir).expect("create source dir");
        fs::write(&source_file, "fn helper() {}\n").expect("write source file");

        let canonical_file = source_file.canonicalize().expect("canonical source file");
        let aliased_workspace = workspace.join("src").join("..");

        assert_eq!(
            normalize_relative_match_path(&canonical_file, &aliased_workspace),
            Some("src/lib.rs".to_string())
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn renders_mcp_reports_as_json() {
        let workspace = temp_dir("mcp-json-workspace");
        let config_home = temp_dir("mcp-json-home");
        fs::create_dir_all(workspace.join(".claw")).expect("workspace config dir");
        fs::create_dir_all(&config_home).expect("config home");
        fs::write(
            workspace.join(".claw").join("settings.json"),
            r#"{
              "mcpServers": {
                "alpha": {
                  "command": "uvx",
                  "args": ["alpha-server"],
                  "env": {"ALPHA_TOKEN": "secret"},
                  "toolCallTimeoutMs": 1200
                },
                "remote": {
                  "type": "http",
                  "url": "https://remote.example/mcp",
                  "headers": {"Authorization": "Bearer secret"},
                  "headersHelper": "./bin/headers",
                  "oauth": {
                    "clientId": "remote-client",
                    "callbackPort": 7878
                  }
                }
              }
            }"#,
        )
        .expect("write settings");
        fs::write(
            workspace.join(".claw").join("settings.local.json"),
            r#"{
              "mcpServers": {
                "remote": {
                  "type": "ws",
                  "url": "wss://remote.example/mcp"
                }
              }
            }"#,
        )
        .expect("write local settings");

        let loader = ConfigLoader::new(&workspace, &config_home);
        let list =
            render_mcp_report_json_for(&loader, &workspace, None).expect("mcp list json render");
        assert_eq!(list["kind"], "mcp");
        assert_eq!(list["action"], "list");
        assert_eq!(list["configured_servers"], 2);
        assert_eq!(list["servers"][0]["name"], "alpha");
        assert_eq!(list["servers"][0]["transport"]["id"], "stdio");
        assert_eq!(list["servers"][0]["details"]["command"], "uvx");
        assert_eq!(list["servers"][1]["name"], "remote");
        assert_eq!(list["servers"][1]["scope"]["id"], "local");
        assert_eq!(list["servers"][1]["transport"]["id"], "ws");
        assert_eq!(
            list["servers"][1]["details"]["url"],
            "wss://remote.example/mcp"
        );

        let show = render_mcp_report_json_for(&loader, &workspace, Some("show alpha"))
            .expect("mcp show json render");
        assert_eq!(show["action"], "show");
        assert_eq!(show["found"], true);
        assert_eq!(show["server"]["name"], "alpha");
        assert_eq!(show["server"]["details"]["env_keys"][0], "ALPHA_TOKEN");
        assert_eq!(show["server"]["details"]["tool_call_timeout_ms"], 1200);

        let missing = render_mcp_report_json_for(&loader, &workspace, Some("show missing"))
            .expect("mcp missing json render");
        assert_eq!(missing["found"], false);
        assert_eq!(missing["server_name"], "missing");

        let help =
            render_mcp_report_json_for(&loader, &workspace, Some("help")).expect("mcp help json");
        assert_eq!(help["action"], "help");
        assert_eq!(help["usage"]["sources"][0], ".claw/settings.json");

        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(config_home);
    }

    #[test]
    fn rejects_invalid_mcp_arguments() {
        let show_error = parse_error_message("/mcp show alpha beta");
        assert!(show_error.contains("Unexpected arguments for /mcp show."));
        assert!(show_error.contains("  Usage            /mcp show <server>"));

        let action_error = parse_error_message("/mcp inspect alpha");
        assert!(action_error
            .contains("Unknown /mcp action 'inspect'. Use list, show <server>, or help."));
        assert!(action_error.contains("  Usage            /mcp [list|show <server>|help]"));
    }

    #[test]
    fn renders_help_from_shared_specs() {
        let help = render_slash_command_help();
        assert!(help.contains("Start here        /status, /diff, /agents, /skills, /commit"));
        assert!(help.contains("[resume]          also works with --resume SESSION.jsonl"));
        assert!(help.contains("Session"));
        assert!(help.contains("Tools"));
        assert!(help.contains("Config"));
        assert!(help.contains("Debug"));
        assert!(help.contains("/help"));
        assert!(help.contains("/status"));
        assert!(help.contains("/sandbox"));
        assert!(help.contains("/compact"));
        assert!(help.contains("/bughunter [scope]"));
        assert!(help.contains("/commit"));
        assert!(help.contains("/pr [context]"));
        assert!(help.contains("/issue [context]"));
        assert!(help.contains("/ultraplan [task]"));
        assert!(help.contains("/teleport <symbol-or-path>"));
        assert!(help.contains("/debug-tool-call"));
        assert!(help.contains("/model [model]"));
        assert!(help.contains("/permissions [read-only|workspace-write|danger-full-access]"));
        assert!(help.contains("/clear [--confirm]"));
        assert!(help.contains("/cost"));
        assert!(help.contains("/resume <session-path>"));
        assert!(help.contains("/config [env|hooks|model|provider|plugins]"));
        assert!(help.contains("/mcp [list|show <server>|help]"));
        assert!(help.contains("/memory"));
        assert!(help.contains("/init"));
        assert!(help.contains("/diff"));
        assert!(help.contains("/version"));
        assert!(help.contains("/export [file]"));
        assert!(help.contains("/session"), "help must mention /session");
        assert!(help.contains("/sandbox"));
        assert!(help.contains(
            "/plugin [list|install <path>|enable <name>|disable <name>|uninstall <id>|update <id>]"
        ));
        assert!(help.contains("aliases: /plugins, /marketplace"));
        assert!(help.contains("/agents [list|help]"));
        assert!(help.contains("/skills [list|install <path>|help]"));
        assert_eq!(slash_command_specs().len(), 141);
        assert!(resume_supported_slash_commands().len() >= 39);
    }

    #[test]
    fn renders_help_with_grouped_categories_and_keyboard_shortcuts() {
        // given
        let categories = ["Session", "Tools", "Config", "Debug"];

        // when
        let help = render_slash_command_help();

        // then
        for category in categories {
            assert!(
                help.contains(category),
                "expected help to contain category {category}"
            );
        }
        let session_index = help.find("Session").expect("Session header should exist");
        let tools_index = help.find("Tools").expect("Tools header should exist");
        let config_index = help.find("Config").expect("Config header should exist");
        let debug_index = help.find("Debug").expect("Debug header should exist");
        assert!(session_index < tools_index);
        assert!(tools_index < config_index);
        assert!(config_index < debug_index);

        assert!(help.contains("Keyboard shortcuts"));
        assert!(help.contains("Up/Down              Navigate prompt history"));
        assert!(help.contains("Tab                  Complete commands, modes, and recent sessions"));
        assert!(help.contains("Ctrl-C               Clear input (or exit on empty prompt)"));
        assert!(help.contains("Shift+Enter/Ctrl+J   Insert a newline"));

        // every command should still render with a summary line
        for spec in slash_command_specs() {
            let usage = match spec.argument_hint {
                Some(hint) => format!("/{} {hint}", spec.name),
                None => format!("/{}", spec.name),
            };
            assert!(
                help.contains(&usage),
                "expected help to contain command {usage}"
            );
            assert!(
                help.contains(spec.summary),
                "expected help to contain summary for /{}",
                spec.name
            );
        }
    }

    #[test]
    fn renders_per_command_help_detail() {
        // given
        let command = "plugins";

        // when
        let help = render_slash_command_help_detail(command).expect("detail help should exist");

        // then
        assert!(help.contains("/plugin"));
        assert!(help.contains("Summary          Manage Claw Code plugins"));
        assert!(help.contains("Aliases          /plugins, /marketplace"));
        assert!(help.contains("Category         Tools"));
    }

    #[test]
    fn renders_per_command_help_detail_for_mcp() {
        let help = render_slash_command_help_detail("mcp").expect("detail help should exist");
        assert!(help.contains("/mcp"));
        assert!(help.contains("Summary          Inspect configured MCP servers"));
        assert!(help.contains("Category         Tools"));
        assert!(help.contains("Resume           Supported with --resume SESSION.jsonl"));
    }

    #[test]
    fn renders_status_help_with_repo_snapshot_summary() {
        let help = render_slash_command_help_detail("status").expect("detail help should exist");
        assert!(help.contains("/status"));
        assert!(help.contains(
            "Summary          Show current session status with branch freshness, worktrees, and recent commits"
        ));
        assert!(help.contains("Resume           Supported with --resume SESSION.jsonl"));
    }

    #[test]
    fn validate_slash_command_input_rejects_extra_single_value_arguments() {
        // given
        let session_input = "/session switch current next";
        let plugin_input = "/plugin enable demo extra";

        // when
        let session_error = validate_slash_command_input(session_input)
            .expect_err("session input should be rejected")
            .to_string();
        let plugin_error = validate_slash_command_input(plugin_input)
            .expect_err("plugin input should be rejected")
            .to_string();

        // then
        assert!(session_error.contains("Unexpected arguments for /session switch."));
        assert!(session_error.contains("  Usage            /session switch <session-id>"));
        assert!(plugin_error.contains("Unexpected arguments for /plugin enable."));
        assert!(plugin_error.contains("  Usage            /plugin enable <name>"));
    }

    #[test]
    fn suggests_closest_slash_commands_for_typos_and_aliases() {
        let suggestions = suggest_slash_commands("stats", 3);
        assert!(suggestions.contains(&"/stats".to_string()));
        assert!(suggestions.contains(&"/status".to_string()));
        assert!(suggestions.len() <= 3);
        let plugin_suggestions = suggest_slash_commands("/plugns", 3);
        assert!(plugin_suggestions.contains(&"/plugin".to_string()));
        assert_eq!(suggest_slash_commands("zzz", 3), Vec::<String>::new());
    }

    #[test]
    fn compacts_sessions_via_slash_command() {
        let mut session = Session::new();
        session.messages = vec![
            ConversationMessage::user_text("a ".repeat(200)),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "b ".repeat(200),
            }]),
            ConversationMessage::tool_result("1", "bash", "ok ".repeat(200), false),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "recent".to_string(),
            }]),
        ];

        let mut compacted_session = session.clone();
        compacted_session.messages = vec![
            ConversationMessage::compact_boundary(CompactBoundaryMetadata {
                trigger: CompactTrigger::Manual,
                pre_tokens: 321,
                user_context: None,
                messages_summarized: Some(2),
                pre_compact_discovered_tools: vec!["bash".to_string()],
                preserved_segment: None,
            }),
            ConversationMessage::compact_summary_user_text(
                "This session is being continued from a previous conversation.\nSummary:\nCarry over context.",
                true,
            ),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "recent".to_string(),
            }]),
        ];

        let result = handle_slash_command_with_compactor(
            "/compact",
            &session,
            CompactionConfig {
                preserve_recent_messages: 2,
                max_estimated_tokens: 1,
            },
            &mut |input_session, _compaction| {
                assert_eq!(input_session.messages.len(), 4);
                Ok(CompactionResult {
                    summary: "Carry over context.".to_string(),
                    formatted_summary: "formatted".to_string(),
                    compacted_session: compacted_session.clone(),
                    removed_message_count: 2,
                    user_display_message: Some("post compact note".to_string()),
                })
            },
        )
        .expect("slash command should be handled");

        assert!(result.message.contains("full compact summary"));
        assert!(result.message.contains("post compact note"));
        assert_eq!(result.session.messages[0].role, MessageRole::System);
    }

    #[test]
    fn help_command_is_non_mutating() {
        let session = Session::new();
        let result = handle_slash_command("/help", &session, CompactionConfig::default())
            .expect("help command should be handled");
        assert_eq!(result.session, session);
        assert!(result.message.contains("Slash commands"));
    }

    #[test]
    fn ignores_unknown_or_runtime_bound_slash_commands() {
        let session = Session::new();
        assert!(handle_slash_command("/unknown", &session, CompactionConfig::default()).is_none());
        assert!(handle_slash_command("/status", &session, CompactionConfig::default()).is_none());
        assert!(handle_slash_command("/sandbox", &session, CompactionConfig::default()).is_none());
        assert!(
            handle_slash_command("/bughunter", &session, CompactionConfig::default()).is_none()
        );
        assert!(handle_slash_command("/commit", &session, CompactionConfig::default()).is_none());
        assert!(handle_slash_command("/pr", &session, CompactionConfig::default()).is_none());
        assert!(handle_slash_command("/issue", &session, CompactionConfig::default()).is_none());
        assert!(
            handle_slash_command("/ultraplan", &session, CompactionConfig::default()).is_none()
        );
        assert!(
            handle_slash_command("/teleport foo", &session, CompactionConfig::default()).is_none()
        );
        assert!(
            handle_slash_command("/debug-tool-call", &session, CompactionConfig::default())
                .is_none()
        );
        assert!(
            handle_slash_command("/model claude", &session, CompactionConfig::default()).is_none()
        );
        assert!(handle_slash_command(
            "/permissions read-only",
            &session,
            CompactionConfig::default()
        )
        .is_none());
        assert!(handle_slash_command("/clear", &session, CompactionConfig::default()).is_none());
        assert!(
            handle_slash_command("/clear --confirm", &session, CompactionConfig::default())
                .is_none()
        );
        assert!(handle_slash_command("/cost", &session, CompactionConfig::default()).is_none());
        assert!(handle_slash_command(
            "/resume session.json",
            &session,
            CompactionConfig::default()
        )
        .is_none());
        assert!(handle_slash_command(
            "/resume session.jsonl",
            &session,
            CompactionConfig::default()
        )
        .is_none());
        assert!(handle_slash_command("/config", &session, CompactionConfig::default()).is_none());
        assert!(
            handle_slash_command("/config env", &session, CompactionConfig::default()).is_none()
        );
        assert!(handle_slash_command("/mcp list", &session, CompactionConfig::default()).is_none());
        assert!(handle_slash_command("/diff", &session, CompactionConfig::default()).is_none());
        assert!(handle_slash_command("/version", &session, CompactionConfig::default()).is_none());
        assert!(
            handle_slash_command("/export note.txt", &session, CompactionConfig::default())
                .is_none()
        );
        assert!(
            handle_slash_command("/session list", &session, CompactionConfig::default()).is_none()
        );
        assert!(
            handle_slash_command("/plugins list", &session, CompactionConfig::default()).is_none()
        );
    }

    #[test]
    fn renders_plugins_report_with_name_version_and_status() {
        let rendered = render_plugins_report(&[
            PluginSummary {
                metadata: PluginMetadata {
                    id: "demo@external".to_string(),
                    name: "demo".to_string(),
                    version: "1.2.3".to_string(),
                    description: "demo plugin".to_string(),
                    kind: PluginKind::External,
                    source: "demo".to_string(),
                    default_enabled: false,
                    root: None,
                },
                enabled: true,
            },
            PluginSummary {
                metadata: PluginMetadata {
                    id: "sample@external".to_string(),
                    name: "sample".to_string(),
                    version: "0.9.0".to_string(),
                    description: "sample plugin".to_string(),
                    kind: PluginKind::External,
                    source: "sample".to_string(),
                    default_enabled: false,
                    root: None,
                },
                enabled: false,
            },
        ]);

        assert!(rendered.contains("demo"));
        assert!(rendered.contains("v1.2.3"));
        assert!(rendered.contains("enabled"));
        assert!(rendered.contains("sample"));
        assert!(rendered.contains("v0.9.0"));
        assert!(rendered.contains("disabled"));
    }

    #[test]
    fn lists_agents_from_project_and_user_roots() {
        let workspace = temp_dir("agents-workspace");
        let project_agents = workspace.join(".codex").join("agents");
        let user_home = temp_dir("agents-home");
        let user_agents = user_home.join(".claude").join("agents");

        write_agent(
            &project_agents,
            "planner",
            "Project planner",
            "gpt-5.4",
            "medium",
        );
        write_agent(
            &user_agents,
            "planner",
            "User planner",
            "gpt-5.4-mini",
            "high",
        );
        write_agent(
            &user_agents,
            "verifier",
            "Verification agent",
            "gpt-5.4-mini",
            "high",
        );

        let roots = vec![
            (DefinitionSource::ProjectCodex, project_agents),
            (DefinitionSource::UserCodex, user_agents),
        ];
        let report =
            render_agents_report(&load_agents_from_roots(&roots).expect("agent roots should load"));

        assert!(report.contains("Agents"));
        assert!(report.contains("2 active agents"));
        assert!(report.contains("Project (.claw):"));
        assert!(report.contains("planner · Project planner · gpt-5.4 · medium"));
        assert!(report.contains("User (~/.claw):"));
        assert!(report.contains("(shadowed by Project (.claw)) planner · User planner"));
        assert!(report.contains("verifier · Verification agent · gpt-5.4-mini · high"));

        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(user_home);
    }

    #[test]
    fn renders_agents_reports_as_json() {
        let workspace = temp_dir("agents-json-workspace");
        let project_agents = workspace.join(".codex").join("agents");
        let user_home = temp_dir("agents-json-home");
        let user_agents = user_home.join(".codex").join("agents");

        write_agent(
            &project_agents,
            "planner",
            "Project planner",
            "gpt-5.4",
            "medium",
        );
        write_agent(
            &project_agents,
            "verifier",
            "Verification agent",
            "gpt-5.4-mini",
            "high",
        );
        write_agent(
            &user_agents,
            "planner",
            "User planner",
            "gpt-5.4-mini",
            "high",
        );

        let roots = vec![
            (DefinitionSource::ProjectCodex, project_agents),
            (DefinitionSource::UserCodex, user_agents),
        ];
        let report = render_agents_report_json(
            &workspace,
            &load_agents_from_roots(&roots).expect("agent roots should load"),
        );

        assert_eq!(report["kind"], "agents");
        assert_eq!(report["action"], "list");
        assert_eq!(report["working_directory"], workspace.display().to_string());
        assert_eq!(report["count"], 3);
        assert_eq!(report["summary"]["active"], 2);
        assert_eq!(report["summary"]["shadowed"], 1);
        assert_eq!(report["agents"][0]["name"], "planner");
        assert_eq!(report["agents"][0]["model"], "gpt-5.4");
        assert_eq!(report["agents"][0]["active"], true);
        assert_eq!(report["agents"][1]["name"], "verifier");
        assert_eq!(report["agents"][2]["name"], "planner");
        assert_eq!(report["agents"][2]["active"], false);
        assert_eq!(report["agents"][2]["shadowed_by"]["id"], "project_claw");

        let help = handle_agents_slash_command_json(Some("help"), &workspace).expect("agents help");
        assert_eq!(help["kind"], "agents");
        assert_eq!(help["action"], "help");
        assert_eq!(help["usage"]["direct_cli"], "claw agents [list|help]");

        let unexpected = handle_agents_slash_command_json(Some("show planner"), &workspace)
            .expect("agents usage");
        assert_eq!(unexpected["action"], "help");
        assert_eq!(unexpected["unexpected"], "show planner");

        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(user_home);
    }

    #[test]
    fn lists_skills_from_project_and_user_roots() {
        let workspace = temp_dir("skills-workspace");
        let project_skills = workspace.join(".codex").join("skills");
        let project_commands = workspace.join(".claude").join("commands");
        let user_home = temp_dir("skills-home");
        let user_skills = user_home.join(".codex").join("skills");

        write_skill(&project_skills, "plan", "Project planning guidance");
        write_legacy_command(&project_commands, "deploy", "Legacy deployment guidance");
        write_skill(&user_skills, "plan", "User planning guidance");
        write_skill(&user_skills, "help", "Help guidance");

        let roots = vec![
            SkillRoot {
                source: DefinitionSource::ProjectCodex,
                path: project_skills,
                origin: SkillOrigin::SkillsDir,
            },
            SkillRoot {
                source: DefinitionSource::ProjectClaude,
                path: project_commands,
                origin: SkillOrigin::LegacyCommandsDir,
            },
            SkillRoot {
                source: DefinitionSource::UserCodex,
                path: user_skills,
                origin: SkillOrigin::SkillsDir,
            },
        ];
        let report =
            render_skills_report(&load_skills_from_roots(&roots).expect("skill roots should load"));

        assert!(report.contains("Skills"));
        assert!(report.contains("7 available skills"));
        assert!(report.contains("Bundled:"));
        assert!(report.contains("verify"));
        assert!(report.contains("Verify a code change does what it should by running the app."));
        assert!(report.contains("Project (.claw):"));
        assert!(report.contains("plan · Project planning guidance"));
        assert!(report.contains("deploy · Legacy deployment guidance · legacy /commands"));
        assert!(report.contains("User (~/.claw):"));
        assert!(report.contains("(shadowed by Project (.claw)) plan · User planning guidance"));
        assert!(report.contains("help · Help guidance"));

        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(user_home);
    }

    #[test]
    fn agents_and_skills_usage_support_help_and_unexpected_args() {
        let cwd = temp_dir("slash-usage");

        let agents_help =
            super::handle_agents_slash_command(Some("help"), &cwd).expect("agents help");
        assert!(agents_help.contains("Usage            /agents [list|help]"));
        assert!(agents_help.contains("Direct CLI       claw agents"));
        assert!(agents_help
            .contains("Sources          .claw/agents, ~/.claw/agents, $CLAW_CONFIG_HOME/agents"));

        let agents_unexpected =
            super::handle_agents_slash_command(Some("show planner"), &cwd).expect("agents usage");
        assert!(agents_unexpected.contains("Unexpected       show planner"));

        let skills_help =
            super::handle_skills_slash_command(Some("--help"), &cwd).expect("skills help");
        assert!(skills_help
            .contains("Usage            /skills [list|install <path>|help|<skill> [args]]"));
        assert!(skills_help.contains("Invoke           /skills help overview -> $help overview"));
        assert!(skills_help.contains("Install root     $CLAW_CONFIG_HOME/skills or ~/.claw/skills"));
        assert!(skills_help.contains("bundled built-ins"));
        assert!(skills_help.contains("legacy /commands"));

        let skills_unexpected =
            super::handle_skills_slash_command(Some("show help"), &cwd).expect("skills usage");
        assert!(skills_unexpected.contains("Unexpected       show"));

        let skills_install_help = super::handle_skills_slash_command(Some("install --help"), &cwd)
            .expect("nested skills help");
        assert!(skills_install_help
            .contains("Usage            /skills [list|install <path>|help|<skill> [args]]"));
        assert!(skills_install_help.contains("Unexpected       install"));

        let skills_unknown_help =
            super::handle_skills_slash_command(Some("show --help"), &cwd).expect("skills help");
        assert!(skills_unknown_help
            .contains("Usage            /skills [list|install <path>|help|<skill> [args]]"));
        assert!(skills_unknown_help.contains("Unexpected       show"));

        let _ = fs::remove_dir_all(cwd);
    }

    #[test]
    fn mcp_usage_supports_help_and_unexpected_args() {
        let cwd = temp_dir("mcp-usage");

        let help = super::handle_mcp_slash_command(Some("help"), &cwd).expect("mcp help");
        assert!(help.contains("Usage            /mcp [list|show <server>|help]"));
        assert!(help.contains("Direct CLI       claw mcp [list|show <server>|help]"));

        let unexpected =
            super::handle_mcp_slash_command(Some("show alpha beta"), &cwd).expect("mcp usage");
        assert!(unexpected.contains("Unexpected       show alpha beta"));

        let _ = fs::remove_dir_all(cwd);
    }

    #[test]
    fn renders_mcp_reports_from_loaded_config() {
        let workspace = temp_dir("mcp-config-workspace");
        let config_home = temp_dir("mcp-config-home");
        fs::create_dir_all(workspace.join(".claw")).expect("workspace config dir");
        fs::create_dir_all(&config_home).expect("config home");
        fs::write(
            workspace.join(".claw").join("settings.json"),
            r#"{
              "mcpServers": {
                "alpha": {
                  "command": "uvx",
                  "args": ["alpha-server"],
                  "env": {"ALPHA_TOKEN": "secret"},
                  "toolCallTimeoutMs": 1200
                },
                "remote": {
                  "type": "http",
                  "url": "https://remote.example/mcp",
                  "headers": {"Authorization": "Bearer secret"},
                  "headersHelper": "./bin/headers",
                  "oauth": {
                    "clientId": "remote-client",
                    "callbackPort": 7878
                  }
                }
              }
            }"#,
        )
        .expect("write settings");
        fs::write(
            workspace.join(".claw").join("settings.local.json"),
            r#"{
              "mcpServers": {
                "remote": {
                  "type": "ws",
                  "url": "wss://remote.example/mcp"
                }
              }
            }"#,
        )
        .expect("write local settings");

        let loader = ConfigLoader::new(&workspace, &config_home);
        let list = super::render_mcp_report_for(&loader, &workspace, None)
            .expect("mcp list report should render");
        assert!(list.contains("Configured servers 2"));
        assert!(list.contains("alpha"));
        assert!(list.contains("stdio"));
        assert!(list.contains("project"));
        assert!(list.contains("uvx alpha-server"));
        assert!(list.contains("remote"));
        assert!(list.contains("ws"));
        assert!(list.contains("local"));
        assert!(list.contains("wss://remote.example/mcp"));

        let show = super::render_mcp_report_for(&loader, &workspace, Some("show alpha"))
            .expect("mcp show report should render");
        assert!(show.contains("Name              alpha"));
        assert!(show.contains("Command           uvx"));
        assert!(show.contains("Args              alpha-server"));
        assert!(show.contains("Env keys          ALPHA_TOKEN"));
        assert!(show.contains("Tool timeout      1200 ms"));

        let remote = super::render_mcp_report_for(&loader, &workspace, Some("show remote"))
            .expect("mcp show remote report should render");
        assert!(remote.contains("Transport         ws"));
        assert!(remote.contains("URL               wss://remote.example/mcp"));

        let missing = super::render_mcp_report_for(&loader, &workspace, Some("show missing"))
            .expect("missing report should render");
        assert!(missing.contains("server `missing` is not configured"));

        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(config_home);
    }

    #[test]
    fn parses_quoted_skill_frontmatter_values() {
        let contents = "---\nname: \"hud\"\ndescription: 'Quoted description'\n---\n";
        let document = super::parse_skill_document(contents, "hud".to_string(), "Skill");
        assert_eq!(document.display_name.as_deref(), Some("hud"));
        assert_eq!(document.description, "Quoted description");
        assert!(document.has_user_specified_description);
    }

    #[test]
    fn installs_skill_into_user_registry_and_preserves_nested_files() {
        let workspace = temp_dir("skills-install-workspace");
        let source_root = workspace.join("source").join("help");
        let install_root = temp_dir("skills-install-root");
        write_skill(
            source_root.parent().expect("parent"),
            "help",
            "Helpful skill",
        );
        let script_dir = source_root.join("scripts");
        fs::create_dir_all(&script_dir).expect("script dir");
        fs::write(script_dir.join("run.sh"), "#!/bin/sh\necho help\n").expect("write script");

        let installed = super::install_skill_into(
            source_root.to_str().expect("utf8 skill path"),
            &workspace,
            &install_root,
        )
        .expect("skill should install");

        assert_eq!(installed.invocation_name, "help");
        assert_eq!(installed.display_name.as_deref(), Some("help"));
        assert!(installed.installed_path.ends_with(Path::new("help")));
        assert!(installed.installed_path.join("SKILL.md").is_file());
        assert!(installed
            .installed_path
            .join("scripts")
            .join("run.sh")
            .is_file());

        let report = super::render_skill_install_report(&installed);
        assert!(report.contains("Result           installed help"));
        assert!(report.contains("Invoke as        $help"));
        assert!(report.contains(&install_root.display().to_string()));

        let roots = vec![SkillRoot {
            source: DefinitionSource::UserCodexHome,
            path: install_root.clone(),
            origin: SkillOrigin::SkillsDir,
        }];
        let listed = render_skills_report(
            &load_skills_from_roots(&roots).expect("installed skills should load"),
        );
        assert!(listed.contains("User ($CLAW_CONFIG_HOME):"));
        assert!(listed.contains("help · Helpful skill"));

        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(install_root);
    }

    #[test]
    fn installs_plugin_from_path_and_lists_it() {
        let config_home = temp_dir("home");
        let source_root = temp_dir("source");
        write_external_plugin(&source_root, "demo", "1.0.0");

        let mut manager = PluginManager::new(PluginManagerConfig::new(&config_home));
        let install = handle_plugins_slash_command(
            Some("install"),
            Some(source_root.to_str().expect("utf8 path")),
            &mut manager,
        )
        .expect("install command should succeed");
        assert!(install.reload_runtime);
        assert!(install.message.contains("installed demo@external"));
        assert!(install.message.contains("Name             demo"));
        assert!(install.message.contains("Version          1.0.0"));
        assert!(install.message.contains("Status           enabled"));

        let list = handle_plugins_slash_command(Some("list"), None, &mut manager)
            .expect("list command should succeed");
        assert!(!list.reload_runtime);
        assert!(list.message.contains("demo"));
        assert!(list.message.contains("v1.0.0"));
        assert!(list.message.contains("enabled"));

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn enables_and_disables_plugin_by_name() {
        let config_home = temp_dir("toggle-home");
        let source_root = temp_dir("toggle-source");
        write_external_plugin(&source_root, "demo", "1.0.0");

        let mut manager = PluginManager::new(PluginManagerConfig::new(&config_home));
        handle_plugins_slash_command(
            Some("install"),
            Some(source_root.to_str().expect("utf8 path")),
            &mut manager,
        )
        .expect("install command should succeed");

        let disable = handle_plugins_slash_command(Some("disable"), Some("demo"), &mut manager)
            .expect("disable command should succeed");
        assert!(disable.reload_runtime);
        assert!(disable.message.contains("disabled demo@external"));
        assert!(disable.message.contains("Name             demo"));
        assert!(disable.message.contains("Status           disabled"));

        let list = handle_plugins_slash_command(Some("list"), None, &mut manager)
            .expect("list command should succeed");
        assert!(list.message.contains("demo"));
        assert!(list.message.contains("disabled"));

        let enable = handle_plugins_slash_command(Some("enable"), Some("demo"), &mut manager)
            .expect("enable command should succeed");
        assert!(enable.reload_runtime);
        assert!(enable.message.contains("enabled demo@external"));
        assert!(enable.message.contains("Name             demo"));
        assert!(enable.message.contains("Status           enabled"));

        let list = handle_plugins_slash_command(Some("list"), None, &mut manager)
            .expect("list command should succeed");
        assert!(list.message.contains("demo"));
        assert!(list.message.contains("enabled"));

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn lists_auto_installed_bundled_plugins_with_status() {
        let config_home = temp_dir("bundled-home");
        let bundled_root = temp_dir("bundled-root");
        let bundled_plugin = bundled_root.join("starter");
        write_bundled_plugin(&bundled_plugin, "starter", "0.1.0", false);

        let mut config = PluginManagerConfig::new(&config_home);
        config.bundled_root = Some(bundled_root.clone());
        let mut manager = PluginManager::new(config);

        let list = handle_plugins_slash_command(Some("list"), None, &mut manager)
            .expect("list command should succeed");
        assert!(!list.reload_runtime);
        assert!(list.message.contains("starter"));
        assert!(list.message.contains("v0.1.0"));
        assert!(list.message.contains("disabled"));

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(bundled_root);
    }
}
