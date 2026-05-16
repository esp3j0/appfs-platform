import React from 'react';
import type { TimelineEntry, CrossAgentInteraction } from '../types';
import { getAgentColor } from '../types';
import { MessageBubble } from './MessageBubble';
import { InteractionArrow } from './InteractionArrow';

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
  const agentColorMap = new Map(selectedAgents.map((name, i) => [name, i]));

  const rendered: React.ReactNode[] = [];
  let interactionIdx = 0;

  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];
    rendered.push(
      <MessageBubble key={`msg-${i}`} entry={entry} agentColorIndex={agentColorMap.get(entry.agentName) ?? 0} />
    );

    while (interactionIdx < interactions.length) {
      const inter = interactions[interactionIdx];
      if (inter.fromAgent === entry.agentName && i < entries.length - 1) {
        rendered.push(<InteractionArrow key={`arrow-${interactionIdx}`} interaction={inter} />);
        interactionIdx++;
      } else {
        break;
      }
    }
  }

  return (
    <div className="timeline">
      <div className="timeline-header">
        <div>
          <h2>{selectedAgents.length > 1 ? 'Merged Timeline' : `${selectedAgents[0] ?? 'No Agent'} Timeline`}</h2>
          {selectedAgents.length > 1 && (
            <div className="selected-tags">
              {selectedAgents.map((name, i) => {
                const color = getAgentColor(i);
                return <span key={name} className="selected-tag" style={{ background: `${color}33`, color }}>{name}</span>;
              })}
              <span style={{ color: '#484f58', fontSize: 11 }}>{selectedAgents.length} agents, {entries.length} messages</span>
            </div>
          )}
        </div>
        <div className="filters">
          {FILTERS.map(f => (
            <button key={f.key} className={`filter-btn ${filter === f.key ? 'active' : ''}`} onClick={() => onFilterChange(f.key)}>{f.label}</button>
          ))}
        </div>
      </div>
      {rendered}
    </div>
  );
}
