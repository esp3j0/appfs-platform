import React from 'react';
import type { TimelineEntry, ContentBlock, ConversationMessage, DebugDumpRecord } from '../types';
import { getAgentColor } from '../types';
import { CollapsibleBlock } from './CollapsibleBlock';

interface Props { entry: TimelineEntry; agentColorIndex: number; }

export function MessageBubble({ entry, agentColorIndex }: Props) {
  const color = getAgentColor(agentColorIndex);
  const isDebugDump = entry.source === 'debug-dump';
  const roleClass = isDebugDump ? 'debug-dump' : entry.role;

  return (
    <div className={`msg ${roleClass}`}>
      <div className="msg-role">
        <span className="msg-agent-tag" style={{ background: `${color}33`, color }}>
          {entry.agentName}
        </span>
        {isDebugDump ? '[debug-dump] MessageRequest' : entry.role}
      </div>
      <div className="msg-content">
        {isDebugDump ? renderDebugDump(entry) : renderBlocks(entry)}
      </div>
      {entry.usage && (
        <div className="msg-tokens">
          input: {entry.usage.input_tokens.toLocaleString()} | output: {entry.usage.output_tokens.toLocaleString()}
          {entry.usage.cache_read_input_tokens > 0 && ` | cache_read: ${entry.usage.cache_read_input_tokens.toLocaleString()}`}
        </div>
      )}
    </div>
  );
}

function renderBlocks(entry: TimelineEntry): React.ReactNode {
  const msg = entry.raw as ConversationMessage;
  return msg.blocks.map((block, i) => {
    if (block.type === 'text' && block.text) {
      return <div key={i}>{block.text}</div>;
    }
    if (block.type === 'tool_use') {
      return <div key={i}>{block.name}({(block.input ?? '').length > 200 ? (block.input ?? '').slice(0, 200) + '...' : block.input})</div>;
    }
    if (block.type === 'tool_result') {
      return (
        <div key={i}>
          <CollapsibleBlock label={`Show output (${(block.output ?? '').length.toLocaleString()} chars)`}>
            {block.output ?? ''}
          </CollapsibleBlock>
        </div>
      );
    }
    return null;
  });
}

function renderDebugDump(entry: TimelineEntry): React.ReactNode {
  const dump = entry.raw as DebugDumpRecord;
  return (
    <div>
      <div style={{ fontSize: 12, color: '#bc8cff', marginBottom: 4 }}>
        model: {dump.model ?? 'unknown'} | max_tokens: {dump.max_tokens ?? '?'}
      </div>
      {dump.system_prompt && (
        <CollapsibleBlock label={`Show system prompt (${dump.system_prompt_length ?? dump.system_prompt.length} chars)`}>
          {dump.system_prompt}
        </CollapsibleBlock>
      )}
      {dump.tools && dump.tools.length > 0 && (
        <CollapsibleBlock label={`Show tools (${dump.tools.length} definitions)`}>
          {JSON.stringify(dump.tools, null, 2)}
        </CollapsibleBlock>
      )}
    </div>
  );
}
