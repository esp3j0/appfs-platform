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

export type JsonlRecord = SessionMetaRecord | MessageRecord | { type: string };

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
  | { type: 'tool_use'; id: string; name: string; input: string }
  | { type: 'tool_result'; tool_use_id: string; tool_name: string; output: string; is_error: boolean };

export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  cache_creation_input_tokens: number;
  cache_read_input_tokens: number;
}

export interface AttachmentMetadata {
  kind: string;
}

// ── Debug-dump record (from agent debug-dump feature) ──

export interface DebugDumpRecord {
  type: 'message_request';
  timestamp_ms: number;
  request_index: number;
  model: string;
  max_tokens: number;
  system_prompt: string;
  system_prompt_length: number;
  message_count: number;
  messages: { role: string; content: string }[];
  tools_count: number;
  tools: { name: string; description: string }[];
  stream: boolean;
  reasoning_effort: string | null;
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
}

// ── Dashboard API types ──

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

export interface TimelineEntry {
  agentName: string;
  timestamp: number;
  source: 'session' | 'debug-dump';
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string;
  raw: ConversationMessage | DebugDumpRecord;
  usage?: TokenUsage;
}

export interface CrossAgentInteraction {
  fromAgent: string;
  toAgent: string;
  eventType: 'message.sent' | 'message.received' | 'message.read';
  timestamp: number;
  seq?: number;
  label: string;
}
