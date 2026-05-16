import React from 'react';
import type { AgentInfo, CrossAgentInteraction } from '../types';

interface Props { agents: AgentInfo[]; interactions: CrossAgentInteraction[]; }

export function InfoPanel({ agents, interactions }: Props) {
  if (agents.length === 0) {
    return <div className="info-panel"><div className="info-section"><div className="info-title">No agents selected</div></div></div>;
  }

  return (
    <div className="info-panel">
      {agents.length > 1 && (
        <div className="info-section">
          <div className="info-title">Merged View</div>
          <div className="info-row"><span className="usage-label">Agents</span><span className="usage-value highlight">{agents.length} selected</span></div>
          <div className="info-row"><span className="usage-label">Interactions</span><span className="usage-value">{interactions.length} cross-agent</span></div>
        </div>
      )}

      {agents.map(agent => (
        <div className="info-section" key={agent.name}>
          <div className="info-title">{agent.name}</div>
          <div className="info-row"><span className="usage-label">Model</span><span className="usage-value">{agent.model}</span></div>
          <div className="info-row"><span className="usage-label">Messages</span><span className="usage-value highlight">{agent.messageCount}</span></div>
          <div className="info-row"><span className="usage-label">Input</span><span className="usage-value">{agent.totalInputTokens.toLocaleString()} tok</span></div>
          <div className="info-row"><span className="usage-label">Output</span><span className="usage-value">{agent.totalOutputTokens.toLocaleString()} tok</span></div>
        </div>
      ))}

      {interactions.length > 0 && (
        <div className="info-section">
          <div className="info-title">Cross-Agent Events</div>
          <div style={{ background: '#0d1117', border: '1px solid #30363d', borderRadius: 4, padding: 8, fontSize: 11 }}>
            {interactions.map((inter, i) => (
              <div key={i} style={{ marginBottom: 4, color: '#8b949e' }}>
                <span style={{ color: '#3fb950' }}>{inter.fromAgent}</span>
                {' \u2192 '}
                <span style={{ color: '#58a6ff' }}>{inter.toAgent}</span>
                {' '}<span style={{ color: '#484f58' }}>({inter.eventType})</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
