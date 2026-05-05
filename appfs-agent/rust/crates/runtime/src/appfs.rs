use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::session::{AttachmentKind, ConversationMessage, Session, SessionError};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

const CONTROL_DIR_NAME: &str = "_appfs";
const REGISTER_APP_ACTION: &str = "register_app.act";
const UNREGISTER_APP_ACTION: &str = "unregister_app.act";
const LIST_APPS_ACTION: &str = "list_apps.act";
const REGISTRY_FILE: &str = "apps.registry.json";
const APP_CONTROL_DIR_NAME: &str = "_app";
const APP_STREAM_DIR_NAME: &str = "_stream";
const EVENTS_FILE: &str = "events.evt.jsonl";
const STREAM_CURSOR_FILE: &str = "cursor.res.json";
const APPFS_EVENT_REMINDER_MAX_EVENTS: usize = 20;
const APPFS_EVENT_REMINDER_FIELD_LIMIT: usize = 360;
pub const APPFS_RUNTIME_MANIFEST_REL_PATH: &str = ".well-known/appfs/runtime.json";
pub const APPFS_ATTACH_SCHEMA_ENV: &str = "APPFS_ATTACH_SCHEMA";
pub const APPFS_RUNTIME_MANIFEST_ENV: &str = "APPFS_RUNTIME_MANIFEST";
pub const APPFS_MOUNT_ROOT_ENV: &str = "APPFS_MOUNT_ROOT";
pub const APPFS_RUNTIME_SESSION_ID_ENV: &str = "APPFS_RUNTIME_SESSION_ID";
pub const APPFS_ATTACH_ID_ENV: &str = "APPFS_ATTACH_ID";
pub const APPFS_AGENT_ROLE_ENV: &str = "APPFS_AGENT_ROLE";
pub const APPFS_MULTI_AGENT_MODE_SHARED: &str = "shared_mount_distinct_attach";
pub const APPFS_RUNTIME_KIND: &str = "appfs";
pub const APPFS_SCHEMA_VERSION: u32 = 1;

static ATTACH_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppfsAttachSource {
    Env,
    Manifest,
    Heuristic,
}

impl AppfsAttachSource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::Manifest => "manifest",
            Self::Heuristic => "heuristic",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppfsRegisteredApp {
    pub app_id: String,
    pub active_scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppfsRuntimeManifestControlPlane {
    pub register_action: String,
    pub unregister_action: String,
    pub list_action: String,
    pub registry: String,
    pub events: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct AppfsRuntimeManifestCapabilities {
    pub app_registration: bool,
    pub event_stream: bool,
    pub multi_app: bool,
    pub scope_switch: bool,
    pub multi_agent_attach: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppfsRuntimeManifest {
    pub schema_version: u32,
    pub runtime_kind: String,
    pub mount_root: PathBuf,
    pub runtime_session_id: String,
    pub managed: bool,
    pub multi_agent_mode: String,
    pub control_plane: AppfsRuntimeManifestControlPlane,
    pub capabilities: AppfsRuntimeManifestCapabilities,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppfsEnvironment {
    pub attach_source: AppfsAttachSource,
    pub mount_root: PathBuf,
    pub runtime_session_id: Option<String>,
    pub attach_id: String,
    pub attach_role: Option<String>,
    pub multi_agent_mode: String,
    pub manifest_path: Option<PathBuf>,
    pub control_dir: Option<PathBuf>,
    pub control_events_path: Option<PathBuf>,
    pub registry_path: Option<PathBuf>,
    pub register_app_path: Option<PathBuf>,
    pub unregister_app_path: Option<PathBuf>,
    pub list_apps_path: Option<PathBuf>,
    pub current_app_id: Option<String>,
    pub current_app_root: Option<PathBuf>,
    pub current_app_events_path: Option<PathBuf>,
    pub registered_apps: Vec<AppfsRegisteredApp>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AppfsPromptControlDoc {
    app_id: String,
    description: Option<String>,
    events_path: Option<String>,
    current_scope_path: Option<String>,
    available_scopes_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AppfsPromptCurrentScopeDoc {
    app_id: String,
    active_scope: String,
    display_name: Option<String>,
    primary_resource: Option<String>,
    structure_revision_hint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AppfsPromptAvailableScopeEntry {
    scope_id: String,
    display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AppfsPromptAvailableScopesDoc {
    app_id: String,
    active_scope: Option<String>,
    #[serde(default)]
    scopes: Vec<AppfsPromptAvailableScopeEntry>,
}

#[derive(Debug, Clone, Default)]
struct AppfsAttachEnv {
    schema: Option<String>,
    manifest_path: Option<PathBuf>,
    mount_root: Option<PathBuf>,
    runtime_session_id: Option<String>,
    attach_id: Option<String>,
    attach_role: Option<String>,
}

impl AppfsAttachEnv {
    fn has_attach_hint(&self) -> bool {
        self.schema.is_some()
            || self.manifest_path.is_some()
            || self.mount_root.is_some()
            || self.runtime_session_id.is_some()
            || self.attach_id.is_some()
            || self.attach_role.is_some()
    }
}

#[derive(Debug, Clone)]
struct HeuristicDetection {
    mount_root: PathBuf,
    control_dir: PathBuf,
    control_events_path: PathBuf,
    registry_path: PathBuf,
    register_app_path: PathBuf,
    unregister_app_path: PathBuf,
    list_apps_path: PathBuf,
    current_app_id: Option<String>,
    current_app_root: Option<PathBuf>,
    current_app_events_path: Option<PathBuf>,
    registered_apps: Vec<AppfsRegisteredApp>,
}

#[derive(Debug, Clone, Default)]
struct ResolvedControlPlanePaths {
    control_dir: Option<PathBuf>,
    control_events_path: Option<PathBuf>,
    registry_path: Option<PathBuf>,
    register_app_path: Option<PathBuf>,
    unregister_app_path: Option<PathBuf>,
    list_apps_path: Option<PathBuf>,
}

#[must_use]
pub fn detect_appfs_environment(cwd: &Path) -> Option<AppfsEnvironment> {
    resolve_appfs_environment(cwd)
}

#[must_use]
pub fn resolve_appfs_environment(cwd: &Path) -> Option<AppfsEnvironment> {
    resolve_appfs_environment_with_attach_env(cwd, load_attach_env())
}

#[must_use]
pub fn build_appfs_prompt_section(cwd: &Path) -> Option<String> {
    let environment = detect_appfs_environment(cwd)?;
    Some(render_appfs_prompt_section(&environment))
}

pub fn sync_appfs_event_reminders(session: &mut Session, cwd: &Path) -> Result<(), SessionError> {
    let Some(environment) = detect_appfs_environment(cwd) else {
        return Ok(());
    };
    let streams = collect_appfs_event_streams(&environment);
    if streams.is_empty() {
        return Ok(());
    }

    let mut cursor_updates = BTreeMap::new();
    let mut new_events = Vec::new();
    for stream in streams {
        let stream_max_seq = read_appfs_stream_max_seq_hint(&stream);
        if let Some(max_seq) = stream_max_seq {
            match session.appfs_event_cursor(&stream.stream_id) {
                Some(last_seq) if max_seq <= last_seq => continue,
                None => {
                    // First attach establishes a baseline so old event backlog does not
                    // surprise the model; subsequent model-call cycles will surface deltas.
                    cursor_updates.insert(stream.stream_id.clone(), max_seq);
                    continue;
                }
                _ => {}
            }
        }

        let Some(records) = read_appfs_event_records(&stream) else {
            continue;
        };
        let max_seq = stream_max_seq
            .or_else(|| records.iter().map(|record| record.seq).max())
            .unwrap_or(0);
        match session.appfs_event_cursor(&stream.stream_id) {
            Some(last_seq) => {
                if max_seq > last_seq {
                    cursor_updates.insert(stream.stream_id.clone(), max_seq);
                }
                new_events.extend(records.into_iter().filter(|record| record.seq > last_seq));
            }
            None => {
                // First attach establishes a baseline so old event backlog does not
                // surprise the model; subsequent model-call cycles will surface deltas.
                cursor_updates.insert(stream.stream_id.clone(), max_seq);
            }
        }
    }

    if !new_events.is_empty() {
        let reminder = render_appfs_event_reminder(&new_events);
        session.push_message(ConversationMessage::attachment_user_text(
            reminder,
            AttachmentKind::AppfsEvents,
        ))?;
    }

    if !cursor_updates.is_empty() {
        session.update_appfs_event_cursors(cursor_updates)?;
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct AppfsEventStream {
    stream_id: String,
    label: String,
    app_id: Option<String>,
    path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct AppfsStreamCursor {
    max_seq: i64,
}

#[derive(Debug, Clone)]
struct AppfsEventRecord {
    label: String,
    app_id: Option<String>,
    seq: i64,
    event_type: String,
    event_path: Option<String>,
    request_id: Option<String>,
    content: Option<Value>,
    error: Option<Value>,
}

fn collect_appfs_event_streams(environment: &AppfsEnvironment) -> Vec<AppfsEventStream> {
    let mut streams = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(path) = &environment.control_events_path {
        push_appfs_event_stream(
            &mut streams,
            &mut seen,
            AppfsEventStream {
                stream_id: "platform".to_string(),
                label: "AppFS platform".to_string(),
                app_id: None,
                path: path.clone(),
            },
        );
    }

    for app in &environment.registered_apps {
        push_appfs_event_stream(
            &mut streams,
            &mut seen,
            AppfsEventStream {
                stream_id: format!("app:{}", app.app_id),
                label: format!("AppFS app `{}`", app.app_id),
                app_id: Some(app.app_id.clone()),
                path: environment
                    .mount_root
                    .join(&app.app_id)
                    .join(APP_STREAM_DIR_NAME)
                    .join(EVENTS_FILE),
            },
        );
    }

    if let (Some(app_id), Some(path)) = (
        environment.current_app_id.as_ref(),
        environment.current_app_events_path.as_ref(),
    ) {
        push_appfs_event_stream(
            &mut streams,
            &mut seen,
            AppfsEventStream {
                stream_id: format!("app:{app_id}"),
                label: format!("AppFS app `{app_id}`"),
                app_id: Some(app_id.clone()),
                path: path.clone(),
            },
        );
    }

    streams
}

fn push_appfs_event_stream(
    streams: &mut Vec<AppfsEventStream>,
    seen: &mut BTreeSet<String>,
    stream: AppfsEventStream,
) {
    if seen.insert(stream.stream_id.clone()) {
        streams.push(stream);
    }
}

fn read_appfs_event_records(stream: &AppfsEventStream) -> Option<Vec<AppfsEventRecord>> {
    let contents = fs::read_to_string(&stream.path).ok()?;
    let mut records = Vec::new();
    for line in contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(seq) = value
            .get("seq")
            .and_then(Value::as_i64)
            .filter(|seq| *seq >= 0)
        else {
            continue;
        };
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let event_path = value
            .get("path")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let request_id = value
            .get("request_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let content = value.get("content").cloned();
        let error = value.get("error").cloned();
        records.push(AppfsEventRecord {
            label: stream.label.clone(),
            app_id: stream.app_id.clone(),
            seq,
            event_type,
            event_path,
            request_id,
            content,
            error,
        });
    }
    Some(records)
}

fn read_appfs_stream_max_seq_hint(stream: &AppfsEventStream) -> Option<i64> {
    let cursor_path = stream.path.parent()?.join(STREAM_CURSOR_FILE);
    let cursor: AppfsStreamCursor = read_json_file(&cursor_path)?;
    Some(cursor.max_seq.max(0))
}

fn render_appfs_event_reminder(events: &[AppfsEventRecord]) -> String {
    let omitted_count = events.len().saturating_sub(APPFS_EVENT_REMINDER_MAX_EVENTS);
    let visible_start = events.len().saturating_sub(APPFS_EVENT_REMINDER_MAX_EVENTS);
    let visible_events = &events[visible_start..];
    let mut lines = vec![
        "<system-reminder>".to_string(),
        "New AppFS events were received since the previous model call.".to_string(),
        "Use these as fresh context; do not re-run completed actions unless the user asks."
            .to_string(),
    ];
    if omitted_count > 0 {
        lines.push(format!(
            "{omitted_count} older event(s) were omitted from this reminder."
        ));
    }
    for event in visible_events {
        let mut line = format!("- [{}] seq={} {}", event.label, event.seq, event.event_type);
        if let Some(app_id) = &event.app_id {
            line.push_str(&format!(" app={app_id}"));
        }
        if let Some(path) = &event.event_path {
            line.push_str(&format!(" path={}", sanitize_reminder_text(path)));
        }
        if let Some(request_id) = &event.request_id {
            line.push_str(&format!(
                " request_id={}",
                sanitize_reminder_text(request_id)
            ));
        }
        if let Some(summary) = summarize_appfs_event(event) {
            line.push_str(&format!(" summary={}", sanitize_reminder_text(&summary)));
        }
        lines.push(line);
    }
    lines.push("</system-reminder>".to_string());
    lines.join("\n")
}

fn summarize_appfs_event(event: &AppfsEventRecord) -> Option<String> {
    match event.event_type.as_str() {
        "action.accepted" => summarize_progress_like_event(event, "action accepted"),
        "action.progress" => summarize_progress_like_event(event, "action progress"),
        "action.completed" => summarize_completed_event(event),
        "action.failed" => summarize_failed_event(event),
        "app.registered" => summarize_lifecycle_event(event, "app registered"),
        "app.unregistered" => summarize_lifecycle_event(event, "app unregistered"),
        "app.started" => summarize_lifecycle_event(event, "app started"),
        "app.stopped" => summarize_lifecycle_event(event, "app stopped"),
        _ => event
            .content
            .as_ref()
            .map(|content| compact_event_json(content)),
    }
}

fn summarize_progress_like_event(event: &AppfsEventRecord, label: &str) -> Option<String> {
    let mut parts = vec![label.to_string()];
    if let Some(content) = &event.content {
        append_summary_field(&mut parts, content);
        append_message_field(&mut parts, content);
        append_number_field(&mut parts, content, "percent");
        append_string_field(&mut parts, content, "phase");
        append_string_field(&mut parts, content, "status");
        if parts.len() == 1 {
            parts.push(format!("details={}", compact_event_json(content)));
        }
    }
    Some(truncate_chars(
        &parts.join("; "),
        APPFS_EVENT_REMINDER_FIELD_LIMIT,
    ))
}

fn summarize_completed_event(event: &AppfsEventRecord) -> Option<String> {
    let Some(content) = &event.content else {
        return Some("action completed".to_string());
    };
    let mut parts = vec!["action completed".to_string()];
    append_summary_field(&mut parts, content);
    append_message_field(&mut parts, content);
    for field in ["app_id", "session_id", "scope", "target_scope"] {
        append_string_field(&mut parts, content, field);
    }
    for field in ["ok", "registered", "unregistered"] {
        append_bool_field(&mut parts, content, field);
    }
    if let Some(payload) = content.get("payload").or_else(|| content.get("echo")) {
        parts.push(format!("payload={}", compact_event_json(payload)));
    } else if parts.len() == 1 {
        parts.push(format!("details={}", compact_event_json(content)));
    }
    Some(truncate_chars(
        &parts.join("; "),
        APPFS_EVENT_REMINDER_FIELD_LIMIT,
    ))
}

fn summarize_failed_event(event: &AppfsEventRecord) -> Option<String> {
    let Some(error) = &event.error else {
        return Some("action failed".to_string());
    };
    let mut parts = vec!["action failed".to_string()];
    append_string_field(&mut parts, error, "code");
    append_message_field(&mut parts, error);
    append_bool_field(&mut parts, error, "retryable");
    if parts.len() == 1 {
        parts.push(format!("error={}", compact_event_json(error)));
    }
    Some(truncate_chars(
        &parts.join("; "),
        APPFS_EVENT_REMINDER_FIELD_LIMIT,
    ))
}

fn summarize_lifecycle_event(event: &AppfsEventRecord, label: &str) -> Option<String> {
    let mut parts = vec![label.to_string()];
    if let Some(content) = &event.content {
        append_string_field(&mut parts, content, "app_id");
        append_string_field(&mut parts, content, "session_id");
        append_summary_field(&mut parts, content);
        append_message_field(&mut parts, content);
        if parts.len() == 1 {
            parts.push(format!("details={}", compact_event_json(content)));
        }
    }
    Some(truncate_chars(
        &parts.join("; "),
        APPFS_EVENT_REMINDER_FIELD_LIMIT,
    ))
}

fn append_summary_field(parts: &mut Vec<String>, value: &Value) {
    append_string_field(parts, value, "summary");
}

fn append_message_field(parts: &mut Vec<String>, value: &Value) {
    append_string_field(parts, value, "message");
}

fn append_string_field(parts: &mut Vec<String>, value: &Value, field: &str) {
    if let Some(text) = value.get(field).and_then(Value::as_str) {
        parts.push(format!("{field}={}", quote_summary_value(text)));
    }
}

fn append_bool_field(parts: &mut Vec<String>, value: &Value, field: &str) {
    if let Some(flag) = value.get(field).and_then(Value::as_bool) {
        parts.push(format!("{field}={flag}"));
    }
}

fn append_number_field(parts: &mut Vec<String>, value: &Value, field: &str) {
    if let Some(number) = value.get(field).and_then(Value::as_f64) {
        parts.push(format!("{field}={number}"));
    }
}

fn quote_summary_value(value: &str) -> String {
    let sanitized =
        sanitize_reminder_text(&truncate_chars(value, APPFS_EVENT_REMINDER_FIELD_LIMIT))
            .replace('\'', "\\'");
    format!("'{sanitized}'")
}

fn compact_event_json(value: &Value) -> String {
    let rendered = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
    truncate_chars(&rendered, APPFS_EVENT_REMINDER_FIELD_LIMIT)
}

fn truncate_chars(value: &str, limit: usize) -> String {
    let mut iter = value.chars();
    let truncated = iter.by_ref().take(limit).collect::<String>();
    if iter.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn sanitize_reminder_text(value: &str) -> String {
    value
        .replace("<system-reminder", "<system-reminder_")
        .replace("</system-reminder", "</system-reminder_")
}

fn resolve_appfs_environment_with_attach_env(
    cwd: &Path,
    attach_env: AppfsAttachEnv,
) -> Option<AppfsEnvironment> {
    let heuristic = detect_heuristic_environment(cwd);
    let manifest_path = attach_env
        .manifest_path
        .clone()
        .or_else(|| find_runtime_manifest_from_ancestors(cwd));

    let mut warnings = Vec::new();
    let manifest = manifest_path
        .as_ref()
        .and_then(|path| match read_runtime_manifest(path) {
            Ok(manifest) => Some(manifest),
            Err(err) => {
                warnings.push(format!(
                    "failed to read AppFS runtime manifest {}: {err}",
                    path.display()
                ));
                None
            }
        });

    if attach_env.has_attach_hint() {
        return build_env_environment(
            cwd,
            attach_env,
            manifest_path,
            manifest.as_ref(),
            heuristic.as_ref(),
            warnings,
        );
    }

    if let Some(manifest) = manifest {
        return build_manifest_environment(
            cwd,
            manifest_path,
            &manifest,
            heuristic.as_ref(),
            warnings,
        );
    }

    heuristic.map(|detection| build_heuristic_environment(detection, warnings))
}

fn build_env_environment(
    cwd: &Path,
    attach_env: AppfsAttachEnv,
    manifest_path: Option<PathBuf>,
    manifest: Option<&AppfsRuntimeManifest>,
    heuristic: Option<&HeuristicDetection>,
    mut warnings: Vec<String>,
) -> Option<AppfsEnvironment> {
    if let Some(schema) = attach_env.schema.as_deref() {
        if schema.trim() != "1" {
            warnings.push(format!(
                "{APPFS_ATTACH_SCHEMA_ENV} expected '1' but found '{}'",
                schema.trim()
            ));
        }
    }

    if let (Some(env_mount_root), Some(manifest)) = (attach_env.mount_root.as_ref(), manifest) {
        if env_mount_root != &manifest.mount_root {
            warnings.push(format!(
                "env mount root {} does not match manifest mount root {}",
                env_mount_root.display(),
                manifest.mount_root.display()
            ));
        }
    }
    if let (Some(env_session_id), Some(manifest)) =
        (attach_env.runtime_session_id.as_ref(), manifest)
    {
        if env_session_id != &manifest.runtime_session_id {
            warnings.push(format!(
                "env runtime session '{}' does not match manifest runtime session '{}'",
                env_session_id, manifest.runtime_session_id
            ));
        }
    }

    let mount_root = attach_env
        .mount_root
        .clone()
        .or_else(|| manifest.map(|doc| doc.mount_root.clone()))
        .or_else(|| {
            heuristic
                .as_ref()
                .map(|detection| detection.mount_root.clone())
        })?;
    let runtime_session_id = attach_env
        .runtime_session_id
        .clone()
        .or_else(|| manifest.map(|doc| doc.runtime_session_id.clone()));
    let attach_id = attach_env
        .attach_id
        .clone()
        .unwrap_or_else(generate_ephemeral_attach_id);
    let multi_agent_mode = manifest.map_or_else(
        || APPFS_MULTI_AGENT_MODE_SHARED.to_string(),
        |doc| doc.multi_agent_mode.clone(),
    );
    let control_paths = manifest
        .map(|doc| resolve_control_plane_paths(&mount_root, &doc.control_plane))
        .or_else(|| heuristic.map(control_plane_from_heuristic))
        .unwrap_or_default();
    let current_detection = detect_current_app(&mount_root, cwd);
    let registered_apps = load_registered_apps_from_paths(
        control_paths
            .registry_path
            .as_deref()
            .or_else(|| heuristic.as_ref().map(|d| d.registry_path.as_path())),
    );

    Some(AppfsEnvironment {
        attach_source: AppfsAttachSource::Env,
        mount_root,
        runtime_session_id,
        attach_id,
        attach_role: attach_env.attach_role,
        multi_agent_mode,
        manifest_path,
        control_dir: control_paths.control_dir,
        control_events_path: control_paths.control_events_path,
        registry_path: control_paths.registry_path,
        register_app_path: control_paths.register_app_path,
        unregister_app_path: control_paths.unregister_app_path,
        list_apps_path: control_paths.list_apps_path,
        current_app_id: current_detection.current_app_id,
        current_app_root: current_detection.current_app_root,
        current_app_events_path: current_detection.current_app_events_path,
        registered_apps,
        warnings,
    })
}

fn build_manifest_environment(
    cwd: &Path,
    manifest_path: Option<PathBuf>,
    manifest: &AppfsRuntimeManifest,
    heuristic: Option<&HeuristicDetection>,
    warnings: Vec<String>,
) -> Option<AppfsEnvironment> {
    let mount_root = manifest.mount_root.clone();
    if !mount_root.exists() && heuristic.is_none() {
        return None;
    }

    let control_paths = resolve_control_plane_paths(&mount_root, &manifest.control_plane);
    let current_detection = detect_current_app(&mount_root, cwd);
    let registered_apps = load_registered_apps_from_paths(control_paths.registry_path.as_deref());

    Some(AppfsEnvironment {
        attach_source: AppfsAttachSource::Manifest,
        mount_root,
        runtime_session_id: Some(manifest.runtime_session_id.clone()),
        attach_id: generate_ephemeral_attach_id(),
        attach_role: None,
        multi_agent_mode: manifest.multi_agent_mode.clone(),
        manifest_path,
        control_dir: control_paths.control_dir,
        control_events_path: control_paths.control_events_path,
        registry_path: control_paths.registry_path,
        register_app_path: control_paths.register_app_path,
        unregister_app_path: control_paths.unregister_app_path,
        list_apps_path: control_paths.list_apps_path,
        current_app_id: current_detection.current_app_id,
        current_app_root: current_detection.current_app_root,
        current_app_events_path: current_detection.current_app_events_path,
        registered_apps,
        warnings,
    })
}

fn build_heuristic_environment(
    detection: HeuristicDetection,
    warnings: Vec<String>,
) -> AppfsEnvironment {
    AppfsEnvironment {
        attach_source: AppfsAttachSource::Heuristic,
        mount_root: detection.mount_root,
        runtime_session_id: None,
        attach_id: generate_ephemeral_attach_id(),
        attach_role: None,
        multi_agent_mode: APPFS_MULTI_AGENT_MODE_SHARED.to_string(),
        manifest_path: None,
        control_dir: Some(detection.control_dir),
        control_events_path: Some(detection.control_events_path),
        registry_path: Some(detection.registry_path),
        register_app_path: Some(detection.register_app_path),
        unregister_app_path: Some(detection.unregister_app_path),
        list_apps_path: Some(detection.list_apps_path),
        current_app_id: detection.current_app_id,
        current_app_root: detection.current_app_root,
        current_app_events_path: detection.current_app_events_path,
        registered_apps: detection.registered_apps,
        warnings,
    }
}

#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_field_names)]
struct CurrentAppDetection {
    current_app_id: Option<String>,
    current_app_root: Option<PathBuf>,
    current_app_events_path: Option<PathBuf>,
}

fn detect_current_app(mount_root: &Path, cwd: &Path) -> CurrentAppDetection {
    let Ok(relative) = cwd.strip_prefix(mount_root) else {
        return CurrentAppDetection::default();
    };
    let Some(Component::Normal(first_component)) = relative.components().next() else {
        return CurrentAppDetection::default();
    };

    let app_id = first_component.to_string_lossy().to_string();
    if app_id == CONTROL_DIR_NAME || app_id == ".well-known" {
        return CurrentAppDetection::default();
    }

    let app_root = mount_root.join(&app_id);
    if !looks_like_app_root(&app_root) {
        return CurrentAppDetection::default();
    }

    let events_path = app_root.join(APP_STREAM_DIR_NAME).join(EVENTS_FILE);
    CurrentAppDetection {
        current_app_id: Some(app_id),
        current_app_root: Some(app_root),
        current_app_events_path: events_path.exists().then_some(events_path),
    }
}

fn control_plane_from_heuristic(detection: &HeuristicDetection) -> ResolvedControlPlanePaths {
    ResolvedControlPlanePaths {
        control_dir: Some(detection.control_dir.clone()),
        control_events_path: Some(detection.control_events_path.clone()),
        registry_path: Some(detection.registry_path.clone()),
        register_app_path: Some(detection.register_app_path.clone()),
        unregister_app_path: Some(detection.unregister_app_path.clone()),
        list_apps_path: Some(detection.list_apps_path.clone()),
    }
}

fn resolve_control_plane_paths(
    mount_root: &Path,
    control_plane: &AppfsRuntimeManifestControlPlane,
) -> ResolvedControlPlanePaths {
    let register_app_path = absolute_mount_path(mount_root, &control_plane.register_action);
    let unregister_app_path = absolute_mount_path(mount_root, &control_plane.unregister_action);
    let list_apps_path = absolute_mount_path(mount_root, &control_plane.list_action);
    let registry_path = absolute_mount_path(mount_root, &control_plane.registry);
    let control_events_path = absolute_mount_path(mount_root, &control_plane.events);
    let control_dir = register_app_path.parent().map(Path::to_path_buf);

    ResolvedControlPlanePaths {
        control_dir,
        control_events_path: Some(control_events_path),
        registry_path: Some(registry_path),
        register_app_path: Some(register_app_path),
        unregister_app_path: Some(unregister_app_path),
        list_apps_path: Some(list_apps_path),
    }
}

fn absolute_mount_path(mount_root: &Path, virtual_path: &str) -> PathBuf {
    let trimmed = virtual_path.trim().trim_start_matches(['/', '\\']);
    if trimmed.is_empty() {
        return mount_root.to_path_buf();
    }
    let mut path = mount_root.to_path_buf();
    for segment in trimmed.split(['/', '\\']) {
        if segment.is_empty() {
            continue;
        }
        path.push(segment);
    }
    path
}

fn load_attach_env() -> AppfsAttachEnv {
    AppfsAttachEnv {
        schema: env::var(APPFS_ATTACH_SCHEMA_ENV).ok(),
        manifest_path: env::var_os(APPFS_RUNTIME_MANIFEST_ENV).map(PathBuf::from),
        mount_root: env::var_os(APPFS_MOUNT_ROOT_ENV).map(PathBuf::from),
        runtime_session_id: env::var(APPFS_RUNTIME_SESSION_ID_ENV).ok(),
        attach_id: env::var(APPFS_ATTACH_ID_ENV).ok(),
        attach_role: env::var(APPFS_AGENT_ROLE_ENV).ok(),
    }
}

fn find_runtime_manifest_from_ancestors(cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors()
        .map(|candidate| candidate.join(APPFS_RUNTIME_MANIFEST_REL_PATH))
        .find(|path| path.exists())
}

fn read_runtime_manifest(path: &Path) -> Result<AppfsRuntimeManifest, String> {
    let bytes = fs::read(path).map_err(|err| err.to_string())?;
    let manifest: AppfsRuntimeManifest =
        serde_json::from_slice(&bytes).map_err(|err| err.to_string())?;
    if manifest.schema_version != APPFS_SCHEMA_VERSION {
        return Err(format!(
            "unsupported schema_version {}",
            manifest.schema_version
        ));
    }
    if manifest.runtime_kind != APPFS_RUNTIME_KIND {
        return Err(format!(
            "unsupported runtime_kind '{}'",
            manifest.runtime_kind
        ));
    }
    Ok(manifest)
}

fn detect_heuristic_environment(cwd: &Path) -> Option<HeuristicDetection> {
    let mount_root = cwd
        .ancestors()
        .find(|candidate| looks_like_appfs_mount_root(candidate))?
        .to_path_buf();
    let control_dir = mount_root.join(CONTROL_DIR_NAME);
    let registry_path = control_dir.join(REGISTRY_FILE);
    let register_app_path = control_dir.join(REGISTER_APP_ACTION);
    let unregister_app_path = control_dir.join(UNREGISTER_APP_ACTION);
    let list_apps_path = control_dir.join(LIST_APPS_ACTION);
    let control_events_path = control_dir.join(APP_STREAM_DIR_NAME).join(EVENTS_FILE);
    let current_detection = detect_current_app(&mount_root, cwd);

    Some(HeuristicDetection {
        mount_root,
        control_dir,
        control_events_path,
        registry_path: registry_path.clone(),
        register_app_path,
        unregister_app_path,
        list_apps_path,
        current_app_id: current_detection.current_app_id,
        current_app_root: current_detection.current_app_root,
        current_app_events_path: current_detection.current_app_events_path,
        registered_apps: load_registered_apps_from_paths(Some(registry_path.as_path())),
    })
}

fn looks_like_appfs_mount_root(candidate: &Path) -> bool {
    let control_dir = candidate.join(CONTROL_DIR_NAME);
    if !control_dir.is_dir() {
        return false;
    }

    [
        control_dir.join(REGISTER_APP_ACTION),
        control_dir.join(UNREGISTER_APP_ACTION),
        control_dir.join(LIST_APPS_ACTION),
        control_dir.join(REGISTRY_FILE),
    ]
    .iter()
    .any(|path| path.exists())
}

fn looks_like_app_root(candidate: &Path) -> bool {
    candidate.join(APP_CONTROL_DIR_NAME).is_dir() || candidate.join(APP_STREAM_DIR_NAME).is_dir()
}

fn load_registered_apps_from_paths(registry_path: Option<&Path>) -> Vec<AppfsRegisteredApp> {
    let Some(registry_path) = registry_path else {
        return Vec::new();
    };
    load_registered_apps(registry_path)
}

fn load_registered_apps(registry_path: &Path) -> Vec<AppfsRegisteredApp> {
    let Ok(contents) = fs::read_to_string(registry_path) else {
        return Vec::new();
    };
    let Ok(doc) = serde_json::from_str::<Value>(&contents) else {
        return Vec::new();
    };
    let Some(apps) = doc.get("apps").and_then(Value::as_array) else {
        return Vec::new();
    };

    apps.iter()
        .filter_map(|entry| {
            let object = entry.as_object()?;
            let app_id = object.get("app_id")?.as_str()?.to_string();
            let active_scope = object
                .get("active_scope")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            Some(AppfsRegisteredApp {
                app_id,
                active_scope,
            })
        })
        .collect()
}

fn generate_ephemeral_attach_id() -> String {
    let seq = ATTACH_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("attach-{now:x}-{}-{seq:x}", process::id())
}

fn render_appfs_prompt_section(environment: &AppfsEnvironment) -> String {
    if let (Some(app_id), Some(app_root)) = (
        environment.current_app_id.as_deref(),
        environment.current_app_root.as_deref(),
    ) {
        return render_current_app_prompt_section(environment, app_id, app_root);
    }

    render_mount_prompt_section(environment)
}

fn render_mount_prompt_section(environment: &AppfsEnvironment) -> String {
    let register_path = environment
        .register_app_path
        .as_deref()
        .map(|path| display_virtualish_path(&environment.mount_root, path))
        .unwrap_or_else(|| "_appfs/register_app.act".to_string());
    let list_path = environment
        .list_apps_path
        .as_deref()
        .map(|path| display_virtualish_path(&environment.mount_root, path))
        .unwrap_or_else(|| "_appfs/list_apps.act".to_string());
    let events_path = environment
        .control_events_path
        .as_deref()
        .map(|path| display_virtualish_path(&environment.mount_root, path))
        .unwrap_or_else(|| "_appfs/_stream/events.evt.jsonl".to_string());
    let lines = render_appfs_overview_lines(
        environment,
        None,
        None,
        &events_path,
        Some(&register_path),
        Some(&list_path),
    );
    lines.join("\n")
}

fn summarize_registered_app_ids(environment: &AppfsEnvironment) -> Option<String> {
    let app_ids = environment
        .registered_apps
        .iter()
        .map(|app| format!("`{}`", app.app_id))
        .collect::<Vec<_>>();
    (!app_ids.is_empty()).then(|| app_ids.join(", "))
}

fn render_current_app_prompt_section(
    environment: &AppfsEnvironment,
    app_id: &str,
    app_root: &Path,
) -> String {
    let control_doc: Option<AppfsPromptControlDoc> =
        read_json_file(&app_root.join("_app").join("control.res.json"));
    let current_scope_doc: Option<AppfsPromptCurrentScopeDoc> =
        read_json_file(&app_root.join("_app").join("current_scope.res.json"));
    let available_scopes_doc: Option<AppfsPromptAvailableScopesDoc> =
        read_json_file(&app_root.join("_app").join("available_scopes.res.json"));
    let events_path = control_doc
        .as_ref()
        .and_then(|doc| doc.events_path.clone())
        .or_else(|| {
            environment
                .current_app_events_path
                .as_deref()
                .map(|path| display_virtualish_path(app_root, path))
        })
        .unwrap_or_else(|| "_stream/events.evt.jsonl".to_string());

    let current_app_id = control_doc
        .as_ref()
        .map_or(app_id, |doc| doc.app_id.as_str());
    let mut lines = render_appfs_overview_lines(
        environment,
        Some(current_app_id),
        Some(app_root),
        &events_path,
        None,
        None,
    );

    if let Some(doc) = control_doc
        .as_ref()
        .and_then(|doc| doc.description.as_deref())
    {
        lines.push(format!("- Current app purpose: {doc}"));
    }

    if let Some(scope_doc) = current_scope_doc.as_ref() {
        let scope_label = scope_doc
            .display_name
            .as_deref()
            .map_or_else(String::new, |name| format!(" ({name})"));
        lines.push(format!(
            "- You are currently inside app `{}` rooted at `{}` with active scope `{}`{scope_label}.",
            scope_doc.app_id,
            app_root.display(),
            scope_doc.active_scope
        ));
        lines.push(format!(
            "- Current scope details for app `{}`: `{}`{scope_label}.",
            scope_doc.app_id, scope_doc.active_scope
        ));
        if let Some(resource) = scope_doc.primary_resource.as_deref() {
            lines.push(format!(
                "- Primary resource for the current scope: `{resource}`."
            ));
        }
        if let Some(revision) = scope_doc.structure_revision_hint.as_deref() {
            lines.push(format!("- Structure revision hint: `{revision}`."));
        }
    } else if let Some(doc) = control_doc
        .as_ref()
        .and_then(|doc| doc.current_scope_path.as_deref())
    {
        lines.push(format!("- Current scope details are described in `{doc}`."));
    }

    if let Some(scopes_doc) = available_scopes_doc.as_ref() {
        let scope_names = scopes_doc
            .scopes
            .iter()
            .take(6)
            .map(|scope| {
                scope.display_name.as_deref().map_or_else(
                    || format!("`{}`", scope.scope_id),
                    |display_name| format!("`{}` ({display_name})", scope.scope_id),
                )
            })
            .collect::<Vec<_>>();
        if !scope_names.is_empty() {
            lines.push(format!(
                "- Known scopes for app `{}` (active `{}`): {}.",
                scopes_doc.app_id,
                scopes_doc.active_scope.as_deref().unwrap_or("<unknown>"),
                scope_names.join(", ")
            ));
        }
    } else if let Some(doc) = control_doc
        .as_ref()
        .and_then(|doc| doc.available_scopes_path.as_deref())
    {
        lines.push(format!("- Alternate scopes are listed in `{doc}`."));
    }

    lines.join("\n")
}

fn render_appfs_overview_lines(
    environment: &AppfsEnvironment,
    current_app_id: Option<&str>,
    current_app_root: Option<&Path>,
    events_path: &str,
    register_path: Option<&str>,
    list_path: Option<&str>,
) -> Vec<String> {
    let mut lines = vec![
        "# AppFS workspace guidance".to_string(),
        "- AppFS mounts bridge-backed software into a filesystem. After an app implements the AppFS bridge contract, reading and writing inside the mount interacts with the underlying software."
            .to_string(),
        "- Each mounted app appears as a directory under the AppFS mount root. The platform control plane lives under `/_appfs`."
            .to_string(),
        format!(
            "- AppFS `*.act` files are append-only JSONL action sinks: append exactly one JSON object line to trigger an operation. Prefer the AppFS event reminder injected into the next model call to confirm `action.completed` or `action.failed`; only inspect `{events_path}` manually for debugging or if no reminder appears."
        ),
        "- Never use `write_file` or `edit_file` on `*.act` files because those tools overwrite the sink. Use `bash` (or another append-capable tool) to append exactly one JSON object plus a trailing newline."
            .to_string(),
        "- Do not guess act schemas or payload shapes. For each mounted app, load its `appfs-<app>` skill to learn what actions exist, what parameters each action expects, and when to use them."
            .to_string(),
        "- Mounted app skills are listed separately in the skill listing attachment. Use the `Skill` tool to load the matching app skill before doing app-specific work."
            .to_string(),
        format!("- Current AppFS mount root: `{}`.", environment.mount_root.display()),
    ];

    if let (Some(app_id), Some(app_root)) = (current_app_id, current_app_root) {
        lines.push(format!(
            "- Your current working area is inside app `{app_id}` rooted at `{}`.",
            app_root.display()
        ));
    } else {
        lines.push(
            "- You are inside an AppFS mount, but not currently inside a specific app root."
                .to_string(),
        );
        if let Some(app_ids) = summarize_registered_app_ids(environment) {
            lines.push(format!(
                "- Mounted apps currently detected under this root: {app_ids}. Use the skill listing to load the matching `appfs-<app>` skill before doing app-specific work."
            ));
        }
    }

    if let Some(register_path) = register_path {
        lines.push(format!(
            "- Platform registration actions are available at `{register_path}`."
        ));
    }
    if let Some(list_path) = list_path {
        lines.push(format!(
            "- To inspect the mounted app registry from the filesystem, use `{list_path}` and the platform event stream."
        ));
    }

    lines
}
fn display_virtualish_path(base: &Path, path: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(base) {
        let rendered = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/");
        if !rendered.is_empty() {
            return rendered;
        }
    }
    path.display().to_string()
}

fn read_json_file<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        build_appfs_prompt_section, detect_appfs_environment,
        resolve_appfs_environment_with_attach_env, sync_appfs_event_reminders, AppfsAttachEnv,
        AppfsAttachSource, AppfsRuntimeManifest, AppfsRuntimeManifestCapabilities,
        AppfsRuntimeManifestControlPlane, APPFS_MULTI_AGENT_MODE_SHARED, APPFS_RUNTIME_KIND,
        APPFS_RUNTIME_MANIFEST_REL_PATH, APPFS_SCHEMA_VERSION,
    };
    use crate::session::{AttachmentKind, ContentBlock, Session};
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(test_name: &str) -> Self {
            let unique_suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map_or(0, |duration| duration.as_nanos());
            let path = env::temp_dir().join(format!(
                "appfs-agent-{test_name}-{}-{unique_suffix}",
                process::id()
            ));
            fs::create_dir_all(&path).expect("create temp test dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn seed_heuristic_mount(mount_root: &Path) {
        let control_dir = mount_root.join("_appfs");
        let app_root = mount_root.join("aiim");
        fs::create_dir_all(&control_dir).expect("create control dir");
        fs::create_dir_all(control_dir.join("_stream")).expect("create control stream dir");
        fs::create_dir_all(app_root.join("_app")).expect("create app control dir");
        fs::create_dir_all(app_root.join("_stream")).expect("create app stream dir");
        fs::write(control_dir.join("register_app.act"), "").expect("write register action");
        fs::write(control_dir.join("list_apps.act"), "").expect("write list action");
        fs::write(
            control_dir.join("apps.registry.json"),
            r#"{"version":1,"apps":[{"app_id":"aiim","active_scope":"chat-long"},{"app_id":"notion"}]}"#,
        )
        .expect("write registry");
        fs::write(app_root.join("_stream").join("events.evt.jsonl"), "").expect("write events");
    }

    fn seed_aiim_prompt_files(app_root: &Path) {
        fs::create_dir_all(app_root.join("_app")).expect("create _app dir");
        fs::write(
            app_root.join("_app").join("control.res.json"),
            r#"{
  "app_id": "aiim",
  "description": "AIIM demo app for incident chat, contact messaging, and scope switching.",
  "events_path": "_stream/events.evt.jsonl",
  "current_scope_path": "_app/current_scope.res.json",
  "available_scopes_path": "_app/available_scopes.res.json",
  "actions": [
    {
      "name": "enter_scope",
      "path": "_app/enter_scope.act",
      "summary": "Switch the app structure to a named scope such as chat-001 or chat-long.",
      "input_schema": "_meta/schemas/enter_scope.input.schema.json",
      "example_payload": {
        "target_scope": "chat-long"
      }
    }
  ]
}"#,
        )
        .expect("write control doc");
        fs::write(
            app_root.join("_app").join("actions.res.json"),
            r#"{
  "app_id": "aiim",
  "recommended_actions": [
    {
      "name": "send_message",
      "path": "contacts/zhangsan/send_message.act",
      "summary": "Send a direct message to 张三 / zhangsan.",
      "input_schema": "_meta/schemas/send_message.input.schema.json",
      "use_when": [
        "User asks to tell 张三 / zhangsan / 老张 something."
      ],
      "example_payload": {
        "text": "明天上午十点开会",
        "priority": "normal"
      }
    }
  ],
  "contact_routes": [
    {
      "contact_id": "zhangsan",
      "profile_path": "contacts/zhangsan/profile.res.json",
      "send_message_path": "contacts/zhangsan/send_message.act",
      "mention_tokens": [
        "张三",
        "老张",
        "zhangsan"
      ]
    }
  ]
}"#,
        )
        .expect("write actions doc");
        fs::write(
            app_root.join("_app").join("current_scope.res.json"),
            r#"{
  "app_id": "aiim",
  "active_scope": "chat-001",
  "display_name": "Default chat view",
  "primary_resource": "chats/chat-001/messages.res.jsonl",
  "entered_via": "_app/enter_scope.act"
}"#,
        )
        .expect("write current scope");
        fs::write(
            app_root.join("_app").join("available_scopes.res.json"),
            r#"{
  "app_id": "aiim",
  "active_scope": "chat-001",
  "scopes": [
    {
      "scope_id": "chat-001",
      "display_name": "Default chat view"
    },
    {
      "scope_id": "chat-long",
      "display_name": "Long chat stress view"
    }
  ]
}"#,
        )
        .expect("write available scopes");
    }

    fn seed_scheduler_prompt_files(app_root: &Path) {
        fs::create_dir_all(app_root.join("_app")).expect("create _app dir");
        fs::write(
            app_root.join("_app").join("control.res.json"),
            r#"{
  "app_id": "scheduler",
  "description": "Scheduler app for room bookings and meeting setup.",
  "events_path": "_stream/events.evt.jsonl",
  "current_scope_path": "_app/current_scope.res.json",
  "available_scopes_path": "_app/available_scopes.res.json"
}"#,
        )
        .expect("write control doc");
        fs::write(
            app_root.join("_app").join("current_scope.res.json"),
            r#"{
  "app_id": "scheduler",
  "active_scope": "meeting-room-a",
  "display_name": "Room A",
  "primary_resource": "meetings/today.res.jsonl"
}"#,
        )
        .expect("write current scope");
        fs::write(
            app_root.join("_app").join("available_scopes.res.json"),
            r#"{
  "app_id": "scheduler",
  "active_scope": "meeting-room-a",
  "scopes": [
    {
      "scope_id": "meeting-room-a",
      "display_name": "Room A"
    },
    {
      "scope_id": "meeting-room-b",
      "display_name": "Room B"
    }
  ]
}"#,
        )
        .expect("write available scopes");
    }

    fn write_manifest(mount_root: &Path, runtime_session_id: &str) -> PathBuf {
        let manifest_path = mount_root.join(APPFS_RUNTIME_MANIFEST_REL_PATH);
        fs::create_dir_all(
            manifest_path
                .parent()
                .expect("manifest path should have parent"),
        )
        .expect("create manifest dir");
        let manifest = AppfsRuntimeManifest {
            schema_version: APPFS_SCHEMA_VERSION,
            runtime_kind: APPFS_RUNTIME_KIND.to_string(),
            mount_root: mount_root.to_path_buf(),
            runtime_session_id: runtime_session_id.to_string(),
            managed: true,
            multi_agent_mode: APPFS_MULTI_AGENT_MODE_SHARED.to_string(),
            control_plane: AppfsRuntimeManifestControlPlane {
                register_action: "/_appfs/register_app.act".to_string(),
                unregister_action: "/_appfs/unregister_app.act".to_string(),
                list_action: "/_appfs/list_apps.act".to_string(),
                registry: "/_appfs/apps.registry.json".to_string(),
                events: "/_appfs/_stream/events.evt.jsonl".to_string(),
            },
            capabilities: AppfsRuntimeManifestCapabilities {
                app_registration: true,
                event_stream: true,
                multi_app: true,
                scope_switch: true,
                multi_agent_attach: true,
            },
            generated_at: "2026-04-07T00:00:00Z".to_string(),
        };
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
        manifest_path
    }

    #[test]
    fn manifest_attach_generates_ephemeral_attach_id() {
        let temp = TempDirGuard::new("appfs-manifest");
        let mount_root = temp.path().join("mnt");
        let cwd = mount_root.join("aiim").join("workspace");
        seed_heuristic_mount(&mount_root);
        fs::create_dir_all(&cwd).expect("create cwd");
        write_manifest(&mount_root, "rt-shared-01");

        let detected =
            detect_appfs_environment(&cwd).expect("expected appfs environment to be found");

        assert_eq!(detected.attach_source, AppfsAttachSource::Manifest);
        assert_eq!(detected.runtime_session_id.as_deref(), Some("rt-shared-01"));
        assert!(detected.attach_id.starts_with("attach-"));
        assert_eq!(detected.multi_agent_mode, APPFS_MULTI_AGENT_MODE_SHARED);
        assert_eq!(detected.current_app_id.as_deref(), Some("aiim"));
    }

    #[test]
    fn env_attach_overrides_manifest_and_emits_warning_when_mismatched() {
        let temp = TempDirGuard::new("appfs-env");
        let mount_root = temp.path().join("mnt");
        let cwd = mount_root.join("aiim");
        let override_root = temp.path().join("override");
        seed_heuristic_mount(&mount_root);
        fs::create_dir_all(&cwd).expect("create cwd");
        fs::create_dir_all(&override_root).expect("create override root");
        let manifest_path = write_manifest(&mount_root, "rt-manifest-01");
        let detected = resolve_appfs_environment_with_attach_env(
            &cwd,
            AppfsAttachEnv {
                schema: Some("1".to_string()),
                manifest_path: Some(manifest_path),
                mount_root: Some(override_root.clone()),
                runtime_session_id: Some("rt-env-01".to_string()),
                attach_id: Some("agent-a".to_string()),
                attach_role: Some("planner".to_string()),
            },
        )
        .expect("expected appfs environment to be found");

        assert_eq!(detected.attach_source, AppfsAttachSource::Env);
        assert_eq!(detected.mount_root, override_root);
        assert_eq!(detected.runtime_session_id.as_deref(), Some("rt-env-01"));
        assert_eq!(detected.attach_id, "agent-a");
        assert_eq!(detected.attach_role.as_deref(), Some("planner"));
        assert!(detected
            .warnings
            .iter()
            .any(|warning| warning.contains("does not match manifest mount root")));
        assert!(detected
            .warnings
            .iter()
            .any(|warning| warning.contains("does not match manifest runtime session")));
    }

    #[test]
    fn env_attach_without_attach_id_generates_ephemeral_instance_id() {
        let temp = TempDirGuard::new("appfs-env-auto-attach");
        let mount_root = temp.path().join("mnt");
        let cwd = mount_root.join("aiim");
        seed_heuristic_mount(&mount_root);
        fs::create_dir_all(&cwd).expect("create cwd");
        let manifest_path = write_manifest(&mount_root, "rt-shared-02");
        let detected = resolve_appfs_environment_with_attach_env(
            &cwd,
            AppfsAttachEnv {
                schema: Some("1".to_string()),
                manifest_path: Some(manifest_path),
                mount_root: None,
                runtime_session_id: None,
                attach_id: None,
                attach_role: Some("planner".to_string()),
            },
        )
        .expect("expected appfs environment to be found");

        assert_eq!(detected.attach_source, AppfsAttachSource::Env);
        assert_eq!(detected.runtime_session_id.as_deref(), Some("rt-shared-02"));
        assert!(detected.attach_id.starts_with("attach-"));
        assert_eq!(detected.attach_role.as_deref(), Some("planner"));
    }

    #[test]
    fn shared_manifest_supports_distinct_attach_ids_for_multiple_agents() {
        let temp = TempDirGuard::new("appfs-shared-manifest");
        let mount_root = temp.path().join("mnt");
        let cwd = mount_root.join("aiim").join("workspace");
        seed_heuristic_mount(&mount_root);
        fs::create_dir_all(&cwd).expect("create cwd");
        let manifest_path = write_manifest(&mount_root, "rt-shared-03");
        let attach_env = AppfsAttachEnv {
            schema: Some("1".to_string()),
            manifest_path: Some(manifest_path),
            mount_root: Some(mount_root),
            runtime_session_id: Some("rt-shared-03".to_string()),
            attach_id: None,
            attach_role: Some("worker".to_string()),
        };
        let detected_a = resolve_appfs_environment_with_attach_env(
            &cwd,
            AppfsAttachEnv {
                attach_id: Some("agent-a".to_string()),
                ..attach_env.clone()
            },
        )
        .expect("expected first appfs environment");
        let detected_b = resolve_appfs_environment_with_attach_env(
            &cwd,
            AppfsAttachEnv {
                attach_id: Some("agent-b".to_string()),
                ..attach_env
            },
        )
        .expect("expected second appfs environment");

        assert_eq!(detected_a.runtime_session_id, detected_b.runtime_session_id);
        assert_eq!(detected_a.attach_role, detected_b.attach_role);
        assert_eq!(detected_a.attach_source, AppfsAttachSource::Env);
        assert_eq!(detected_b.attach_source, AppfsAttachSource::Env);
        assert_eq!(detected_a.multi_agent_mode, APPFS_MULTI_AGENT_MODE_SHARED);
        assert_eq!(detected_b.multi_agent_mode, APPFS_MULTI_AGENT_MODE_SHARED);
        assert_ne!(detected_a.attach_id, detected_b.attach_id);
        assert_eq!(detected_a.attach_id, "agent-a");
        assert_eq!(detected_b.attach_id, "agent-b");
    }

    #[test]
    fn heuristic_attach_falls_back_when_manifest_is_missing() {
        let temp = TempDirGuard::new("appfs-heuristic");
        let mount_root = temp.path().join("mnt");
        let cwd = mount_root.join("aiim").join("workspace");
        seed_heuristic_mount(&mount_root);
        fs::create_dir_all(&cwd).expect("create cwd");

        let detected =
            detect_appfs_environment(&cwd).expect("expected appfs environment to be found");

        assert_eq!(detected.attach_source, AppfsAttachSource::Heuristic);
        assert!(detected.runtime_session_id.is_none());
        assert!(detected.attach_id.starts_with("attach-"));
        assert_eq!(detected.current_app_id.as_deref(), Some("aiim"));
    }

    #[test]
    fn appfs_prompt_section_surfaces_current_app_guidance() {
        let temp = TempDirGuard::new("appfs-prompt");
        let mount_root = temp.path().join("mnt");
        let app_root = mount_root.join("aiim");
        let cwd = app_root.join("workspace");
        seed_heuristic_mount(&mount_root);
        seed_aiim_prompt_files(&app_root);
        fs::create_dir_all(&cwd).expect("create cwd");

        let prompt = build_appfs_prompt_section(&cwd).expect("expected appfs prompt section");

        assert!(prompt.contains("AppFS mounts bridge-backed software into a filesystem"));
        assert!(prompt.contains("Do not guess act schemas or payload shapes"));
        assert!(prompt.contains("Prefer the AppFS event reminder"));
        assert!(prompt
            .contains("Mounted app skills are listed separately in the skill listing attachment"));
        assert!(prompt.contains("Never use `write_file` or `edit_file` on `*.act` files"));
        assert!(prompt.contains("chat-long"));
        assert!(prompt.contains("_stream/events.evt.jsonl"));
        assert!(!prompt.contains("## Mounted apps"));
        assert!(!prompt.contains("`aiim` -> skill `appfs-aiim`"));
    }

    #[test]
    fn appfs_prompt_section_surfaces_generated_skill_without_actions_doc() {
        let temp = TempDirGuard::new("appfs-prompt-control-only");
        let mount_root = temp.path().join("mnt");
        let app_root = mount_root.join("scheduler");
        let cwd = app_root.join("workspace");
        seed_heuristic_mount(&mount_root);
        seed_scheduler_prompt_files(&app_root);
        fs::create_dir_all(&cwd).expect("create cwd");

        let prompt = build_appfs_prompt_section(&cwd).expect("expected appfs prompt section");

        assert!(prompt
            .contains("Mounted app skills are listed separately in the skill listing attachment"));
        assert!(prompt.contains("Scheduler app for room bookings and meeting setup."));
        assert!(prompt.contains("meeting-room-b"));
        assert!(prompt.contains("Never use `write_file` or `edit_file` on `*.act` files"));
        assert!(!prompt.contains("## Mounted apps"));
        assert!(!prompt.contains("`scheduler` -> skill `appfs-scheduler`"));
        assert!(!prompt.contains("message 张三 / 老张 / zhangsan"));
    }

    #[test]
    fn appfs_prompt_section_surfaces_registered_apps_from_mount_root() {
        let temp = TempDirGuard::new("appfs-prompt-mount-root");
        let mount_root = temp.path().join("mnt");
        let app_root = mount_root.join("aiim");
        seed_heuristic_mount(&mount_root);
        seed_aiim_prompt_files(&app_root);

        let prompt =
            build_appfs_prompt_section(&mount_root).expect("expected appfs prompt section");

        assert!(prompt.contains(
            "You are inside an AppFS mount, but not currently inside a specific app root."
        ));
        assert!(
            prompt.contains("Mounted apps currently detected under this root: `aiim`, `notion`.")
        );
        assert!(prompt.contains("Use the skill listing to load the matching `appfs-<app>` skill"));
        assert!(!prompt.contains("## Mounted apps"));
        assert!(!prompt.contains("`aiim` -> skill `appfs-aiim`"));
    }

    #[test]
    fn sync_appfs_event_reminders_baselines_then_injects_new_events() {
        let temp = TempDirGuard::new("appfs-event-reminders");
        let mount_root = temp.path().join("mnt");
        let app_root = mount_root.join("aiim");
        let notion_root = mount_root.join("notion");
        seed_heuristic_mount(&mount_root);
        fs::create_dir_all(notion_root.join("_stream")).expect("create notion stream");

        let write_cursor = |stream_dir: &Path, max_seq: i64| {
            fs::write(
                stream_dir.join("cursor.res.json"),
                format!(r#"{{"min_seq":0,"max_seq":{max_seq},"retention_hint_sec":86400}}"#),
            )
            .expect("write stream cursor");
        };

        let control_events = mount_root
            .join("_appfs")
            .join("_stream")
            .join("events.evt.jsonl");
        let app_events = app_root.join("_stream").join("events.evt.jsonl");
        let notion_events = notion_root.join("_stream").join("events.evt.jsonl");
        fs::write(
            &control_events,
            r#"{"seq":1,"type":"runtime.started","path":"/_appfs","request_id":"old-platform"}"#,
        )
        .expect("write control baseline event");
        fs::write(
            &app_events,
            r#"{"seq":1,"app":"aiim","type":"action.completed","path":"/old.act","request_id":"old-app"}"#,
        )
        .expect("write app baseline event");
        fs::write(
            &notion_events,
            r#"{"seq":1,"app":"notion","type":"action.completed","path":"/old.act","request_id":"old-notion"}"#,
        )
        .expect("write second app baseline event");
        write_cursor(&mount_root.join("_appfs").join("_stream"), 1);
        write_cursor(&app_root.join("_stream"), 1);
        write_cursor(&notion_root.join("_stream"), 1);

        let mut session = Session::new();
        sync_appfs_event_reminders(&mut session, &mount_root).expect("baseline should sync");

        assert!(session.messages.is_empty());
        assert_eq!(session.appfs_event_cursor("platform"), Some(1));
        assert_eq!(session.appfs_event_cursor("app:aiim"), Some(1));
        assert_eq!(session.appfs_event_cursor("app:notion"), Some(1));

        fs::write(
            &control_events,
            concat!(
                r#"{"seq":1,"type":"runtime.started","path":"/_appfs","request_id":"old-platform"}"#,
                "\n",
                r#"{"seq":2,"type":"action.completed","path":"/_appfs/register_app.act","request_id":"new-platform","content":{"app_id":"scheduler","registered":true}}"#,
                "\n"
            ),
        )
        .expect("append control event");
        fs::write(
            &app_events,
            concat!(
                r#"{"seq":1,"app":"aiim","type":"action.completed","path":"/old.act","request_id":"old-app"}"#,
                "\n",
                r#"{"seq":2,"app":"aiim","type":"action.accepted","path":"/contacts/zhangsan/send_message.act","request_id":"new-app","content":{"status":"accepted"}}"#,
                "\n",
                r#"{"seq":3,"app":"aiim","type":"action.progress","path":"/contacts/zhangsan/send_message.act","request_id":"new-app","content":{"percent":50}}"#,
                "\n",
                r#"{"seq":4,"app":"aiim","type":"action.completed","path":"/contacts/zhangsan/send_message.act","request_id":"new-app","content":{"ok":true,"echo":{"text":"明天开会","priority":"normal"}}}"#,
                "\n"
            ),
        )
        .expect("append app event");
        fs::write(
            &notion_events,
            concat!(
                r#"{"seq":1,"app":"notion","type":"action.completed","path":"/old.act","request_id":"old-notion"}"#,
                "\n",
                r#"{"seq":2,"app":"notion","type":"action.failed","path":"/pages/create.act","request_id":"new-notion","error":{"code":"ERR_TIMEOUT","message":"timed out","retryable":true}}"#,
                "\n"
            ),
        )
        .expect("append second app event");
        write_cursor(&mount_root.join("_appfs").join("_stream"), 2);
        write_cursor(&app_root.join("_stream"), 4);
        write_cursor(&notion_root.join("_stream"), 2);

        sync_appfs_event_reminders(&mut session, &mount_root).expect("new events should sync");

        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0]
                .attachment_metadata
                .as_ref()
                .map(|metadata| metadata.kind),
            Some(AttachmentKind::AppfsEvents)
        );
        let [ContentBlock::Text { text }] = session.messages[0].blocks.as_slice() else {
            panic!("expected text reminder");
        };
        assert!(text.contains("<system-reminder>"));
        assert!(text.contains("action.completed"));
        assert!(text.contains("app_id='scheduler'"));
        assert!(text.contains("registered=true"));
        assert!(text.contains("action.completed"));
        assert!(text.contains("action.accepted"));
        assert!(text.contains("action.progress"));
        assert!(text.contains("action.failed"));
        assert!(text.contains("app=aiim"));
        assert!(text.contains("app=notion"));
        assert!(text.contains("/contacts/zhangsan/send_message.act"));
        assert!(text.contains("summary=action accepted; status='accepted'"));
        assert!(text.contains("summary=action progress; percent=50"));
        assert!(text.contains("summary=action completed; ok=true; payload="));
        assert!(text.contains(
            "summary=action failed; code='ERR_TIMEOUT'; message='timed out'; retryable=true"
        ));
        assert!(!text.contains("stream=app:"));
        assert!(!text.contains("old-platform"));
        assert!(!text.contains("/old.act"));
        assert_eq!(session.appfs_event_cursor("platform"), Some(2));
        assert_eq!(session.appfs_event_cursor("app:aiim"), Some(4));
        assert_eq!(session.appfs_event_cursor("app:notion"), Some(2));

        sync_appfs_event_reminders(&mut session, &mount_root).expect("empty sync should succeed");
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn returns_none_when_control_plane_is_missing() {
        let temp = TempDirGuard::new("appfs-miss");
        let cwd = temp.path().join("workspace");
        fs::create_dir_all(&cwd).expect("create cwd");

        assert!(detect_appfs_environment(&cwd).is_none());
    }
}
