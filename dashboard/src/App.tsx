import React, { useEffect, useState, useCallback } from 'react';
import type { AgentInfo, TimelineResponse } from './types';
import { useSSE } from './hooks/useSSE';
import { TopBar } from './components/TopBar';
import { AgentSidebar } from './components/AgentSidebar';
import { TimelinePanel } from './components/TimelinePanel';
import { InfoPanel } from './components/InfoPanel';

export function App() {
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [selectedAgents, setSelectedAgents] = useState<Set<string>>(new Set());
  const [timeline, setTimeline] = useState<TimelineResponse>({ entries: [], interactions: [] });
  const [filter, setFilter] = useState<string>('all');

  useEffect(() => {
    fetch('/api/agents')
      .then(r => r.json())
      .then((data: AgentInfo[]) => {
        setAgents(data);
        if (data.length > 0) {
          setSelectedAgents(new Set([data[0].name]));
        }
      })
      .catch(() => {});
  }, []);

  const loadTimeline = useCallback((names: string[]) => {
    if (names.length === 0) {
      setTimeline({ entries: [], interactions: [] });
      return;
    }
    fetch(`/api/timeline?agents=${names.map(encodeURIComponent).join(',')}`)
      .then(r => r.json())
      .then((data: TimelineResponse) => setTimeline(data))
      .catch(() => {});
  }, []);

  useEffect(() => {
    loadTimeline(Array.from(selectedAgents));
  }, [selectedAgents, loadTimeline]);

  useSSE('/api/events', {
    onMessage: (entry) => {
      setTimeline(prev => ({ ...prev, entries: [...prev.entries, entry] }));
    },
    onDebugDump: (entry) => {
      setTimeline(prev => ({ ...prev, entries: [...prev.entries, entry] }));
    },
    onAgentOnline: (agent: any) => {
      setAgents(prev => {
        if (prev.some(a => a.name === agent.name)) return prev;
        return [...prev, agent];
      });
    },
  });

  const toggleAgent = (name: string) => {
    setSelectedAgents(prev => {
      const next = new Set(prev);
      if (next.has(name)) {
        if (next.size > 1) next.delete(name);
      } else {
        next.add(name);
      }
      return next;
    });
  };

  const filtered = filter === 'all'
    ? timeline.entries
    : timeline.entries.filter(e => {
        if (filter === 'model') return e.role === 'assistant' || e.source === 'debug-dump';
        if (filter === 'tools') return e.role === 'tool' || e.content.includes('tool_use');
        if (filter === 'cross') return timeline.interactions.some(i => i.fromAgent === e.agentName || i.toAgent === e.agentName);
        return true;
      });

  return (
    <>
      <TopBar agentCount={agents.filter(a => a.status === 'online').length} />
      <div className="main-layout">
        <AgentSidebar agents={agents} selected={selectedAgents} onToggle={toggleAgent} />
        <TimelinePanel selectedAgents={Array.from(selectedAgents)} entries={filtered} interactions={timeline.interactions} filter={filter} onFilterChange={setFilter} />
        <InfoPanel agents={agents.filter(a => selectedAgents.has(a.name))} interactions={timeline.interactions} />
      </div>
    </>
  );
}
