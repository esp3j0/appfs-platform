use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use crate::bash_shell_path;
use crate::config::{RuntimeFeatureConfig, RuntimeHookConfig};
use crate::permissions::PermissionOverride;
use crate::session::CompactTrigger;

pub type HookPermissionDecision = PermissionOverride;

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
    #[must_use]
    pub fn as_str(self) -> &'static str {
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
pub enum HookProgressEvent {
    Started {
        event: HookEvent,
        tool_name: String,
        command: String,
    },
    Completed {
        event: HookEvent,
        tool_name: String,
        command: String,
    },
    Cancelled {
        event: HookEvent,
        tool_name: String,
        command: String,
    },
}

pub trait HookProgressReporter {
    fn on_event(&mut self, event: &HookProgressEvent);
}

#[derive(Debug, Clone, Default)]
pub struct HookAbortSignal {
    aborted: Arc<AtomicBool>,
}

impl HookAbortSignal {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn abort(&self) {
        self.aborted.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRunResult {
    denied: bool,
    failed: bool,
    cancelled: bool,
    messages: Vec<String>,
    permission_override: Option<PermissionOverride>,
    permission_reason: Option<String>,
    updated_input: Option<String>,
}

impl HookRunResult {
    #[must_use]
    pub fn allow(messages: Vec<String>) -> Self {
        Self {
            denied: false,
            failed: false,
            cancelled: false,
            messages,
            permission_override: None,
            permission_reason: None,
            updated_input: None,
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
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    #[must_use]
    pub fn messages(&self) -> &[String] {
        &self.messages
    }

    #[must_use]
    pub fn permission_override(&self) -> Option<PermissionOverride> {
        self.permission_override
    }

    #[must_use]
    pub fn permission_decision(&self) -> Option<HookPermissionDecision> {
        self.permission_override
    }

    #[must_use]
    pub fn permission_reason(&self) -> Option<&str> {
        self.permission_reason.as_deref()
    }

    #[must_use]
    pub fn updated_input(&self) -> Option<&str> {
        self.updated_input.as_deref()
    }

    #[must_use]
    pub fn updated_input_json(&self) -> Option<&str> {
        self.updated_input()
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
    cancelled: bool,
    output: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HookRunner {
    config: RuntimeHookConfig,
}

impl HookRunner {
    #[must_use]
    pub fn new(config: RuntimeHookConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn from_feature_config(feature_config: &RuntimeFeatureConfig) -> Self {
        Self::new(feature_config.hooks().clone())
    }

    #[must_use]
    pub fn run_pre_tool_use(&self, tool_name: &str, tool_input: &str) -> HookRunResult {
        self.run_pre_tool_use_with_context(tool_name, tool_input, None, None)
    }

    #[must_use]
    pub fn run_pre_tool_use_with_context(
        &self,
        tool_name: &str,
        tool_input: &str,
        abort_signal: Option<&HookAbortSignal>,
        reporter: Option<&mut dyn HookProgressReporter>,
    ) -> HookRunResult {
        Self::run_commands(
            HookEvent::PreToolUse,
            self.config.pre_tool_use(),
            tool_name,
            tool_input,
            None,
            false,
            abort_signal,
            reporter,
        )
    }

    #[must_use]
    pub fn run_pre_tool_use_with_signal(
        &self,
        tool_name: &str,
        tool_input: &str,
        abort_signal: Option<&HookAbortSignal>,
    ) -> HookRunResult {
        self.run_pre_tool_use_with_context(tool_name, tool_input, abort_signal, None)
    }

    #[must_use]
    pub fn run_post_tool_use(
        &self,
        tool_name: &str,
        tool_input: &str,
        tool_output: &str,
        is_error: bool,
    ) -> HookRunResult {
        self.run_post_tool_use_with_context(
            tool_name,
            tool_input,
            tool_output,
            is_error,
            None,
            None,
        )
    }

    #[must_use]
    pub fn run_post_tool_use_with_context(
        &self,
        tool_name: &str,
        tool_input: &str,
        tool_output: &str,
        is_error: bool,
        abort_signal: Option<&HookAbortSignal>,
        reporter: Option<&mut dyn HookProgressReporter>,
    ) -> HookRunResult {
        Self::run_commands(
            HookEvent::PostToolUse,
            self.config.post_tool_use(),
            tool_name,
            tool_input,
            Some(tool_output),
            is_error,
            abort_signal,
            reporter,
        )
    }

    #[must_use]
    pub fn run_post_tool_use_with_signal(
        &self,
        tool_name: &str,
        tool_input: &str,
        tool_output: &str,
        is_error: bool,
        abort_signal: Option<&HookAbortSignal>,
    ) -> HookRunResult {
        self.run_post_tool_use_with_context(
            tool_name,
            tool_input,
            tool_output,
            is_error,
            abort_signal,
            None,
        )
    }

    #[must_use]
    pub fn run_post_tool_use_failure(
        &self,
        tool_name: &str,
        tool_input: &str,
        tool_error: &str,
    ) -> HookRunResult {
        self.run_post_tool_use_failure_with_context(tool_name, tool_input, tool_error, None, None)
    }

    #[must_use]
    pub fn run_post_tool_use_failure_with_context(
        &self,
        tool_name: &str,
        tool_input: &str,
        tool_error: &str,
        abort_signal: Option<&HookAbortSignal>,
        reporter: Option<&mut dyn HookProgressReporter>,
    ) -> HookRunResult {
        Self::run_commands(
            HookEvent::PostToolUseFailure,
            self.config.post_tool_use_failure(),
            tool_name,
            tool_input,
            Some(tool_error),
            true,
            abort_signal,
            reporter,
        )
    }

    #[must_use]
    pub fn run_post_tool_use_failure_with_signal(
        &self,
        tool_name: &str,
        tool_input: &str,
        tool_error: &str,
        abort_signal: Option<&HookAbortSignal>,
    ) -> HookRunResult {
        self.run_post_tool_use_failure_with_context(
            tool_name,
            tool_input,
            tool_error,
            abort_signal,
            None,
        )
    }

    #[must_use]
    pub fn run_pre_compact(
        &self,
        trigger: CompactTrigger,
        custom_instructions: Option<&str>,
    ) -> PreCompactHookResult {
        self.run_pre_compact_with_context(trigger, custom_instructions, None, None)
    }

    #[must_use]
    pub fn run_pre_compact_with_context(
        &self,
        trigger: CompactTrigger,
        custom_instructions: Option<&str>,
        abort_signal: Option<&HookAbortSignal>,
        reporter: Option<&mut dyn HookProgressReporter>,
    ) -> PreCompactHookResult {
        let payload = hook_pre_compact_payload(trigger, custom_instructions).to_string();
        let command_results = Self::run_contextual_commands(
            HookEvent::PreCompact,
            self.config.pre_compact(),
            "compact",
            &payload,
            abort_signal,
            reporter,
            |child| {
                child.env("HOOK_TRIGGER", trigger.as_str());
                if let Some(custom_instructions) = custom_instructions {
                    child.env("HOOK_CUSTOM_INSTRUCTIONS", custom_instructions);
                }
            },
        );

        build_pre_compact_hook_result(&command_results)
    }

    #[must_use]
    pub fn run_post_compact(
        &self,
        trigger: CompactTrigger,
        compact_summary: &str,
    ) -> PostCompactHookResult {
        self.run_post_compact_with_context(trigger, compact_summary, None, None)
    }

    #[must_use]
    pub fn run_post_compact_with_context(
        &self,
        trigger: CompactTrigger,
        compact_summary: &str,
        abort_signal: Option<&HookAbortSignal>,
        reporter: Option<&mut dyn HookProgressReporter>,
    ) -> PostCompactHookResult {
        let payload = hook_post_compact_payload(trigger, compact_summary).to_string();
        let command_results = Self::run_contextual_commands(
            HookEvent::PostCompact,
            self.config.post_compact(),
            "compact",
            &payload,
            abort_signal,
            reporter,
            |child| {
                child.env("HOOK_TRIGGER", trigger.as_str());
                child.env("HOOK_COMPACT_SUMMARY", compact_summary);
            },
        );

        build_post_compact_hook_result(&command_results)
    }

    #[must_use]
    pub fn run_session_start(&self, source: &str, model: Option<&str>) -> HookRunResult {
        self.run_session_start_with_context(source, model, None, None)
    }

    #[must_use]
    pub fn run_session_start_with_context(
        &self,
        source: &str,
        model: Option<&str>,
        abort_signal: Option<&HookAbortSignal>,
        mut reporter: Option<&mut dyn HookProgressReporter>,
    ) -> HookRunResult {
        let commands = self.config.session_start();
        if commands.is_empty() {
            return HookRunResult::allow(Vec::new());
        }

        if abort_signal.is_some_and(HookAbortSignal::is_aborted) {
            return HookRunResult {
                denied: false,
                failed: false,
                cancelled: true,
                messages: vec![String::from("SessionStart hook cancelled before execution")],
                permission_override: None,
                permission_reason: None,
                updated_input: None,
            };
        }

        let payload = hook_session_start_payload(source, model).to_string();
        let mut result = HookRunResult::allow(Vec::new());

        for command in commands {
            if let Some(reporter) = reporter.as_deref_mut() {
                reporter.on_event(&HookProgressEvent::Started {
                    event: HookEvent::SessionStart,
                    tool_name: source.to_string(),
                    command: command.clone(),
                });
            }

            match Self::run_session_start_command(command, source, model, &payload, abort_signal) {
                HookCommandOutcome::Allow { parsed } => {
                    if let Some(reporter) = reporter.as_deref_mut() {
                        reporter.on_event(&HookProgressEvent::Completed {
                            event: HookEvent::SessionStart,
                            tool_name: source.to_string(),
                            command: command.clone(),
                        });
                    }
                    merge_parsed_hook_output(&mut result, parsed);
                }
                HookCommandOutcome::Deny { parsed } => {
                    if let Some(reporter) = reporter.as_deref_mut() {
                        reporter.on_event(&HookProgressEvent::Completed {
                            event: HookEvent::SessionStart,
                            tool_name: source.to_string(),
                            command: command.clone(),
                        });
                    }
                    merge_parsed_hook_output(&mut result, parsed);
                    result.denied = true;
                    return result;
                }
                HookCommandOutcome::Failed { parsed } => {
                    if let Some(reporter) = reporter.as_deref_mut() {
                        reporter.on_event(&HookProgressEvent::Completed {
                            event: HookEvent::SessionStart,
                            tool_name: source.to_string(),
                            command: command.clone(),
                        });
                    }
                    merge_parsed_hook_output(&mut result, parsed);
                    result.failed = true;
                    return result;
                }
                HookCommandOutcome::Cancelled { message } => {
                    if let Some(reporter) = reporter.as_deref_mut() {
                        reporter.on_event(&HookProgressEvent::Cancelled {
                            event: HookEvent::SessionStart,
                            tool_name: source.to_string(),
                            command: command.clone(),
                        });
                    }
                    result.cancelled = true;
                    result.messages.push(message);
                    return result;
                }
            }
        }

        result
    }

    fn run_contextual_commands<F>(
        event: HookEvent,
        commands: &[String],
        context_name: &str,
        payload: &str,
        abort_signal: Option<&HookAbortSignal>,
        mut reporter: Option<&mut dyn HookProgressReporter>,
        mut configure_env: F,
    ) -> Vec<ContextHookCommandResult>
    where
        F: FnMut(&mut CommandWithStdin),
    {
        let mut results = Vec::new();
        for command in commands {
            if abort_signal.is_some_and(HookAbortSignal::is_aborted) {
                results.push(ContextHookCommandResult {
                    command: command.clone(),
                    succeeded: false,
                    cancelled: true,
                    output: format!("{} hook cancelled before execution", event.as_str()),
                });
                break;
            }

            if let Some(reporter) = reporter.as_deref_mut() {
                reporter.on_event(&HookProgressEvent::Started {
                    event,
                    tool_name: context_name.to_string(),
                    command: command.clone(),
                });
            }

            let result = Self::run_contextual_command(
                command,
                event,
                context_name,
                payload,
                abort_signal,
                &mut configure_env,
            );
            if let Some(reporter) = reporter.as_deref_mut() {
                let event = if result.cancelled {
                    HookProgressEvent::Cancelled {
                        event,
                        tool_name: context_name.to_string(),
                        command: command.clone(),
                    }
                } else {
                    HookProgressEvent::Completed {
                        event,
                        tool_name: context_name.to_string(),
                        command: command.clone(),
                    }
                };
                reporter.on_event(&event);
            }
            results.push(result);
        }
        results
    }

    #[allow(clippy::too_many_arguments)]
    fn run_commands(
        event: HookEvent,
        commands: &[String],
        tool_name: &str,
        tool_input: &str,
        tool_output: Option<&str>,
        is_error: bool,
        abort_signal: Option<&HookAbortSignal>,
        mut reporter: Option<&mut dyn HookProgressReporter>,
    ) -> HookRunResult {
        if commands.is_empty() {
            return HookRunResult::allow(Vec::new());
        }

        if abort_signal.is_some_and(HookAbortSignal::is_aborted) {
            return HookRunResult {
                denied: false,
                failed: false,
                cancelled: true,
                messages: vec![format!(
                    "{} hook cancelled before execution",
                    event.as_str()
                )],
                permission_override: None,
                permission_reason: None,
                updated_input: None,
            };
        }

        let payload = hook_payload(event, tool_name, tool_input, tool_output, is_error).to_string();
        let mut result = HookRunResult::allow(Vec::new());

        for command in commands {
            if let Some(reporter) = reporter.as_deref_mut() {
                reporter.on_event(&HookProgressEvent::Started {
                    event,
                    tool_name: tool_name.to_string(),
                    command: command.clone(),
                });
            }

            match Self::run_command(
                command,
                event,
                tool_name,
                tool_input,
                tool_output,
                is_error,
                &payload,
                abort_signal,
            ) {
                HookCommandOutcome::Allow { parsed } => {
                    if let Some(reporter) = reporter.as_deref_mut() {
                        reporter.on_event(&HookProgressEvent::Completed {
                            event,
                            tool_name: tool_name.to_string(),
                            command: command.clone(),
                        });
                    }
                    merge_parsed_hook_output(&mut result, parsed);
                }
                HookCommandOutcome::Deny { parsed } => {
                    if let Some(reporter) = reporter.as_deref_mut() {
                        reporter.on_event(&HookProgressEvent::Completed {
                            event,
                            tool_name: tool_name.to_string(),
                            command: command.clone(),
                        });
                    }
                    merge_parsed_hook_output(&mut result, parsed);
                    result.denied = true;
                    return result;
                }
                HookCommandOutcome::Failed { parsed } => {
                    if let Some(reporter) = reporter.as_deref_mut() {
                        reporter.on_event(&HookProgressEvent::Completed {
                            event,
                            tool_name: tool_name.to_string(),
                            command: command.clone(),
                        });
                    }
                    merge_parsed_hook_output(&mut result, parsed);
                    result.failed = true;
                    return result;
                }
                HookCommandOutcome::Cancelled { message } => {
                    if let Some(reporter) = reporter.as_deref_mut() {
                        reporter.on_event(&HookProgressEvent::Cancelled {
                            event,
                            tool_name: tool_name.to_string(),
                            command: command.clone(),
                        });
                    }
                    result.cancelled = true;
                    result.messages.push(message);
                    return result;
                }
            }
        }

        result
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
        abort_signal: Option<&HookAbortSignal>,
    ) -> HookCommandOutcome {
        let mut child = match shell_command(command) {
            Ok(child) => child,
            Err(error) => {
                return HookCommandOutcome::Failed {
                    parsed: ParsedHookOutput {
                        messages: vec![format!(
                            "{} hook `{command}` failed to start for `{}`: {error}",
                            event.as_str(),
                            tool_name
                        )],
                        ..ParsedHookOutput::default()
                    },
                };
            }
        };
        child.stdin(Stdio::piped());
        child.stdout(Stdio::piped());
        child.stderr(Stdio::piped());
        child.env("HOOK_EVENT", event.as_str());
        child.env("HOOK_TOOL_NAME", tool_name);
        child.env("HOOK_TOOL_INPUT", tool_input);
        child.env("HOOK_TOOL_IS_ERROR", if is_error { "1" } else { "0" });
        if let Some(tool_output) = tool_output {
            child.env("HOOK_TOOL_OUTPUT", tool_output);
        }

        match child.output_with_stdin(payload.as_bytes(), abort_signal) {
            Ok(CommandExecution::Finished(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let parsed = parse_hook_output(&stdout);
                let primary_message = parsed.primary_message().map(ToOwned::to_owned);
                match output.status.code() {
                    Some(0) => {
                        if parsed.deny {
                            HookCommandOutcome::Deny { parsed }
                        } else {
                            HookCommandOutcome::Allow { parsed }
                        }
                    }
                    Some(2) => HookCommandOutcome::Deny {
                        parsed: parsed.with_fallback_message(format!(
                            "{} hook denied tool `{tool_name}`",
                            event.as_str()
                        )),
                    },
                    Some(code) => HookCommandOutcome::Failed {
                        parsed: parsed.with_fallback_message(format_hook_failure(
                            command,
                            code,
                            primary_message.as_deref(),
                            stderr.as_str(),
                        )),
                    },
                    None => HookCommandOutcome::Failed {
                        parsed: parsed.with_fallback_message(format!(
                            "{} hook `{command}` terminated by signal while handling `{}`",
                            event.as_str(),
                            tool_name
                        )),
                    },
                }
            }
            Ok(CommandExecution::Cancelled) => HookCommandOutcome::Cancelled {
                message: format!(
                    "{} hook `{command}` cancelled while handling `{tool_name}`",
                    event.as_str()
                ),
            },
            Err(error) => HookCommandOutcome::Failed {
                parsed: ParsedHookOutput {
                    messages: vec![format!(
                        "{} hook `{command}` failed to start for `{}`: {error}",
                        event.as_str(),
                        tool_name
                    )],
                    ..ParsedHookOutput::default()
                },
            },
        }
    }

    fn run_session_start_command(
        command: &str,
        source: &str,
        model: Option<&str>,
        payload: &str,
        abort_signal: Option<&HookAbortSignal>,
    ) -> HookCommandOutcome {
        let mut child = match shell_command(command) {
            Ok(child) => child,
            Err(error) => {
                return HookCommandOutcome::Failed {
                    parsed: ParsedHookOutput {
                        messages: vec![format!(
                            "SessionStart hook `{command}` failed to start for `{source}`: {error}"
                        )],
                        ..ParsedHookOutput::default()
                    },
                };
            }
        };
        child.stdin(Stdio::piped());
        child.stdout(Stdio::piped());
        child.stderr(Stdio::piped());
        child.env("HOOK_EVENT", HookEvent::SessionStart.as_str());
        child.env("HOOK_SESSION_SOURCE", source);
        if let Some(model) = model {
            child.env("HOOK_MODEL", model);
        }

        match child.output_with_stdin(payload.as_bytes(), abort_signal) {
            Ok(CommandExecution::Finished(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let parsed = parse_hook_output(&stdout);
                let primary_message = parsed.primary_message().map(ToOwned::to_owned);
                match output.status.code() {
                    Some(0) => {
                        if parsed.deny {
                            HookCommandOutcome::Deny { parsed }
                        } else {
                            HookCommandOutcome::Allow { parsed }
                        }
                    }
                    Some(2) => HookCommandOutcome::Deny {
                        parsed: parsed.with_fallback_message(format!(
                            "SessionStart hook denied `{source}`"
                        )),
                    },
                    Some(code) => HookCommandOutcome::Failed {
                        parsed: parsed.with_fallback_message(format_hook_failure(
                            command,
                            code,
                            primary_message.as_deref(),
                            stderr.as_str(),
                        )),
                    },
                    None => HookCommandOutcome::Failed {
                        parsed: parsed.with_fallback_message(format!(
                            "SessionStart hook `{command}` terminated by signal while handling `{source}`"
                        )),
                    },
                }
            }
            Ok(CommandExecution::Cancelled) => HookCommandOutcome::Cancelled {
                message: format!(
                    "SessionStart hook `{command}` cancelled while handling `{source}`"
                ),
            },
            Err(error) => HookCommandOutcome::Failed {
                parsed: ParsedHookOutput {
                    messages: vec![format!(
                        "SessionStart hook `{command}` failed to start for `{source}`: {error}"
                    )],
                    ..ParsedHookOutput::default()
                },
            },
        }
    }

    fn run_contextual_command<F>(
        command: &str,
        event: HookEvent,
        context_name: &str,
        payload: &str,
        abort_signal: Option<&HookAbortSignal>,
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
                    cancelled: false,
                    output: format!(
                        "{} hook `{command}` failed to start for `{context_name}`: {error}",
                        event.as_str()
                    ),
                };
            }
        };
        child.stdin(Stdio::piped());
        child.stdout(Stdio::piped());
        child.stderr(Stdio::piped());
        child.env("HOOK_EVENT", event.as_str());
        configure_env(&mut child);

        match child.output_with_stdin(payload.as_bytes(), abort_signal) {
            Ok(CommandExecution::Finished(output)) => {
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
                        cancelled: false,
                        output: rendered,
                    },
                    Some(code) => ContextHookCommandResult {
                        command: command.to_string(),
                        succeeded: false,
                        cancelled: false,
                        output: format_hook_failure(
                            command,
                            code,
                            (!rendered.is_empty()).then_some(rendered.as_str()),
                            stderr.as_str(),
                        ),
                    },
                    None => ContextHookCommandResult {
                        command: command.to_string(),
                        succeeded: false,
                        cancelled: false,
                        output: format!(
                            "{} hook `{command}` terminated by signal while handling `{context_name}`",
                            event.as_str()
                        ),
                    },
                }
            }
            Ok(CommandExecution::Cancelled) => ContextHookCommandResult {
                command: command.to_string(),
                succeeded: false,
                cancelled: true,
                output: format!(
                    "{} hook `{command}` cancelled while handling `{context_name}`",
                    event.as_str()
                ),
            },
            Err(error) => ContextHookCommandResult {
                command: command.to_string(),
                succeeded: false,
                cancelled: false,
                output: format!(
                    "{} hook `{command}` failed to start for `{context_name}`: {error}",
                    event.as_str()
                ),
            },
        }
    }
}

enum HookCommandOutcome {
    Allow { parsed: ParsedHookOutput },
    Deny { parsed: ParsedHookOutput },
    Failed { parsed: ParsedHookOutput },
    Cancelled { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ParsedHookOutput {
    messages: Vec<String>,
    deny: bool,
    permission_override: Option<PermissionOverride>,
    permission_reason: Option<String>,
    updated_input: Option<String>,
}

impl ParsedHookOutput {
    fn with_fallback_message(mut self, fallback: String) -> Self {
        if self.messages.is_empty() {
            self.messages.push(fallback);
        }
        self
    }

    fn primary_message(&self) -> Option<&str> {
        self.messages.first().map(String::as_str)
    }
}

fn merge_parsed_hook_output(target: &mut HookRunResult, parsed: ParsedHookOutput) {
    target.messages.extend(parsed.messages);
    if parsed.permission_override.is_some() {
        target.permission_override = parsed.permission_override;
    }
    if parsed.permission_reason.is_some() {
        target.permission_reason = parsed.permission_reason;
    }
    if parsed.updated_input.is_some() {
        target.updated_input = parsed.updated_input;
    }
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

fn parse_hook_output(stdout: &str) -> ParsedHookOutput {
    if stdout.is_empty() {
        return ParsedHookOutput::default();
    }

    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(stdout) else {
        return ParsedHookOutput {
            messages: vec![stdout.to_string()],
            ..ParsedHookOutput::default()
        };
    };

    let mut parsed = ParsedHookOutput::default();

    if let Some(message) = root.get("systemMessage").and_then(Value::as_str) {
        parsed.messages.push(message.to_string());
    }
    if let Some(message) = root.get("reason").and_then(Value::as_str) {
        parsed.messages.push(message.to_string());
    }
    if root.get("continue").and_then(Value::as_bool) == Some(false)
        || root.get("decision").and_then(Value::as_str) == Some("block")
    {
        parsed.deny = true;
    }

    if let Some(Value::Object(specific)) = root.get("hookSpecificOutput") {
        if let Some(Value::String(additional_context)) = specific.get("additionalContext") {
            parsed.messages.push(additional_context.clone());
        }
        if let Some(decision) = specific.get("permissionDecision").and_then(Value::as_str) {
            parsed.permission_override = match decision {
                "allow" => Some(PermissionOverride::Allow),
                "deny" => Some(PermissionOverride::Deny),
                "ask" => Some(PermissionOverride::Ask),
                _ => None,
            };
        }
        if let Some(reason) = specific
            .get("permissionDecisionReason")
            .and_then(Value::as_str)
        {
            parsed.permission_reason = Some(reason.to_string());
        }
        if let Some(updated_input) = specific.get("updatedInput") {
            parsed.updated_input = serde_json::to_string(updated_input).ok();
        }
    }

    if parsed.messages.is_empty() {
        parsed.messages.push(stdout.to_string());
    }

    parsed
}

fn hook_payload(
    event: HookEvent,
    tool_name: &str,
    tool_input: &str,
    tool_output: Option<&str>,
    is_error: bool,
) -> Value {
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

fn hook_session_start_payload(source: &str, model: Option<&str>) -> Value {
    json!({
        "hook_event_name": HookEvent::SessionStart.as_str(),
        "session_source": source,
        "model": model,
    })
}

fn hook_pre_compact_payload(trigger: CompactTrigger, custom_instructions: Option<&str>) -> Value {
    json!({
        "hook_event_name": HookEvent::PreCompact.as_str(),
        "trigger": trigger.as_str(),
        "custom_instructions": custom_instructions,
    })
}

fn hook_post_compact_payload(trigger: CompactTrigger, compact_summary: &str) -> Value {
    json!({
        "hook_event_name": HookEvent::PostCompact.as_str(),
        "trigger": trigger.as_str(),
        "compact_summary": compact_summary,
    })
}

fn parse_tool_input(tool_input: &str) -> Value {
    serde_json::from_str(tool_input).unwrap_or_else(|_| json!({ "raw": tool_input }))
}

fn format_hook_failure(command: &str, code: i32, stdout: Option<&str>, stderr: &str) -> String {
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

fn shell_command(command: &str) -> std::io::Result<CommandWithStdin> {
    let mut command_builder = Command::new(bash_shell_path()?);
    command_builder.arg("-lc").arg(command);
    Ok(CommandWithStdin::new(command_builder))
}

struct CommandWithStdin {
    command: Command,
}

impl CommandWithStdin {
    fn new(command: Command) -> Self {
        Self { command }
    }

    fn stdin(&mut self, cfg: Stdio) -> &mut Self {
        self.command.stdin(cfg);
        self
    }

    fn stdout(&mut self, cfg: Stdio) -> &mut Self {
        self.command.stdout(cfg);
        self
    }

    fn stderr(&mut self, cfg: Stdio) -> &mut Self {
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

    fn output_with_stdin(
        &mut self,
        stdin: &[u8],
        abort_signal: Option<&HookAbortSignal>,
    ) -> std::io::Result<CommandExecution> {
        let mut child = self.command.spawn()?;
        if let Some(mut child_stdin) = child.stdin.take() {
            child_stdin.write_all(stdin)?;
        }

        loop {
            if abort_signal.is_some_and(HookAbortSignal::is_aborted) {
                let _ = child.kill();
                let _ = child.wait_with_output();
                return Ok(CommandExecution::Cancelled);
            }

            match child.try_wait()? {
                Some(_) => return child.wait_with_output().map(CommandExecution::Finished),
                None => thread::sleep(Duration::from_millis(20)),
            }
        }
    }
}

enum CommandExecution {
    Finished(std::process::Output),
    Cancelled,
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::process::Command;
    #[cfg(windows)]
    use std::sync::OnceLock;
    use std::thread;
    use std::time::Duration;

    use super::{
        HookAbortSignal, HookEvent, HookProgressEvent, HookProgressReporter, HookRunResult,
        HookRunner,
    };
    use crate::config::{RuntimeFeatureConfig, RuntimeHookConfig};
    use crate::permissions::PermissionOverride;
    use crate::session::CompactTrigger;
    #[cfg(windows)]
    use crate::{bash_shell_path, set_shell_if_windows};

    struct RecordingReporter {
        events: Vec<HookProgressEvent>,
    }

    impl HookProgressReporter for RecordingReporter {
        fn on_event(&mut self, event: &HookProgressEvent) {
            self.events.push(event.clone());
        }
    }

    fn windows_bash_smoke_ok() -> bool {
        #[cfg(windows)]
        {
            static OK: OnceLock<bool> = OnceLock::new();
            *OK.get_or_init(|| {
                if set_shell_if_windows().is_err() {
                    return false;
                }
                let Ok(shell_path) = bash_shell_path() else {
                    return false;
                };
                Command::new(shell_path)
                    .args(["-lc", "printf ok"])
                    .output()
                    .is_ok_and(|output| {
                        output.status.success()
                            && String::from_utf8_lossy(&output.stdout).trim() == "ok"
                    })
            })
        }

        #[cfg(not(windows))]
        {
            true
        }
    }

    #[test]
    fn allows_exit_code_zero_and_captures_stdout() {
        if !windows_bash_smoke_ok() {
            return;
        }
        let runner = HookRunner::new(RuntimeHookConfig::new(
            vec![shell_echo("pre ok")],
            Vec::new(),
            Vec::new(),
        ));

        let result = runner.run_pre_tool_use("Read", r#"{"path":"README.md"}"#);

        assert_eq!(result, HookRunResult::allow(vec!["pre ok".to_string()]));
    }

    #[test]
    fn denies_exit_code_two() {
        if !windows_bash_smoke_ok() {
            return;
        }
        let runner = HookRunner::new(RuntimeHookConfig::new(
            vec![shell_echo_and_exit("blocked by hook", 2)],
            Vec::new(),
            Vec::new(),
        ));

        let result = runner.run_pre_tool_use("Bash", r#"{"command":"pwd"}"#);

        assert!(result.is_denied());
        assert_eq!(result.messages(), &["blocked by hook".to_string()]);
    }

    #[test]
    fn propagates_other_non_zero_statuses_as_failures() {
        let runner = HookRunner::from_feature_config(&RuntimeFeatureConfig::default().with_hooks(
            RuntimeHookConfig::new(
                vec![shell_snippet("printf 'warning hook'; exit 1")],
                Vec::new(),
                Vec::new(),
            ),
        ));

        // given
        // when
        let result = runner.run_pre_tool_use("Edit", r#"{"file":"src/lib.rs"}"#);

        // then
        assert!(result.is_failed());
        assert!(result
            .messages()
            .iter()
            .any(|message| message.contains("warning hook")));
    }

    #[test]
    fn parses_pre_hook_permission_override_and_updated_input() {
        if !windows_bash_smoke_ok() {
            return;
        }
        let runner = HookRunner::new(RuntimeHookConfig::new(
            vec![shell_snippet(
                r#"printf '%s' '{"systemMessage":"updated","hookSpecificOutput":{"permissionDecision":"allow","permissionDecisionReason":"hook ok","updatedInput":{"command":"git status"}}}'"#,
            )],
            Vec::new(),
            Vec::new(),
        ));

        let result = runner.run_pre_tool_use("bash", r#"{"command":"pwd"}"#);

        assert_eq!(
            result.permission_override(),
            Some(PermissionOverride::Allow)
        );
        assert_eq!(result.permission_reason(), Some("hook ok"));
        assert_eq!(result.updated_input(), Some(r#"{"command":"git status"}"#));
        assert!(result.messages().iter().any(|message| message == "updated"));
    }

    #[test]
    fn runs_post_tool_use_failure_hooks() {
        if !windows_bash_smoke_ok() {
            return;
        }
        // given
        let runner = HookRunner::new(RuntimeHookConfig::new(
            Vec::new(),
            Vec::new(),
            vec![shell_snippet("printf 'failure hook ran'")],
        ));

        // when
        let result =
            runner.run_post_tool_use_failure("bash", r#"{"command":"false"}"#, "command failed");

        // then
        assert!(!result.is_denied());
        assert_eq!(result.messages(), &["failure hook ran".to_string()]);
    }

    #[test]
    fn runs_session_start_hooks_with_compact_source_context() {
        if !windows_bash_smoke_ok() {
            return;
        }
        let runner =
            HookRunner::new(
                RuntimeHookConfig::default().with_session_start(vec![shell_snippet(
                    r#"printf '%s|%s' "$HOOK_SESSION_SOURCE" "$HOOK_MODEL""#,
                )]),
            );

        let result = runner.run_session_start("compact", Some("claude-opus-4-6"));

        assert_eq!(
            result,
            HookRunResult::allow(vec!["compact|claude-opus-4-6".to_string()])
        );
    }

    #[test]
    fn runs_pre_and_post_compact_hooks_without_blocking_compaction() {
        if !windows_bash_smoke_ok() {
            return;
        }
        let runner = HookRunner::new(
            RuntimeHookConfig::default()
                .with_pre_compact(vec![
                    shell_snippet(r#"printf '%s|%s' "$HOOK_TRIGGER" "$HOOK_CUSTOM_INSTRUCTIONS""#),
                    shell_snippet("printf 'broken pre'; exit 1"),
                ])
                .with_post_compact(vec![shell_snippet(
                    r#"printf '%s|%s' "$HOOK_TRIGGER" "$HOOK_COMPACT_SUMMARY""#,
                )]),
        );

        let pre_result = runner.run_pre_compact(CompactTrigger::Manual, Some("keep latest logs"));
        assert_eq!(
            pre_result.new_custom_instructions(),
            Some("manual|keep latest logs")
        );
        let pre_display = pre_result
            .user_display_message()
            .expect("pre-compact display message");
        assert!(pre_display.contains("manual|keep latest logs"));
        assert!(pre_display.contains("broken pre"));

        let post_result = runner.run_post_compact(CompactTrigger::Auto, "summary body");
        let post_display = post_result
            .user_display_message()
            .expect("post-compact display message");
        assert!(post_display.contains("auto|summary body"));
    }

    #[test]
    fn stops_running_failure_hooks_after_failure() {
        // given
        let runner = HookRunner::new(RuntimeHookConfig::new(
            Vec::new(),
            Vec::new(),
            vec![
                shell_snippet("printf 'broken failure hook'; exit 1"),
                shell_snippet("printf 'later failure hook'"),
            ],
        ));

        // when
        let result =
            runner.run_post_tool_use_failure("bash", r#"{"command":"false"}"#, "command failed");

        // then
        assert!(result.is_failed());
        assert!(result
            .messages()
            .iter()
            .any(|message| message.contains("broken failure hook")));
        assert!(!result
            .messages()
            .iter()
            .any(|message| message == "later failure hook"));
    }

    #[test]
    fn executes_hooks_in_configured_order() {
        if !windows_bash_smoke_ok() {
            return;
        }
        // given
        let runner = HookRunner::new(RuntimeHookConfig::new(
            vec![
                shell_snippet("printf 'first'"),
                shell_snippet("printf 'second'"),
            ],
            Vec::new(),
            Vec::new(),
        ));
        let mut reporter = RecordingReporter { events: Vec::new() };

        // when
        let result = runner.run_pre_tool_use_with_context(
            "Read",
            r#"{"path":"README.md"}"#,
            None,
            Some(&mut reporter),
        );

        // then
        assert_eq!(
            result,
            HookRunResult::allow(vec!["first".to_string(), "second".to_string()])
        );
        assert_eq!(reporter.events.len(), 4);
        assert!(matches!(
            &reporter.events[0],
            HookProgressEvent::Started {
                event: HookEvent::PreToolUse,
                command,
                ..
            } if command == "printf 'first'"
        ));
        assert!(matches!(
            &reporter.events[1],
            HookProgressEvent::Completed {
                event: HookEvent::PreToolUse,
                command,
                ..
            } if command == "printf 'first'"
        ));
        assert!(matches!(
            &reporter.events[2],
            HookProgressEvent::Started {
                event: HookEvent::PreToolUse,
                command,
                ..
            } if command == "printf 'second'"
        ));
        assert!(matches!(
            &reporter.events[3],
            HookProgressEvent::Completed {
                event: HookEvent::PreToolUse,
                command,
                ..
            } if command == "printf 'second'"
        ));
    }

    #[test]
    fn stops_running_hooks_after_failure() {
        // given
        let runner = HookRunner::new(RuntimeHookConfig::new(
            vec![
                shell_snippet("printf 'broken'; exit 1"),
                shell_snippet("printf 'later'"),
            ],
            Vec::new(),
            Vec::new(),
        ));

        // when
        let result = runner.run_pre_tool_use("Edit", r#"{"file":"src/lib.rs"}"#);

        // then
        assert!(result.is_failed());
        assert!(result
            .messages()
            .iter()
            .any(|message| message.contains("broken")));
        assert!(!result.messages().iter().any(|message| message == "later"));
    }

    #[test]
    fn abort_signal_cancels_long_running_hook_and_reports_progress() {
        let runner = HookRunner::new(RuntimeHookConfig::new(
            vec![shell_snippet("sleep 5")],
            Vec::new(),
            Vec::new(),
        ));
        let abort_signal = HookAbortSignal::new();
        let abort_signal_for_thread = abort_signal.clone();
        let mut reporter = RecordingReporter { events: Vec::new() };

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            abort_signal_for_thread.abort();
        });

        let result = runner.run_pre_tool_use_with_context(
            "bash",
            r#"{"command":"sleep 5"}"#,
            Some(&abort_signal),
            Some(&mut reporter),
        );

        assert!(result.is_cancelled());
        assert!(reporter.events.iter().any(|event| matches!(
            event,
            HookProgressEvent::Started {
                event: HookEvent::PreToolUse,
                ..
            }
        )));
        assert!(reporter.events.iter().any(|event| matches!(
            event,
            HookProgressEvent::Cancelled {
                event: HookEvent::PreToolUse,
                ..
            }
        )));
    }

    fn shell_echo(message: &str) -> String {
        format!("printf '{message}'")
    }

    fn shell_snippet(command: &str) -> String {
        command.to_string()
    }

    fn shell_echo_and_exit(message: &str, code: i32) -> String {
        format!("printf '{message}'; exit {code}")
    }
}
