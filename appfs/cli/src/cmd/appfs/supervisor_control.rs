use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::{json, Value as JsonValue};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::action_consumer::{
    self, ActionConsumerConfig, ActionInvocation, ActionInvocationHandler, InvalidActionPayload,
};
use super::action_dispatcher::{
    parse_attach_principal_request, parse_create_principal_request, parse_delete_principal_request,
    parse_detach_principal_request, parse_list_apps_request, parse_register_app_request,
    parse_unregister_app_request, parse_update_principal_request, AttachPrincipalRequest,
    CreatePrincipalRequest, DeletePrincipalRequest, DetachPrincipalRequest, RegisterAppRequest,
    UnregisterAppRequest, UpdatePrincipalRequest,
};
use super::errors::ERR_INVALID_ARGUMENT;
use super::shared::{extract_client_token, write_pretty_json_file};
use super::{
    ActionCursorDoc, ActionCursorState, ActionSpec, CursorState, ExecutionMode, InputMode,
    ProcessOutcome, DEFAULT_RETENTION_HINT_SEC,
};

const CONTROL_APP_ID: &str = "_appfs";
const CONTROL_SESSION_ID: &str = "runtime-control";
const CONTROL_REGISTER_ACTION: &str = "register_app.act";
const CONTROL_UNREGISTER_ACTION: &str = "unregister_app.act";
const CONTROL_LIST_ACTION: &str = "list_apps.act";
const CONTROL_CREATE_PRINCIPAL_ACTION: &str = "principals/create_principal.act";
const CONTROL_UPDATE_PRINCIPAL_ACTION: &str = "principals/update_principal.act";
const CONTROL_DELETE_PRINCIPAL_ACTION: &str = "principals/delete_principal.act";
const CONTROL_ATTACH_PRINCIPAL_ACTION: &str = "principals/attach_principal.act";
const CONTROL_DETACH_PRINCIPAL_ACTION: &str = "principals/detach_principal.act";

#[derive(Debug, Clone)]
pub(super) enum SupervisorControlInvocation {
    Register {
        request_id: String,
        client_token: Option<String>,
        request: RegisterAppRequest,
    },
    Unregister {
        request_id: String,
        client_token: Option<String>,
        request: UnregisterAppRequest,
    },
    List {
        request_id: String,
        client_token: Option<String>,
    },
    CreatePrincipal {
        request_id: String,
        client_token: Option<String>,
        request: CreatePrincipalRequest,
    },
    UpdatePrincipal {
        request_id: String,
        client_token: Option<String>,
        request: UpdatePrincipalRequest,
    },
    DeletePrincipal {
        request_id: String,
        client_token: Option<String>,
        request: DeletePrincipalRequest,
    },
    AttachPrincipal {
        request_id: String,
        client_token: Option<String>,
        request: AttachPrincipalRequest,
    },
    DetachPrincipal {
        request_id: String,
        client_token: Option<String>,
        request: DetachPrincipalRequest,
    },
}

pub(super) struct SupervisorControlPlane {
    root: PathBuf,
    events_path: PathBuf,
    cursor_path: PathBuf,
    replay_dir: PathBuf,
    action_cursors_path: PathBuf,
    cursor: CursorState,
    action_cursors: HashMap<String, ActionCursorState>,
    next_seq: i64,
    actionline_strict: bool,
}

pub(super) struct PendingSupervisorControlInvocations {
    pub(super) invocations: Vec<SupervisorControlInvocation>,
    action_cursors: HashMap<String, ActionCursorState>,
    cursor_dirty: bool,
}

impl SupervisorControlPlane {
    pub(super) fn new(root: PathBuf, actionline_strict: bool) -> Result<Self> {
        let control_dir = root.join(CONTROL_APP_ID);
        let stream_dir = control_dir.join("_stream");
        let cursor_path = stream_dir.join("cursor.res.json");
        let replay_dir = stream_dir.join("from-seq");
        let action_cursors_path = stream_dir.join(super::ACTION_CURSORS_FILENAME);
        let cursor = load_cursor_or_default(&cursor_path)?;
        let next_seq = cursor.max_seq + 1;
        Ok(Self {
            root,
            events_path: stream_dir.join("events.evt.jsonl"),
            cursor_path,
            replay_dir,
            action_cursors_path: action_cursors_path.clone(),
            cursor,
            action_cursors: action_consumer::load_action_cursors(&action_cursors_path)?,
            next_seq,
            actionline_strict,
        })
    }

    pub(super) fn prepare_action_sinks(&mut self) -> Result<()> {
        let control_dir = self.root.join(CONTROL_APP_ID);
        let principals_dir = control_dir.join("principals");
        let stream_dir = control_dir.join("_stream");
        ensure_dir_exists(&control_dir, "AppFS control dir")?;
        ensure_dir_exists(&principals_dir, "AppFS principals control dir")?;
        ensure_dir_exists(&stream_dir, "AppFS control stream dir")?;
        ensure_dir_exists(&self.replay_dir, "AppFS control replay dir")?;
        if !self.events_path.exists() {
            fs::write(&self.events_path, b"").with_context(|| {
                format!(
                    "Failed to initialize AppFS control stream {}",
                    self.events_path.display()
                )
            })?;
        }
        if !self.cursor_path.exists() {
            write_pretty_json_file(
                &self.cursor_path,
                &CursorState {
                    min_seq: 0,
                    max_seq: 0,
                    retention_hint_sec: DEFAULT_RETENTION_HINT_SEC,
                },
                "AppFS control cursor",
            )?;
        }
        if !self.action_cursors_path.exists() {
            write_pretty_json_file(
                &self.action_cursors_path,
                &ActionCursorDoc::default(),
                "AppFS control action cursors",
            )?;
        }
        for action_name in [
            CONTROL_REGISTER_ACTION,
            CONTROL_UNREGISTER_ACTION,
            CONTROL_LIST_ACTION,
            CONTROL_CREATE_PRINCIPAL_ACTION,
            CONTROL_UPDATE_PRINCIPAL_ACTION,
            CONTROL_DELETE_PRINCIPAL_ACTION,
            CONTROL_ATTACH_PRINCIPAL_ACTION,
            CONTROL_DETACH_PRINCIPAL_ACTION,
        ] {
            let action_path = control_dir.join(action_name);
            if !action_path.exists() {
                fs::write(&action_path, b"").with_context(|| {
                    format!(
                        "Failed to initialize AppFS control action {}",
                        action_path.display()
                    )
                })?;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn drain_invocations(&mut self) -> Result<Vec<SupervisorControlInvocation>> {
        let pending = self.drain_invocations_uncommitted()?;
        let invocations = pending.invocations.clone();
        self.commit_pending_invocations(pending)?;
        Ok(invocations)
    }

    pub(super) fn drain_invocations_uncommitted(
        &mut self,
    ) -> Result<PendingSupervisorControlInvocations> {
        let config = self.action_consumer_config();
        let action_cursors = self.action_cursors.clone();
        let mut collector = ControlInvocationCollector {
            control: self,
            invocations: Vec::new(),
        };
        let result = action_consumer::drain_action_sinks_uncommitted(
            &config,
            &action_cursors,
            &mut collector,
        );
        let invocations = collector.invocations;
        let result = result?;
        Ok(PendingSupervisorControlInvocations {
            invocations,
            action_cursors: result.action_cursors,
            cursor_dirty: result.cursor_dirty,
        })
    }

    pub(super) fn commit_pending_invocations(
        &mut self,
        pending: PendingSupervisorControlInvocations,
    ) -> Result<()> {
        if !pending.cursor_dirty {
            return Ok(());
        }
        let config = self.action_consumer_config();
        action_consumer::commit_action_cursors(
            &config,
            &mut self.action_cursors,
            pending.action_cursors,
        )
    }

    fn action_consumer_config(&self) -> ActionConsumerConfig {
        ActionConsumerConfig {
            app_id: CONTROL_APP_ID.to_string(),
            session_id: CONTROL_SESSION_ID.to_string(),
            app_dir: self.root.join(CONTROL_APP_ID),
            action_cursors_path: self.action_cursors_path.clone(),
            action_specs: control_action_specs(),
            actionline_strict: self.actionline_strict,
            cursor_label: "AppFS control action cursors",
            log_label: "AppFS control",
            fixed_action_order: Some(control_action_order()),
        }
    }

    pub(super) fn emit_completed(
        &mut self,
        action_path: &str,
        request_id: &str,
        content: JsonValue,
        client_token: Option<String>,
    ) -> Result<()> {
        self.emit_event(
            action_path,
            request_id,
            "action.completed",
            Some(content),
            None,
            client_token,
        )
    }

    pub(super) fn emit_failed(
        &mut self,
        action_path: &str,
        request_id: &str,
        error_code: &str,
        message: &str,
        client_token: Option<String>,
    ) -> Result<()> {
        self.emit_event(
            action_path,
            request_id,
            "action.failed",
            None,
            Some(json!({
                "code": error_code,
                "message": message,
                "retryable": false,
            })),
            client_token,
        )
    }

    fn emit_event(
        &mut self,
        action_path: &str,
        request_id: &str,
        event_type: &str,
        content: Option<JsonValue>,
        error: Option<JsonValue>,
        client_token: Option<String>,
    ) -> Result<()> {
        let seq = self.next_seq;
        self.next_seq += 1;

        let mut event = json!({
            "seq": seq,
            "event_id": format!("evt-{seq}"),
            "ts": Utc::now().to_rfc3339(),
            "app": CONTROL_APP_ID,
            "session_id": CONTROL_SESSION_ID,
            "request_id": request_id,
            "path": action_path,
            "type": event_type,
        });

        if let Some(content) = content {
            event["content"] = content;
        }
        if let Some(error) = error {
            event["error"] = error;
        }
        if let Some(client_token) = client_token {
            event["client_token"] = json!(client_token);
        }

        let line = serde_json::to_string(&event)?;
        self.publish_event(seq, &line)
    }

    fn publish_event(&mut self, seq: i64, line: &str) -> Result<()> {
        let mut events = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.events_path)
            .with_context(|| {
                format!(
                    "Failed to open AppFS control stream {}",
                    self.events_path.display()
                )
            })?;
        writeln!(events, "{line}")?;
        events.flush()?;

        let replay_path = self.replay_dir.join(format!("{seq}.evt.jsonl"));
        fs::write(&replay_path, format!("{line}\n")).with_context(|| {
            format!(
                "Failed to write AppFS control replay file {}",
                replay_path.display()
            )
        })?;

        self.cursor.max_seq = seq;
        if self.cursor.min_seq <= 0 {
            self.cursor.min_seq = seq;
        }
        if self.cursor.retention_hint_sec <= 0 {
            self.cursor.retention_hint_sec = DEFAULT_RETENTION_HINT_SEC;
        }
        self.save_cursor()?;
        Ok(())
    }

    fn save_cursor(&self) -> Result<()> {
        write_pretty_json_file(
            &self.cursor_path,
            &serde_json::to_value(&self.cursor)?,
            "AppFS control cursor",
        )
    }
}

struct ControlInvocationCollector<'a> {
    control: &'a mut SupervisorControlPlane,
    invocations: Vec<SupervisorControlInvocation>,
}

impl ActionInvocationHandler for ControlInvocationCollector<'_> {
    fn handle_action_invocation(
        &mut self,
        invocation: ActionInvocation,
        _spec: &ActionSpec,
    ) -> Result<ProcessOutcome> {
        let failure_token = invocation.client_token.clone();
        match parse_invocation(
            &invocation.rel_path,
            &invocation.request_id,
            invocation.client_token,
            &invocation.payload_json,
        ) {
            Ok(control_invocation) => {
                self.invocations.push(control_invocation);
                Ok(ProcessOutcome::Consumed)
            }
            Err(code) => {
                self.control.emit_failed(
                    &invocation.normalized_path,
                    &invocation.request_id,
                    code,
                    "invalid AppFS lifecycle control payload",
                    failure_token.or_else(|| extract_client_token(&invocation.payload_json)),
                )?;
                Ok(ProcessOutcome::Consumed)
            }
        }
    }

    fn handle_invalid_action_payload(
        &mut self,
        invalid: InvalidActionPayload,
    ) -> Result<ProcessOutcome> {
        self.control.emit_failed(
            &invalid.normalized_path,
            &invalid.request_id,
            invalid.code,
            &invalid.message,
            invalid.client_token,
        )?;
        Ok(ProcessOutcome::Consumed)
    }
}

fn parse_invocation(
    action_name: &str,
    request_id: &str,
    client_token: Option<String>,
    payload_json: &str,
) -> std::result::Result<SupervisorControlInvocation, &'static str> {
    match action_name {
        CONTROL_REGISTER_ACTION => Ok(SupervisorControlInvocation::Register {
            request_id: request_id.to_string(),
            client_token,
            request: parse_register_app_request(payload_json)?,
        }),
        CONTROL_UNREGISTER_ACTION => Ok(SupervisorControlInvocation::Unregister {
            request_id: request_id.to_string(),
            client_token,
            request: parse_unregister_app_request(payload_json)?,
        }),
        CONTROL_LIST_ACTION => {
            parse_list_apps_request(payload_json)?;
            Ok(SupervisorControlInvocation::List {
                request_id: request_id.to_string(),
                client_token,
            })
        }
        CONTROL_CREATE_PRINCIPAL_ACTION => Ok(SupervisorControlInvocation::CreatePrincipal {
            request_id: request_id.to_string(),
            client_token,
            request: parse_create_principal_request(payload_json)?,
        }),
        CONTROL_UPDATE_PRINCIPAL_ACTION => Ok(SupervisorControlInvocation::UpdatePrincipal {
            request_id: request_id.to_string(),
            client_token,
            request: parse_update_principal_request(payload_json)?,
        }),
        CONTROL_DELETE_PRINCIPAL_ACTION => Ok(SupervisorControlInvocation::DeletePrincipal {
            request_id: request_id.to_string(),
            client_token,
            request: parse_delete_principal_request(payload_json)?,
        }),
        CONTROL_ATTACH_PRINCIPAL_ACTION => Ok(SupervisorControlInvocation::AttachPrincipal {
            request_id: request_id.to_string(),
            client_token,
            request: parse_attach_principal_request(payload_json)?,
        }),
        CONTROL_DETACH_PRINCIPAL_ACTION => Ok(SupervisorControlInvocation::DetachPrincipal {
            request_id: request_id.to_string(),
            client_token,
            request: parse_detach_principal_request(payload_json)?,
        }),
        _ => Err(ERR_INVALID_ARGUMENT),
    }
}

fn control_action_specs() -> Vec<ActionSpec> {
    control_action_order()
        .into_iter()
        .map(|template| ActionSpec {
            template,
            input_mode: InputMode::Json,
            execution_mode: ExecutionMode::Inline,
            max_payload_bytes: None,
        })
        .collect()
}

fn control_action_order() -> Vec<String> {
    [
        CONTROL_LIST_ACTION,
        CONTROL_REGISTER_ACTION,
        CONTROL_UNREGISTER_ACTION,
        CONTROL_CREATE_PRINCIPAL_ACTION,
        CONTROL_ATTACH_PRINCIPAL_ACTION,
        CONTROL_UPDATE_PRINCIPAL_ACTION,
        CONTROL_DETACH_PRINCIPAL_ACTION,
        CONTROL_DELETE_PRINCIPAL_ACTION,
    ]
    .into_iter()
    .map(ToOwned::to_owned)
    .collect()
}

fn load_cursor_or_default(path: &Path) -> Result<CursorState> {
    if !path.exists() {
        return Ok(CursorState {
            min_seq: 0,
            max_seq: 0,
            retention_hint_sec: DEFAULT_RETENTION_HINT_SEC,
        });
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let mut cursor: CursorState = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    if cursor.retention_hint_sec <= 0 {
        cursor.retention_hint_sec = DEFAULT_RETENTION_HINT_SEC;
    }
    Ok(cursor)
}

#[cfg(test)]
mod tests {
    use super::{
        SupervisorControlInvocation, SupervisorControlPlane, CONTROL_ATTACH_PRINCIPAL_ACTION,
        CONTROL_CREATE_PRINCIPAL_ACTION, CONTROL_UPDATE_PRINCIPAL_ACTION,
    };
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn supervisor_control_prepares_principal_action_sinks() {
        let temp = TempDir::new().expect("tempdir");
        let mut control =
            SupervisorControlPlane::new(temp.path().to_path_buf(), false).expect("control plane");
        control.prepare_action_sinks().expect("prepare sinks");

        for rel_path in [
            "_appfs/principals/create_principal.act",
            "_appfs/principals/update_principal.act",
            "_appfs/principals/delete_principal.act",
            "_appfs/principals/attach_principal.act",
            "_appfs/principals/detach_principal.act",
        ] {
            assert!(
                temp.path().join(rel_path).exists(),
                "{rel_path} should exist"
            );
        }
    }

    #[test]
    fn supervisor_control_keeps_cursor_unadvanced_when_cursor_save_fails() {
        let temp = TempDir::new().expect("tempdir");
        let mut control =
            SupervisorControlPlane::new(temp.path().to_path_buf(), false).expect("control plane");
        control.prepare_action_sinks().expect("prepare sinks");

        fs::write(
            temp.path()
                .join("_appfs")
                .join(CONTROL_CREATE_PRINCIPAL_ACTION),
            "{\"principal_id\":\"default\",\"display_name\":\"default\",\"kind\":\"agent\",\"client_token\":\"create-default\"}\n",
        )
        .expect("write create action");

        let original_action_cursors_path = control.action_cursors_path.clone();
        let blocked_parent = temp.path().join("not-a-dir");
        fs::write(&blocked_parent, "file").expect("write blocking file");
        control.action_cursors_path = blocked_parent.join("action-cursors.res.json");

        control
            .drain_invocations()
            .expect_err("cursor save should fail");
        assert!(
            control
                .action_cursors
                .get(CONTROL_CREATE_PRINCIPAL_ACTION)
                .is_none(),
            "failed cursor publish must not advance the in-memory cursor"
        );

        control.action_cursors_path = original_action_cursors_path;
        let invocations = control
            .drain_invocations()
            .expect("retry should re-read uncommitted action");
        assert_eq!(invocations.len(), 1);
        match &invocations[0] {
            SupervisorControlInvocation::CreatePrincipal {
                client_token,
                request,
                ..
            } => {
                assert_eq!(client_token.as_deref(), Some("create-default"));
                assert_eq!(request.principal_id, "default");
            }
            other => panic!("expected create principal invocation, got {other:?}"),
        }
        assert!(
            control
                .action_cursors
                .get(CONTROL_CREATE_PRINCIPAL_ACTION)
                .is_some(),
            "successful retry should advance the cursor"
        );
    }

    #[test]
    fn supervisor_control_accepts_actionline_create_principal() {
        let temp = TempDir::new().expect("tempdir");
        let mut control =
            SupervisorControlPlane::new(temp.path().to_path_buf(), true).expect("control plane");
        control.prepare_action_sinks().expect("prepare sinks");

        fs::write(
            temp.path()
                .join("_appfs")
                .join(CONTROL_CREATE_PRINCIPAL_ACTION),
            format!(
                "{}\n",
                r#"{"version":"2.0","client_token":"create-actionline-default","payload":{"principal_id":"default","display_name":"default","kind":"agent"}}"#
            ),
        )
        .expect("write create actionline");

        let invocations = control.drain_invocations().expect("drain invocations");
        assert_eq!(invocations.len(), 1);
        match &invocations[0] {
            SupervisorControlInvocation::CreatePrincipal {
                client_token,
                request,
                ..
            } => {
                assert_eq!(client_token.as_deref(), Some("create-actionline-default"));
                assert_eq!(request.principal_id, "default");
            }
            other => panic!("expected create principal invocation, got {other:?}"),
        }
    }

    #[test]
    fn supervisor_control_drains_attach_before_update() {
        let temp = TempDir::new().expect("tempdir");
        let mut control =
            SupervisorControlPlane::new(temp.path().to_path_buf(), false).expect("control plane");
        control.prepare_action_sinks().expect("prepare sinks");

        fs::write(
            temp.path()
                .join("_appfs")
                .join(CONTROL_UPDATE_PRINCIPAL_ACTION),
            "{\"principal_id\":\"default\",\"attach_id\":\"attach-1\",\"agent_status\":{\"state\":\"idle\"},\"client_token\":\"status-1\"}\n",
        )
        .expect("write update action");
        fs::write(
            temp.path()
                .join("_appfs")
                .join(CONTROL_ATTACH_PRINCIPAL_ACTION),
            "{\"principal_id\":\"default\",\"attach_id\":\"attach-1\",\"client_token\":\"attach-1\"}\n",
        )
        .expect("write attach action");

        let invocations = control.drain_invocations().expect("drain invocations");
        assert!(matches!(
            invocations.as_slice(),
            [
                SupervisorControlInvocation::AttachPrincipal { .. },
                SupervisorControlInvocation::UpdatePrincipal { .. }
            ]
        ));
    }
}

fn ensure_dir_exists(path: &Path, label: &str) -> Result<()> {
    fs::create_dir(path)
        .or_else(|err| {
            if err.kind() == std::io::ErrorKind::AlreadyExists {
                Ok(())
            } else {
                Err(err)
            }
        })
        .with_context(|| format!("Failed to create {} {}", label, path.display()))
}
