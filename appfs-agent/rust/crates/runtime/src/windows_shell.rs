use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
const GIT_BASH_ENV_VARS: [&str; 2] = ["CLAW_CODE_GIT_BASH_PATH", "CLAUDE_CODE_GIT_BASH_PATH"];
#[cfg(windows)]
const SHELL_OVERRIDE_ENV_VARS: [&str; 2] = ["CLAW_CODE_SHELL", "CLAUDE_CODE_SHELL"];
#[cfg(windows)]
const GIT_BASH_REQUIRED_MESSAGE: &str =
    "Claw Code on Windows requires git-bash (https://git-scm.com/downloads/win). \
If installed but not in PATH, set CLAW_CODE_GIT_BASH_PATH to your bash.exe, \
for example: CLAW_CODE_GIT_BASH_PATH=C:\\Program Files\\Git\\bin\\bash.exe";

pub fn set_shell_if_windows() -> io::Result<()> {
    #[cfg(windows)]
    {
        env::set_var("SHELL", find_git_bash_path()?);
    }

    Ok(())
}

pub fn bash_shell_path() -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        return resolve_windows_bash_shell_path();
    }

    #[cfg(not(windows))]
    {
        Ok(PathBuf::from("sh"))
    }
}

#[cfg(windows)]
pub fn windows_path_to_posix_path(path: &Path) -> String {
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

#[cfg(windows)]
fn resolve_windows_bash_shell_path() -> io::Result<PathBuf> {
    if let Some(path) = resolve_supported_shell_from_env(&SHELL_OVERRIDE_ENV_VARS)? {
        return Ok(path);
    }
    if let Some(path) = resolve_supported_shell_from_env(&["SHELL"])? {
        return Ok(path);
    }
    find_git_bash_path()
}

#[cfg(windows)]
fn resolve_supported_shell_from_env(names: &[&str]) -> io::Result<Option<PathBuf>> {
    for name in names {
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

#[cfg(test)]
mod tests {
    use super::{bash_shell_path, set_shell_if_windows};

    #[cfg(windows)]
    use std::path::Path;

    #[cfg(windows)]
    use super::windows_path_to_posix_path;

    #[cfg(windows)]
    fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }

    #[cfg(windows)]
    #[test]
    fn set_shell_if_windows_prefers_explicit_git_bash_path() {
        let _guard = crate::test_env_lock();
        let bash_root = std::env::temp_dir().join(format!(
            "claw-runtime-bash-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&bash_root).expect("create temp dir");
        let bash_path = bash_root.join("bash.exe");
        std::fs::write(&bash_path, b"").expect("write fake bash");

        let old_shell = std::env::var_os("SHELL");
        let old_claw = std::env::var_os("CLAW_CODE_GIT_BASH_PATH");
        let old_claude = std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH");
        std::env::set_var("CLAW_CODE_GIT_BASH_PATH", &bash_path);
        std::env::remove_var("CLAUDE_CODE_GIT_BASH_PATH");
        std::env::remove_var("SHELL");

        set_shell_if_windows().expect("set shell");
        assert_eq!(
            std::env::var_os("SHELL").as_deref(),
            Some(bash_path.as_os_str())
        );

        restore_env("SHELL", old_shell);
        restore_env("CLAW_CODE_GIT_BASH_PATH", old_claw);
        restore_env("CLAUDE_CODE_GIT_BASH_PATH", old_claude);
        let _ = std::fs::remove_dir_all(bash_root);
    }

    #[cfg(windows)]
    #[test]
    fn bash_shell_path_ignores_windowsapps_bash_shims() {
        let _guard = crate::test_env_lock();
        let temp_root = std::env::temp_dir().join(format!(
            "claw-runtime-windowsapps-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let windowsapps_root = temp_root.join("AppData\\Local\\Microsoft\\WindowsApps");
        let shim_path = windowsapps_root.join("bash.exe");
        std::fs::create_dir_all(&windowsapps_root).expect("create windowsapps temp dir");
        std::fs::write(&shim_path, b"").expect("write fake shim");

        let git_bash_root = temp_root.join("Git\\bin");
        let git_bash_path = git_bash_root.join("bash.exe");
        std::fs::create_dir_all(&git_bash_root).expect("create git bash temp dir");
        std::fs::write(&git_bash_path, b"").expect("write fake git bash");

        let old_shell = std::env::var_os("SHELL");
        let old_claw_shell = std::env::var_os("CLAW_CODE_SHELL");
        let old_claw_git_bash = std::env::var_os("CLAW_CODE_GIT_BASH_PATH");
        let old_claude_git_bash = std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH");

        std::env::set_var("CLAW_CODE_SHELL", &shim_path);
        std::env::set_var("CLAW_CODE_GIT_BASH_PATH", &git_bash_path);
        std::env::remove_var("CLAUDE_CODE_GIT_BASH_PATH");
        std::env::remove_var("SHELL");

        assert_eq!(
            bash_shell_path().expect("resolve bash shell"),
            git_bash_path
        );

        restore_env("SHELL", old_shell);
        restore_env("CLAW_CODE_SHELL", old_claw_shell);
        restore_env("CLAW_CODE_GIT_BASH_PATH", old_claw_git_bash);
        restore_env("CLAUDE_CODE_GIT_BASH_PATH", old_claude_git_bash);
        let _ = std::fs::remove_dir_all(temp_root);
    }

    #[cfg(windows)]
    #[test]
    fn converts_windows_paths_to_git_bash_style() {
        assert_eq!(
            windows_path_to_posix_path(Path::new(r"C:\Users\esp3j\rep")),
            "/c/Users/esp3j/rep"
        );
        assert_eq!(
            windows_path_to_posix_path(Path::new(r"\\server\share\dir")),
            "//server/share/dir"
        );
    }
}
