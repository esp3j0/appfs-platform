export interface AgentInfo {
  name: string;
  principalId: string;
  sessionId: string;
  model: string;
  pid: number;
  startedAt: number;
  sessionJsonlPath: string;
  status: 'online' | 'offline';
  messageCount: number;
  totalInputTokens: number;
  totalOutputTokens: number;
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
    cache_creation_input_tokens: number;
    cache_read_input_tokens: number;
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
  agentName: string;
  timestamp: number;
  source: 'session' | 'debug-dump' | 'compaction-archive';
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string;
  raw: ConversationMessage | DebugDumpRecord;
  usage?: {
    input_tokens: number;
    output_tokens: number;
    cache_creation_input_tokens: number;
    cache_read_input_tokens: number;
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
