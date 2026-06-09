// ── Session JSONL record types ──

export interface SessionMetaRecord {
  type: 'session_meta';
  version: number;
  session_id: string;
  created_at_ms: number;
  updated_at_ms: number;
  workspace_root?: string;
  appfs_principal_id?: string;
  model?: string;
  invoked_skills?: unknown[];
  appfs_event_cursors?: Record<string, unknown>;
  appfs_wake_event_cursors?: Record<string, unknown>;
}

export interface MessageRecord {
  type: 'message';
  message: ConversationMessage;
}

export type JsonlRecord = SessionMetaRecord | MessageRecord | TurnErrorRecord | { type: string };

// ── Conversation message model ──

export interface ConversationMessage {
  uuid: string;
  role: 'system' | 'user' | 'assistant' | 'tool';
  blocks: ContentBlock[];
  usage?: TokenUsage;
  subtype?: string;
  compact_metadata?: unknown;
  attachment_metadata?: AttachmentMetadata;
  hook_result_metadata?: unknown;
  is_compact_summary?: boolean;
  is_visible_in_transcript_only?: boolean;
  timestamp_ms?: number;
}

export type ContentBlock =
  | { type: 'text'; text: string }
  | { type: 'input_router'; inputs: InputRouterBlockInput[] }
  | { type: 'tool_use'; id: string; name: string; input: string }
  | { type: 'tool_result'; tool_use_id: string; tool_name: string; output: string; is_error: boolean };

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

export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  cache_creation_input_tokens?: number;
  cache_read_input_tokens?: number;
}

export interface AttachmentMetadata {
  kind: string;
}

// ── AppFS/input-router event records extracted from user attachments ──

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

// ── Debug-dump record (from agent debug-dump feature) ──

export interface DebugDumpRecord {
  type: 'message_request';
  timestamp_ms: number;
  request_index: number;
  model: string;
  max_tokens: number;
  system?: string;
  system_prompt?: string;
  system_prompt_length?: number;
  message_count?: number;
  messages: { role: string; content: unknown[] }[];
  tools_count?: number;
  tools?: { name: string; description?: string; input_schema?: unknown }[];
  stream: boolean;
  reasoning_effort?: string | null;
}

export interface TurnErrorRecord {
  type: 'turn_error';
  timestamp_ms: number;
  request_id?: string;
  turn_id?: string;
  session_id?: string;
  source?: string;
  message: string;
}

// ── Compaction archive records (from debug-dump feature) ──

export interface CompactionBoundaryRecord {
  type: 'compaction_boundary';
  timestamp_ms: number;
  compaction_count: number;
  archived_message_count: number;
}

export interface CompactionArchiveRecord {
  type: 'compaction_archive';
  timestamp_ms: number;
  compaction_count?: number;
  /** The archived message in the same format as session JSONL messages */
  message: ConversationMessage;
}

// ── Agent discovery ──

export interface AgentMeta {
  agent_name: string;
  principal_id: string;
  session_id: string;
  model: string;
  pid: number;
  started_at_ms: number;
  session_jsonl_path: string;
  modelProviderId?: string;
  modelId?: string;
  contextWindowTokens?: number;
  maxOutputTokens?: number;
  runtimeModelConfigPath?: string;
}

// ── Dashboard API types ──

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

export interface TimelineEntry {
  id: string;
  sessionId?: string;
  agentName: string;
  timestamp: number;
  source: 'session' | 'debug-dump' | 'compaction-archive';
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string;
  raw: ConversationMessage | DebugDumpRecord;
  usage?: TokenUsage;
  appfsEvents?: AppfsEventRecord[];
}

export interface CrossAgentInteraction {
  entryId: string;
  fromAgent: string;
  toAgent: string;
  eventType: 'message.sent' | 'message.received' | 'message.read';
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
