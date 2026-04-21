use std::path::{Path, PathBuf};

use crate::tool_session::current_tool_session_storage_root;

const CLAW_STATE_DIR: &str = ".claw";
const TOOL_RESULTS_DIR: &str = "tool-results";
const TASK_OUTPUTS_DIR: &str = "tasks";

#[must_use]
pub fn tool_output_root(cwd: &Path) -> PathBuf {
    current_tool_session_storage_root(cwd).unwrap_or_else(|| cwd.join(CLAW_STATE_DIR))
}

#[must_use]
pub fn tool_results_dir(cwd: &Path) -> PathBuf {
    tool_output_root(cwd).join(TOOL_RESULTS_DIR)
}

#[must_use]
pub fn task_outputs_dir(cwd: &Path) -> PathBuf {
    tool_output_root(cwd).join(TASK_OUTPUTS_DIR)
}

#[must_use]
pub fn tool_result_path(cwd: &Path, tool_use_id: &str, extension: &str) -> PathBuf {
    let extension = extension.trim_start_matches('.');
    let safe_id = sanitize_tool_result_identifier(tool_use_id);
    tool_results_dir(cwd).join(format!("{safe_id}.{extension}"))
}

fn sanitize_tool_result_identifier(tool_use_id: &str) -> String {
    let sanitized = tool_use_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    if sanitized.is_empty() {
        String::from("tool-result")
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::{task_outputs_dir, tool_result_path, tool_results_dir};
    use crate::tool_session::with_tool_session_context;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("runtime-tool-output-{unique}-{name}"))
    }

    #[test]
    fn session_scoped_directories_live_under_session_root() {
        let cwd = temp_dir("session-root");
        let persistence_path = cwd
            .join(".claw")
            .join("sessions")
            .join("workspace-hash")
            .join("session-123.jsonl");
        fs::create_dir_all(
            persistence_path
                .parent()
                .expect("persistence path should have a parent"),
        )
        .expect("create session dir");

        let (results_dir, tasks_dir) =
            with_tool_session_context("session-123", Some(&persistence_path), || {
                (tool_results_dir(&cwd), task_outputs_dir(&cwd))
            });

        assert_eq!(
            results_dir,
            cwd.join(".claw")
                .join("sessions")
                .join("workspace-hash")
                .join("session-123")
                .join("tool-results")
        );
        assert_eq!(
            tasks_dir,
            cwd.join(".claw")
                .join("sessions")
                .join("workspace-hash")
                .join("session-123")
                .join("tasks")
        );

        let _ = fs::remove_dir_all(cwd);
    }

    #[test]
    fn tool_result_path_sanitizes_tool_use_identifier() {
        let cwd = temp_dir("sanitize-id");
        let path = tool_result_path(&cwd, "call/with spaces", "txt");
        assert!(
            path.ends_with("call_with_spaces.txt"),
            "unexpected tool result path: {path:?}"
        );
    }
}
