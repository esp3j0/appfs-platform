use crate::{
    connector_error_codes, ActionExecutionMode, AppConnector, AppStructureNode,
    AppStructureNodeKind, AppStructureSnapshot, AppStructureSyncResult, AuthStatus,
    ConnectorContext, ConnectorError, ConnectorInfo, ConnectorTransport, FetchLivePageRequest,
    FetchLivePageResponse, FetchSnapshotChunkRequest, FetchSnapshotChunkResponse,
    GetAppStructureRequest, GetAppStructureResponse, HealthStatus, RefreshAppStructureRequest,
    RefreshAppStructureResponse, SnapshotMeta, SubmitActionRequest, SubmitActionResponse,
};
use serde_json::{json, Value as JsonValue};
use std::time::Duration;

const TINODE_APP_ID: &str = "tinode";
const TINODE_CREDENTIAL_POLICY_AUTO_CREATE: &str = "auto-create";
const TINODE_STRUCTURE_REVISION: &str = "tinode-skeleton-v0";

/// Minimal Tinode connector configuration for the AppFS skeleton.
///
/// The server endpoint and login prefix are configuration, not credentials.
/// Tokens, refresh tokens, passwords, cookies, and API keys must stay in the
/// connector private credential store introduced separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TinodeConnectorConfig {
    pub endpoint: String,
    pub credential_policy: String,
    pub login_prefix: String,
}

impl TinodeConnectorConfig {
    pub fn new(
        endpoint: impl Into<String>,
        credential_policy: impl Into<String>,
        login_prefix: impl Into<String>,
    ) -> std::result::Result<Self, ConnectorError> {
        let config = Self {
            endpoint: endpoint.into(),
            credential_policy: credential_policy.into(),
            login_prefix: login_prefix.into(),
        };
        config.validate()
    }

    pub fn from_env() -> std::result::Result<Self, ConnectorError> {
        let endpoint = std::env::var("APPFS_TINODE_ENDPOINT").map_err(|_| {
            connector_err(
                connector_error_codes::INVALID_ARGUMENT,
                "APPFS_TINODE_ENDPOINT is required for the in-process Tinode connector",
                false,
            )
        })?;
        let login_prefix = std::env::var("APPFS_TINODE_LOGIN_PREFIX").map_err(|_| {
            connector_err(
                connector_error_codes::INVALID_ARGUMENT,
                "APPFS_TINODE_LOGIN_PREFIX is required for the in-process Tinode connector",
                false,
            )
        })?;
        let credential_policy = std::env::var("APPFS_TINODE_CREDENTIAL_POLICY")
            .unwrap_or_else(|_| TINODE_CREDENTIAL_POLICY_AUTO_CREATE.to_string());
        Self::new(endpoint, credential_policy, login_prefix)
    }

    fn validate(mut self) -> std::result::Result<Self, ConnectorError> {
        self.endpoint = self.endpoint.trim().trim_end_matches('/').to_string();
        self.credential_policy = self.credential_policy.trim().to_string();
        self.login_prefix = self.login_prefix.trim().to_string();

        if self.endpoint.is_empty()
            || !(self.endpoint.starts_with("http://") || self.endpoint.starts_with("https://"))
        {
            return Err(connector_err(
                connector_error_codes::INVALID_ARGUMENT,
                "Tinode connector endpoint must be an http(s) URL",
                false,
            ));
        }
        if self.credential_policy != TINODE_CREDENTIAL_POLICY_AUTO_CREATE {
            return Err(connector_err(
                connector_error_codes::INVALID_ARGUMENT,
                "Tinode connector v0 supports only credential_policy=auto-create",
                false,
            ));
        }
        if !is_safe_login_prefix(&self.login_prefix) {
            return Err(connector_err(
                connector_error_codes::INVALID_ARGUMENT,
                "Tinode login prefix must contain only ASCII letters, digits, '_' or '-'",
                false,
            ));
        }

        Ok(self)
    }
}

/// Tinode AppFS connector skeleton.
///
/// PR7 intentionally stops at the safe tree contract. It does not connect to
/// Tinode, create accounts, store credentials, or send messages.
pub struct TinodeConnector {
    config: TinodeConnectorConfig,
    credential_create_attempts: u64,
}

impl TinodeConnector {
    pub fn new(config: TinodeConnectorConfig) -> Self {
        Self {
            config,
            credential_create_attempts: 0,
        }
    }

    pub fn from_env() -> std::result::Result<Self, ConnectorError> {
        Ok(Self::new(TinodeConnectorConfig::from_env()?))
    }

    #[must_use]
    pub fn config(&self) -> &TinodeConnectorConfig {
        &self.config
    }

    #[must_use]
    pub fn credential_create_attempts(&self) -> u64 {
        self.credential_create_attempts
    }

    fn snapshot(&self, ctx: &ConnectorContext) -> AppStructureSnapshot {
        AppStructureSnapshot {
            app_id: TINODE_APP_ID.to_string(),
            revision: TINODE_STRUCTURE_REVISION.to_string(),
            active_scope: None,
            ownership_prefixes: vec![
                "_app".to_string(),
                "contacts".to_string(),
                "groups".to_string(),
                "inbox".to_string(),
                "topics".to_string(),
            ],
            nodes: self.structure_nodes(ctx),
        }
    }

    fn structure_nodes(&self, ctx: &ConnectorContext) -> Vec<AppStructureNode> {
        let mut nodes = vec![
            dir("_app"),
            static_json("_app/self.res.json", self.self_resource(ctx)),
            action("_app/ensure_credentials.act"),
            action("_app/refresh_structure.act"),
            action("_app/refresh_inbox.act"),
            dir("contacts"),
            snapshot_jsonl("contacts/index.res.jsonl"),
            action("contacts/send_message.act"),
            action("contacts/resolve.act"),
            snapshot_jsonl("contacts/search_results.res.jsonl"),
            dir("groups"),
            snapshot_jsonl("groups/index.res.jsonl"),
            action("groups/create_group.act"),
            dir("inbox"),
            snapshot_jsonl("inbox/recent.res.jsonl"),
            snapshot_jsonl("inbox/unread.res.jsonl"),
            action("inbox/mark_read.act"),
            dir("topics"),
            snapshot_jsonl("topics/index.res.jsonl"),
        ];
        nodes.sort_by(|a, b| a.path.cmp(&b.path));
        nodes
    }

    fn self_resource(&self, ctx: &ConnectorContext) -> JsonValue {
        let principal_id = ctx.principal_id.as_deref().unwrap_or("default");
        let profile_id = ctx
            .profile_id
            .as_deref()
            .unwrap_or("tinode:unknown-profile");
        json!({
            "app_id": TINODE_APP_ID,
            "principal_id": principal_id,
            "profile_id": profile_id,
            "credential_policy": self.config.credential_policy,
            "credential_status": "missing",
            "tinode_user_id": null,
            "login": null,
            "display_name": principal_id,
            "owner_ref": null,
        })
    }

    fn empty_snapshot_response(
        &self,
        request: FetchSnapshotChunkRequest,
    ) -> std::result::Result<FetchSnapshotChunkResponse, ConnectorError> {
        if !is_tinode_snapshot_resource(&request.resource_path) {
            return Err(connector_err(
                connector_error_codes::NOT_SUPPORTED,
                format!(
                    "unknown Tinode snapshot resource: {}",
                    request.resource_path
                ),
                false,
            ));
        }
        Ok(FetchSnapshotChunkResponse {
            records: Vec::new(),
            emitted_bytes: 0,
            next_cursor: None,
            has_more: false,
            revision: Some(TINODE_STRUCTURE_REVISION.to_string()),
        })
    }
}

impl AppConnector for TinodeConnector {
    fn connector_id(&self) -> std::result::Result<ConnectorInfo, ConnectorError> {
        Ok(ConnectorInfo {
            connector_id: "tinode-in-process-skeleton".to_string(),
            version: "0.1.0-skeleton".to_string(),
            app_id: TINODE_APP_ID.to_string(),
            transport: ConnectorTransport::InProcess,
            supports_snapshot: true,
            supports_live: false,
            supports_action: true,
            optional_features: vec![
                "tinode".to_string(),
                "skeleton_tree".to_string(),
                "credential_policy:auto-create".to_string(),
            ],
        })
    }

    fn health(
        &mut self,
        _ctx: &ConnectorContext,
    ) -> std::result::Result<HealthStatus, ConnectorError> {
        Ok(HealthStatus {
            healthy: true,
            auth_status: AuthStatus::Invalid,
            message: Some(
                "Tinode connector skeleton is configured; credentials are not created yet"
                    .to_string(),
            ),
            checked_at: "2026-05-06T00:00:00Z".to_string(),
        })
    }

    fn prewarm_snapshot_meta(
        &mut self,
        resource_path: &str,
        _timeout: Duration,
        _ctx: &ConnectorContext,
    ) -> std::result::Result<SnapshotMeta, ConnectorError> {
        if !is_tinode_snapshot_resource(resource_path) {
            return Err(connector_err(
                connector_error_codes::NOT_SUPPORTED,
                format!("unknown Tinode snapshot resource: {resource_path}"),
                false,
            ));
        }
        Ok(SnapshotMeta {
            size_bytes: Some(0),
            revision: Some(TINODE_STRUCTURE_REVISION.to_string()),
            last_modified: Some("2026-05-06T00:00:00Z".to_string()),
            item_count: Some(0),
        })
    }

    fn fetch_snapshot_chunk(
        &mut self,
        request: FetchSnapshotChunkRequest,
        _ctx: &ConnectorContext,
    ) -> std::result::Result<FetchSnapshotChunkResponse, ConnectorError> {
        self.empty_snapshot_response(request)
    }

    fn fetch_live_page(
        &mut self,
        _request: FetchLivePageRequest,
        _ctx: &ConnectorContext,
    ) -> std::result::Result<FetchLivePageResponse, ConnectorError> {
        Err(connector_err(
            connector_error_codes::NOT_SUPPORTED,
            "Tinode connector skeleton does not expose live pageable resources",
            false,
        ))
    }

    fn submit_action(
        &mut self,
        request: SubmitActionRequest,
        ctx: &ConnectorContext,
    ) -> std::result::Result<SubmitActionResponse, ConnectorError> {
        if request.path == "/_app/ensure_credentials.act" {
            return Err(connector_err(
                connector_error_codes::PROFILE_NOT_READY,
                "Tinode credential creation is not implemented in the skeleton connector",
                false,
            ));
        }
        if !is_tinode_action(&request.path) {
            return Err(connector_err(
                connector_error_codes::NOT_SUPPORTED,
                format!("unknown Tinode action: {}", request.path),
                false,
            ));
        }
        if !matches!(request.execution_mode, ActionExecutionMode::Inline) {
            return Err(connector_err(
                connector_error_codes::INVALID_ARGUMENT,
                "Tinode skeleton actions must be inline",
                false,
            ));
        }
        Err(connector_err(
            connector_error_codes::PROFILE_NOT_READY,
            format!(
                "Tinode credentials are missing for profile {}",
                ctx.profile_id.as_deref().unwrap_or("<none>")
            ),
            false,
        ))
    }

    fn get_app_structure(
        &mut self,
        request: GetAppStructureRequest,
        ctx: &ConnectorContext,
    ) -> std::result::Result<GetAppStructureResponse, ConnectorError> {
        let snapshot = self.snapshot(ctx);
        if request.known_revision.as_deref() == Some(snapshot.revision.as_str()) {
            return Ok(GetAppStructureResponse {
                result: AppStructureSyncResult::Unchanged {
                    app_id: request.app_id,
                    revision: snapshot.revision,
                    active_scope: snapshot.active_scope,
                },
            });
        }
        Ok(GetAppStructureResponse {
            result: AppStructureSyncResult::Snapshot { snapshot },
        })
    }

    fn refresh_app_structure(
        &mut self,
        request: RefreshAppStructureRequest,
        ctx: &ConnectorContext,
    ) -> std::result::Result<RefreshAppStructureResponse, ConnectorError> {
        let snapshot = self.snapshot(ctx);
        if request.known_revision.as_deref() == Some(snapshot.revision.as_str()) {
            return Ok(RefreshAppStructureResponse {
                result: AppStructureSyncResult::Unchanged {
                    app_id: request.app_id,
                    revision: snapshot.revision,
                    active_scope: snapshot.active_scope,
                },
            });
        }
        Ok(RefreshAppStructureResponse {
            result: AppStructureSyncResult::Snapshot { snapshot },
        })
    }
}

fn dir(path: &str) -> AppStructureNode {
    AppStructureNode {
        path: path.to_string(),
        kind: AppStructureNodeKind::Directory,
        manifest_entry: None,
        seed_content: None,
        mutable: false,
        scope: None,
    }
}

fn action(path: &str) -> AppStructureNode {
    AppStructureNode {
        path: path.to_string(),
        kind: AppStructureNodeKind::ActionFile,
        manifest_entry: Some(json!({
            "template": path,
            "kind": "action",
            "input_mode": "json",
            "execution_mode": "inline",
        })),
        seed_content: None,
        mutable: true,
        scope: None,
    }
}

fn static_json(path: &str, seed_content: JsonValue) -> AppStructureNode {
    AppStructureNode {
        path: path.to_string(),
        kind: AppStructureNodeKind::StaticJsonResource,
        manifest_entry: None,
        seed_content: Some(seed_content),
        mutable: false,
        scope: None,
    }
}

fn snapshot_jsonl(path: &str) -> AppStructureNode {
    AppStructureNode {
        path: path.to_string(),
        kind: AppStructureNodeKind::SnapshotResource,
        manifest_entry: Some(json!({
            "template": path,
            "kind": "resource",
            "output_mode": "jsonl",
            "snapshot": {
                "max_materialized_bytes": 1048576,
                "prewarm": true,
                "prewarm_timeout_ms": 5000,
                "read_through_timeout_ms": 10000,
                "on_timeout": "return_stale"
            }
        })),
        seed_content: None,
        mutable: false,
        scope: None,
    }
}

fn is_tinode_snapshot_resource(path: &str) -> bool {
    let normalized = normalize_path(path);
    matches!(
        normalized.as_str(),
        "contacts/index.res.jsonl"
            | "contacts/search_results.res.jsonl"
            | "groups/index.res.jsonl"
            | "inbox/recent.res.jsonl"
            | "inbox/unread.res.jsonl"
            | "topics/index.res.jsonl"
    )
}

fn is_tinode_action(path: &str) -> bool {
    let normalized = normalize_path(path);
    matches!(
        normalized.as_str(),
        "_app/ensure_credentials.act"
            | "_app/refresh_structure.act"
            | "_app/refresh_inbox.act"
            | "contacts/send_message.act"
            | "contacts/resolve.act"
            | "groups/create_group.act"
            | "inbox/mark_read.act"
    )
}

fn normalize_path(path: &str) -> String {
    path.trim_start_matches('/')
        .replace('\\', "/")
        .trim()
        .to_string()
}

fn is_safe_login_prefix(prefix: &str) -> bool {
    !prefix.is_empty()
        && prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn connector_err(code: &str, message: impl Into<String>, retryable: bool) -> ConnectorError {
    ConnectorError {
        code: code.to_string(),
        message: message.into(),
        retryable,
        details: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{TinodeConnector, TinodeConnectorConfig};
    use crate::{
        connector_error_codes, AppConnector, AppStructureSyncResult, ConnectorContext,
        FetchSnapshotChunkRequest, GetAppStructureRequest, SnapshotResume, SubmitActionRequest,
    };
    use serde_json::json;

    fn config() -> TinodeConnectorConfig {
        TinodeConnectorConfig::new("http://127.0.0.1:6060", "auto-create", "appfs")
            .expect("tinode config")
    }

    fn ctx() -> ConnectorContext {
        ConnectorContext {
            app_id: "tinode".to_string(),
            session_id: "sess-1".to_string(),
            request_id: "req-1".to_string(),
            client_token: None,
            trace_id: None,
            principal_id: Some("default".to_string()),
            profile_id: Some("tinode:default".to_string()),
        }
    }

    #[test]
    fn tinode_config_requires_endpoint_policy_and_safe_prefix() {
        assert!(TinodeConnectorConfig::new("", "auto-create", "appfs").is_err());
        assert!(TinodeConnectorConfig::new("ws://127.0.0.1:6060", "auto-create", "appfs").is_err());
        assert!(TinodeConnectorConfig::new("http://127.0.0.1:6060", "", "appfs").is_err());
        assert!(TinodeConnectorConfig::new("http://127.0.0.1:6060", "manual", "appfs").is_err());
        assert!(
            TinodeConnectorConfig::new("http://127.0.0.1:6060", "auto-create", "bad prefix")
                .is_err()
        );
    }

    #[test]
    fn tinode_structure_exposes_safe_skeleton_without_credentials() {
        let mut connector = TinodeConnector::new(config());
        let response = connector
            .get_app_structure(
                GetAppStructureRequest {
                    app_id: "tinode".to_string(),
                    known_revision: None,
                },
                &ctx(),
            )
            .expect("structure");
        let AppStructureSyncResult::Snapshot { snapshot } = response.result else {
            panic!("expected snapshot");
        };
        assert_eq!(connector.credential_create_attempts(), 0);
        let paths = snapshot
            .nodes
            .iter()
            .map(|node| node.path.as_str())
            .collect::<Vec<_>>();
        for expected in [
            "_app/self.res.json",
            "_app/ensure_credentials.act",
            "_app/refresh_structure.act",
            "_app/refresh_inbox.act",
            "contacts/index.res.jsonl",
            "contacts/send_message.act",
            "contacts/resolve.act",
            "contacts/search_results.res.jsonl",
            "groups/index.res.jsonl",
            "groups/create_group.act",
            "inbox/recent.res.jsonl",
            "inbox/unread.res.jsonl",
            "inbox/mark_read.act",
            "topics/index.res.jsonl",
        ] {
            assert!(paths.contains(&expected), "missing {expected}");
        }
        assert!(!paths.contains(&"_stream/events.evt.jsonl"));
    }

    #[test]
    fn tinode_self_resource_is_safe_and_supports_non_ascii_principal() {
        let connector = TinodeConnector::new(config());
        let mut ctx = ctx();
        ctx.principal_id = Some("张三-agent".to_string());
        ctx.profile_id = Some("tinode:zhangsan-agent".to_string());
        let self_doc = connector.self_resource(&ctx);
        assert_eq!(self_doc["credential_status"], "missing");
        assert_eq!(self_doc["principal_id"], "张三-agent");
        assert_eq!(self_doc["profile_id"], "tinode:zhangsan-agent");
        let rendered = self_doc.to_string();
        for forbidden in ["token", "refresh_token", "password", "secret", "cookie"] {
            assert!(!rendered.contains(forbidden));
        }
    }

    #[test]
    fn tinode_snapshot_resources_are_empty_before_credentials() {
        let mut connector = TinodeConnector::new(config());
        let response = connector
            .fetch_snapshot_chunk(
                FetchSnapshotChunkRequest {
                    resource_path: "/contacts/index.res.jsonl".to_string(),
                    resume: SnapshotResume::Start,
                    budget_bytes: 1024,
                },
                &ctx(),
            )
            .expect("empty snapshot");
        assert!(response.records.is_empty());
        assert!(!response.has_more);
    }

    #[test]
    fn tinode_skeleton_rejects_credential_required_actions() {
        let mut connector = TinodeConnector::new(config());
        let err = connector
            .submit_action(
                SubmitActionRequest {
                    path: "/contacts/send_message.act".to_string(),
                    payload: json!({"to":"basic:zhangsan","text":"hello"}),
                    execution_mode: crate::ActionExecutionMode::Inline,
                },
                &ctx(),
            )
            .expect_err("skeleton must not send");
        assert_eq!(err.code, connector_error_codes::PROFILE_NOT_READY);
        assert_eq!(connector.credential_create_attempts(), 0);
    }
}
