import React from 'react';
import type { AgentInfo } from '../types';
import { getAgentColor } from '../types';

interface Props {
  agent: AgentInfo;
  checked: boolean;
  colorIndex: number;
  onToggle: () => void;
  onResume?: (agent: AgentInfo) => void;
  onStop?: (agent: AgentInfo) => void;
  resumeDisabled?: boolean;
  stopDisabled?: boolean;
}

export function AgentItem({
  agent,
  checked,
  colorIndex,
  onToggle,
  onResume,
  onStop,
  resumeDisabled = false,
  stopDisabled = false,
}: Props) {
  const color = getAgentColor(colorIndex);
  const canResume = agent.status === 'offline' && Boolean(agent.sessionJsonlPath) && Boolean(onResume);
  const canStop = agent.status === 'online' && agent.controlMode === 'managed' && Boolean(agent.sessionId) && Boolean(onStop);

  return (
    <div className={`agent-item ${checked ? 'active' : ''}`} style={{ borderLeft: checked ? `3px solid ${color}` : '3px solid transparent' }} onClick={onToggle}>
      <input type="checkbox" className="agent-checkbox" checked={checked} onChange={() => onToggle()} onClick={e => e.stopPropagation()} />
      <div className="agent-item-body">
        <div className="agent-name">{agent.name}</div>
        <div className="agent-meta">principal: {agent.principalId}</div>
        <div className="agent-model">{agent.model}</div>
        <div className="agent-item-actions">
          <span className={`status-badge ${agent.status}`}>{agent.status}</span>
          {canResume && (
            <button
              type="button"
              className="agent-action-btn agent-resume-btn"
              onClick={e => {
                e.stopPropagation();
                onResume?.(agent);
              }}
              disabled={resumeDisabled}
              title={agent.sessionJsonlPath}
            >
              Resume
            </button>
          )}
          {canStop && (
            <button
              type="button"
              className="agent-action-btn agent-stop-btn"
              onClick={e => {
                e.stopPropagation();
                onStop?.(agent);
              }}
              disabled={stopDisabled}
              title={stopDisabled ? 'Stopping managed agent...' : `Stop managed agent ${agent.name}`}
            >
              Stop
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
