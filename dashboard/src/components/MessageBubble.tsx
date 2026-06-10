import React from 'react';
import type { AppfsEventRecord, TimelineEntry, ConversationMessage, DebugDumpRecord } from '../types';
import { getAgentColor } from '../types';
import { CollapsibleBlock } from './CollapsibleBlock';
import { cachedInputTokens, effectiveInputTokens } from '../token-usage';

interface Props { entry: TimelineEntry; agentColorIndex: number; compact?: boolean; }

export function MessageBubble({ entry, agentColorIndex, compact }: Props) {
  const color = getAgentColor(agentColorIndex);
  const isDebugDump = entry.source === 'debug-dump';
  const isCompactionArchive = entry.source === 'compaction-archive';
  const roleClass = isDebugDump ? 'debug-dump' : isCompactionArchive ? 'compaction-archive' : entry.role;
  const effectiveInput = effectiveInputTokens(entry.usage);
  const cachedInput = cachedInputTokens(entry.usage);

  return (
    <div className={`msg ${roleClass} ${compact ? 'msg-compact' : ''}`}>
      <div className="msg-role">
        {!compact && (
          <span className="msg-agent-tag" style={{ background: `${color}33`, color }}>
            {entry.agentName}
          </span>
        )}
        {isDebugDump ? '[debug-dump] MessageRequest' : isCompactionArchive ? `[archived] ${entry.role}` : entry.role}
      </div>
      <div className="msg-content">
        {isDebugDump ? renderDebugDump(entry) : renderMessage(entry)}
      </div>
      {entry.usage && (
        <div className="msg-tokens">
          input: {effectiveInput.toLocaleString()}
          {cachedInput > 0 && ` (cached ${cachedInput.toLocaleString()})`}
          {' | '}
          output: {entry.usage.output_tokens.toLocaleString()}
        </div>
      )}
    </div>
  );
}

function renderMessage(entry: TimelineEntry): React.ReactNode {
  return (
    <>
      {renderBlocks(entry)}
      {entry.appfsEvents && entry.appfsEvents.length > 0 && renderAppfsEvents(entry.appfsEvents)}
    </>
  );
}

function renderAppfsEvents(events: AppfsEventRecord[]): React.ReactNode {
  const label = `AppFS events (${events.length}): ${events.map(formatEventLabel).join(', ')}`;
  return (
    <CollapsibleBlock label={label}>
      <div className="appfs-events-list">
        {events.map(event => (
          <div key={event.id} className="appfs-event-row">
            <div className="event-line">
              <span className="event-type">{event.eventType}</span>
              {event.fromAgent && event.toAgent && (
                <span className="event-route">{event.fromAgent} {'->'} {event.toAgent}</span>
              )}
              {event.seq !== undefined && <span className="event-pill">seq {event.seq}</span>}
              {event.app && <span className="event-pill">{event.app}</span>}
            </div>
            {event.text && <div className="event-text">{event.text}</div>}
            <div className="event-raw">{event.rawLine}</div>
          </div>
        ))}
      </div>
    </CollapsibleBlock>
  );
}

function renderBlocks(entry: TimelineEntry): React.ReactNode {
  const msg = entry.raw as ConversationMessage;
  return msg.blocks.map((block, i) => {
    if (block.type === 'text' && block.text) {
      const { visibleText, reminderText } = splitAppfsReminder(block.text, entry.appfsEvents);
      return (
        <React.Fragment key={i}>
          {visibleText && <div>{visibleText}</div>}
          {reminderText && (
            <CollapsibleBlock label={`Show source reminder (${reminderText.length.toLocaleString()} chars)`}>
              {reminderText}
            </CollapsibleBlock>
          )}
        </React.Fragment>
      );
    }
    if (block.type === 'input_router') {
      return null;
    }
    if (block.type === 'tool_use') {
      const inputLen = (block.input ?? '').length;
      let preview = (block.input ?? '').replace(/\n/g, ' ');
      if (preview.length > 80) preview = preview.slice(0, 80) + '…';
      return (
        <div key={i}>
          <CollapsibleBlock label={`${block.name}(${preview}) — ${inputLen.toLocaleString()} chars`}>
            {block.input ?? ''}
          </CollapsibleBlock>
        </div>
      );
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

function splitAppfsReminder(
  text: string,
  events: AppfsEventRecord[] | undefined,
): { visibleText: string; reminderText?: string } {
  if (!events || events.length === 0) {
    return { visibleText: text };
  }

  const reminders: string[] = [];
  let visibleText = text.replace(/<system-reminder>[\s\S]*?<\/system-reminder>/g, (block) => {
    if (isAppfsReminder(block)) {
      reminders.push(block);
      return '';
    }
    return block;
  }).trim();

  if (reminders.length === 0 && isAppfsReminder(text)) {
    reminders.push(text);
    visibleText = '';
  }

  return {
    visibleText,
    reminderText: reminders.length > 0 ? reminders.join('\n\n') : undefined,
  };
}

function isAppfsReminder(text: string): boolean {
  return text.includes('[appfs_event]') ||
    text.includes('[agent_message]') ||
    text.includes('New AppFS events') ||
    text.includes('New routed inputs') ||
    text.includes('来源：');
}

function formatEventLabel(event: AppfsEventRecord): string {
  return `${event.eventType}${event.seq !== undefined ? ` #${event.seq}` : ''}`;
}

function renderDebugDump(entry: TimelineEntry): React.ReactNode {
  const dump = entry.raw as DebugDumpRecord;
  const systemPrompt = dump.system ?? dump.system_prompt ?? '';
  const toolCount = dump.tools?.length ?? dump.tools_count ?? 0;
  const msgCount = dump.messages?.length ?? dump.message_count ?? 0;
  const sysLen = systemPrompt.length;
  return (
    <div>
      <div style={{ fontSize: 12, color: '#bc8cff', marginBottom: 4 }}>
        model: {dump.model || '?'} | max_tokens: {dump.max_tokens} | messages: {msgCount} | tools: {toolCount}
        {dump.reasoning_effort ? ` | effort: ${dump.reasoning_effort}` : ''}
      </div>
      {systemPrompt && (
        <CollapsibleBlock label={`Show system prompt (${sysLen.toLocaleString()} chars)`}>
          {systemPrompt}
        </CollapsibleBlock>
      )}
      {dump.tools && dump.tools.length > 0 && (
        <CollapsibleBlock label={`Show tools (${dump.tools.length} definitions)`}>
          {dump.tools.map(t => t.name).join('\n')}
        </CollapsibleBlock>
      )}
    </div>
  );
}
