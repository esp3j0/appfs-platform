import React from 'react';
import type { TimelineEntry, CrossAgentInteraction } from '../types';
import { getAgentColor } from '../types';
import { MessageBubble } from './MessageBubble';

interface Props {
  selectedAgents: string[];
  entries: TimelineEntry[];
  interactions: CrossAgentInteraction[];
  filter: string;
  onFilterChange: (f: string) => void;
}

const FILTERS = [
  { key: 'all', label: 'All' },
  { key: 'model', label: 'Model I/O' },
  { key: 'tools', label: 'Tools' },
  { key: 'cross', label: 'Cross-agent' },
];

export function TimelinePanel({ selectedAgents, entries, interactions, filter, onFilterChange }: Props) {
  const agentCount = selectedAgents.length;
  const useSwimlane = agentCount >= 2;

  // Header is shared
  const header = (
    <div className="timeline-header" key="header">
      <div>
        <h2>{agentCount > 1 ? 'Swimlane Timeline' : `${selectedAgents[0] ?? 'No Agent'} Timeline`}</h2>
        {agentCount > 1 && (
          <div className="selected-tags">
            {selectedAgents.map((name, i) => {
              const color = getAgentColor(i);
              return <span key={name} className="selected-tag" style={{ background: `${color}33`, color }}>{name}</span>;
            })}
            <span style={{ color: '#484f58', fontSize: 11 }}>{agentCount} agents, {entries.length} messages</span>
          </div>
        )}
      </div>
      <div className="filters">
        {FILTERS.map(f => (
          <button key={f.key} className={`filter-btn ${filter === f.key ? 'active' : ''}`} onClick={() => onFilterChange(f.key)}>{f.label}</button>
        ))}
      </div>
    </div>
  );

  if (!useSwimlane) {
    // Single-agent: simple list
    const agentColorMap = new Map(selectedAgents.map((name, i) => [name, i]));
    return (
      <div className="timeline">
        {header}
        {entries.map((entry) => (
          <MessageBubble key={entry.id} entry={entry} agentColorIndex={agentColorMap.get(entry.agentName) ?? 0} />
        ))}
      </div>
    );
  }

  // Multi-agent swimlane
  const agentIdxMap = new Map(selectedAgents.map((name, i) => [name, i]));

  // Build row data: each row is a "time step" — one agent has a message, others are empty.
  // We also interleave interaction arrows as their own rows.
  interface Row {
    type: 'message';
    entry: TimelineEntry;
    agentIndex: number;
  }
  const rows: Row[] = entries.map(entry => ({
    type: 'message' as const,
    entry,
    agentIndex: agentIdxMap.get(entry.agentName) ?? 0,
  }));

  return (
    <div className="timeline">
      {header}
      {/* Swimlane column headers */}
      <div className="swimlane-cols" style={{ gridTemplateColumns: `repeat(${agentCount}, 1fr)` }}>
        {selectedAgents.map((name, i) => {
          const color = getAgentColor(i);
          return (
            <div key={name} className="swimlane-col-header" style={{ borderColor: color }}>
              <span style={{ color }}>{name}</span>
            </div>
          );
        })}
      </div>
      {/* Swimlane body — each row is its own grid */}
      <div className="swimlane-body">
        {rows.map((row) => {
          const entry = row.entry;
          const colorIdx = agentIdxMap.get(entry.agentName) ?? 0;
          const color = getAgentColor(colorIdx);
          const interactionsForEntry = dedupeInteractions(
            interactions.filter(inter => inter.entryId === entry.id),
          ).filter(
            inter => agentIdxMap.has(inter.fromAgent) && agentIdxMap.has(inter.toAgent),
          );

          return (
            <React.Fragment key={entry.id}>
              {/* Message row — grid with one column per agent */}
              <div className="swimlane-row" style={{ gridTemplateColumns: `repeat(${agentCount}, 1fr)` }}>
                {selectedAgents.map((name, ci) => {
                  if (ci === row.agentIndex) {
                    return (
                      <div key={name} className="swimlane-cell has-message" style={{ borderColor: `${color}22` }}>
                        <MessageBubble entry={entry} agentColorIndex={ci} compact />
                      </div>
                    );
                  }
                  return <div key={name} className="swimlane-cell" style={{ borderColor: `${getAgentColor(ci)}11` }} />;
                })}
              </div>

              {interactionsForEntry.map((interaction, index) => (
                <SwimlaneInteractionArrow
                  key={`${interaction.fromAgent}:${interaction.toAgent}:${interaction.eventType}:${interaction.seq ?? index}`}
                  interaction={interaction}
                  agentCount={agentCount}
                  agentIdxMap={agentIdxMap}
                />
              ))}
            </React.Fragment>
          );
        })}
      </div>
    </div>
  );
}

function SwimlaneInteractionArrow({
  interaction,
  agentCount,
  agentIdxMap,
}: {
  interaction: CrossAgentInteraction;
  agentCount: number;
  agentIdxMap: Map<string, number>;
}) {
  const fromIdx = agentIdxMap.get(interaction.fromAgent);
  const toIdx = agentIdxMap.get(interaction.toAgent);
  if (fromIdx === undefined || toIdx === undefined) {
    return null;
  }

  const startIdx = Math.min(fromIdx, toIdx);
  const endIdx = Math.max(fromIdx, toIdx);
  const isForward = fromIdx <= toIdx;
  const fromColor = getAgentColor(fromIdx);
  const toColor = getAgentColor(toIdx);
  const gradient = isForward
    ? `linear-gradient(to right, ${fromColor}88, ${toColor}88)`
    : `linear-gradient(to right, ${toColor}88, ${fromColor}88)`;

  return (
    <div
      className="swimlane-arrow"
      style={{ gridTemplateColumns: `repeat(${agentCount}, 1fr)` }}
    >
      <div
        className="swimlane-arrow-cell swimlane-arrow-span"
        style={{ gridColumn: `${startIdx + 1} / ${endIdx + 2}` }}
      >
        <div className="arrow-h">
          {isForward ? (
            <>
              <span className="arrow-h-label" style={{ color: fromColor }}>{interaction.fromAgent}</span>
              <div className="arrow-h-line" style={{ background: gradient }} />
              <span className="arrow-h-symbol" style={{ color: toColor }}>-&gt;</span>
              <span className="arrow-h-label" style={{ color: toColor }}>{interaction.toAgent}</span>
            </>
          ) : (
            <>
              <span className="arrow-h-label" style={{ color: toColor }}>{interaction.toAgent}</span>
              <span className="arrow-h-symbol" style={{ color: toColor }}>&lt;-</span>
              <div className="arrow-h-line" style={{ background: gradient }} />
              <span className="arrow-h-label" style={{ color: fromColor }}>{interaction.fromAgent}</span>
            </>
          )}
          <span className="arrow-h-label">{interaction.eventType}</span>
        </div>
      </div>
    </div>
  );
}

function dedupeInteractions(interactions: CrossAgentInteraction[]): CrossAgentInteraction[] {
  const seen = new Set<string>();
  return interactions.filter(interaction => {
    const key = [
      interaction.fromAgent,
      interaction.toAgent,
      interaction.eventType,
      interaction.seq ?? '',
      interaction.timestamp,
    ].join('|');
    if (seen.has(key)) {
      return false;
    }
    seen.add(key);
    return true;
  });
}
