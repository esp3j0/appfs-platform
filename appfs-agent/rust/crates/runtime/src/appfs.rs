use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const CONTROL_DIR_NAME: &str = "_appfs";
const REGISTER_APP_ACTION: &str = "register_app.act";
const UNREGISTER_APP_ACTION: &str = "unregister_app.act";
const LIST_APPS_ACTION: &str = "list_apps.act";
const REGISTRY_FILE: &str = "apps.registry.json";
const APP_CONTROL_DIR_NAME: &str = "_app";
const APP_STREAM_DIR_NAME: &str = "_stream";
const EVENTS_FILE: &str = "events.evt.jsonl";
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

#[cfg(test)]
mod tests {
    use super::{
        detect_appfs_environment, resolve_appfs_environment_with_attach_env, AppfsAttachEnv,
        AppfsAttachSource, AppfsRuntimeManifest, AppfsRuntimeManifestCapabilities,
        AppfsRuntimeManifestControlPlane, APPFS_MULTI_AGENT_MODE_SHARED, APPFS_RUNTIME_KIND,
        APPFS_RUNTIME_MANIFEST_REL_PATH, APPFS_SCHEMA_VERSION,
    };
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
    fn returns_none_when_control_plane_is_missing() {
        let temp = TempDirGuard::new("appfs-miss");
        let cwd = temp.path().join("workspace");
        fs::create_dir_all(&cwd).expect("create cwd");

        assert!(detect_appfs_environment(&cwd).is_none());
    }
}
