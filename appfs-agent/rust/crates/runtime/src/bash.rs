use std::env;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::process::Command as TokioCommand;
use tokio::runtime::Builder;
use tokio::time::timeout;

use crate::sandbox::{
    build_linux_sandbox_command, resolve_sandbox_status_for_request, FilesystemIsolationMode,
    SandboxConfig, SandboxStatus,
};
use crate::tool_output::{task_outputs_dir, tool_results_dir};
#[cfg(windows)]
use crate::windows_path_to_posix_path;
use crate::{bash_shell_path, ConfigLoader};

static SHELL_OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);
const MAX_PERSISTED_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;
const SHELL_TASK_ID_ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
const PYTHON_IO_ENCODING: &str = "utf-8";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BashCommandInput {
    pub command: String,
    pub timeout: Option<u64>,
    pub description: Option<String>,
    #[serde(rename = "run_in_background")]
    pub run_in_background: Option<bool>,
    #[serde(rename = "dangerouslyDisableSandbox")]
    pub dangerously_disable_sandbox: Option<bool>,
    #[serde(rename = "namespaceRestrictions")]
    pub namespace_restrictions: Option<bool>,
    #[serde(rename = "isolateNetwork")]
    pub isolate_network: Option<bool>,
    #[serde(rename = "filesystemMode")]
    pub filesystem_mode: Option<FilesystemIsolationMode>,
    #[serde(rename = "allowedMounts")]
    pub allowed_mounts: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BashCommandOutput {
    pub stdout: String,
    pub stderr: String,
    #[serde(rename = "rawOutputPath", skip_serializing_if = "Option::is_none")]
    pub raw_output_path: Option<String>,
    pub interrupted: bool,
    #[serde(rename = "isImage", skip_serializing_if = "Option::is_none")]
    pub is_image: Option<bool>,
    #[serde(rename = "backgroundTaskId", skip_serializing_if = "Option::is_none")]
    pub background_task_id: Option<String>,
    #[serde(rename = "backgroundedByUser", skip_serializing_if = "Option::is_none")]
    pub backgrounded_by_user: Option<bool>,
    #[serde(
        rename = "assistantAutoBackgrounded",
        skip_serializing_if = "Option::is_none"
    )]
    pub assistant_auto_backgrounded: Option<bool>,
    #[serde(
        rename = "dangerouslyDisableSandbox",
        skip_serializing_if = "Option::is_none"
    )]
    pub dangerously_disable_sandbox: Option<bool>,
    #[serde(
        rename = "returnCodeInterpretation",
        skip_serializing_if = "Option::is_none"
    )]
    pub return_code_interpretation: Option<String>,
    #[serde(rename = "noOutputExpected", skip_serializing_if = "Option::is_none")]
    pub no_output_expected: Option<bool>,
    #[serde(rename = "structuredContent", skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Vec<serde_json::Value>>,
    #[serde(
        rename = "persistedOutputPath",
        skip_serializing_if = "Option::is_none"
    )]
    pub persisted_output_path: Option<String>,
    #[serde(
        rename = "persistedOutputSize",
        skip_serializing_if = "Option::is_none"
    )]
    pub persisted_output_size: Option<u64>,
    #[serde(rename = "sandboxStatus", skip_serializing_if = "Option::is_none")]
    pub sandbox_status: Option<SandboxStatus>,
}

#[derive(Debug)]
pub struct BackgroundShellOutputCapture {
    pub task_id: String,
    pub output_path: String,
    pub stdout: Stdio,
    pub stderr: Stdio,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedShellCommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub persisted_output_path: Option<String>,
    pub persisted_output_size: Option<u64>,
    pub no_output_expected: bool,
}

pub fn prepare_background_shell_output(
    cwd: &Path,
    _tool_name: &str,
) -> io::Result<BackgroundShellOutputCapture> {
    let task_id = next_shell_task_id();
    let output_path = shell_task_output_path(cwd, &task_id);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let stdout = OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(&output_path)?;
    let stderr = stdout.try_clone()?;
    Ok(BackgroundShellOutputCapture {
        task_id,
        output_path: output_path.to_string_lossy().into_owned(),
        stdout: Stdio::from(stdout),
        stderr: Stdio::from(stderr),
    })
}

#[must_use]
pub fn prepare_shell_command_output(
    cwd: &Path,
    _tool_name: &str,
    stdout: &[u8],
    stderr: &[u8],
) -> PreparedShellCommandOutput {
    let raw_stdout = decode_command_output(stdout);
    let raw_stderr = decode_command_output(stderr);
    let persisted = if stdout.len() > MAX_OUTPUT_BYTES {
        persist_stdout_for_model(cwd, stdout).ok()
    } else {
        None
    };

    PreparedShellCommandOutput {
        stdout: preview_output(&raw_stdout, MAX_OUTPUT_BYTES),
        stderr: truncate_output(&raw_stderr),
        persisted_output_path: persisted.as_ref().map(|value| value.path.clone()),
        persisted_output_size: persisted.map(|value| value.original_size),
        no_output_expected: raw_stdout.trim().is_empty() && raw_stderr.trim().is_empty(),
    }
}

pub fn execute_bash(input: BashCommandInput) -> io::Result<BashCommandOutput> {
    let cwd = env::current_dir()?;
    let sandbox_status = sandbox_status_for_input(&input, &cwd);

    if input.run_in_background.unwrap_or(false) {
        let background_output = prepare_background_shell_output(&cwd, "bash")?;
        let mut child = prepare_command(&input.command, &cwd, &sandbox_status, false)?;
        child
            .stdin(Stdio::null())
            .stdout(background_output.stdout)
            .stderr(background_output.stderr)
            .spawn()?;

        return Ok(BashCommandOutput {
            stdout: String::new(),
            stderr: String::new(),
            raw_output_path: Some(background_output.output_path),
            interrupted: false,
            is_image: None,
            background_task_id: Some(background_output.task_id),
            backgrounded_by_user: None,
            assistant_auto_backgrounded: None,
            dangerously_disable_sandbox: input.dangerously_disable_sandbox,
            return_code_interpretation: None,
            no_output_expected: Some(true),
            structured_content: None,
            persisted_output_path: None,
            persisted_output_size: None,
            sandbox_status: Some(sandbox_status),
        });
    }

    let runtime = Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(execute_bash_async(input, sandbox_status, cwd))
}

async fn execute_bash_async(
    input: BashCommandInput,
    sandbox_status: SandboxStatus,
    cwd: std::path::PathBuf,
) -> io::Result<BashCommandOutput> {
    let mut command = prepare_tokio_command(&input.command, &cwd, &sandbox_status, true)?;

    let output_result = if let Some(timeout_ms) = input.timeout {
        match timeout(Duration::from_millis(timeout_ms), command.output()).await {
            Ok(result) => (result?, false),
            Err(_) => {
                return Ok(BashCommandOutput {
                    stdout: String::new(),
                    stderr: format!("Command exceeded timeout of {timeout_ms} ms"),
                    raw_output_path: None,
                    interrupted: true,
                    is_image: None,
                    background_task_id: None,
                    backgrounded_by_user: None,
                    assistant_auto_backgrounded: None,
                    dangerously_disable_sandbox: input.dangerously_disable_sandbox,
                    return_code_interpretation: Some(String::from("timeout")),
                    no_output_expected: Some(false),
                    structured_content: None,
                    persisted_output_path: None,
                    persisted_output_size: None,
                    sandbox_status: Some(sandbox_status),
                });
            }
        }
    } else {
        (command.output().await?, false)
    };

    let (output, interrupted) = output_result;
    let prepared_output =
        prepare_shell_command_output(&cwd, "bash", &output.stdout, &output.stderr);
    let return_code_interpretation = output.status.code().and_then(|code| {
        if code == 0 {
            None
        } else {
            Some(format!("exit_code:{code}"))
        }
    });

    Ok(BashCommandOutput {
        stdout: prepared_output.stdout,
        stderr: prepared_output.stderr,
        raw_output_path: None,
        interrupted,
        is_image: None,
        background_task_id: None,
        backgrounded_by_user: None,
        assistant_auto_backgrounded: None,
        dangerously_disable_sandbox: input.dangerously_disable_sandbox,
        return_code_interpretation,
        no_output_expected: Some(prepared_output.no_output_expected),
        structured_content: None,
        persisted_output_path: prepared_output.persisted_output_path,
        persisted_output_size: prepared_output.persisted_output_size,
        sandbox_status: Some(sandbox_status),
    })
}

fn persist_stdout_for_model(cwd: &Path, stdout: &[u8]) -> io::Result<PersistedShellOutput> {
    let path = tool_results_dir(cwd).join(format!("{}.txt", next_shell_task_id()));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes_to_write = stdout
        .len()
        .min(usize::try_from(MAX_PERSISTED_OUTPUT_BYTES).expect("persisted output cap fits usize"));
    fs::write(&path, &stdout[..bytes_to_write])?;
    Ok(PersistedShellOutput {
        path: path.to_string_lossy().into_owned(),
        original_size: u64::try_from(stdout.len()).expect("stdout length fits u64"),
    })
}

#[must_use]
pub fn shell_task_output_path(cwd: &Path, task_id: &str) -> PathBuf {
    task_outputs_dir(cwd).join(format!("{task_id}.output"))
}

fn next_shell_task_id() -> String {
    let counter = SHELL_OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let time_seed = duration.as_secs().rotate_left(32) ^ u64::from(duration.subsec_nanos());
    let mut value = time_seed ^ counter.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let alphabet_len =
        u64::try_from(SHELL_TASK_ID_ALPHABET.len()).expect("alphabet length fits into u64");
    let mut suffix = [b'0'; 8];

    for ch in suffix.iter_mut().rev() {
        let idx = usize::try_from(value % alphabet_len).expect("task id index fits into usize");
        *ch = SHELL_TASK_ID_ALPHABET[idx];
        value /= alphabet_len;
    }

    let suffix = std::str::from_utf8(&suffix).expect("task id alphabet is ASCII");
    format!("b{suffix}")
}

struct PersistedShellOutput {
    path: String,
    original_size: u64,
}

#[must_use]
pub fn decode_command_output(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }

    #[cfg(windows)]
    if let Some(decoded) = decode_windows_ansi(bytes) {
        return decoded;
    }

    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(windows)]
fn decode_windows_ansi(bytes: &[u8]) -> Option<String> {
    let (decoded, _, had_errors) = encoding_rs::GBK.decode(bytes);
    (!had_errors).then(|| decoded.into_owned())
}

fn preview_output(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }

    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn sandbox_status_for_input(input: &BashCommandInput, cwd: &std::path::Path) -> SandboxStatus {
    let config = ConfigLoader::default_for(cwd).load().map_or_else(
        |_| SandboxConfig::default(),
        |runtime_config| runtime_config.sandbox().clone(),
    );
    let request = config.resolve_request(
        input.dangerously_disable_sandbox.map(|disabled| !disabled),
        input.namespace_restrictions,
        input.isolate_network,
        input.filesystem_mode,
        input.allowed_mounts.clone(),
    );
    resolve_sandbox_status_for_request(&request, cwd)
}

fn prepare_command(
    command: &str,
    cwd: &std::path::Path,
    sandbox_status: &SandboxStatus,
    create_dirs: bool,
) -> io::Result<Command> {
    if create_dirs {
        prepare_sandbox_dirs(cwd);
    }

    if let Some(launcher) = build_linux_sandbox_command(command, cwd, sandbox_status) {
        let mut prepared = Command::new(launcher.program);
        prepared.args(launcher.args);
        prepared.current_dir(cwd);
        prepared.envs(launcher.env);
        configure_bash_tool_env(&mut prepared);
        return Ok(prepared);
    }

    let mut prepared = Command::new(bash_shell_path()?);
    prepared.arg("-lc").arg(command);

    prepared.current_dir(cwd);
    configure_bash_tool_env(&mut prepared);
    if sandbox_status.filesystem_active {
        configure_bash_sandbox_env(&mut prepared, cwd);
    }
    Ok(prepared)
}

fn prepare_tokio_command(
    command: &str,
    cwd: &std::path::Path,
    sandbox_status: &SandboxStatus,
    create_dirs: bool,
) -> io::Result<TokioCommand> {
    if create_dirs {
        prepare_sandbox_dirs(cwd);
    }

    if let Some(launcher) = build_linux_sandbox_command(command, cwd, sandbox_status) {
        let mut prepared = TokioCommand::new(launcher.program);
        prepared.args(launcher.args);
        prepared.current_dir(cwd);
        prepared.envs(launcher.env);
        configure_bash_tool_env(&mut prepared);
        return Ok(prepared);
    }

    let mut prepared = TokioCommand::new(bash_shell_path()?);
    prepared.arg("-lc").arg(command);

    prepared.current_dir(cwd);
    configure_bash_tool_env(&mut prepared);
    if sandbox_status.filesystem_active {
        configure_bash_sandbox_env(&mut prepared, cwd);
    }
    Ok(prepared)
}

fn prepare_sandbox_dirs(cwd: &std::path::Path) {
    let _ = std::fs::create_dir_all(cwd.join(".sandbox-home"));
    let _ = std::fs::create_dir_all(cwd.join(".sandbox-tmp"));
}

fn configure_bash_tool_env<T>(command: &mut T)
where
    T: BashEnvCommand,
{
    // Keep Python stdout/stderr deterministic when bash runs under Windows
    // console code pages. AppFS action files are JSONL and are decoded as UTF-8.
    command.env("PYTHONIOENCODING", PYTHON_IO_ENCODING);
}

fn configure_bash_sandbox_env<T>(command: &mut T, cwd: &std::path::Path)
where
    T: BashEnvCommand,
{
    let home = cwd.join(".sandbox-home");
    let tmpdir = cwd.join(".sandbox-tmp");

    #[cfg(windows)]
    {
        command.env("HOME", windows_path_to_posix_path(&home));
        command.env("TMPDIR", windows_path_to_posix_path(&tmpdir));
    }

    #[cfg(not(windows))]
    {
        command.env("HOME", home);
        command.env("TMPDIR", tmpdir);
    }
}

trait BashEnvCommand {
    fn env<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>;
}

impl BashEnvCommand for Command {
    fn env<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
    {
        Command::env(self, key, value)
    }
}

impl BashEnvCommand for TokioCommand {
    fn env<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
    {
        TokioCommand::env(self, key, value)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_command_output, execute_bash, prepare_background_shell_output,
        prepare_shell_command_output, BashCommandInput, MAX_OUTPUT_BYTES,
    };
    use crate::tool_session::with_tool_session_context;
    #[cfg(windows)]
    use crate::{bash_shell_path, set_shell_if_windows, test_env_lock};
    use std::fs;
    use std::path::PathBuf;
    #[cfg(windows)]
    use std::process::Command;
    #[cfg(windows)]
    use std::sync::OnceLock;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(windows)]
    fn windows_bash_smoke_ok() -> bool {
        static OK: OnceLock<bool> = OnceLock::new();
        *OK.get_or_init(|| {
            let _guard = test_env_lock();
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

    fn success_command() -> String {
        String::from("printf 'hello'")
    }

    const TEST_TIMEOUT_MS: u64 = 5_000;

    fn temp_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("clawd-runtime-bash-{unique}-{name}"))
    }

    #[test]
    fn executes_simple_command() {
        #[cfg(windows)]
        if !windows_bash_smoke_ok() {
            return;
        }
        #[cfg(windows)]
        let _guard = test_env_lock();
        #[cfg(windows)]
        set_shell_if_windows().expect("set shell");

        let output = execute_bash(BashCommandInput {
            command: success_command(),
            timeout: Some(TEST_TIMEOUT_MS),
            description: None,
            run_in_background: Some(false),
            // Keep the basic execution smoke test independent from sandbox
            // behavior, which is covered separately below.
            dangerously_disable_sandbox: Some(true),
            namespace_restrictions: None,
            isolate_network: None,
            filesystem_mode: None,
            allowed_mounts: None,
        })
        .expect("bash command should execute");

        #[cfg(windows)]
        let shell_path = bash_shell_path()
            .expect("resolve shell")
            .display()
            .to_string();
        #[cfg(not(windows))]
        let shell_path = String::from("sh");

        assert_eq!(
            output.stdout.trim(),
            "hello",
            "shell={shell_path} stderr={:?} cwd={}",
            output.stderr,
            std::env::current_dir().expect("cwd").display()
        );
        assert!(!output.interrupted);
        assert!(!output.sandbox_status.expect("sandbox status").enabled);
    }

    #[test]
    fn disables_sandbox_when_requested() {
        #[cfg(windows)]
        if !windows_bash_smoke_ok() {
            return;
        }
        #[cfg(windows)]
        let _guard = test_env_lock();
        #[cfg(windows)]
        set_shell_if_windows().expect("set shell");

        let output = execute_bash(BashCommandInput {
            command: success_command(),
            timeout: Some(TEST_TIMEOUT_MS),
            description: None,
            run_in_background: Some(false),
            dangerously_disable_sandbox: Some(true),
            namespace_restrictions: None,
            isolate_network: None,
            filesystem_mode: None,
            allowed_mounts: None,
        })
        .expect("bash command should execute");

        assert!(!output.sandbox_status.expect("sandbox status").enabled);
    }

    #[test]
    fn injects_pythonioencoding_utf8_into_bash_tool() {
        #[cfg(windows)]
        if !windows_bash_smoke_ok() {
            return;
        }
        #[cfg(windows)]
        let _guard = test_env_lock();
        #[cfg(windows)]
        set_shell_if_windows().expect("set shell");

        let output = execute_bash(BashCommandInput {
            command: String::from("printf '%s' \"$PYTHONIOENCODING\""),
            timeout: Some(TEST_TIMEOUT_MS),
            description: None,
            run_in_background: Some(false),
            dangerously_disable_sandbox: Some(true),
            namespace_restrictions: None,
            isolate_network: None,
            filesystem_mode: None,
            allowed_mounts: None,
        })
        .expect("bash command should execute");

        assert_eq!(output.stdout, "utf-8");
    }

    #[test]
    fn prepare_shell_command_output_persists_large_stdout() {
        let cwd = temp_path("persisted-output");
        fs::create_dir_all(&cwd).expect("create cwd");
        let stdout = vec![b'x'; MAX_OUTPUT_BYTES + 128];

        let prepared = prepare_shell_command_output(&cwd, "bash", &stdout, b"");

        let persisted_path = prepared
            .persisted_output_path
            .as_ref()
            .expect("persisted output path");
        let persisted_bytes = fs::read(persisted_path).expect("read persisted output");
        assert_eq!(persisted_bytes, stdout);
        assert_eq!(
            prepared.persisted_output_size,
            Some(u64::try_from(MAX_OUTPUT_BYTES + 128).expect("size fits u64"))
        );
        assert_eq!(prepared.stdout, "x".repeat(MAX_OUTPUT_BYTES));

        let _ = fs::remove_dir_all(cwd);
    }

    #[test]
    fn prepare_background_shell_output_creates_output_file() {
        let cwd = temp_path("background-output");
        fs::create_dir_all(&cwd).expect("create cwd");

        let prepared =
            prepare_background_shell_output(&cwd, "bash").expect("prepare background output");
        let output_path = PathBuf::from(&prepared.output_path);

        assert!(output_path.exists(), "expected {output_path:?} to exist");
        assert!(prepared.task_id.starts_with('b'));
        assert_eq!(prepared.task_id.len(), 9);

        drop(prepared);
        let _ = fs::remove_dir_all(cwd);
    }

    #[test]
    fn prepare_background_shell_output_uses_session_isolated_directory_when_context_exists() {
        let cwd = temp_path("session-background-output");
        let session_path = cwd
            .join(".claw")
            .join("sessions")
            .join("workspace-hash")
            .join("session-123.jsonl");
        fs::create_dir_all(
            session_path
                .parent()
                .expect("session path should have a parent"),
        )
        .expect("create session dir");

        let output_path = with_tool_session_context("session-123", Some(&session_path), || {
            prepare_background_shell_output(&cwd, "bash")
                .expect("prepare background output")
                .output_path
        });

        let output_path = PathBuf::from(output_path);
        assert!(
            output_path.starts_with(
                cwd.join(".claw")
                    .join("sessions")
                    .join("workspace-hash")
                    .join("session-123")
                    .join("tasks")
            ),
            "expected {output_path:?} to be under the session-specific tasks directory"
        );
        assert_eq!(
            output_path
                .parent()
                .expect("output path should have a parent"),
            cwd.join(".claw")
                .join("sessions")
                .join("workspace-hash")
                .join("session-123")
                .join("tasks")
        );

        let _ = fs::remove_dir_all(cwd);
    }

    #[test]
    fn prepare_shell_command_output_uses_session_isolated_tool_results_directory() {
        let cwd = temp_path("session-persisted-output");
        let session_path = cwd
            .join(".claw")
            .join("sessions")
            .join("workspace-hash")
            .join("session-456.jsonl");
        fs::create_dir_all(
            session_path
                .parent()
                .expect("session path should have a parent"),
        )
        .expect("create session dir");
        let stdout = vec![b'y'; MAX_OUTPUT_BYTES + 64];

        let prepared = with_tool_session_context("session-456", Some(&session_path), || {
            prepare_shell_command_output(&cwd, "bash", &stdout, b"")
        });

        let persisted_path = PathBuf::from(
            prepared
                .persisted_output_path
                .expect("persisted output path should exist"),
        );
        assert_eq!(
            persisted_path
                .parent()
                .expect("persisted path should have a parent"),
            cwd.join(".claw")
                .join("sessions")
                .join("workspace-hash")
                .join("session-456")
                .join("tool-results")
        );

        let _ = fs::remove_dir_all(cwd);
    }

    #[test]
    #[cfg(windows)]
    fn decode_command_output_falls_back_to_windows_ansi_for_gbk_bytes() {
        let bytes = [
            61, 61, 61, 32, 195, 176, 197, 221, 197, 197, 208, 242, 32, 61, 61, 61, 10, 212, 173,
            202, 253, 215, 233, 58, 32, 91, 49, 44, 32, 50, 44, 32, 51, 93, 10,
        ];

        assert_eq!(
            decode_command_output(&bytes),
            "=== 冒泡排序 ===\n原数组: [1, 2, 3]\n"
        );
    }
}

/// Maximum output bytes before truncation (16 KiB, matching upstream).
const MAX_OUTPUT_BYTES: usize = 16_384;

/// Truncate output to `MAX_OUTPUT_BYTES`, appending a marker when trimmed.
fn truncate_output(s: &str) -> String {
    if s.len() <= MAX_OUTPUT_BYTES {
        return s.to_string();
    }
    // Find the last valid UTF-8 boundary at or before MAX_OUTPUT_BYTES
    let mut end = MAX_OUTPUT_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = s[..end].to_string();
    truncated.push_str("\n\n[output truncated — exceeded 16384 bytes]");
    truncated
}

#[cfg(test)]
mod truncation_tests {
    use super::*;

    #[test]
    fn short_output_unchanged() {
        let s = "hello world";
        assert_eq!(truncate_output(s), s);
    }

    #[test]
    fn long_output_truncated() {
        let s = "x".repeat(20_000);
        let result = truncate_output(&s);
        assert!(result.len() < 20_000);
        assert!(result.ends_with("[output truncated — exceeded 16384 bytes]"));
    }

    #[test]
    fn exact_boundary_unchanged() {
        let s = "a".repeat(MAX_OUTPUT_BYTES);
        assert_eq!(truncate_output(&s), s);
    }

    #[test]
    fn one_over_boundary_truncated() {
        let s = "a".repeat(MAX_OUTPUT_BYTES + 1);
        let result = truncate_output(&s);
        assert!(result.contains("[output truncated"));
    }
}
