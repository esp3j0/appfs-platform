use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use fd_lock::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::ConfigLoader;
use crate::tool_session::current_tool_session_id;

const HIGH_WATER_MARK_FILE: &str = ".highwatermark";
const LOCK_FILE: &str = ".lock";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskBoardStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskBoardTask {
    pub id: String,
    pub subject: String,
    pub description: String,
    #[serde(rename = "activeForm", skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub status: TaskBoardStatus,
    #[serde(default)]
    pub blocks: Vec<String>,
    #[serde(rename = "blockedBy", default)]
    pub blocked_by: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskBoardPatch {
    pub subject: Option<String>,
    pub description: Option<String>,
    pub active_form: Option<String>,
    pub owner: Option<String>,
    pub status: Option<TaskBoardStatus>,
    pub metadata: Option<BTreeMap<String, Value>>,
    pub add_blocks: Vec<String>,
    pub add_blocked_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskBoardUpdateOutcome {
    pub task: TaskBoardTask,
    pub updated_fields: Vec<String>,
    pub status_change: Option<(TaskBoardStatus, TaskBoardStatus)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskBoardClaimOutcome {
    pub task: TaskBoardTask,
    pub claimed: bool,
    pub current_owner: Option<String>,
    pub reason: Option<TaskBoardClaimRejectionReason>,
    pub blocking_tasks: Vec<TaskBoardTask>,
    pub active_task: Option<TaskBoardTask>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskBoardClaimRejectionReason {
    AlreadyClaimed,
    AlreadyCompleted,
    Blocked,
    OwnerBusy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBoardStore {
    root: PathBuf,
    task_list_id: String,
}

impl TaskBoardStore {
    #[must_use]
    pub fn default_for_cwd(cwd: &Path) -> Self {
        let loader = ConfigLoader::default_for(cwd);
        Self::new(loader.config_home().join("tasks"), resolve_task_list_id())
    }

    #[must_use]
    pub fn new(root: PathBuf, task_list_id: String) -> Self {
        Self {
            root,
            task_list_id: sanitize_path_component(&task_list_id),
        }
    }

    #[must_use]
    pub fn task_list_id(&self) -> &str {
        &self.task_list_id
    }

    #[must_use]
    pub fn task_list_dir(&self) -> PathBuf {
        self.root.join(&self.task_list_id)
    }

    pub fn create(
        &self,
        subject: String,
        description: String,
        active_form: Option<String>,
        metadata: Option<BTreeMap<String, Value>>,
    ) -> Result<TaskBoardTask, String> {
        self.with_write_lock(|store| {
            let id = (store.highest_task_id_unlocked()? + 1).to_string();
            let task = TaskBoardTask {
                id: id.clone(),
                subject,
                description,
                active_form,
                owner: None,
                status: TaskBoardStatus::Pending,
                blocks: Vec::new(),
                blocked_by: Vec::new(),
                metadata,
            };
            store.write_task_unlocked(&task)?;
            store.write_high_water_mark_unlocked(id.parse::<u64>().unwrap_or_default())?;
            Ok(task)
        })
    }

    pub fn get(&self, task_id: &str) -> Result<Option<TaskBoardTask>, String> {
        self.with_read_lock(|store| store.read_task_unlocked(task_id))
    }

    pub fn list(&self) -> Result<Vec<TaskBoardTask>, String> {
        self.with_read_lock(Self::list_unlocked)
    }

    pub fn update(
        &self,
        task_id: &str,
        patch: TaskBoardPatch,
    ) -> Result<Option<TaskBoardUpdateOutcome>, String> {
        self.with_write_lock(|store| {
            let Some(mut task) = store.read_task_unlocked(task_id)? else {
                return Ok(None);
            };

            let original_status = task.status;
            let mut updated_fields = Vec::new();

            if let Some(subject) = patch.subject {
                if subject != task.subject {
                    task.subject = subject;
                    updated_fields.push(String::from("subject"));
                }
            }
            if let Some(description) = patch.description {
                if description != task.description {
                    task.description = description;
                    updated_fields.push(String::from("description"));
                }
            }
            if let Some(active_form) = patch.active_form {
                if task.active_form.as_deref() != Some(active_form.as_str()) {
                    task.active_form = Some(active_form);
                    updated_fields.push(String::from("activeForm"));
                }
            }
            if let Some(owner) = patch.owner {
                if task.owner.as_deref() != Some(owner.as_str()) {
                    task.owner = Some(owner);
                    updated_fields.push(String::from("owner"));
                }
            }
            if let Some(status) = patch.status {
                if status != task.status {
                    task.status = status;
                    updated_fields.push(String::from("status"));
                }
            }
            if let Some(metadata_patch) = patch.metadata {
                let mut metadata = task.metadata.unwrap_or_default();
                for (key, value) in metadata_patch {
                    if value.is_null() {
                        metadata.remove(&key);
                    } else {
                        metadata.insert(key, value);
                    }
                }
                task.metadata = Some(metadata);
                updated_fields.push(String::from("metadata"));
            }

            if !updated_fields.is_empty() {
                store.write_task_unlocked(&task)?;
            }

            if store.add_blocks_unlocked(task_id, &patch.add_blocks)? {
                updated_fields.push(String::from("blocks"));
            }
            if store.add_blocked_by_unlocked(task_id, &patch.add_blocked_by)? {
                updated_fields.push(String::from("blockedBy"));
            }

            let task = store
                .read_task_unlocked(task_id)?
                .expect("task should still exist after update");
            let status_change =
                (original_status != task.status).then_some((original_status, task.status));

            Ok(Some(TaskBoardUpdateOutcome {
                task,
                updated_fields,
                status_change,
            }))
        })
    }

    pub fn claim(
        &self,
        task_id: &str,
        owner: String,
        active_form: Option<String>,
        metadata: Option<BTreeMap<String, Value>>,
    ) -> Result<Option<TaskBoardClaimOutcome>, String> {
        self.with_write_lock(|store| {
            let Some(mut task) = store.read_task_unlocked(task_id)? else {
                return Ok(None);
            };
            if task.status == TaskBoardStatus::Completed {
                return Ok(Some(TaskBoardClaimOutcome {
                    task,
                    claimed: false,
                    current_owner: None,
                    reason: Some(TaskBoardClaimRejectionReason::AlreadyCompleted),
                    blocking_tasks: Vec::new(),
                    active_task: None,
                }));
            }
            if let Some(current_owner) = task.owner.as_deref() {
                if current_owner != owner {
                    let current_owner = current_owner.to_string();
                    return Ok(Some(TaskBoardClaimOutcome {
                        task,
                        claimed: false,
                        current_owner: Some(current_owner),
                        reason: Some(TaskBoardClaimRejectionReason::AlreadyClaimed),
                        blocking_tasks: Vec::new(),
                        active_task: None,
                    }));
                }
            }

            let blocking_tasks = store.unfinished_blocking_tasks_unlocked(&task)?;
            if !blocking_tasks.is_empty() {
                return Ok(Some(TaskBoardClaimOutcome {
                    task,
                    claimed: false,
                    current_owner: None,
                    reason: Some(TaskBoardClaimRejectionReason::Blocked),
                    blocking_tasks,
                    active_task: None,
                }));
            }

            if let Some(active_task) = store.active_task_for_owner_unlocked(&owner, task_id)? {
                return Ok(Some(TaskBoardClaimOutcome {
                    task,
                    claimed: false,
                    current_owner: None,
                    reason: Some(TaskBoardClaimRejectionReason::OwnerBusy),
                    blocking_tasks: Vec::new(),
                    active_task: Some(active_task),
                }));
            }

            task.owner = Some(owner);
            task.status = TaskBoardStatus::InProgress;
            if let Some(active_form) = active_form {
                task.active_form = Some(active_form);
            }
            if let Some(metadata_patch) = metadata {
                let mut existing = task.metadata.unwrap_or_default();
                for (key, value) in metadata_patch {
                    if value.is_null() {
                        existing.remove(&key);
                    } else {
                        existing.insert(key, value);
                    }
                }
                task.metadata = Some(existing);
            }
            store.write_task_unlocked(&task)?;
            Ok(Some(TaskBoardClaimOutcome {
                task,
                claimed: true,
                current_owner: None,
                reason: None,
                blocking_tasks: Vec::new(),
                active_task: None,
            }))
        })
    }

    pub fn delete(&self, task_id: &str) -> Result<bool, String> {
        self.with_write_lock(|store| {
            if store.read_task_unlocked(task_id)?.is_none() {
                return Ok(false);
            }

            let path = store.task_path(task_id);
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }

            if let Ok(numeric_id) = task_id.parse::<u64>() {
                let current = store.read_high_water_mark_unlocked()?;
                if numeric_id > current {
                    store.write_high_water_mark_unlocked(numeric_id)?;
                }
            }

            for mut task in store.list_unlocked()? {
                let before_blocks = task.blocks.len();
                let before_blocked_by = task.blocked_by.len();
                task.blocks.retain(|id| id != task_id);
                task.blocked_by.retain(|id| id != task_id);
                if before_blocks != task.blocks.len() || before_blocked_by != task.blocked_by.len()
                {
                    store.write_task_unlocked(&task)?;
                }
            }

            Ok(true)
        })
    }

    fn add_blocks_unlocked(
        &self,
        from_task_id: &str,
        block_ids: &[String],
    ) -> Result<bool, String> {
        let mut changed = false;
        for block_id in block_ids {
            if self.block_task_unlocked(from_task_id, block_id)? {
                changed = true;
            }
        }
        Ok(changed)
    }

    fn unfinished_blocking_tasks_unlocked(
        &self,
        task: &TaskBoardTask,
    ) -> Result<Vec<TaskBoardTask>, String> {
        let mut blockers = Vec::new();
        for blocker_id in &task.blocked_by {
            if let Some(blocker) = self.read_task_unlocked(blocker_id)? {
                if blocker.status != TaskBoardStatus::Completed {
                    blockers.push(blocker);
                }
            }
        }
        blockers.sort_by(|a, b| compare_task_ids(&a.id, &b.id));
        Ok(blockers)
    }

    fn active_task_for_owner_unlocked(
        &self,
        owner: &str,
        excluded_task_id: &str,
    ) -> Result<Option<TaskBoardTask>, String> {
        Ok(self.list_unlocked()?.into_iter().find(|task| {
            task.id != excluded_task_id
                && task.owner.as_deref() == Some(owner)
                && task.status != TaskBoardStatus::Completed
        }))
    }

    fn add_blocked_by_unlocked(
        &self,
        task_id: &str,
        blocker_ids: &[String],
    ) -> Result<bool, String> {
        let mut changed = false;
        for blocker_id in blocker_ids {
            if self.block_task_unlocked(blocker_id, task_id)? {
                changed = true;
            }
        }
        Ok(changed)
    }

    fn block_task_unlocked(&self, from_task_id: &str, to_task_id: &str) -> Result<bool, String> {
        if from_task_id == to_task_id {
            return Ok(false);
        }
        let Some(mut from_task) = self.read_task_unlocked(from_task_id)? else {
            return Ok(false);
        };
        let Some(mut to_task) = self.read_task_unlocked(to_task_id)? else {
            return Ok(false);
        };

        let mut changed = false;
        if !from_task.blocks.iter().any(|id| id == to_task_id) {
            from_task.blocks.push(to_task_id.to_string());
            changed = true;
        }
        if !to_task.blocked_by.iter().any(|id| id == from_task_id) {
            to_task.blocked_by.push(from_task_id.to_string());
            changed = true;
        }
        if changed {
            self.write_task_unlocked(&from_task)?;
            self.write_task_unlocked(&to_task)?;
        }
        Ok(changed)
    }

    fn with_read_lock<R>(
        &self,
        action: impl FnOnce(&TaskBoardStore) -> Result<R, String>,
    ) -> Result<R, String> {
        self.ensure_dir().map_err(|error| error.to_string())?;
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.lock_path())
            .map_err(|error| error.to_string())?;
        let lock = RwLock::new(lock_file);
        let _guard = lock.read().map_err(|error| error.to_string())?;
        action(self)
    }

    fn with_write_lock<R>(
        &self,
        action: impl FnOnce(&TaskBoardStore) -> Result<R, String>,
    ) -> Result<R, String> {
        self.ensure_dir().map_err(|error| error.to_string())?;
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.lock_path())
            .map_err(|error| error.to_string())?;
        let mut lock = RwLock::new(lock_file);
        let _guard = lock.write().map_err(|error| error.to_string())?;
        action(self)
    }

    fn ensure_dir(&self) -> io::Result<()> {
        fs::create_dir_all(self.task_list_dir())
    }

    fn lock_path(&self) -> PathBuf {
        self.task_list_dir().join(LOCK_FILE)
    }

    fn high_water_mark_path(&self) -> PathBuf {
        self.task_list_dir().join(HIGH_WATER_MARK_FILE)
    }

    fn task_path(&self, task_id: &str) -> PathBuf {
        self.task_list_dir()
            .join(format!("{}.json", sanitize_path_component(task_id)))
    }

    fn read_task_unlocked(&self, task_id: &str) -> Result<Option<TaskBoardTask>, String> {
        let path = self.task_path(task_id);
        let content = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        serde_json::from_str::<TaskBoardTask>(&content)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    fn write_task_unlocked(&self, task: &TaskBoardTask) -> Result<(), String> {
        let path = self.task_path(&task.id);
        let content = serde_json::to_string_pretty(task).map_err(|error| error.to_string())?;
        fs::write(path, content).map_err(|error| error.to_string())
    }

    fn list_unlocked(&self) -> Result<Vec<TaskBoardTask>, String> {
        let entries = match fs::read_dir(self.task_list_dir()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.to_string()),
        };
        let mut tasks = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            if let Ok(task) = serde_json::from_str::<TaskBoardTask>(&content) {
                tasks.push(task);
            }
        }
        tasks.sort_by(|a, b| compare_task_ids(&a.id, &b.id));
        Ok(tasks)
    }

    fn highest_task_id_unlocked(&self) -> Result<u64, String> {
        let from_files = self
            .list_unlocked()?
            .iter()
            .filter_map(|task| task.id.parse::<u64>().ok())
            .max()
            .unwrap_or_default();
        Ok(from_files.max(self.read_high_water_mark_unlocked()?))
    }

    fn read_high_water_mark_unlocked(&self) -> Result<u64, String> {
        let mut file = match File::open(self.high_water_mark_path()) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.to_string()),
        };
        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|error| error.to_string())?;
        Ok(content.trim().parse::<u64>().unwrap_or_default())
    }

    fn write_high_water_mark_unlocked(&self, value: u64) -> Result<(), String> {
        fs::write(self.high_water_mark_path(), value.to_string()).map_err(|error| error.to_string())
    }
}

fn resolve_task_list_id() -> String {
    first_non_empty_env(&[
        "APPFS_TASK_LIST_ID",
        "CLAW_TASK_LIST_ID",
        "CLAUDE_CODE_TASK_LIST_ID",
        "APPFS_TEAM_NAME",
        "CLAUDE_CODE_TEAM_NAME",
    ])
    .or_else(current_tool_session_id)
    .unwrap_or_else(|| String::from("default"))
}

fn first_non_empty_env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn compare_task_ids(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ => a.cmp(b),
    }
}

fn sanitize_path_component(value: &str) -> String {
    let sanitized = value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        String::from("default")
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::{TaskBoardClaimRejectionReason, TaskBoardPatch, TaskBoardStatus, TaskBoardStore};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("runtime-task-board-{unique}-{name}"))
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn restore_env(name: &str, value: Option<String>) {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }

    #[test]
    fn resolves_appfs_task_scope_environment_before_legacy_names() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let names = [
            "APPFS_TASK_LIST_ID",
            "CLAW_TASK_LIST_ID",
            "CLAUDE_CODE_TASK_LIST_ID",
            "APPFS_TEAM_NAME",
            "CLAUDE_CODE_TEAM_NAME",
        ];
        let original: Vec<(&str, Option<String>)> = names
            .iter()
            .map(|name| (*name, std::env::var(name).ok()))
            .collect();
        for name in names {
            std::env::remove_var(name);
        }
        let root = temp_dir("env-scope");

        std::env::set_var("APPFS_TEAM_NAME", "Alpha Team");
        assert_eq!(
            TaskBoardStore::default_for_cwd(&root).task_list_id(),
            "Alpha_Team"
        );
        std::env::set_var("CLAW_TASK_LIST_ID", "legacy-list");
        assert_eq!(
            TaskBoardStore::default_for_cwd(&root).task_list_id(),
            "legacy-list"
        );
        std::env::set_var("APPFS_TASK_LIST_ID", "appfs-list");
        assert_eq!(
            TaskBoardStore::default_for_cwd(&root).task_list_id(),
            "appfs-list"
        );

        for (name, value) in original {
            restore_env(name, value);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn creates_and_lists_persistent_tasks() {
        let root = temp_dir("create-list");
        let store = TaskBoardStore::new(root.join("tasks"), String::from("session-1"));
        let task = store
            .create(
                String::from("Ship task board"),
                String::from("Build the persistent task board"),
                Some(String::from("Shipping task board")),
                None,
            )
            .expect("create task");

        assert_eq!(task.id, "1");
        assert_eq!(store.list().expect("list tasks").len(), 1);

        let reopened = TaskBoardStore::new(root.join("tasks"), String::from("session-1"));
        assert_eq!(
            reopened
                .get("1")
                .expect("get task")
                .expect("task exists")
                .subject,
            "Ship task board"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn updates_dependencies_and_metadata() {
        let root = temp_dir("update");
        let store = TaskBoardStore::new(root.join("tasks"), String::from("session-1"));
        store
            .create(String::from("A"), String::from("first"), None, None)
            .expect("create first");
        store
            .create(String::from("B"), String::from("second"), None, None)
            .expect("create second");

        let mut metadata = BTreeMap::new();
        metadata.insert(String::from("priority"), json!("high"));
        let outcome = store
            .update(
                "1",
                TaskBoardPatch {
                    status: Some(TaskBoardStatus::InProgress),
                    metadata: Some(metadata),
                    add_blocks: vec![String::from("2")],
                    ..TaskBoardPatch::default()
                },
            )
            .expect("update")
            .expect("task exists");

        assert!(outcome.updated_fields.contains(&String::from("status")));
        assert!(outcome.updated_fields.contains(&String::from("blocks")));
        let first = store.get("1").expect("get first").expect("first exists");
        let second = store.get("2").expect("get second").expect("second exists");
        assert_eq!(first.blocks, vec![String::from("2")]);
        assert_eq!(second.blocked_by, vec![String::from("1")]);
        assert_eq!(
            first
                .metadata
                .expect("metadata")
                .get("priority")
                .expect("priority"),
            "high"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn claim_sets_owner_and_rejects_competing_owner() {
        let root = temp_dir("claim");
        let store = TaskBoardStore::new(root.join("tasks"), String::from("session-1"));
        store
            .create(String::from("A"), String::from("first"), None, None)
            .expect("create task");

        let mut metadata = BTreeMap::new();
        metadata.insert(String::from("claimedBy"), json!("coder"));
        let claimed = store
            .claim(
                "1",
                String::from("coder"),
                Some(String::from("Coder is working")),
                Some(metadata),
            )
            .expect("claim")
            .expect("task exists");

        assert!(claimed.claimed);
        assert_eq!(claimed.current_owner, None);
        assert_eq!(claimed.task.owner.as_deref(), Some("coder"));
        assert_eq!(claimed.task.status, TaskBoardStatus::InProgress);
        assert_eq!(
            claimed.task.active_form.as_deref(),
            Some("Coder is working")
        );
        assert_eq!(
            claimed
                .task
                .metadata
                .as_ref()
                .expect("metadata")
                .get("claimedBy")
                .expect("claimedBy"),
            "coder"
        );

        let same_owner = store
            .claim("1", String::from("coder"), None, None)
            .expect("same owner claim")
            .expect("task exists");
        assert!(same_owner.claimed);
        assert_eq!(same_owner.task.owner.as_deref(), Some("coder"));

        let rejected = store
            .claim("1", String::from("reviewer"), None, None)
            .expect("competing claim")
            .expect("task exists");
        assert!(!rejected.claimed);
        assert_eq!(rejected.current_owner.as_deref(), Some("coder"));
        assert_eq!(rejected.task.owner.as_deref(), Some("coder"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn claim_rejects_completed_blocked_and_busy_tasks() {
        let root = temp_dir("claim-guards");
        let store = TaskBoardStore::new(root.join("tasks"), String::from("session-1"));
        store
            .create(String::from("Blocker"), String::from("first"), None, None)
            .expect("create blocker");
        store
            .create(String::from("Blocked"), String::from("second"), None, None)
            .expect("create blocked");
        store
            .update(
                "1",
                TaskBoardPatch {
                    add_blocks: vec![String::from("2")],
                    ..TaskBoardPatch::default()
                },
            )
            .expect("link blocker");

        let blocked = store
            .claim("2", String::from("coder"), None, None)
            .expect("blocked claim")
            .expect("task exists");
        assert!(!blocked.claimed);
        assert_eq!(blocked.reason, Some(TaskBoardClaimRejectionReason::Blocked));
        assert_eq!(blocked.blocking_tasks[0].id, "1");

        store
            .update(
                "1",
                TaskBoardPatch {
                    status: Some(TaskBoardStatus::Completed),
                    ..TaskBoardPatch::default()
                },
            )
            .expect("complete blocker");
        let claimed = store
            .claim("2", String::from("coder"), None, None)
            .expect("claim after unblock")
            .expect("task exists");
        assert!(claimed.claimed);

        store
            .create(String::from("Next"), String::from("third"), None, None)
            .expect("create next");
        let busy = store
            .claim("3", String::from("coder"), None, None)
            .expect("busy claim")
            .expect("task exists");
        assert!(!busy.claimed);
        assert_eq!(busy.reason, Some(TaskBoardClaimRejectionReason::OwnerBusy));
        assert_eq!(
            busy.active_task
                .as_ref()
                .expect("active task should be reported")
                .id,
            "2"
        );

        store
            .update(
                "3",
                TaskBoardPatch {
                    status: Some(TaskBoardStatus::Completed),
                    ..TaskBoardPatch::default()
                },
            )
            .expect("complete next");
        let completed = store
            .claim("3", String::from("reviewer"), None, None)
            .expect("completed claim")
            .expect("task exists");
        assert!(!completed.claimed);
        assert_eq!(
            completed.reason,
            Some(TaskBoardClaimRejectionReason::AlreadyCompleted)
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn delete_removes_references_and_preserves_high_water_mark() {
        let root = temp_dir("delete");
        let store = TaskBoardStore::new(root.join("tasks"), String::from("session-1"));
        store
            .create(String::from("A"), String::from("first"), None, None)
            .expect("create first");
        store
            .create(String::from("B"), String::from("second"), None, None)
            .expect("create second");
        store
            .update(
                "1",
                TaskBoardPatch {
                    add_blocks: vec![String::from("2")],
                    ..TaskBoardPatch::default()
                },
            )
            .expect("block");

        assert!(store.delete("2").expect("delete"));
        assert_eq!(store.get("2").expect("get deleted"), None);
        assert!(store
            .get("1")
            .expect("get first")
            .expect("first exists")
            .blocks
            .is_empty());
        let next = store
            .create(String::from("C"), String::from("third"), None, None)
            .expect("create third");
        assert_eq!(next.id, "3");

        let _ = std::fs::remove_dir_all(root);
    }
}
