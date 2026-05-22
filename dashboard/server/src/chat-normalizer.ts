import type {
  ContentBlock,
  MessageRecord,
  TokenUsage,
} from './types.js';
import { isModelContextAttachmentMessage } from './session-message-filters.js';

export interface ChatThread {
  sessionId: string;
  items: ChatItem[];
}

export type ChatItem = ChatMessageItem | ChatToolItem;

export interface ChatMessageItem {
  kind: 'message';
  id: string;
  role: 'user' | 'assistant';
  text: string;
  timestamp: number;
  usage?: TokenUsage;
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

export function normalizeChatThread(
  sessionId: string,
  records: MessageRecord[],
): ChatThread {
  const items: ChatItem[] = [];
  const toolItemsByCallId = new Map<string, ChatToolItem>();

  for (const record of records) {
    const msg = record.message;
    if (msg.role === 'system' || isModelContextAttachmentMessage(msg)) {
      continue;
    }

    const timestamp = msg.timestamp_ms ?? 0;
    const messageText = extractVisibleMessageText(msg.blocks);
    if ((msg.role === 'user' || msg.role === 'assistant') && messageText) {
      items.push({
        kind: 'message',
        id: `${msg.uuid}:message`,
        role: msg.role,
        text: messageText,
        timestamp,
        usage: msg.usage,
      });
    }

    for (const block of msg.blocks) {
      if (block.type === 'tool_use') {
        const toolItem: ChatToolItem = {
          kind: 'tool',
          id: `tool:${block.id}`,
          toolCallId: block.id,
          toolName: block.name,
          status: 'pending',
          summary: summarizeToolUse(block.name),
          timestamp,
        };
        items.push(toolItem);
        toolItemsByCallId.set(block.id, toolItem);
      } else if (block.type === 'tool_result') {
        const existing = toolItemsByCallId.get(block.tool_use_id);
        if (existing) {
          existing.isError = block.is_error;
          existing.status = block.is_error ? 'error' : 'completed';
          existing.summary = summarizeToolResult(existing.toolName, block.is_error);
        } else {
          items.push({
            kind: 'tool',
            id: `tool-result:${block.tool_use_id}`,
            toolCallId: block.tool_use_id,
            toolName: block.tool_name,
            status: block.is_error ? 'error' : 'completed',
            summary: summarizeToolResult(block.tool_name, block.is_error),
            isError: block.is_error,
            timestamp,
          });
        }
      }
    }
  }

  return { sessionId, items };
}

function extractVisibleMessageText(blocks: ContentBlock[]): string {
  return blocks
    .filter((block): block is { type: 'text'; text: string } => block.type === 'text')
    .map(block => block.text.trim())
    .filter(Boolean)
    .filter(text => !text.trim().startsWith('<system-reminder>'))
    .join('\n\n');
}

function summarizeToolUse(toolName: string): string {
  return `${toolName} started`;
}

function summarizeToolResult(
  toolName: string,
  isError: boolean,
): string {
  const status = isError ? 'failed' : 'completed';
  return `${toolName} ${status}`;
}
