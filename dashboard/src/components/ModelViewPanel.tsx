import React from 'react';
import type { DebugDumpRecord, TimelineCompactionBoundary, TimelineEntry } from '../types';
import { getAgentColor } from '../types';
import { CollapsibleBlock } from './CollapsibleBlock';

type ModelFilter = 'segment-final' | 'all';

interface Props {
  selectedAgents: string[];
  entries: TimelineEntry[];
  compactionBoundaries: TimelineCompactionBoundary[];
}

export function ModelViewPanel({ selectedAgents, entries, compactionBoundaries }: Props) {
  const [modelFilter, setModelFilter] = React.useState<ModelFilter>('segment-final');
  const agentIdxMap = new Map(selectedAgents.map((name, i) => [name, i]));
  const selectedAgentSet = new Set(selectedAgents);
  const dumps = entries
    .filter((entry): entry is TimelineEntry & { raw: DebugDumpRecord } => (
      entry.source === 'debug-dump' &&
      (selectedAgentSet.size === 0 || selectedAgentSet.has(entry.agentName))
    ))
    .sort((a, b) => a.timestamp - b.timestamp);
  const selection = modelFilter === 'segment-final'
    ? selectSegmentFinalDumps(dumps, compactionBoundaries, selectedAgents)
    : { entries: dumps, reasons: new Map<string, string>() };
  const visibleDumps = selection.entries;
  const laneLayout = selectedAgents.length > 1;
  const laneAgents = laneLayout ? selectedAgents : [];
  const boundaryCount = compactionBoundaries.filter(boundary => selectedAgents.includes(boundary.agentName)).length;
  const requestMeta = modelFilter === 'segment-final' && visibleDumps.length !== dumps.length
    ? `${visibleDumps.length} of ${dumps.length} MessageRequests`
    : `${dumps.length} MessageRequests`;
  const lanes = laneAgents.map((agentName, index) => ({
    agentName,
    colorIndex: agentIdxMap.get(agentName) ?? index,
    visibleRequests: visibleDumps.filter(entry => entry.agentName === agentName),
    totalRequests: dumps.filter(entry => entry.agentName === agentName).length,
    compactions: compactionBoundaries.filter(boundary => boundary.agentName === agentName).length,
  }));

  return (
    <div className="model-view">
      <div className="model-view-header">
        <div>
          <h2>Model View</h2>
          <div className="model-view-meta">
            {selectedAgents.length} agents, {requestMeta}
            {boundaryCount > 0 ? `, ${boundaryCount} compactions` : ''}
          </div>
        </div>
        <div className="model-view-controls">
          <button
            className={`filter-btn ${modelFilter === 'segment-final' ? 'active' : ''}`}
            onClick={() => setModelFilter('segment-final')}
          >
            Segment finals
          </button>
          <button
            className={`filter-btn ${modelFilter === 'all' ? 'active' : ''}`}
            onClick={() => setModelFilter('all')}
          >
            All requests
          </button>
        </div>
      </div>

      {dumps.length === 0 ? (
        <div className="model-empty">
          No model requests found. Run the agent with debug-dump enabled to populate this view.
        </div>
      ) : laneLayout ? (
        <div
          className="model-lanes"
          style={{ gridTemplateColumns: `repeat(${Math.max(lanes.length, 1)}, minmax(360px, 1fr))` }}
        >
          {lanes.map(lane => {
            const color = getAgentColor(lane.colorIndex);
            return (
              <section className="model-lane" key={lane.agentName}>
                <div className="model-lane-head" style={{ borderColor: color }}>
                  <div className="model-lane-title">
                    <span className="msg-agent-tag" style={{ background: `${color}33`, color }}>
                      {lane.agentName}
                    </span>
                    <span>{lane.visibleRequests.length} / {lane.totalRequests} requests</span>
                  </div>
                  <div className="model-lane-meta">
                    {lane.compactions > 0 ? `${lane.compactions} compactions` : 'no compaction'}
                  </div>
                </div>
                <div className="model-lane-body">
                  {lane.visibleRequests.length === 0 ? (
                    <div className="model-empty model-empty-lane">
                      No requests for this agent in the current filter.
                    </div>
                  ) : (
                    <div className="model-request-list model-request-list-lane">
                      {lane.visibleRequests.map(entry => (
                        <ModelRequestCard
                          key={entry.id}
                          entry={entry}
                          colorIndex={lane.colorIndex}
                          selectionReason={selection.reasons.get(entry.id)}
                        />
                      ))}
                    </div>
                  )}
                </div>
              </section>
            );
          })}
        </div>
      ) : (
        <div className="model-request-list">
          {visibleDumps.map(entry => (
            <ModelRequestCard
              key={entry.id}
              entry={entry}
              colorIndex={agentIdxMap.get(entry.agentName) ?? 0}
              selectionReason={selection.reasons.get(entry.id)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function ModelRequestCard({
  entry,
  colorIndex,
  selectionReason,
}: {
  entry: TimelineEntry & { raw: DebugDumpRecord };
  colorIndex: number;
  selectionReason?: string;
}) {
  const dump = entry.raw;
  const color = getAgentColor(colorIndex);
  const systemPrompt = dump.system ?? dump.system_prompt ?? '';
  const messages = dump.messages ?? [];
  const tools = dump.tools ?? [];

  return (
    <section className="model-request">
      <div className="model-request-head" style={{ borderColor: color }}>
        <div>
          <div className="model-request-title">
            <span className="msg-agent-tag" style={{ background: `${color}33`, color }}>
              {entry.agentName}
            </span>
            <span>Request #{dump.request_index ?? '?'}</span>
            {selectionReason && <span className="model-request-reason">{selectionReason}</span>}
          </div>
          <div className="model-request-time">{formatTime(entry.timestamp)}</div>
        </div>
        <div className="model-request-stats">
          <span>{dump.model || '?'}</span>
          <span>{messages.length} messages</span>
          <span>{tools.length} tools</span>
          <span>{dump.max_tokens?.toLocaleString?.() ?? dump.max_tokens} max</span>
        </div>
      </div>

      <div className="model-request-body">
        <CollapsibleBlock label={`System (${systemPrompt.length.toLocaleString()} chars)`}>
          {systemPrompt || '(none)'}
        </CollapsibleBlock>

        <div className="model-section-title">Messages</div>
        <div className="model-message-list">
          {messages.map((message, index) => (
            <div className="model-message" key={`${index}-${message.role}`}>
              <div className="model-message-role">
                <span>{index + 1}</span>
                <strong>{message.role}</strong>
              </div>
              <div className="model-message-content">
                {renderContentBlocks(message.content)}
              </div>
            </div>
          ))}
        </div>

        <CollapsibleBlock label={`Tools (${tools.length.toLocaleString()} definitions)`}>
          {tools.length === 0 ? '(none)' : (
            <div className="model-tool-list">
              {tools.map(tool => (
                <div className="model-tool" key={tool.name}>
                  <div className="model-tool-name">{tool.name}</div>
                  {tool.description && <div className="model-tool-description">{tool.description}</div>}
                  {tool.input_schema !== undefined && (
                    <pre>{formatJson(tool.input_schema)}</pre>
                  )}
                </div>
              ))}
            </div>
          )}
        </CollapsibleBlock>
      </div>
    </section>
  );
}

function selectSegmentFinalDumps(
  dumps: Array<TimelineEntry & { raw: DebugDumpRecord }>,
  boundaries: TimelineCompactionBoundary[],
  selectedAgents: string[],
): {
  entries: Array<TimelineEntry & { raw: DebugDumpRecord }>;
  reasons: Map<string, string>;
} {
  const selectedIds = new Set<string>();
  const reasons = new Map<string, string[]>();
  const agents = selectedAgents.length > 0
    ? selectedAgents
    : Array.from(new Set(dumps.map(dump => dump.agentName)));

  for (const agentName of agents) {
    const agentDumps = dumps
      .filter(dump => dump.agentName === agentName)
      .sort((a, b) => a.timestamp - b.timestamp);
    if (agentDumps.length === 0) {
      continue;
    }

    const agentBoundaries = boundaries
      .filter(boundary => boundary.agentName === agentName)
      .sort((a, b) => a.timestamp - b.timestamp);

    for (const boundary of agentBoundaries) {
      const beforeCompaction = findLastRequestAtOrBefore(agentDumps, boundary.timestamp);
      if (beforeCompaction) {
        selectedIds.add(beforeCompaction.id);
        appendReason(
          reasons,
          beforeCompaction.id,
          `before compaction #${boundary.compactionCount}`,
        );
      }
    }

    const latest = agentDumps[agentDumps.length - 1];
    selectedIds.add(latest.id);
    appendReason(reasons, latest.id, 'latest');
  }

  return {
    entries: dumps.filter(dump => selectedIds.has(dump.id)),
    reasons: new Map(Array.from(reasons.entries()).map(([id, values]) => [id, values.join(' / ')])),
  };
}

function findLastRequestAtOrBefore(
  dumps: Array<TimelineEntry & { raw: DebugDumpRecord }>,
  timestamp: number,
): (TimelineEntry & { raw: DebugDumpRecord }) | undefined {
  for (let i = dumps.length - 1; i >= 0; i--) {
    if (dumps[i].timestamp <= timestamp) {
      return dumps[i];
    }
  }
  return undefined;
}

function appendReason(reasons: Map<string, string[]>, id: string, reason: string): void {
  const values = reasons.get(id) ?? [];
  if (!values.includes(reason)) {
    values.push(reason);
  }
  reasons.set(id, values);
}

function renderContentBlocks(content: unknown[]): React.ReactNode {
  if (!Array.isArray(content) || content.length === 0) {
    return <div className="model-empty-inline">(empty)</div>;
  }

  return content.map((block, index) => (
    <div className="model-block" key={index}>
      <div className="model-block-kind">{blockKind(block)}</div>
      <div className="model-block-content">{blockPreview(block)}</div>
      <CollapsibleBlock label="Show block JSON">
        {formatJson(block)}
      </CollapsibleBlock>
    </div>
  ));
}

function blockKind(block: unknown): string {
  if (isRecord(block) && typeof block.type === 'string') {
    return block.type;
  }
  return typeof block;
}

function blockPreview(block: unknown): React.ReactNode {
  if (!isRecord(block)) {
    return <pre>{formatJson(block)}</pre>;
  }

  if (typeof block.text === 'string') {
    return <div>{block.text}</div>;
  }
  if (typeof block.name === 'string') {
    return <div>{block.name}</div>;
  }
  if (typeof block.tool_use_id === 'string') {
    return <div>tool_use_id={block.tool_use_id}</div>;
  }
  if (Array.isArray(block.content)) {
    return <div>{summarizeNestedContent(block.content)}</div>;
  }
  return <pre>{formatJson(block)}</pre>;
}

function summarizeNestedContent(content: unknown[]): string {
  return content
    .map(item => {
      if (isRecord(item) && typeof item.text === 'string') {
        return item.text;
      }
      return formatJson(item);
    })
    .join('\n');
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function formatJson(value: unknown): string {
  if (typeof value === 'string') {
    return value;
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function formatTime(timestamp: number): string {
  if (!timestamp) {
    return 'unknown time';
  }
  return new Date(timestamp).toLocaleString();
}
