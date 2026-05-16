import React from 'react';

interface Props { agentCount: number; }

export function TopBar({ agentCount }: Props) {
  return (
    <div className="topbar">
      <h1>AppFS Debug Dashboard</h1>
      <div className="status">
        <span style={{ color: agentCount > 0 ? '#3fb950' : '#484f58' }}>
          {agentCount} agent{agentCount !== 1 ? 's' : ''} online
        </span>
      </div>
    </div>
  );
}
