use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::tool_output::tool_output_root;
use crate::tool_session::{current_tool_session_id, current_tool_session_storage_root};

static SHELL_CWDS: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
static CWD_TRACKING_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellCwdPathFormat {
    Native,
    Bash,
}

pub fn current_shell_cwd() -> io::Result<PathBuf> {
    let process_cwd = std::env::current_dir()?;
    let key = shell_cwd_key(&process_cwd);
    let mut store = shell_cwd_store()?;

    if let Some(saved) = store.get(&key) {
        if let Ok(cwd) = saved.canonicalize() {
            return Ok(cwd);
        }
        store.remove(&key);
    }

    process_cwd.canonicalize()
}

pub fn update_shell_cwd(path: &Path) -> io::Result<()> {
    let Ok(cwd) = path.canonicalize() else {
        return Ok(());
    };

    let process_cwd = std::env::current_dir()?;
    let key = shell_cwd_key(&process_cwd);
    shell_cwd_store()?.insert(key, cwd);
    Ok(())
}

pub fn shell_cwd_tracking_file(shell: &str) -> io::Result<PathBuf> {
    let process_cwd = std::env::current_dir()?;
    let dir = tool_output_root(&process_cwd).join("shell-cwd");
    fs::create_dir_all(&dir)?;

    let safe_shell = shell
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let counter = CWD_TRACKING_COUNTER.fetch_add(1, Ordering::Relaxed);

    Ok(dir.join(format!(
        "{}-{}-{counter}.cwd",
        safe_shell,
        std::process::id()
    )))
}

pub fn update_shell_cwd_from_tracking_file(
    path: &Path,
    format: ShellCwdPathFormat,
) -> io::Result<()> {
    let contents = fs::read_to_string(path)?;
    let cwd = contents.trim();
    if !cwd.is_empty() {
        update_shell_cwd(&reported_cwd_to_native_path(cwd, format))?;
    }
    let _ = fs::remove_file(path);
    Ok(())
}

fn shell_cwd_store() -> io::Result<std::sync::MutexGuard<'static, HashMap<String, PathBuf>>> {
    SHELL_CWDS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "shell cwd state lock poisoned"))
}

fn shell_cwd_key(process_cwd: &Path) -> String {
    if let Some(root) = current_tool_session_storage_root(process_cwd) {
        return format!("session-root:{}", root.display());
    }

    if let Some(session_id) = current_tool_session_id() {
        return format!("session-id:{session_id}");
    }

    let fallback = process_cwd
        .canonicalize()
        .unwrap_or_else(|_| process_cwd.to_path_buf());
    format!("process:{}", fallback.display())
}

fn reported_cwd_to_native_path(path: &str, format: ShellCwdPathFormat) -> PathBuf {
    match format {
        ShellCwdPathFormat::Native => PathBuf::from(path),
        ShellCwdPathFormat::Bash => {
            #[cfg(windows)]
            {
                crate::posix_path_to_windows_path(path)
            }
            #[cfg(not(windows))]
            {
                PathBuf::from(path)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        current_shell_cwd, shell_cwd_tracking_file, update_shell_cwd,
        update_shell_cwd_from_tracking_file, ShellCwdPathFormat,
    };
    use crate::test_env_lock;
    use crate::tool_session::with_tool_session_context;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("runtime-shell-cwd-{unique}-{name}"))
    }

    #[test]
    fn shell_cwd_persists_per_tool_session() {
        let _guard = test_env_lock();
        let root = temp_dir("persist");
        let subdir = root.join("subdir");
        fs::create_dir_all(&subdir).expect("create subdir");
        let session_path = root.join(".claw").join("sessions").join("session-1.jsonl");
        fs::create_dir_all(session_path.parent().expect("session parent"))
            .expect("create session parent");

        let original = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(&root).expect("set cwd");

        with_tool_session_context("session-1", Some(&session_path), || {
            assert_eq!(
                current_shell_cwd().expect("initial shell cwd"),
                root.canonicalize().expect("canonical root")
            );
            update_shell_cwd(&subdir).expect("update shell cwd");
            assert_eq!(
                current_shell_cwd().expect("updated shell cwd"),
                subdir.canonicalize().expect("canonical subdir")
            );
        });

        std::env::set_current_dir(original).expect("restore cwd");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_cwd_update_is_ignored() {
        let _guard = test_env_lock();
        let root = temp_dir("missing");
        fs::create_dir_all(&root).expect("create root");
        let missing = root.join("missing");
        let original = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(&root).expect("set cwd");

        update_shell_cwd(&missing).expect("missing cwd should be ignored");
        assert_eq!(
            current_shell_cwd().expect("shell cwd"),
            root.canonicalize().expect("canonical root")
        );

        std::env::set_current_dir(original).expect("restore cwd");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tracking_file_updates_native_cwd() {
        let _guard = test_env_lock();
        let root = temp_dir("tracking");
        let subdir = root.join("subdir");
        fs::create_dir_all(&subdir).expect("create subdir");
        let original = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(&root).expect("set cwd");

        let tracking = shell_cwd_tracking_file("bash").expect("tracking file");
        fs::write(&tracking, subdir.display().to_string()).expect("write tracking file");
        update_shell_cwd_from_tracking_file(&tracking, ShellCwdPathFormat::Native)
            .expect("update from tracking file");

        assert_eq!(
            current_shell_cwd().expect("shell cwd"),
            subdir.canonicalize().expect("canonical subdir")
        );
        assert!(!tracking.exists());

        std::env::set_current_dir(original).expect("restore cwd");
        let _ = fs::remove_dir_all(root);
    }
}
