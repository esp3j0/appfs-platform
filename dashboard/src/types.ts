export interface AgentInfo {
  name: string;
  principalId: string;
  sessionId: string;
  workspaceFingerprint?: string;
  model: string;
  pid: number;
  startedAt: number;
  sessionJsonlPath: string;
  status: 'online' | 'offline';
  controlMode: 'managed' | 'external';
  messageCount: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  currentContextTokens?: number;
  projectId?: string;
  projectRoot?: string;
  modelProviderId?: string;
  modelId?: string;
  contextWindowTokens?: number;
  maxOutputTokens?: number;
  runtimeModelConfigPath?: string;
  archived?: boolean;
  archivedAt?: number;
  archivedReason?: string;
}

export interface ProjectRecord {
  projectId: string;
  projectRoot: string;
  composeFilePath: string;
  mountRoot: string;
  status: 'stopped' | 'starting' | 'running' | 'error';
  agentSessionIds: string[];
  managedAgentSessionIds: string[];
}

export interface ContentBlock {
  type: string;
  text?: string;
  inputs?: InputRouterBlockInput[];
  id?: string;
  name?: string;
  input?: string;
  tool_use_id?: string;
  tool_name?: string;
  output?: string;
  is_error?: boolean;
}

export interface InputRouterBlockInput {
  source: string;
  input_type: string;
  text: string;
  event_id?: string;
  ts?: string;
  client_token?: string;
  event_path?: string;
  principal_id?: string;
  app_id?: string;
  stream_id?: string;
  seq?: number;
  correlation_id?: string;
  requires_attention?: boolean;
  delivery?: string;
  payload?: unknown;
  raw_event?: unknown;
  event_render_metadata?: unknown;
}

export interface ConversationMessage {
  uuid: string;
  role: 'system' | 'user' | 'assistant' | 'tool';
  blocks: ContentBlock[];
  usage?: {
    input_tokens: number;
    output_tokens: number;
    cache_creation_input_tokens?: number;
    cache_read_input_tokens?: number;
  };
  attachment_metadata?: { kind: string };
  is_compact_summary?: boolean;
}

export interface AppfsEventRecord {
  id: string;
  parentMessageUuid: string;
  source: string;
  eventType: string;
  principal?: string;
  fromAgent?: string;
  toAgent?: string;
  app?: string;
  stream?: string;
  seq?: number;
  correlationId?: string;
  contactKey?: string;
  text?: string;
  rawLine: string;
}

export interface DebugDumpRecord {
  type: 'message_request';
  timestamp_ms: number;
  request_index: number;
  model: string;
  max_tokens: number;
  /** The system prompt sent to the API (field name matches MessageRequest) */
  system?: string;
  /** Legacy debug-dump field from early dashboard fixtures */
  system_prompt?: string;
  system_prompt_length?: number;
  message_count?: number;
  /** Messages sent to the API — InputMessage[] from the api crate */
  messages: { role: string; content: unknown[] }[];
  /** Tool definitions sent to the API */
  tools?: { name: string; description?: string; input_schema: unknown }[];
  tools_count?: number;
  stream: boolean;
  reasoning_effort?: string | null;
}

export interface TimelineEntry {
  id: string;
  sessionId?: string;
  agentName: string;
  timestamp: number;
  source: 'session' | 'debug-dump' | 'compaction-archive';
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string;
  raw: ConversationMessage | DebugDumpRecord;
  usage?: {
    input_tokens: number;
    output_tokens: number;
    cache_creation_input_tokens?: number;
    cache_read_input_tokens?: number;
  };
  appfsEvents?: AppfsEventRecord[];
}

export interface CrossAgentInteraction {
  entryId: string;
  fromAgent: string;
  toAgent: string;
  eventType: string;
  timestamp: number;
  seq?: number;
  label: string;
}

export interface TimelineCompactionBoundary {
  agentName: string;
  timestamp: number;
  compactionCount: number;
  archivedMessageCount: number;
}

export interface TimelineResponse {
  entries: TimelineEntry[];
  interactions: CrossAgentInteraction[];
  compactionBoundaries: TimelineCompactionBoundary[];
}

export interface ChatThread {
  sessionId: string;
  items: ChatItem[];
}

export type ChatItem = ChatMessageItem | ChatToolItem | ChatErrorItem;

export interface ChatMessageItem {
  kind: 'message';
  id: string;
  role: 'user' | 'assistant';
  text: string;
  timestamp: number;
  usage?: {
    input_tokens: number;
    output_tokens: number;
    cache_creation_input_tokens?: number;
    cache_read_input_tokens?: number;
  };
}

export interface ChatToolItem {
  kind: 'tool';
  id: string;
  toolCallId?: string;
  toolName: string;
  status: 'pending' | 'completed' | 'error';
  summary?: string;
  isError?: boolean;
  timestamp: number;
}

export interface ChatErrorItem {
  kind: 'error';
  id: string;
  text: string;
  timestamp: number;
  requestId?: string;
  turnId?: string;
}

export type AgentLaunchSpec =
  | {
      kind: 'cargo';
      manifestPath: string;
      targetDir?: string;
      package: string;
      features?: string[];
    }
  | {
      kind: 'binary';
      binaryPath: string;
    };

export interface SpawnConfig {
  cwd: string;
  principalId: string;
  model: string;
  permissionMode: string;
  appfsMountRoot: string;
  launchSpec: AgentLaunchSpec;
  env: Record<string, string>;
  appfsIdleWake?: boolean;
  sessionPath?: string;
  projectId?: string;
  projectRoot?: string;
  modelProviderId?: string;
  modelId?: string;
  contextWindowTokens?: number;
  maxOutputTokens?: number;
  runtimeModelConfigPath?: string;
}

export interface PrincipalLifecycleInfo {
  principal_id: string;
  display_name?: string;
  description?: string | null;
  kind?: string;
  created_at?: string;
  updated_at?: string;
  presence?: string;
  active_attach_count?: number;
  active_attaches?: Array<{
    attach_id?: string;
    last_seen_at?: string;
    [key: string]: unknown;
  }>;
  online: boolean;
  status: string;
  agent_status?: {
    state?: string;
    current_task_preview?: string | null;
    session_id?: string | null;
    [key: string]: unknown;
  } | null;
  pid?: number;
  sessionId?: string | null;
  model?: string;
  permissionMode?: string;
}

export interface PrincipalListResponse {
  version: number;
  default_principal_id?: string;
  principals: PrincipalLifecycleInfo[];
}

export interface CreatePrincipalRequest {
  principalId: string;
  displayName?: string;
  description?: string | null;
  kind?: string;
}

export interface PrincipalStartRequest {
  model?: string;
  modelProviderId?: string;
  modelId?: string;
  contextWindowTokens?: number;
  maxOutputTokens?: number;
  permissionMode?: string;
}

export interface PrincipalResumeRequest extends PrincipalStartRequest {
  sessionId?: string;
}

export type ModelProviderType = 'anthropic' | 'openai' | 'xai';

export interface ModelCredentialConfig {
  mode: 'env';
  apiKeyEnv?: string;
  authTokenEnv?: string;
}

export interface ModelCatalogEntry {
  id: string;
  name: string;
  displayName?: string;
  contextWindowTokens: number;
  maxOutputTokens: number;
}

export interface ModelProviderConfig {
  id: string;
  providerName: string;
  type: ModelProviderType;
  baseUrl?: string;
  credential: ModelCredentialConfig;
  models: ModelCatalogEntry[];
}

export interface DashboardModelConfig {
  version: 1;
  defaultProviderId: string;
  defaultModelId: string;
  providers: ModelProviderConfig[];
}

export interface ModelConfigResponse {
  config: DashboardModelConfig;
  path: string;
}

export interface AppEventRenderScopeOverride {
  events: Record<string, unknown>;
}

export interface AppEventRenderOverridesDoc {
  version: number;
  streams?: Record<string, AppEventRenderScopeOverride>;
  apps?: Record<string, AppEventRenderScopeOverride>;
  platform?: AppEventRenderScopeOverride;
  discoveredApps?: Record<string, {
    appId: string;
    principalId: string;
    events: Record<string, unknown>;
  }>;
}

export const AGENT_COLORS = ['#58a6ff', '#3fb950', '#d2a8ff', '#d29922', '#39d2c0', '#f778ba'] as const;

export function getAgentColor(index: number): string {
  return AGENT_COLORS[index % AGENT_COLORS.length];
}
