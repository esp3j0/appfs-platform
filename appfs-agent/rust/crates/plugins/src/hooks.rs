use std::ffi::OsStr;
use std::process::Command;

use serde_json::json;

use crate::shell::prepare_hook_command;
use crate::{PluginError, PluginHooks, PluginRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    PreCompact,
    PostCompact,
    SessionStart,
}

impl HookEvent {
    fn as_str(self) -> &'static str {
        match self {
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::PostToolUseFailure => "PostToolUseFailure",
            Self::PreCompact => "PreCompact",
            Self::PostCompact => "PostCompact",
            Self::SessionStart => "SessionStart",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRunResult {
    denied: bool,
    failed: bool,
    messages: Vec<String>,
}

impl HookRunResult {
    #[must_use]
    pub fn allow(messages: Vec<String>) -> Self {
        Self {
            denied: false,
            failed: false,
            messages,
        }
    }

    #[must_use]
    pub fn is_denied(&self) -> bool {
        self.denied
    }

    #[must_use]
    pub fn is_failed(&self) -> bool {
        self.failed
    }

    #[must_use]
    pub fn messages(&self) -> &[String] {
        &self.messages
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreCompactHookResult {
    new_custom_instructions: Option<String>,
    user_display_message: Option<String>,
}

impl PreCompactHookResult {
    #[must_use]
    pub fn new_custom_instructions(&self) -> Option<&str> {
        self.new_custom_instructions.as_deref()
    }

    #[must_use]
    pub fn user_display_message(&self) -> Option<&str> {
        self.user_display_message.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PostCompactHookResult {
    user_display_message: Option<String>,
}

impl PostCompactHookResult {
    #[must_use]
    pub fn user_display_message(&self) -> Option<&str> {
        self.user_display_message.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContextHookCommandResult {
    command: String,
    succeeded: bool,
    output: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HookRunner {
    hooks: PluginHooks,
}

impl HookRunner {
    #[must_use]
    pub fn new(hooks: PluginHooks) -> Self {
        Self { hooks }
    }

    pub fn from_registry(plugin_registry: &PluginRegistry) -> Result<Self, PluginError> {
        Ok(Self::new(plugin_registry.aggregated_hooks()?))
    }

    #[must_use]
    pub fn run_pre_tool_use(&self, tool_name: &str, tool_input: &str) -> HookRunResult {
        Self::run_commands(
            HookEvent::PreToolUse,
            &self.hooks.pre_tool_use,
            tool_name,
            tool_input,
            None,
            false,
        )
    }

    #[must_use]
    pub fn run_post_tool_use(
        &self,
        tool_name: &str,
        tool_input: &str,
        tool_output: &str,
        is_error: bool,
    ) -> HookRunResult {
        Self::run_commands(
            HookEvent::PostToolUse,
            &self.hooks.post_tool_use,
            tool_name,
            tool_input,
            Some(tool_output),
            is_error,
        )
    }

    #[must_use]
    pub fn run_post_tool_use_failure(
        &self,
        tool_name: &str,
        tool_input: &str,
        tool_error: &str,
    ) -> HookRunResult {
        Self::run_commands(
            HookEvent::PostToolUseFailure,
            &self.hooks.post_tool_use_failure,
            tool_name,
            tool_input,
            Some(tool_error),
            true,
        )
    }

    #[must_use]
    pub fn run_pre_compact(
        &self,
        trigger: &str,
        custom_instructions: Option<&str>,
    ) -> PreCompactHookResult {
        let command_results = Self::run_contextual_commands(
            HookEvent::PreCompact,
            &self.hooks.pre_compact,
            "compact",
            &hook_pre_compact_payload(trigger, custom_instructions).to_string(),
            |child| {
                child.env("HOOK_TRIGGER", trigger);
                if let Some(custom_instructions) = custom_instructions {
                    child.env("HOOK_CUSTOM_INSTRUCTIONS", custom_instructions);
                }
            },
        );
        build_pre_compact_hook_result(&command_results)
    }

    #[must_use]
    pub fn run_post_compact(&self, trigger: &str, compact_summary: &str) -> PostCompactHookResult {
        let command_results = Self::run_contextual_commands(
            HookEvent::PostCompact,
            &self.hooks.post_compact,
            "compact",
            &hook_post_compact_payload(trigger, compact_summary).to_string(),
            |child| {
                child.env("HOOK_TRIGGER", trigger);
                child.env("HOOK_COMPACT_SUMMARY", compact_summary);
            },
        );
        build_post_compact_hook_result(&command_results)
    }

    #[must_use]
    pub fn run_session_start(&self, source: &str, model: Option<&str>) -> HookRunResult {
        Self::run_session_start_commands(&self.hooks.session_start, source, model)
    }

    fn run_commands(
        event: HookEvent,
        commands: &[String],
        tool_name: &str,
        tool_input: &str,
        tool_output: Option<&str>,
        is_error: bool,
    ) -> HookRunResult {
        if commands.is_empty() {
            return HookRunResult::allow(Vec::new());
        }

        let payload = hook_payload(event, tool_name, tool_input, tool_output, is_error).to_string();

        let mut messages = Vec::new();

        for command in commands {
            match Self::run_command(
                command,
                event,
                tool_name,
                tool_input,
                tool_output,
                is_error,
                &payload,
            ) {
                HookCommandOutcome::Allow { message } => {
                    if let Some(message) = message {
                        messages.push(message);
                    }
                }
                HookCommandOutcome::Deny { message } => {
                    messages.push(message.unwrap_or_else(|| {
                        format!("{} hook denied tool `{tool_name}`", event.as_str())
                    }));
                    return HookRunResult {
                        denied: true,
                        failed: false,
                        messages,
                    };
                }
                HookCommandOutcome::Failed { message } => {
                    messages.push(message);
                    return HookRunResult {
                        denied: false,
                        failed: true,
                        messages,
                    };
                }
            }
        }

        HookRunResult::allow(messages)
    }

    fn run_contextual_commands<F>(
        event: HookEvent,
        commands: &[String],
        context_name: &str,
        payload: &str,
        mut configure_env: F,
    ) -> Vec<ContextHookCommandResult>
    where
        F: FnMut(&mut CommandWithStdin),
    {
        commands
            .iter()
            .map(|command| {
                Self::run_contextual_command(
                    command,
                    event,
                    context_name,
                    payload,
                    &mut configure_env,
                )
            })
            .collect()
    }

    fn run_session_start_commands(
        commands: &[String],
        source: &str,
        model: Option<&str>,
    ) -> HookRunResult {
        if commands.is_empty() {
            return HookRunResult::allow(Vec::new());
        }

        let payload = hook_session_start_payload(source, model).to_string();
        let mut messages = Vec::new();

        for command in commands {
            match Self::run_session_start_command(command, source, model, &payload) {
                HookCommandOutcome::Allow { message } => {
                    if let Some(message) = message {
                        messages.push(message);
                    }
                }
                HookCommandOutcome::Deny { message } => {
                    messages.push(
                        message.unwrap_or_else(|| format!("SessionStart hook denied `{source}`")),
                    );
                    return HookRunResult {
                        denied: true,
                        failed: false,
                        messages,
                    };
                }
                HookCommandOutcome::Failed { message } => {
                    messages.push(message);
                    return HookRunResult {
                        denied: false,
                        failed: true,
                        messages,
                    };
                }
            }
        }

        HookRunResult::allow(messages)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_command(
        command: &str,
        event: HookEvent,
        tool_name: &str,
        tool_input: &str,
        tool_output: Option<&str>,
        is_error: bool,
        payload: &str,
    ) -> HookCommandOutcome {
        let mut child = match shell_command(command) {
            Ok(child) => child,
            Err(error) => {
                return HookCommandOutcome::Failed {
                    message: format!(
                        "{} hook `{command}` failed to start for `{tool_name}`: {error}",
                        event.as_str()
                    ),
                };
            }
        };
        child.stdin(std::process::Stdio::piped());
        child.stdout(std::process::Stdio::piped());
        child.stderr(std::process::Stdio::piped());
        child.env("HOOK_EVENT", event.as_str());
        child.env("HOOK_TOOL_NAME", tool_name);
        child.env("HOOK_TOOL_INPUT", tool_input);
        child.env("HOOK_TOOL_IS_ERROR", if is_error { "1" } else { "0" });
        if let Some(tool_output) = tool_output {
            child.env("HOOK_TOOL_OUTPUT", tool_output);
        }

        match child.output_with_stdin(payload.as_bytes()) {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let message = (!stdout.is_empty()).then_some(stdout);
                match output.status.code() {
                    Some(0) => HookCommandOutcome::Allow { message },
                    Some(2) => HookCommandOutcome::Deny { message },
                    Some(code) => HookCommandOutcome::Failed {
                        message: format_hook_warning(
                            command,
                            code,
                            message.as_deref(),
                            stderr.as_str(),
                        ),
                    },
                    None => HookCommandOutcome::Failed {
                        message: format!(
                            "{} hook `{command}` terminated by signal while handling `{tool_name}`",
                            event.as_str()
                        ),
                    },
                }
            }
            Err(error) => HookCommandOutcome::Failed {
                message: format!(
                    "{} hook `{command}` failed to start for `{tool_name}`: {error}",
                    event.as_str()
                ),
            },
        }
    }

    fn run_session_start_command(
        command: &str,
        source: &str,
        model: Option<&str>,
        payload: &str,
    ) -> HookCommandOutcome {
        let mut child = match shell_command(command) {
            Ok(child) => child,
            Err(error) => {
                return HookCommandOutcome::Failed {
                    message: format!(
                        "SessionStart hook `{command}` failed to start for `{source}`: {error}"
                    ),
                };
            }
        };
        child.stdin(std::process::Stdio::piped());
        child.stdout(std::process::Stdio::piped());
        child.stderr(std::process::Stdio::piped());
        child.env("HOOK_EVENT", HookEvent::SessionStart.as_str());
        child.env("HOOK_SESSION_SOURCE", source);
        if let Some(model) = model {
            child.env("HOOK_MODEL", model);
        }

        match child.output_with_stdin(payload.as_bytes()) {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let message = (!stdout.is_empty()).then_some(stdout);
                match output.status.code() {
                    Some(0) => HookCommandOutcome::Allow { message },
                    Some(2) => HookCommandOutcome::Deny { message },
                    Some(code) => HookCommandOutcome::Failed {
                        message: format_hook_warning(
                            command,
                            code,
                            message.as_deref(),
                            stderr.as_str(),
                        ),
                    },
                    None => HookCommandOutcome::Failed {
                        message: format!(
                            "SessionStart hook `{command}` terminated by signal while handling `{source}`"
                        ),
                    },
                }
            }
            Err(error) => HookCommandOutcome::Failed {
                message: format!(
                    "SessionStart hook `{command}` failed to start for `{source}`: {error}"
                ),
            },
        }
    }

    fn run_contextual_command<F>(
        command: &str,
        event: HookEvent,
        context_name: &str,
        payload: &str,
        configure_env: &mut F,
    ) -> ContextHookCommandResult
    where
        F: FnMut(&mut CommandWithStdin),
    {
        let mut child = match shell_command(command) {
            Ok(child) => child,
            Err(error) => {
                return ContextHookCommandResult {
                    command: command.to_string(),
                    succeeded: false,
                    output: format!(
                        "{} hook `{command}` failed to start for `{context_name}`: {error}",
                        event.as_str()
                    ),
                };
            }
        };
        child.stdin(std::process::Stdio::piped());
        child.stdout(std::process::Stdio::piped());
        child.stderr(std::process::Stdio::piped());
        child.env("HOOK_EVENT", event.as_str());
        configure_env(&mut child);

        match child.output_with_stdin(payload.as_bytes()) {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let rendered = if stdout.is_empty() {
                    stderr.clone()
                } else {
                    stdout.clone()
                };
                match output.status.code() {
                    Some(0) => ContextHookCommandResult {
                        command: command.to_string(),
                        succeeded: true,
                        output: rendered,
                    },
                    Some(code) => ContextHookCommandResult {
                        command: command.to_string(),
                        succeeded: false,
                        output: format_hook_warning(
                            command,
                            code,
                            (!rendered.is_empty()).then_some(rendered.as_str()),
                            stderr.as_str(),
                        ),
                    },
                    None => ContextHookCommandResult {
                        command: command.to_string(),
                        succeeded: false,
                        output: format!(
                            "{} hook `{command}` terminated by signal while handling `{context_name}`",
                            event.as_str()
                        ),
                    },
                }
            }
            Err(error) => ContextHookCommandResult {
                command: command.to_string(),
                succeeded: false,
                output: format!(
                    "{} hook `{command}` failed to start for `{context_name}`: {error}",
                    event.as_str()
                ),
            },
        }
    }
}

enum HookCommandOutcome {
    Allow { message: Option<String> },
    Deny { message: Option<String> },
    Failed { message: String },
}

fn hook_payload(
    event: HookEvent,
    tool_name: &str,
    tool_input: &str,
    tool_output: Option<&str>,
    is_error: bool,
) -> serde_json::Value {
    match event {
        HookEvent::PostToolUseFailure => json!({
            "hook_event_name": event.as_str(),
            "tool_name": tool_name,
            "tool_input": parse_tool_input(tool_input),
            "tool_input_json": tool_input,
            "tool_error": tool_output,
            "tool_result_is_error": true,
        }),
        _ => json!({
            "hook_event_name": event.as_str(),
            "tool_name": tool_name,
            "tool_input": parse_tool_input(tool_input),
            "tool_input_json": tool_input,
            "tool_output": tool_output,
            "tool_result_is_error": is_error,
        }),
    }
}

fn hook_session_start_payload(source: &str, model: Option<&str>) -> serde_json::Value {
    json!({
        "hook_event_name": HookEvent::SessionStart.as_str(),
        "session_source": source,
        "model": model,
    })
}

fn hook_pre_compact_payload(trigger: &str, custom_instructions: Option<&str>) -> serde_json::Value {
    json!({
        "hook_event_name": HookEvent::PreCompact.as_str(),
        "trigger": trigger,
        "custom_instructions": custom_instructions,
    })
}

fn hook_post_compact_payload(trigger: &str, compact_summary: &str) -> serde_json::Value {
    json!({
        "hook_event_name": HookEvent::PostCompact.as_str(),
        "trigger": trigger,
        "compact_summary": compact_summary,
    })
}

fn parse_tool_input(tool_input: &str) -> serde_json::Value {
    serde_json::from_str(tool_input).unwrap_or_else(|_| json!({ "raw": tool_input }))
}

fn format_hook_warning(command: &str, code: i32, stdout: Option<&str>, stderr: &str) -> String {
    let mut message = format!("Hook `{command}` exited with status {code}");
    if let Some(stdout) = stdout.filter(|stdout| !stdout.is_empty()) {
        message.push_str(": ");
        message.push_str(stdout);
    } else if !stderr.is_empty() {
        message.push_str(": ");
        message.push_str(stderr);
    }
    message
}

fn build_pre_compact_hook_result(
    command_results: &[ContextHookCommandResult],
) -> PreCompactHookResult {
    if command_results.is_empty() {
        return PreCompactHookResult::default();
    }

    let new_custom_instructions = command_results
        .iter()
        .filter(|result| result.succeeded)
        .map(|result| result.output.trim())
        .filter(|output| !output.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    let user_display_message = command_results
        .iter()
        .map(|result| {
            if result.succeeded {
                if result.output.trim().is_empty() {
                    format!("PreCompact [{}] completed successfully", result.command)
                } else {
                    format!(
                        "PreCompact [{}] completed successfully: {}",
                        result.command,
                        result.output.trim()
                    )
                }
            } else if result.output.trim().is_empty() {
                format!("PreCompact [{}] failed", result.command)
            } else {
                format!(
                    "PreCompact [{}] failed: {}",
                    result.command,
                    result.output.trim()
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    PreCompactHookResult {
        new_custom_instructions: (!new_custom_instructions.is_empty())
            .then_some(new_custom_instructions),
        user_display_message: (!user_display_message.is_empty()).then_some(user_display_message),
    }
}

fn build_post_compact_hook_result(
    command_results: &[ContextHookCommandResult],
) -> PostCompactHookResult {
    if command_results.is_empty() {
        return PostCompactHookResult::default();
    }

    let user_display_message = command_results
        .iter()
        .map(|result| {
            if result.succeeded {
                if result.output.trim().is_empty() {
                    format!("PostCompact [{}] completed successfully", result.command)
                } else {
                    format!(
                        "PostCompact [{}] completed successfully: {}",
                        result.command,
                        result.output.trim()
                    )
                }
            } else if result.output.trim().is_empty() {
                format!("PostCompact [{}] failed", result.command)
            } else {
                format!(
                    "PostCompact [{}] failed: {}",
                    result.command,
                    result.output.trim()
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    PostCompactHookResult {
        user_display_message: (!user_display_message.is_empty()).then_some(user_display_message),
    }
}

fn shell_command(command: &str) -> std::io::Result<CommandWithStdin> {
    prepare_hook_command(command).map(CommandWithStdin::new)
}

struct CommandWithStdin {
    command: Command,
}

impl CommandWithStdin {
    fn new(command: Command) -> Self {
        Self { command }
    }

    fn stdin(&mut self, cfg: std::process::Stdio) -> &mut Self {
        self.command.stdin(cfg);
        self
    }

    fn stdout(&mut self, cfg: std::process::Stdio) -> &mut Self {
        self.command.stdout(cfg);
        self
    }

    fn stderr(&mut self, cfg: std::process::Stdio) -> &mut Self {
        self.command.stderr(cfg);
        self
    }

    fn env<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.command.env(key, value);
        self
    }

    fn output_with_stdin(&mut self, stdin: &[u8]) -> std::io::Result<std::process::Output> {
        let mut child = self.command.spawn()?;
        if let Some(mut child_stdin) = child.stdin.take() {
            use std::io::Write as _;
            if let Err(error) = child_stdin.write_all(stdin) {
                if error.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err(error);
                }
            }
        }
        child.wait_with_output()
    }
}

#[cfg(test)]
mod tests {
    use super::{HookRunResult, HookRunner};
    use crate::{PluginManager, PluginManagerConfig};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Clone, Copy)]
    struct HookPluginMessages<'a> {
        pre: &'a str,
        post: &'a str,
        failure: &'a str,
        pre_compact: &'a str,
        post_compact: &'a str,
        session: &'a str,
    }

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("plugins-hook-runner-{label}-{nanos}"))
    }

    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let perms = fs::Permissions::from_mode(0o755);
            fs::set_permissions(path, perms)
                .unwrap_or_else(|error| panic!("chmod +x {}: {error}", path.display()));
        }
        #[cfg(not(unix))]
        let _ = path;
    }

    fn write_hook_plugin(root: &Path, name: &str, messages: HookPluginMessages<'_>) {
        fs::create_dir_all(root.join(".claude-plugin")).expect("manifest dir");
        fs::create_dir_all(root.join("hooks")).expect("hooks dir");
        let pre_hook = hook_script_name("pre");
        let post_hook = hook_script_name("post");
        let pre_path = root.join("hooks").join(&pre_hook);
        fs::write(&pre_path, hook_script_contents(messages.pre)).expect("write pre hook");
        make_executable(&pre_path);

        let post_path = root.join("hooks").join(&post_hook);
        fs::write(&post_path, hook_script_contents(messages.post)).expect("write post hook");
        make_executable(&post_path);

        let failure_path = root.join("hooks").join("failure.sh");
        fs::write(
            &failure_path,
            format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", messages.failure),
        )
        .expect("write failure hook");
        make_executable(&failure_path);
        let pre_compact_path = root.join("hooks").join("pre-compact.sh");
        fs::write(
            &pre_compact_path,
            format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", messages.pre_compact),
        )
        .expect("write pre-compact hook");
        make_executable(&pre_compact_path);
        let post_compact_path = root.join("hooks").join("post-compact.sh");
        fs::write(
            &post_compact_path,
            format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", messages.post_compact),
        )
        .expect("write post-compact hook");
        make_executable(&post_compact_path);
        let session_path = root.join("hooks").join("session.sh");
        fs::write(
            &session_path,
            format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", messages.session),
        )
        .expect("write session hook");
        make_executable(&session_path);
        fs::write(
            root.join(".claude-plugin").join("plugin.json"),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"description\": \"hook plugin\",\n  \"hooks\": {{\n    \"PreToolUse\": [\"./hooks/{pre_hook}\"],\n    \"PostToolUse\": [\"./hooks/{post_hook}\"],\n    \"PostToolUseFailure\": [\"./hooks/failure.sh\"],\n    \"PreCompact\": [\"./hooks/pre-compact.sh\"],\n    \"PostCompact\": [\"./hooks/post-compact.sh\"],\n    \"SessionStart\": [\"./hooks/session.sh\"]\n  }}\n}}"
            ),
        )
        .expect("write plugin manifest");
    }

    #[test]
    fn collects_and_runs_hooks_from_enabled_plugins() {
        if !crate::shell::windows_bash_smoke_ok() {
            return;
        }
        // given
        let config_home = temp_dir("config");
        let first_source_root = temp_dir("source-a");
        let second_source_root = temp_dir("source-b");
        write_hook_plugin(
            &first_source_root,
            "first",
            HookPluginMessages {
                pre: "plugin pre one",
                post: "plugin post one",
                failure: "plugin failure one",
                pre_compact: "plugin pre compact one",
                post_compact: "plugin post compact one",
                session: "plugin session one",
            },
        );
        write_hook_plugin(
            &second_source_root,
            "second",
            HookPluginMessages {
                pre: "plugin pre two",
                post: "plugin post two",
                failure: "plugin failure two",
                pre_compact: "plugin pre compact two",
                post_compact: "plugin post compact two",
                session: "plugin session two",
            },
        );

        let mut manager = PluginManager::new(PluginManagerConfig::new(&config_home));
        manager
            .install(first_source_root.to_str().expect("utf8 path"))
            .expect("first plugin install should succeed");
        manager
            .install(second_source_root.to_str().expect("utf8 path"))
            .expect("second plugin install should succeed");
        let registry = manager.plugin_registry().expect("registry should build");

        // when
        let runner = HookRunner::from_registry(&registry).expect("plugin hooks should load");

        // then
        assert_eq!(
            runner.run_pre_tool_use("Read", r#"{"path":"README.md"}"#),
            HookRunResult::allow(vec![
                "plugin pre one".to_string(),
                "plugin pre two".to_string(),
            ])
        );
        assert_eq!(
            runner.run_post_tool_use("Read", r#"{"path":"README.md"}"#, "ok", false),
            HookRunResult::allow(vec![
                "plugin post one".to_string(),
                "plugin post two".to_string(),
            ])
        );
        assert_eq!(
            runner.run_post_tool_use_failure("Read", r#"{"path":"README.md"}"#, "tool failed",),
            HookRunResult::allow(vec![
                "plugin failure one".to_string(),
                "plugin failure two".to_string(),
            ])
        );
        assert_eq!(
            runner.run_session_start("compact", Some("claude-opus-4-6")),
            HookRunResult::allow(vec![
                "plugin session one".to_string(),
                "plugin session two".to_string(),
            ])
        );
        assert_eq!(
            runner
                .run_pre_compact("manual", None)
                .new_custom_instructions(),
            Some("plugin pre compact one\n\nplugin pre compact two")
        );
        let post_compact = runner.run_post_compact("manual", "summary");
        let post_compact_display = post_compact
            .user_display_message()
            .expect("post-compact display");
        assert!(post_compact_display.contains("plugin post compact one"));
        assert!(post_compact_display.contains("plugin post compact two"));

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(first_source_root);
        let _ = fs::remove_dir_all(second_source_root);
    }

    #[test]
    fn pre_tool_use_denies_when_plugin_hook_exits_two() {
        if !crate::shell::windows_bash_smoke_ok() {
            return;
        }
        // given
        let runner = HookRunner::new(crate::PluginHooks {
            pre_tool_use: vec![shell_echo_and_exit("blocked by plugin", 2)],
            post_tool_use: Vec::new(),
            post_tool_use_failure: Vec::new(),
            pre_compact: Vec::new(),
            post_compact: Vec::new(),
            session_start: Vec::new(),
        });

        // when
        let result = runner.run_pre_tool_use("Bash", r#"{"command":"pwd"}"#);

        // then
        assert!(result.is_denied());
        assert_eq!(result.messages(), &["blocked by plugin".to_string()]);
    }

    #[test]
    fn propagates_plugin_hook_failures() {
        if !crate::shell::windows_bash_smoke_ok() {
            return;
        }
        // given
        let runner = HookRunner::new(crate::PluginHooks {
            pre_tool_use: vec![
                "printf 'broken plugin hook'; exit 1".to_string(),
                "printf 'later plugin hook'".to_string(),
            ],
            post_tool_use: Vec::new(),
            post_tool_use_failure: Vec::new(),
            pre_compact: Vec::new(),
            post_compact: Vec::new(),
            session_start: Vec::new(),
        });

        // when
        let result = runner.run_pre_tool_use("Bash", r#"{"command":"pwd"}"#);

        // then
        assert!(result.is_failed());
        assert!(result
            .messages()
            .iter()
            .any(|message| message.contains("broken plugin hook")));
        assert!(!result
            .messages()
            .iter()
            .any(|message| message == "later plugin hook"));
    }

    #[test]
    #[cfg(unix)]
    fn generated_hook_scripts_are_executable() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_dir("exec-guard");
        write_hook_plugin(
            &root,
            "exec-check",
            HookPluginMessages {
                pre: "pre",
                post: "post",
                failure: "fail",
                pre_compact: "pre compact",
                post_compact: "post compact",
                session: "session",
            },
        );

        for script in [
            "pre.sh",
            "post.sh",
            "failure.sh",
            "pre-compact.sh",
            "post-compact.sh",
            "session.sh",
        ] {
            let path = root.join("hooks").join(script);
            let mode = fs::metadata(&path)
                .unwrap_or_else(|error| panic!("{script} metadata: {error}"))
                .permissions()
                .mode();
            assert!(
                mode & 0o111 != 0,
                "{script} must have at least one execute bit set, got mode {mode:#o}"
            );
        }

        let _ = fs::remove_dir_all(root);
    }

    fn hook_script_name(stem: &str) -> String {
        format!("{stem}.sh")
    }

    fn hook_script_contents(message: &str) -> String {
        format!("#!/bin/sh\nprintf '%s\\n' '{message}'\n")
    }

    fn shell_echo_and_exit(message: &str, code: i32) -> String {
        format!("printf '{message}'; exit {code}")
    }
}
