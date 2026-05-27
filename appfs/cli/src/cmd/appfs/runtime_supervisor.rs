use super::action_dispatcher;
use super::registry;
use super::registry_manager::{self, RegistryRuntimeSnapshot};
use super::runtime_entry::{
    build_runtime_entry, build_runtime_entry_with_metadata, read_active_scope, transport_summary,
    AppRuntimeEntry, AppRuntimeRegistryMetadata,
};
use super::runtime_manifest;
use super::supervisor_control;
use super::{AppRuntimeStartupBootstrap, ProcessOutcome, ResolvedAppfsRuntimeCliArgs};
use anyhow::Result;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Duration;

pub(super) struct AppfsRuntimeSupervisor {
    root: PathBuf,
    managed: bool,
    runtime_session_id: String,
    control_plane: supervisor_control::SupervisorControlPlane,
    pub(super) runtimes: BTreeMap<String, AppRuntimeEntry>,
}

impl AppfsRuntimeSupervisor {
    pub(super) fn new(
        root: PathBuf,
        runtime_args: Vec<ResolvedAppfsRuntimeCliArgs>,
        managed: bool,
        startup_bootstrap: Option<HashMap<String, AppRuntimeStartupBootstrap>>,
        existing_registry: Option<&registry::AppfsAppsRegistryDoc>,
    ) -> Result<Self> {
        let mut runtimes = BTreeMap::new();
        let mut startup_bootstrap = startup_bootstrap.unwrap_or_default();
        let mut metadata_by_runtime = HashMap::new();
        if let Some(registry) = existing_registry {
            for app in &registry.apps {
                metadata_by_runtime.insert(
                    runtime_metadata_key(&app.app_id, &app.session_id),
                    AppRuntimeRegistryMetadata::from_registered_app(app),
                );
            }
        }
        for runtime in runtime_args {
            let app_id = runtime.app_id.clone();
            let session_id = runtime.session_id.clone();
            let metadata = metadata_by_runtime
                .remove(&runtime_metadata_key(&app_id, &session_id))
                .unwrap_or_else(|| AppRuntimeRegistryMetadata::public(app_id.clone()));
            let entry = build_runtime_entry_with_metadata(
                &root,
                runtime,
                metadata,
                startup_bootstrap.remove(&app_id),
            )?;
            let instance_id = entry.registry_metadata.instance_id.clone();
            if runtimes.insert(instance_id, entry).is_some() {
                anyhow::bail!("duplicate runtime instance_id during supervisor bootstrap");
            }
        }
        Ok(Self {
            managed,
            runtime_session_id: runtime_manifest::generate_runtime_session_id(),
            control_plane: supervisor_control::SupervisorControlPlane::new(
                root.clone(),
                std::env::var("APPFS_ACTIONLINE_STRICT")
                    .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "True"))
                    .unwrap_or(false),
            )?,
            root,
            runtimes,
        })
    }

    pub(super) fn prepare_action_sinks(&mut self) -> Result<()> {
        self.control_plane.prepare_action_sinks()?;
        for entry in self.runtimes.values_mut() {
            entry.adapter.prepare_action_sinks()?;
        }
        Ok(())
    }

    pub(super) fn poll_once(&mut self) -> Result<()> {
        self.drain_control_plane_and_materialize_private_apps()?;
        for entry in self.runtimes.values_mut() {
            entry.adapter.poll_once()?;
        }
        self.sync_runtime_registry_to_disk(None)?;
        Ok(())
    }

    pub(super) fn poll_action_work_once(&mut self) -> Result<()> {
        self.drain_control_plane_and_materialize_private_apps()?;
        for entry in self.runtimes.values_mut() {
            entry.adapter.poll_action_work_once()?;
        }
        self.sync_runtime_registry_to_disk(None)?;
        Ok(())
    }

    pub(super) fn poll_inbound_events_once(&mut self) -> Result<()> {
        for entry in self
            .runtimes
            .values_mut()
            .filter(|entry| entry.registry_metadata.inbound_poll_ms > 0)
        {
            entry.adapter.poll_inbound_events_once()?;
        }
        Ok(())
    }

    pub(super) fn inbound_poll_interval(&self) -> Option<Duration> {
        self.runtimes
            .values()
            .filter_map(|entry| {
                let inbound_poll_ms = entry.registry_metadata.inbound_poll_ms;
                (inbound_poll_ms > 0)
                    .then(|| Duration::from_millis(inbound_poll_ms.max(super::MIN_POLL_MS)))
            })
            .min()
    }

    pub(super) fn log_started(&self) {
        eprintln!(
            "AppFS runtime session started (mount_root={} runtime_session_id={} managed={})",
            self.root.display(),
            self.runtime_session_id,
            self.managed
        );
        for entry in self.runtimes.values() {
            let adapter = &entry.adapter;
            eprintln!(
                "AppFS adapter started for {} (app_id={} session={})",
                adapter.app_dir.display(),
                adapter.app_id,
                adapter.session_id
            );
        }
    }

    pub(super) fn sync_registry_to_disk(
        &self,
        existing: Option<&registry::AppfsAppsRegistryDoc>,
    ) -> Result<()> {
        self.sync_runtime_registry_to_disk(existing)?;
        runtime_manifest::write_runtime_manifest(&self.root, &self.runtime_session_id, self.managed)
    }

    fn sync_runtime_registry_to_disk(
        &self,
        existing: Option<&registry::AppfsAppsRegistryDoc>,
    ) -> Result<()> {
        let snapshots = self
            .runtimes
            .values()
            .map(|entry| RegistryRuntimeSnapshot {
                runtime: entry.runtime.clone(),
                app_dir: entry.adapter.app_dir.clone(),
                metadata: entry.registry_metadata.clone(),
            })
            .collect::<Vec<_>>();
        registry_manager::persist_runtime_registry(&self.root, &snapshots, existing)
    }

    fn drain_control_plane_and_materialize_private_apps(&mut self) -> Result<()> {
        let invocations = self.control_plane.drain_invocations()?;
        for invocation in invocations {
            self.handle_control_invocation(invocation)?;
        }
        Ok(())
    }

    fn handle_control_invocation(
        &mut self,
        invocation: supervisor_control::SupervisorControlInvocation,
    ) -> Result<()> {
        match invocation {
            supervisor_control::SupervisorControlInvocation::Register {
                request_id,
                client_token,
                request,
            } => self.handle_register_app(&request_id, client_token, request),
            supervisor_control::SupervisorControlInvocation::Unregister {
                request_id,
                client_token,
                request,
            } => self.handle_unregister_app(&request_id, client_token, request),
            supervisor_control::SupervisorControlInvocation::List {
                request_id,
                client_token,
            } => self.handle_list_apps(&request_id, client_token),
            supervisor_control::SupervisorControlInvocation::CreatePrincipal {
                request_id,
                client_token,
                request,
            } => self.handle_create_principal(&request_id, client_token, request),
            supervisor_control::SupervisorControlInvocation::UpdatePrincipal {
                request_id,
                client_token,
                request,
            } => self.handle_update_principal(&request_id, client_token, request),
            supervisor_control::SupervisorControlInvocation::DeletePrincipal {
                request_id,
                client_token,
                request,
            } => self.handle_delete_principal(&request_id, client_token, request),
            supervisor_control::SupervisorControlInvocation::AttachPrincipal {
                request_id,
                client_token,
                request,
            } => self.handle_attach_principal(&request_id, client_token, request),
            supervisor_control::SupervisorControlInvocation::DetachPrincipal {
                request_id,
                client_token,
                request,
            } => self.handle_detach_principal(&request_id, client_token, request),
        }
    }

    fn handle_register_app(
        &mut self,
        request_id: &str,
        client_token: Option<String>,
        request: action_dispatcher::RegisterAppRequest,
    ) -> Result<()> {
        if self.runtimes.contains_key(&request.app_id) {
            self.control_plane.emit_failed(
                "/_appfs/register_app.act",
                request_id,
                "APP_ALREADY_REGISTERED",
                &format!("app {} is already registered", request.app_id),
                client_token,
            )?;
            return Ok(());
        }

        let runtime = match registry_manager::register_request_to_runtime(request) {
            Ok(runtime) => runtime,
            Err(err) => {
                self.control_plane.emit_failed(
                    "/_appfs/register_app.act",
                    request_id,
                    "APP_REGISTER_INVALID",
                    &err.to_string(),
                    client_token,
                )?;
                return Ok(());
            }
        };

        match build_runtime_entry(&self.root, runtime.clone(), None) {
            Ok(mut entry) => {
                entry.adapter.prepare_action_sinks()?;
                let app_id = entry.runtime.app_id.clone();
                let session_id = entry.runtime.session_id.clone();
                let transport = transport_summary(&entry.runtime.bridge);
                self.runtimes.insert(app_id.clone(), entry);
                self.sync_registry_to_disk(None)?;
                self.control_plane.emit_completed(
                    "/_appfs/register_app.act",
                    request_id,
                    serde_json::json!({
                        "app_id": app_id,
                        "session_id": session_id,
                        "transport": transport,
                        "registered": true,
                    }),
                    client_token,
                )?;
            }
            Err(err) => {
                self.control_plane.emit_failed(
                    "/_appfs/register_app.act",
                    request_id,
                    "APP_REGISTER_FAILED",
                    &format!("failed to register app: {err}"),
                    client_token,
                )?;
            }
        }
        Ok(())
    }

    fn handle_unregister_app(
        &mut self,
        request_id: &str,
        client_token: Option<String>,
        request: action_dispatcher::UnregisterAppRequest,
    ) -> Result<()> {
        let Some(entry) = self.runtimes.remove(&request.app_id) else {
            self.control_plane.emit_failed(
                "/_appfs/unregister_app.act",
                request_id,
                "APP_NOT_REGISTERED",
                &format!("app {} is not registered", request.app_id),
                client_token,
            )?;
            return Ok(());
        };
        self.sync_registry_to_disk(None)?;
        self.control_plane.emit_completed(
            "/_appfs/unregister_app.act",
            request_id,
            serde_json::json!({
                "app_id": entry.runtime.app_id,
                "session_id": entry.runtime.session_id,
                "unregistered": true,
            }),
            client_token,
        )?;
        Ok(())
    }

    fn handle_list_apps(&mut self, request_id: &str, client_token: Option<String>) -> Result<()> {
        let apps = self
            .runtimes
            .values()
            .map(|entry| {
                serde_json::json!({
                    "app_id": entry.runtime.app_id,
                    "session_id": entry.runtime.session_id,
                    "transport": transport_summary(&entry.runtime.bridge),
                    "active_scope": read_active_scope(&entry.adapter.app_dir),
                })
            })
            .collect::<Vec<_>>();
        self.control_plane.emit_completed(
            "/_appfs/list_apps.act",
            request_id,
            serde_json::json!({ "apps": apps }),
            client_token,
        )?;
        Ok(())
    }

    fn handle_create_principal(
        &mut self,
        request_id: &str,
        client_token: Option<String>,
        request: action_dispatcher::CreatePrincipalRequest,
    ) -> Result<()> {
        let mut doc = self.load_principal_registry()?;
        if let Some(existing) = doc
            .principals
            .iter()
            .find(|principal| principal.principal_id == request.principal_id)
            .cloned()
        {
            registry::write_principal_record_view(&self.root, &existing)?;
            let materialized = self.materialize_private_apps_for_principal(&existing)?;
            self.control_plane.emit_completed(
                "/_appfs/principals/create_principal.act",
                request_id,
                serde_json::json!({
                    "principal_event": "principal.exists",
                    "principal_id": existing.principal_id,
                    "created": false,
                    "exists": true,
                    "app_instances": materialized,
                }),
                client_token,
            )?;
            return Ok(());
        }

        let now = chrono::Utc::now().to_rfc3339();
        let record = registry::PrincipalRecord {
            principal_id: request.principal_id,
            display_name: request.display_name,
            description: request.description,
            kind: request.kind,
            created_at: now.clone(),
            updated_at: now,
            active_attach_count: 0,
            active_attaches: Vec::new(),
            agent_status: None,
        };
        doc.principals.push(record.clone());
        registry::write_principal_registry(&self.root, &doc)?;
        registry::write_principal_record_view(&self.root, &record)?;
        let materialized = self.materialize_private_apps_for_principal(&record)?;
        self.control_plane.emit_completed(
            "/_appfs/principals/create_principal.act",
            request_id,
            serde_json::json!({
                "principal_event": "principal.created",
                "principal_id": record.principal_id,
                "created": true,
                "app_instances": materialized,
            }),
            client_token,
        )?;
        Ok(())
    }

    fn handle_update_principal(
        &mut self,
        request_id: &str,
        client_token: Option<String>,
        request: action_dispatcher::UpdatePrincipalRequest,
    ) -> Result<()> {
        let mut doc = self.load_principal_registry()?;
        let Some(record) = doc
            .principals
            .iter_mut()
            .find(|principal| principal.principal_id == request.principal_id)
        else {
            self.control_plane.emit_failed(
                "/_appfs/principals/update_principal.act",
                request_id,
                "PRINCIPAL_NOT_FOUND",
                &format!("principal {} is not registered", request.principal_id),
                client_token,
            )?;
            return Ok(());
        };

        if let Some(display_name) = request.display_name {
            record.display_name = display_name;
        }
        if let Some(description) = request.description {
            record.description = Some(description);
        }
        if let Some(kind) = request.kind {
            record.kind = kind;
        }
        if let Some(agent_status) = request.agent_status {
            let Some(request_attach_id) = request.attach_id.as_deref() else {
                self.control_plane.emit_failed(
                    "/_appfs/principals/update_principal.act",
                    request_id,
                    "PRINCIPAL_STATUS_ATTACH_REQUIRED",
                    "attach_id is required when updating agent_status",
                    client_token,
                )?;
                return Ok(());
            };
            let Some(active_attach) = current_active_attach(record) else {
                self.control_plane.emit_failed(
                    "/_appfs/principals/update_principal.act",
                    request_id,
                    "PRINCIPAL_NOT_ATTACHED",
                    &format!("principal {} has no active attach", record.principal_id),
                    client_token,
                )?;
                return Ok(());
            };
            if active_attach.attach_id != request_attach_id {
                self.control_plane.emit_failed(
                    "/_appfs/principals/update_principal.act",
                    request_id,
                    "PRINCIPAL_ATTACH_MISMATCH",
                    &format!(
                        "attach_id {} cannot update principal {}",
                        request_attach_id, record.principal_id
                    ),
                    client_token,
                )?;
                return Ok(());
            }
            if let Some(active_attach) = record
                .active_attaches
                .iter_mut()
                .find(|lease| lease.attach_id == request_attach_id)
            {
                active_attach.last_seen_at = chrono::Utc::now().to_rfc3339();
            }
            apply_agent_status_patch(record, request_attach_id, agent_status);
        }
        record.updated_at = chrono::Utc::now().to_rfc3339();
        let updated = record.clone();
        registry::write_principal_registry(&self.root, &doc)?;
        registry::write_principal_record_view(&self.root, &updated)?;
        self.control_plane.emit_completed(
            "/_appfs/principals/update_principal.act",
            request_id,
            serde_json::json!({
                "principal_event": if updated.agent_status.is_some() { "principal.status.updated" } else { "principal.updated" },
                "principal_id": updated.principal_id,
                "updated": true,
                "agent_status": updated.agent_status,
            }),
            client_token,
        )?;
        Ok(())
    }

    fn handle_delete_principal(
        &mut self,
        request_id: &str,
        client_token: Option<String>,
        request: action_dispatcher::DeletePrincipalRequest,
    ) -> Result<()> {
        let mut doc = self.load_principal_registry()?;
        let before = doc.principals.len();
        doc.principals
            .retain(|principal| principal.principal_id != request.principal_id);
        if doc.principals.len() == before {
            self.control_plane.emit_failed(
                "/_appfs/principals/delete_principal.act",
                request_id,
                "PRINCIPAL_NOT_FOUND",
                &format!("principal {} is not registered", request.principal_id),
                client_token,
            )?;
            return Ok(());
        }
        let cleanup_keys = self
            .runtimes
            .iter()
            .filter(|(_, entry)| {
                entry.registry_metadata.principal_id.as_deref()
                    == Some(request.principal_id.as_str())
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let mut credential_cleanup_requests = Vec::new();
        for runtime_key in cleanup_keys {
            let Some(entry) = self.runtimes.get_mut(&runtime_key) else {
                continue;
            };
            let profile_id = entry.registry_metadata.profile_id.clone();
            let instance_id = entry.registry_metadata.instance_id.clone();
            let app_id = entry.runtime.app_id.clone();
            let mut cleanup_request = serde_json::json!({
                "instance_id": instance_id,
                "app_id": app_id,
                "principal_id": request.principal_id.as_str(),
                "status": "skipped",
                "cleanup_action": "/_app/forget_credentials.act",
            });

            if let Some(profile_id) = profile_id.as_deref() {
                cleanup_request["profile_id"] = serde_json::json!(profile_id);
                match entry.adapter.submit_internal_action(
                    "/_app/forget_credentials.act",
                    serde_json::json!({
                        "reason": "principal_deleted",
                    }),
                    Some(format!("principal-delete-{}", request.principal_id)),
                ) {
                    Ok(ProcessOutcome::Consumed) => {
                        cleanup_request["status"] = serde_json::json!("completed");
                    }
                    Ok(ProcessOutcome::RetryPending) => {
                        cleanup_request["status"] = serde_json::json!("retry_pending");
                    }
                    Err(err) => {
                        cleanup_request["status"] = serde_json::json!("failed");
                        cleanup_request["error"] = serde_json::json!(err.to_string());
                    }
                }
            }

            credential_cleanup_requests.push(cleanup_request);
        }
        registry::write_principal_registry(&self.root, &doc)?;
        registry::delete_principal_record_view(&self.root, &request.principal_id)?;
        self.control_plane.emit_completed(
            "/_appfs/principals/delete_principal.act",
            request_id,
            serde_json::json!({
                "principal_event": "principal.deleted",
                "principal_id": request.principal_id,
                "deleted": true,
                "credentials_cleanup": "requested",
                "credential_cleanup_requests": credential_cleanup_requests,
            }),
            client_token,
        )?;
        Ok(())
    }

    fn handle_attach_principal(
        &mut self,
        request_id: &str,
        client_token: Option<String>,
        request: action_dispatcher::AttachPrincipalRequest,
    ) -> Result<()> {
        let mut doc = self.load_principal_registry()?;
        let Some(record) = doc
            .principals
            .iter_mut()
            .find(|principal| principal.principal_id == request.principal_id)
        else {
            self.control_plane.emit_failed(
                "/_appfs/principals/attach_principal.act",
                request_id,
                "PRINCIPAL_NOT_FOUND",
                &format!("principal {} is not registered", request.principal_id),
                client_token,
            )?;
            return Ok(());
        };

        let now = chrono::Utc::now().to_rfc3339();
        let now_dt = chrono::Utc::now();
        let mut lease_created = false;
        if let Some(existing) = record
            .active_attaches
            .iter_mut()
            .find(|lease| lease.attach_id == request.attach_id)
        {
            existing.role = request.role.clone();
            existing.session_id = request.session_id.clone();
            existing.last_seen_at = now.clone();
        } else {
            let has_live_conflict = record
                .active_attaches
                .iter()
                .any(|lease| !registry::is_principal_attach_stale(lease, now_dt));
            if has_live_conflict && !request.takeover {
                self.control_plane.emit_failed(
                    "/_appfs/principals/attach_principal.act",
                    request_id,
                    "PRINCIPAL_ATTACH_CONFLICT",
                    &format!(
                        "principal {} already has an active attach",
                        request.principal_id
                    ),
                    client_token,
                )?;
                return Ok(());
            }
            if request.takeover || !record.active_attaches.is_empty() {
                record.active_attaches.clear();
                if let Some(status) = &mut record.agent_status {
                    status.state = registry::PrincipalAgentState::Unknown;
                    status.attach_id = None;
                    status.session_id = None;
                    status.current_task_preview = None;
                    status.current_task_source = None;
                    status.turn_id = None;
                    status.updated_at = now.clone();
                }
            }
            record.active_attaches.push(registry::PrincipalAttachLease {
                attach_id: request.attach_id.clone(),
                role: request.role.clone(),
                session_id: request.session_id.clone(),
                attached_at: now.clone(),
                last_seen_at: now.clone(),
            });
            lease_created = true;
        }
        record.active_attach_count = record.active_attaches.len() as u32;
        record.updated_at = now;
        let updated = record.clone();
        registry::write_principal_registry(&self.root, &doc)?;
        registry::write_principal_record_view(&self.root, &updated)?;
        let materialized = self.materialize_private_apps_for_principal(&updated)?;
        self.control_plane.emit_completed(
            "/_appfs/principals/attach_principal.act",
            request_id,
            serde_json::json!({
                "principal_event": if lease_created { "principal.attached" } else { "principal.attach_refreshed" },
                "principal_id": updated.principal_id,
                "attach_id": request.attach_id,
                "attached": true,
                "lease_created": lease_created,
                "takeover": request.takeover,
                "active_attach_count": updated.active_attach_count,
                "app_instances": materialized,
            }),
            client_token,
        )?;
        Ok(())
    }

    fn handle_detach_principal(
        &mut self,
        request_id: &str,
        client_token: Option<String>,
        request: action_dispatcher::DetachPrincipalRequest,
    ) -> Result<()> {
        let mut doc = self.load_principal_registry()?;
        let Some(record) = doc
            .principals
            .iter_mut()
            .find(|principal| principal.principal_id == request.principal_id)
        else {
            self.control_plane.emit_failed(
                "/_appfs/principals/detach_principal.act",
                request_id,
                "PRINCIPAL_NOT_FOUND",
                &format!("principal {} is not registered", request.principal_id),
                client_token,
            )?;
            return Ok(());
        };

        let before = record.active_attaches.len();
        record
            .active_attaches
            .retain(|lease| lease.attach_id != request.attach_id);
        let detached = record.active_attaches.len() != before;
        if detached
            && (record.active_attaches.is_empty()
                || record
                    .agent_status
                    .as_ref()
                    .and_then(|status| status.attach_id.as_deref())
                    == Some(request.attach_id.as_str()))
        {
            let now = chrono::Utc::now().to_rfc3339();
            if let Some(status) = &mut record.agent_status {
                status.state = registry::PrincipalAgentState::Stopped;
                status.current_task_preview = None;
                status.current_task_source = None;
                status.turn_id = None;
                status.updated_at = now;
            }
        }
        record.active_attach_count = record.active_attaches.len() as u32;
        record.updated_at = chrono::Utc::now().to_rfc3339();
        let updated = record.clone();
        registry::write_principal_registry(&self.root, &doc)?;
        registry::write_principal_record_view(&self.root, &updated)?;
        self.control_plane.emit_completed(
            "/_appfs/principals/detach_principal.act",
            request_id,
            serde_json::json!({
                "principal_event": if detached { "principal.detached" } else { "principal.detach_ignored" },
                "principal_id": updated.principal_id,
                "attach_id": request.attach_id,
                "detached": detached,
                "reason": request.reason,
                "active_attach_count": updated.active_attach_count,
            }),
            client_token,
        )?;
        Ok(())
    }

    fn load_principal_registry(&self) -> Result<registry::PrincipalRegistryDoc> {
        Ok(registry::read_principal_registry(&self.root)?.unwrap_or(
            registry::PrincipalRegistryDoc {
                version: registry::APPFS_REGISTRY_VERSION,
                default_principal_id: registry::APPFS_DEFAULT_PRINCIPAL_ID.to_string(),
                principals: Vec::new(),
            },
        ))
    }

    fn materialize_private_apps_for_principal(
        &mut self,
        principal: &registry::PrincipalRecord,
    ) -> Result<Vec<serde_json::Value>> {
        let Some(policy_doc) = registry::read_app_policy_registry(&self.root)? else {
            return Ok(Vec::new());
        };
        let mut materialized = Vec::new();
        for policy in policy_doc
            .apps
            .iter()
            .filter(|policy| policy.visibility == registry::AppfsAppPolicyVisibility::Private)
        {
            let instance_id = format!("{}--{}", policy.app_id, principal.principal_id);
            if self.runtimes.contains_key(&instance_id) {
                continue;
            }
            let path_template = policy.path_template.as_deref().ok_or_else(|| {
                anyhow::anyhow!("private app policy {} missing path_template", policy.app_id)
            })?;
            let path = render_principal_template(path_template, &principal.principal_id);
            let profile_id = policy
                .profile_template
                .as_deref()
                .map(|template| render_principal_template(template, &principal.principal_id))
                .unwrap_or_else(|| format!("{}:{}", policy.app_id, principal.principal_id));
            let mut bridge = registry::bridge_args_from_transport_doc(&policy.transport);
            bridge.connector_config = policy.connector_config.clone();
            let runtime = ResolvedAppfsRuntimeCliArgs {
                app_id: policy.app_id.clone(),
                session_id: super::normalize_appfs_session_id(None),
                bridge,
            };
            let metadata = AppRuntimeRegistryMetadata {
                instance_id: instance_id.clone(),
                visibility: registry::AppfsRegisteredAppVisibility::PrivateInstance,
                parent_app_id: Some(policy.app_id.clone()),
                principal_id: Some(principal.principal_id.clone()),
                profile_id: Some(profile_id.clone()),
                path: path.clone(),
                inbound_poll_ms: policy.inbound_poll_ms.unwrap_or(0),
                connector_config: policy.connector_config.clone(),
            };
            let mut entry = build_runtime_entry_with_metadata(&self.root, runtime, metadata, None)?;
            entry.adapter.prepare_action_sinks()?;
            self.runtimes.insert(instance_id.clone(), entry);
            materialized.push(serde_json::json!({
                "instance_id": instance_id,
                "app_id": policy.app_id,
                "principal_id": principal.principal_id,
                "profile_id": profile_id,
                "path": path,
            }));
        }
        if !materialized.is_empty() {
            self.sync_registry_to_disk(None)?;
        }
        Ok(materialized)
    }
}

fn runtime_metadata_key(app_id: &str, session_id: &str) -> String {
    format!("{app_id}\u{0}{session_id}")
}

fn render_principal_template(template: &str, principal_id: &str) -> String {
    template.replace("{principal_id}", principal_id)
}

fn current_active_attach(
    record: &registry::PrincipalRecord,
) -> Option<&registry::PrincipalAttachLease> {
    record.active_attaches.first()
}

fn apply_agent_status_patch(
    record: &mut registry::PrincipalRecord,
    attach_id: &str,
    patch: action_dispatcher::PrincipalAgentStatusPatch,
) {
    let now = chrono::Utc::now().to_rfc3339();
    let active_session_id =
        current_active_attach(record).and_then(|lease| lease.session_id.clone());
    let status = record
        .agent_status
        .get_or_insert_with(|| registry::PrincipalAgentStatus {
            state: registry::PrincipalAgentState::Unknown,
            current_task_preview: None,
            current_task_source: None,
            turn_id: None,
            attach_id: Some(attach_id.to_string()),
            session_id: active_session_id.clone(),
            model: None,
            updated_at: now.clone(),
            last_activity_at: None,
            last_outcome: None,
        });

    if let Some(state) = patch.state {
        status.state = state;
    }
    apply_nullable_patch(&mut status.current_task_preview, patch.current_task_preview);
    apply_nullable_patch(&mut status.current_task_source, patch.current_task_source);
    apply_nullable_patch(&mut status.turn_id, patch.turn_id);
    apply_nullable_patch(&mut status.session_id, patch.session_id);
    apply_nullable_patch(&mut status.model, patch.model);
    apply_nullable_patch(&mut status.last_outcome, patch.last_outcome);
    status.attach_id = Some(attach_id.to_string());
    if status.session_id.is_none() {
        status.session_id = active_session_id;
    }
    status.updated_at = now.clone();
    if matches!(
        status.state,
        registry::PrincipalAgentState::Running
            | registry::PrincipalAgentState::Stopping
            | registry::PrincipalAgentState::Error
    ) || status.last_outcome.is_some()
    {
        status.last_activity_at = Some(now);
    }
}

fn apply_nullable_patch<T>(
    target: &mut Option<T>,
    patch: Option<action_dispatcher::NullablePatch<T>>,
) {
    match patch {
        Some(action_dispatcher::NullablePatch::Clear) => *target = None,
        Some(action_dispatcher::NullablePatch::Set(value)) => *target = Some(value),
        None => {}
    }
}
