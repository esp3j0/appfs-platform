import React from 'react';
import type { AgentInfo } from '../types';
import { getAgentColor } from '../types';

interface Props { agent: AgentInfo; checked: boolean; colorIndex: number; onToggle: () => void; }

export function AgentItem({ agent, checked, colorIndex, onToggle }: Props) {
  const color = getAgentColor(colorIndex);
  return (
    <div className={`agent-item ${checked ? 'active' : ''}`} style={{ borderLeft: checked ? `3px solid ${color}` : '3px solid transparent' }} onClick={onToggle}>
      <input type="checkbox" className="agent-checkbox" checked={checked} onChange={() => onToggle()} onClick={e => e.stopPropagation()} />
      <div>
        <div className="agent-name">{agent.name}</div>
        <div className="agent-meta">principal: {agent.principalId}</div>
        <div className="agent-model">{agent.model}</div>
        <div style={{ marginTop: 4 }}><span className={`status-badge ${agent.status}`}>{agent.status}</span></div>
      </div>
    </div>
  );
}
