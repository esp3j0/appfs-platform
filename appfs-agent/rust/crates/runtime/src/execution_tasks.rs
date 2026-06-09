use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

use crate::hooks::HookAbortSignal;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTaskStatus {
    Running,
    Completed,
    Failed,
    Killed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ExecutionTaskSnapshot {
    #[serde(rename = "taskId")]
    pub task_id: String,
    pub kind: String,
    pub status: ExecutionTaskStatus,
    #[serde(rename = "outputFile")]
    pub output_file: String,
}

enum ExecutionTaskController {
    Child(Child),
    AbortSignal(HookAbortSignal),
}

struct RegisteredExecutionTask {
    task_id: String,
    kind: String,
    output_file: PathBuf,
    status: ExecutionTaskStatus,
    controller: ExecutionTaskController,
}

static EXECUTION_TASKS: OnceLock<Mutex<BTreeMap<String, RegisteredExecutionTask>>> =
    OnceLock::new();

pub fn register_child_execution_task(
    task_id: impl Into<String>,
    kind: impl Into<String>,
    output_file: impl Into<PathBuf>,
    child: Child,
) {
    let task_id = task_id.into();
    let task = RegisteredExecutionTask {
        task_id: task_id.clone(),
        kind: kind.into(),
        output_file: output_file.into(),
        status: ExecutionTaskStatus::Running,
        controller: ExecutionTaskController::Child(child),
    };
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(task_id, task);
}

pub fn register_abortable_execution_task(
    task_id: impl Into<String>,
    kind: impl Into<String>,
    output_file: impl Into<PathBuf>,
    abort_signal: HookAbortSignal,
) {
    let task_id = task_id.into();
    let task = RegisteredExecutionTask {
        task_id: task_id.clone(),
        kind: kind.into(),
        output_file: output_file.into(),
        status: ExecutionTaskStatus::Running,
        controller: ExecutionTaskController::AbortSignal(abort_signal),
    };
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(task_id, task);
}

pub fn mark_execution_task_status(task_id: &str, status: ExecutionTaskStatus) {
    if let Some(task) = registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_mut(task_id)
    {
        task.status = status;
    }
}

pub fn unregister_execution_task(task_id: &str) {
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(task_id);
}

pub fn execution_task_snapshot(task_id: &str) -> Option<ExecutionTaskSnapshot> {
    let mut tasks = registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let task = tasks.get_mut(task_id)?;
    refresh_child_status(task);
    Some(task.snapshot())
}

pub fn stop_execution_task(task_id: &str) -> Result<ExecutionTaskSnapshot, String> {
    let mut tasks = registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let task = tasks
        .get_mut(task_id)
        .ok_or_else(|| format!("background execution task not found: {task_id}"))?;

    refresh_child_status(task);
    if task.status != ExecutionTaskStatus::Running {
        return Ok(task.snapshot());
    }

    match &mut task.controller {
        ExecutionTaskController::Child(child) => {
            child.kill().map_err(|error| error.to_string())?;
            let _ = child.wait();
            task.status = ExecutionTaskStatus::Killed;
        }
        ExecutionTaskController::AbortSignal(abort_signal) => {
            abort_signal.abort();
            task.status = ExecutionTaskStatus::Killed;
        }
    }

    Ok(task.snapshot())
}

pub fn execution_task_output_file(task_id: &str) -> Option<PathBuf> {
    execution_task_snapshot(task_id).map(|snapshot| PathBuf::from(snapshot.output_file))
}

pub fn read_execution_task_output(path: &Path) -> Result<String, String> {
    std::fs::read(path)
        .map(|bytes| crate::bash::decode_command_output(&bytes))
        .map_err(|error| error.to_string())
}

fn registry() -> &'static Mutex<BTreeMap<String, RegisteredExecutionTask>> {
    EXECUTION_TASKS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn refresh_child_status(task: &mut RegisteredExecutionTask) {
    if task.status != ExecutionTaskStatus::Running {
        return;
    }
    let ExecutionTaskController::Child(child) = &mut task.controller else {
        return;
    };
    match child.try_wait() {
        Ok(Some(status)) => {
            task.status = if status.success() {
                ExecutionTaskStatus::Completed
            } else {
                ExecutionTaskStatus::Failed
            };
        }
        Ok(None) | Err(_) => {}
    }
}

impl RegisteredExecutionTask {
    fn snapshot(&self) -> ExecutionTaskSnapshot {
        ExecutionTaskSnapshot {
            task_id: self.task_id.clone(),
            kind: self.kind.clone(),
            status: self.status,
            output_file: self.output_file.to_string_lossy().into_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        execution_task_snapshot, register_child_execution_task, stop_execution_task,
        ExecutionTaskStatus,
    };
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn stop_execution_task_kills_registered_child() {
        let task_id = format!(
            "exec-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        );
        let mut command = long_running_command();
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn child");
        register_child_execution_task(
            task_id.clone(),
            "test",
            PathBuf::from(format!("{task_id}.output")),
            child,
        );

        let stopped = stop_execution_task(&task_id).expect("stop child");
        assert_eq!(stopped.status, ExecutionTaskStatus::Killed);
        let snapshot = execution_task_snapshot(&task_id).expect("snapshot");
        assert_eq!(snapshot.status, ExecutionTaskStatus::Killed);
    }

    fn long_running_command() -> Command {
        #[cfg(windows)]
        {
            let mut command = Command::new("cmd");
            command.args(["/C", "ping -n 6 127.0.0.1 >NUL"]);
            command
        }
        #[cfg(not(windows))]
        {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 5"]);
            command
        }
    }
}
