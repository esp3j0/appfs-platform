import React from 'react';
import type { AgentInfo } from '../types';
import { AgentItem } from './AgentItem';

interface Props { agents: AgentInfo[]; selected: Set<string>; onToggle: (name: string) => void; }

export function AgentSidebar({ agents, selected, onToggle }: Props) {
  const totalInput = agents.reduce((s, a) => s + a.totalInputTokens, 0);
  const totalOutput = agents.reduce((s, a) => s + a.totalOutputTokens, 0);
  const maxTokens = Math.max(totalInput, totalOutput, 1);

  return (
    <div className="sidebar">
      <div className="sidebar-header">Agents (multi-select)</div>
      <div className="select-hint">Click to toggle. Select 2+ for merged timeline.</div>
      <div className="agent-list">
        {agents.map((agent, i) => (
          <AgentItem key={agent.name} agent={agent} checked={selected.has(agent.name)} colorIndex={i} onToggle={() => onToggle(agent.name)} />
        ))}
      </div>
      <div className="sidebar-usage">
        <div className="sidebar-header" style={{ padding: 0, border: 'none', marginBottom: 8 }}>Total Usage</div>
        <div className="usage-row"><span className="usage-label">Input</span><span className="usage-value">{totalInput.toLocaleString()} tok</span></div>
        <div className="usage-bar"><div className="usage-fill input" style={{ width: `${(totalInput / maxTokens) * 100}%` }} /></div>
        <div className="usage-row" style={{ marginTop: 6 }}><span className="usage-label">Output</span><span className="usage-value">{totalOutput.toLocaleString()} tok</span></div>
        <div className="usage-bar"><div className="usage-fill output" style={{ width: `${(totalOutput / maxTokens) * 100}%` }} /></div>
      </div>
    </div>
  );
}
