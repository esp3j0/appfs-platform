use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
const GIT_BASH_ENV_VARS: [&str; 2] = ["CLAW_CODE_GIT_BASH_PATH", "CLAUDE_CODE_GIT_BASH_PATH"];
#[cfg(windows)]
const SHELL_OVERRIDE_ENV_VARS: [&str; 3] = ["CLAW_CODE_SHELL", "CLAUDE_CODE_SHELL", "SHELL"];
#[cfg(windows)]
const GIT_BASH_REQUIRED_MESSAGE: &str =
    "Claw Code on Windows requires git-bash (https://git-scm.com/downloads/win). \
If installed but not in PATH, set CLAW_CODE_GIT_BASH_PATH to your bash.exe, \
for example: CLAW_CODE_GIT_BASH_PATH=C:\\Program Files\\Git\\bin\\bash.exe";

pub(crate) fn prepare_hook_command(entry: &str) -> io::Result<Command> {
    if Path::new(entry).exists() {
        return prepare_script_command(Path::new(entry));
    }
    prepare_shell_command(entry)
}

pub(crate) fn prepare_tool_command(entry: &str) -> io::Result<(Command, bool)> {
    let path = Path::new(entry);
    if path.exists() && is_bash_script(path) {
        return Ok((prepare_script_command(path)?, true));
    }
    Ok((Command::new(entry), false))
}

pub(crate) fn env_path_for_bash(path: &Path) -> String {
    #[cfg(windows)]
    {
        windows_path_to_posix_path(path)
    }

    #[cfg(not(windows))]
    {
        path.display().to_string()
    }
}

fn prepare_shell_command(command: &str) -> io::Result<Command> {
    #[cfg(windows)]
    {
        let mut process = Command::new(resolve_windows_bash_shell_path()?);
        process.arg("-lc").arg(command);
        Ok(process)
    }

    #[cfg(not(windows))]
    {
        let mut process = Command::new("sh");
        process.arg("-lc").arg(command);
        Ok(process)
    }
}

fn prepare_script_command(script: &Path) -> io::Result<Command> {
    #[cfg(windows)]
    {
        let mut process = Command::new(resolve_windows_bash_shell_path()?);
        process.arg(windows_path_to_posix_path(script));
        Ok(process)
    }

    #[cfg(not(windows))]
    {
        let mut process = Command::new("sh");
        process.arg(script);
        Ok(process)
    }
}

fn is_bash_script(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("sh"))
}

#[cfg(windows)]
fn resolve_windows_bash_shell_path() -> io::Result<PathBuf> {
    if let Some(path) = resolve_supported_shell_from_env()? {
        return Ok(path);
    }
    find_git_bash_path()
}

#[cfg(windows)]
fn resolve_supported_shell_from_env() -> io::Result<Option<PathBuf>> {
    for name in SHELL_OVERRIDE_ENV_VARS {
        if let Some(value) = env::var_os(name) {
            let candidate = PathBuf::from(value);
            if candidate.as_os_str().is_empty() {
                continue;
            }
            if is_supported_shell_path(&candidate) {
                return Ok(Some(candidate));
            }
        }
    }

    Ok(None)
}

#[cfg(windows)]
fn find_git_bash_path() -> io::Result<PathBuf> {
    for name in GIT_BASH_ENV_VARS {
        if let Some(value) = env::var_os(name) {
            let candidate = PathBuf::from(value);
            if candidate.is_file() {
                return Ok(candidate);
            }
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "Claw Code was unable to find {name} path \"{}\"",
                    candidate.display()
                ),
            ));
        }
    }

    if let Some(git_path) = find_executable("git") {
        let bash_path = git_path
            .parent()
            .and_then(Path::parent)
            .map(|root| root.join("bin").join("bash.exe"));
        if let Some(path) = bash_path.filter(|path| path.is_file()) {
            return Ok(path);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        GIT_BASH_REQUIRED_MESSAGE,
    ))
}

#[cfg(windows)]
fn find_executable(executable: &str) -> Option<PathBuf> {
    if executable.eq_ignore_ascii_case("git") {
        for location in [
            PathBuf::from(r"C:\Program Files\Git\cmd\git.exe"),
            PathBuf::from(r"C:\Program Files (x86)\Git\cmd\git.exe"),
        ] {
            if location.is_file() {
                return Some(location);
            }
        }
    }

    let output = Command::new("where.exe").arg(executable).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let cwd = env::current_dir()
        .ok()
        .as_deref()
        .map(normalize_windows_path_for_compare);

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let candidate = PathBuf::from(line.trim());
        if candidate.as_os_str().is_empty() || !candidate.is_file() {
            continue;
        }

        if let Some(cwd) = &cwd {
            let candidate_dir = candidate.parent().map(normalize_windows_path_for_compare);
            if let Some(candidate_dir) = candidate_dir {
                if candidate_dir == *cwd || candidate_dir.starts_with(&format!("{cwd}\\")) {
                    continue;
                }
            }
        }

        return Some(candidate);
    }

    None
}

#[cfg(windows)]
fn normalize_windows_path_for_compare(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

#[cfg(windows)]
fn is_supported_shell_path(path: &Path) -> bool {
    let normalized = normalize_windows_path_for_compare(path);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);

    matches!(
        file_name.as_deref(),
        Some(name) if (name.contains("bash") || name.contains("zsh")) && path.is_file()
    ) && !normalized.contains(r"\appdata\local\microsoft\windowsapps\")
}

#[cfg(windows)]
fn windows_path_to_posix_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(rest) = path.strip_prefix(r"\\") {
        return format!("//{}", rest.replace('\\', "/"));
    }

    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/') {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let suffix = path[2..].replace('\\', "/");
        return format!("/{drive}{suffix}");
    }

    path.replace('\\', "/")
}
