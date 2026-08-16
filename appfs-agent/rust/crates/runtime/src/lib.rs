mod appfs;
mod bash;
pub mod bash_validation;
mod bootstrap;
pub mod branch_lock;
mod compact;
mod config;
pub mod config_validate;
mod context;
mod conversation;
mod execution_tasks;
mod file_ops;
mod git_context;
pub mod green_contract;
mod hooks;
mod input_router;
mod json;
mod lane_events;
pub mod lsp_client;
mod mcp;
mod mcp_client;
pub mod mcp_lifecycle_hardened;
pub mod mcp_server;
mod mcp_stdio;
pub mod mcp_tool_bridge;
mod oauth;
pub mod permission_enforcer;
mod permissions;
pub mod plugin_lifecycle;
mod policy_engine;
mod prompt;
pub mod recovery_recipes;
mod remote;
pub mod sandbox;
mod session;
pub mod session_control;
pub use session_control::SessionStore;
mod shell_cwd;
mod sse;
pub mod stale_base;
pub mod stale_branch;
pub mod summary_compression;
pub mod task_board;
pub mod task_packet;
pub mod task_registry;
pub mod team_cron_registry;
mod tool_output;
mod tool_session;
#[cfg(test)]
mod trust_resolver;
mod usage;
mod windows_shell;
pub mod worker_boot;

pub use appfs::{
    attach_appfs_principal, attach_appfs_principal_with_environment,
    auto_mark_read_for_wake_inputs, create_appfs_principal, detach_appfs_principal,
    detect_appfs_environment, ensure_appfs_attach_identity, heartbeat_appfs_principal,
    resolve_appfs_environment, sanitize_appfs_task_preview,
    scan_appfs_attention_events_for_idle_wake, update_appfs_principal_agent_status,
    warmup_appfs_private_apps, AppfsAgentOutcome, AppfsAgentState, AppfsAgentStatusUpdate,
    AppfsAttachEnsureOutcome, AppfsAttachEnsureStatus, AppfsAttachLease, AppfsAttachSource,
    AppfsEnvironment, AppfsIdleWakeScanOutcome, AppfsPrincipalCreateOutcome,
    AppfsPrincipalCreateRequest, AppfsPrincipalCreateStatus, AppfsPrincipalSummary,
    AppfsPrivateAppWarmupOutcome, AppfsPrivateAppWarmupStatus, AppfsRegisteredApp,
    AppfsRegisteredAppVisibility, AppfsRuntimeManifest, AppfsRuntimeManifestCapabilities,
    AppfsRuntimeManifestControlPlane, APPFS_DEFAULT_PRINCIPAL_ID, APPFS_MULTI_AGENT_MODE_SHARED,
    APPFS_PRINCIPAL_ID_ENV, APPFS_RUNTIME_MANIFEST_REL_PATH,
};
pub use bash::{
    decode_command_output, execute_bash, prepare_background_shell_output,
    prepare_shell_command_output, shell_task_output_path, BackgroundShellOutputCapture,
    BashCommandInput, BashCommandOutput, PreparedShellCommandOutput,
};
pub use bootstrap::{BootstrapPhase, BootstrapPlan};
pub use branch_lock::{detect_branch_lock_collisions, BranchLockCollision, BranchLockIntent};
pub use compact::{
    compact_session, estimate_session_tokens, format_compact_summary,
    get_compact_continuation_message, should_compact, CompactionConfig, CompactionResult,
};
pub use config::{
    ConfigEntry, ConfigError, ConfigLoader, ConfigSource, McpConfigCollection,
    McpManagedProxyServerConfig, McpOAuthConfig, McpRemoteServerConfig, McpSdkServerConfig,
    McpServerConfig, McpStdioServerConfig, McpTransport, McpWebSocketServerConfig, OAuthConfig,
    ProviderFallbackConfig, ResolvedPermissionMode, RuntimeConfig, RuntimeFeatureConfig,
    RuntimeHookConfig, RuntimePermissionRuleConfig, RuntimePluginConfig, RuntimeProviderConfig,
    RuntimeProviderKind, ScopedMcpServerConfig, CLAW_SETTINGS_SCHEMA_NAME,
};
pub use config_validate::{
    check_unsupported_format, format_diagnostics, validate_config_file, ConfigDiagnostic,
    DiagnosticKind, ValidationResult,
};
pub use context::{
    analyze_context_usage, ContextCategoryUsage, ContextSectionUsage, ContextUsageReport,
};
pub use conversation::{
    auto_compaction_threshold_from_env, ApiClient, ApiRequest, AssistantEvent,
    AutoCompactionConfig, AutoCompactionEvent, ConversationRuntime, PromptCacheEvent, RuntimeError,
    StaticToolExecutor, ToolContextUpdate, ToolError, ToolExecutionResult, ToolExecutor,
    TurnSummary,
};
pub use execution_tasks::{
    execution_task_output_file, execution_task_snapshot, mark_execution_task_status,
    read_execution_task_output, register_abortable_execution_task, register_child_execution_task,
    stop_execution_task, unregister_execution_task, ExecutionTaskSnapshot, ExecutionTaskStatus,
};
pub use file_ops::{
    edit_file, glob_search, grep_search, read_file, resolve_tool_path,
    resolve_tool_path_allow_missing, write_file, EditFileOutput, GlobSearchOutput, GrepSearchInput,
    GrepSearchOutput, ReadFileOutput, StructuredPatchHunk, TextFilePayload, WriteFileOutput,
};
pub use git_context::{GitCommitEntry, GitContext};
pub use hooks::{
    HookAbortSignal, HookEvent, HookProgressEvent, HookProgressReporter, HookRunResult, HookRunner,
};
pub use input_router::{
    render_event_template_for_target, render_input_router_block, EventTemplateTarget,
    InputEnvelope, InputSource, PendingInput, PendingInputDelivery, SharedPendingInputQueue,
};
pub use lane_events::{
    dedupe_superseded_commit_events, LaneCommitProvenance, LaneEvent, LaneEventBlocker,
    LaneEventName, LaneEventStatus, LaneFailureClass,
};
pub use mcp::{
    mcp_server_signature, mcp_tool_name, mcp_tool_prefix, normalize_name_for_mcp,
    scoped_mcp_config_hash, unwrap_ccr_proxy_url,
};
pub use mcp_client::{
    McpClientAuth, McpClientBootstrap, McpClientTransport, McpManagedProxyTransport,
    McpRemoteTransport, McpSdkTransport, McpStdioTransport,
};
pub use mcp_lifecycle_hardened::{
    McpDegradedReport, McpErrorSurface, McpFailedServer, McpLifecyclePhase, McpLifecycleState,
    McpLifecycleValidator, McpPhaseResult,
};
pub use mcp_server::{McpServer, McpServerSpec, ToolCallHandler, MCP_SERVER_PROTOCOL_VERSION};
pub use mcp_stdio::{
    spawn_mcp_stdio_process, JsonRpcError, JsonRpcId, JsonRpcRequest, JsonRpcResponse,
    ManagedMcpTool, McpDiscoveryFailure, McpInitializeClientInfo, McpInitializeParams,
    McpInitializeResult, McpInitializeServerInfo, McpListResourcesParams, McpListResourcesResult,
    McpListToolsParams, McpListToolsResult, McpReadResourceParams, McpReadResourceResult,
    McpResource, McpResourceContents, McpServerManager, McpServerManagerError, McpStdioProcess,
    McpTool, McpToolCallContent, McpToolCallParams, McpToolCallResult, McpToolDiscoveryReport,
    UnsupportedMcpServer,
};
pub use oauth::{
    clear_oauth_credentials, code_challenge_s256, credentials_path, generate_pkce_pair,
    generate_state, load_oauth_credentials, loopback_redirect_uri, parse_oauth_callback_query,
    parse_oauth_callback_request_target, save_oauth_credentials, OAuthAuthorizationRequest,
    OAuthCallbackParams, OAuthRefreshRequest, OAuthTokenExchangeRequest, OAuthTokenSet,
    PkceChallengeMethod, PkceCodePair,
};
pub use permissions::{
    PermissionContext, PermissionMode, PermissionOutcome, PermissionOverride, PermissionPolicy,
    PermissionPromptDecision, PermissionPrompter, PermissionRequest,
};
pub use plugin_lifecycle::{
    DegradedMode, DiscoveryResult, PluginHealthcheck, PluginLifecycle, PluginLifecycleEvent,
    PluginState, ResourceInfo, ServerHealth, ServerStatus, ToolInfo,
};
pub use policy_engine::{
    evaluate, DiffScope, GreenLevel, LaneBlocker, LaneContext, PolicyAction, PolicyCondition,
    PolicyEngine, PolicyRule, ReconcileReason, ReviewStatus,
};
pub use prompt::{
    load_system_prompt, load_system_prompt_with_appfs, prepend_bullets, ContextFile,
    ProjectContext, PromptBuildError, SystemPromptBuilder, FRONTIER_MODEL_NAME,
    SYSTEM_PROMPT_DYNAMIC_BOUNDARY,
};
pub use recovery_recipes::{
    attempt_recovery, recipe_for, EscalationPolicy, FailureScenario, RecoveryContext,
    RecoveryEvent, RecoveryRecipe, RecoveryResult, RecoveryStep,
};
pub use remote::{
    inherited_upstream_proxy_env, no_proxy_list, read_token, upstream_proxy_ws_url,
    RemoteSessionContext, UpstreamProxyBootstrap, UpstreamProxyState, DEFAULT_REMOTE_BASE_URL,
    DEFAULT_SESSION_TOKEN_PATH, DEFAULT_SYSTEM_CA_BUNDLE, NO_PROXY_HOSTS, UPSTREAM_PROXY_ENV_KEYS,
};
pub use sandbox::{
    build_linux_sandbox_command, detect_container_environment, detect_container_environment_from,
    resolve_sandbox_status, resolve_sandbox_status_for_request, ContainerEnvironment,
    FilesystemIsolationMode, LinuxSandboxCommand, SandboxConfig, SandboxDetectionInputs,
    SandboxRequest, SandboxStatus,
};
pub use session::{
    AttachmentKind, AttachmentMetadata, CompactBoundaryMetadata, CompactPreservedSegment,
    CompactTrigger, ContentBlock, ConversationMessage, HookResultEvent, HookResultMetadata,
    InputRouterBlockInput, InvokedSkill, MessageRole, Session, SessionCompaction, SessionError,
    SessionFork, SessionPromptEntry, SystemMessageSubtype,
};
pub use sse::{IncrementalSseParser, SseEvent};
pub use stale_base::{
    check_base_commit, format_stale_base_warning, read_claw_base_file, resolve_expected_base,
    BaseCommitSource, BaseCommitState,
};
pub use stale_branch::{
    apply_policy, check_freshness, BranchFreshness, StaleBranchAction, StaleBranchEvent,
    StaleBranchPolicy,
};
pub use task_board::{
    TaskBoardClaimOutcome, TaskBoardClaimRejectionReason, TaskBoardPatch, TaskBoardStatus,
    TaskBoardStore, TaskBoardTask, TaskBoardUpdateOutcome,
};
pub use task_packet::{validate_packet, TaskPacket, TaskPacketValidationError, ValidatedPacket};
pub use tool_output::{tool_output_root, tool_result_path, tool_results_dir};
pub use shell_cwd::{
    current_shell_cwd, shell_cwd_tracking_file, update_shell_cwd,
    update_shell_cwd_from_tracking_file, ShellCwdPathFormat,
};
pub use tool_session::{
    current_tool_session_compaction_summary, current_tool_session_id,
    current_tool_session_messages, with_tool_session_snapshot,
};
#[cfg(test)]
pub use trust_resolver::{TrustConfig, TrustDecision, TrustEvent, TrustPolicy, TrustResolver};
pub use usage::{
    format_usd, pricing_for_model, ModelPricing, TokenUsage, UsageCostEstimate, UsageTracker,
};
pub use windows_shell::{bash_shell_path, set_shell_if_windows};
pub use worker_boot::{
    Worker, WorkerEvent, WorkerEventKind, WorkerEventPayload, WorkerFailure, WorkerFailureKind,
    WorkerPromptTarget, WorkerReadySnapshot, WorkerRegistry, WorkerStatus, WorkerTaskReceipt,
    WorkerTrustResolution,
};

#[cfg(windows)]
pub use windows_shell::{posix_path_to_windows_path, windows_path_to_posix_path};

#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
