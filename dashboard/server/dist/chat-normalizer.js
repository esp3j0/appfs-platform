import { isModelContextAttachmentMessage } from './session-message-filters.js';
export function normalizeChatThread(sessionId, records) {
    const items = [];
    const toolItemsByCallId = new Map();
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
                const toolItem = {
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
            }
            else if (block.type === 'tool_result') {
                const existing = toolItemsByCallId.get(block.tool_use_id);
                if (existing) {
                    existing.isError = block.is_error;
                    existing.status = block.is_error ? 'error' : 'completed';
                    existing.summary = summarizeToolResult(existing.toolName, block.is_error);
                }
                else {
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
function extractVisibleMessageText(blocks) {
    return blocks
        .filter((block) => block.type === 'text')
        .map(block => block.text.trim())
        .filter(Boolean)
        .filter(text => !text.trim().startsWith('<system-reminder>'))
        .join('\n\n');
}
function summarizeToolUse(toolName) {
    return `${toolName} started`;
}
function summarizeToolResult(toolName, isError) {
    const status = isError ? 'failed' : 'completed';
    return `${toolName} ${status}`;
}
