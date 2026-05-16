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
  id?: string;
  name?: string;
  input?: string;
  tool_use_id?: string;
  tool_name?: string;
  output?: string;
  is_error?: boolean;
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

export interface TimelineEntry {
  agentName: string;
  timestamp: number;
  source: 'session' | 'debug-dump';
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string;
  raw: ConversationMessage | DebugDumpRecord;
  usage?: {
    input_tokens: number;
    output_tokens: number;
    cache_creation_input_tokens: number;
    cache_read_input_tokens: number;
  };
}

export interface CrossAgentInteraction {
  fromAgent: string;
  toAgent: string;
  eventType: string;
  timestamp: number;
  label: string;
}

export interface TimelineResponse {
  entries: TimelineEntry[];
  interactions: CrossAgentInteraction[];
}

export const AGENT_COLORS = ['#58a6ff', '#3fb950', '#d2a8ff', '#d29922', '#39d2c0', '#f778ba'] as const;

export function getAgentColor(index: number): string {
  return AGENT_COLORS[index % AGENT_COLORS.length];
}
