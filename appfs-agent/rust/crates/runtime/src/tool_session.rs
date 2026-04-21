use std::cell::RefCell;
use std::path::{Path, PathBuf};

use crate::session_control::SessionStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolSessionContext {
    session_id: String,
    persistence_path: Option<PathBuf>,
}

thread_local! {
    static CURRENT_TOOL_SESSION_CONTEXT: RefCell<Option<ToolSessionContext>> = const { RefCell::new(None) };
}

pub(crate) fn with_tool_session_context<R>(
    session_id: &str,
    persistence_path: Option<&Path>,
    action: impl FnOnce() -> R,
) -> R {
    struct ResetGuard {
        previous: Option<ToolSessionContext>,
    }

    impl Drop for ResetGuard {
        fn drop(&mut self) {
            CURRENT_TOOL_SESSION_CONTEXT.with(|slot| {
                *slot.borrow_mut() = self.previous.take();
            });
        }
    }

    let previous = CURRENT_TOOL_SESSION_CONTEXT.with(|slot| {
        slot.replace(Some(ToolSessionContext {
            session_id: session_id.to_string(),
            persistence_path: persistence_path.map(Path::to_path_buf),
        }))
    });
    let _guard = ResetGuard { previous };
    action()
}

pub(crate) fn current_tool_session_storage_root(cwd: &Path) -> Option<PathBuf> {
    CURRENT_TOOL_SESSION_CONTEXT.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|context| derive_tool_session_storage_root(context, cwd))
    })
}

fn derive_tool_session_storage_root(context: &ToolSessionContext, cwd: &Path) -> Option<PathBuf> {
    if let Some(parent) = context.persistence_path.as_deref().and_then(Path::parent) {
        return Some(parent.join(&context.session_id));
    }

    SessionStore::from_cwd(cwd)
        .ok()
        .map(|store| store.sessions_dir().join(&context.session_id))
}

#[cfg(test)]
mod tests {
    use super::{current_tool_session_storage_root, with_tool_session_context};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("runtime-tool-session-{unique}-{name}"))
    }

    #[test]
    fn derives_storage_root_from_persistence_path_parent_and_session_id() {
        let cwd = temp_dir("root-from-path");
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

        let root = with_tool_session_context("session-123", Some(&persistence_path), || {
            current_tool_session_storage_root(&cwd)
        })
        .expect("storage root");

        assert_eq!(
            root,
            cwd.join(".claw")
                .join("sessions")
                .join("workspace-hash")
                .join("session-123")
        );

        let _ = fs::remove_dir_all(cwd);
    }
}
