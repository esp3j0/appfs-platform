import React, { useEffect, useState, useCallback } from 'react';
import type { AgentInfo, TimelineResponse } from './types';
import { useSSE } from './hooks/useSSE';
import { useDashboardSSE } from './hooks/useDashboardSSE';
import { TopBar } from './components/TopBar';
import { AgentSidebar } from './components/AgentSidebar';
import { TimelinePanel } from './components/TimelinePanel';
import { InfoPanel } from './components/InfoPanel';
import { ModelViewPanel } from './components/ModelViewPanel';
import { AppControlPanel } from './components/AppControlPanel';
import { PlaygroundPanel } from './components/PlaygroundPanel';
import { ProjectPicker } from './components/ProjectPicker';
import { ProjectComposeEditor } from './components/ProjectComposeEditor';

type MainView = 'timeline' | 'apps' | 'model' | 'chat' | 'compose';

const EMPTY_TIMELINE: TimelineResponse = {
  entries: [],
  interactions: [],
  compactionBoundaries: [],
};

export function App() {
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [archivedAgents, setArchivedAgents] = useState<AgentInfo[]>([]);
  const [selectedSessionIds, setSelectedSessionIds] = useState<Set<string>>(new Set());
  const [timeline, setTimeline] = useState<TimelineResponse>(EMPTY_TIMELINE);
  const [filter, setFilter] = useState<string>('all');
  const [mainView, setMainView] = useState<MainView>('timeline');

  // Selected Project states
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [selectedProjectRoot, setSelectedProjectRoot] = useState<string | null>(null);
  const [projects, setProjects] = useState<any[]>([]);
  const [runtimeBusy, setRuntimeBusy] = useState<boolean>(false);
  const [runtimeError, setRuntimeError] = useState<string | null>(null);

  const upsertProject = useCallback((project: any) => {
    setProjects(prev => {
      const index = prev.findIndex(p => p.projectId === project.projectId);
      if (index >= 0) {
        const next = [...prev];
        next[index] = project;
        return next;
      }
      return [...prev, project];
    });
  }, []);

  const loadProjects = useCallback(() => {
    fetch('/api/projects')
      .then(r => r.ok ? r.json() : Promise.reject())
      .then((data: { projects: any[] }) => {
        if (data && data.projects) {
          setProjects(data.projects);
        }
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    loadProjects();
    const interval = setInterval(loadProjects, 5000);
    return () => clearInterval(interval);
  }, [loadProjects]);

  const loadAgents = useCallback(() => {
    Promise.all([
      fetch('/api/agents').then(r => r.json() as Promise<AgentInfo[]>),
      fetch('/api/agents?archived=only').then(r => r.json() as Promise<AgentInfo[]>),
    ])
      .then(([active, archived]) => {
        setAgents(active);
        setArchivedAgents(archived);
        if (active.length > 0) {
          setSelectedSessionIds(prev => {
            const visibleSessionIds = preferredSessionIds(active);
            const knownSessionIds = new Set([
              ...active.map(agent => agent.sessionId),
              ...archived.map(agent => agent.sessionId),
            ]);
            const kept = Array.from(prev).filter(sessionId => knownSessionIds.has(sessionId));
            if (kept.length > 0) {
              return new Set(kept);
            }
            return new Set([Array.from(visibleSessionIds)[0] ?? active[0].sessionId]);
          });
        }
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    loadAgents();
  }, [loadAgents]);

  const bootstrapProject = useCallback(async (projectId: string) => {
    setRuntimeBusy(true);
    setRuntimeError(null);
    try {
      const res = await fetch(`/api/projects/${projectId}/bootstrap`, {
        method: 'POST',
      });
      const data = await res.json().catch(() => ({}));
      if (!res.ok) {
        throw new Error(data.error || `HTTP ${res.status}`);
      }

      if (data.project) {
        upsertProject(data.project);
      }

      const errors: string[] = [];
      if (data.runtime?.error) {
        errors.push(data.runtime.error);
      }
      if (Array.isArray(data.resume?.errors)) {
        errors.push(...data.resume.errors.map((item: any) => item.error).filter(Boolean));
      }
      setRuntimeError(errors.length > 0 ? errors.join('; ') : null);
    } catch (err: any) {
      setRuntimeError(err.message || String(err));
    } finally {
      setRuntimeBusy(false);
      loadAgents();
      loadProjects();
    }
  }, [loadAgents, loadProjects, upsertProject]);

  // Load electron shell metadata and auto-open the last selected project
  useEffect(() => {
    if (typeof window.appfsShell !== 'undefined') {
      window.appfsShell.getShellMetadata().then(meta => {
        if (meta.lastSelectedProjectRoot) {
          fetch('/api/projects/open', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ projectRoot: meta.lastSelectedProjectRoot }),
          })
            .then(r => {
              if (!r.ok) throw new Error();
              return r.json();
            })
            .then(data => {
              if (data && data.projectId) {
                setSelectedProjectRoot(data.projectRoot || meta.lastSelectedProjectRoot!);
                setSelectedProjectId(data.projectId);
                setMainView('compose');
                setRuntimeError(null);
                loadAgents();
                void bootstrapProject(data.projectId);
              }
            })
            .catch(() => {
              // Ignore and let user select manually
            });
        }
      });
    }
  }, [bootstrapProject, loadAgents]);

  const loadTimeline = useCallback((sessionIds: string[]) => {
    if (sessionIds.length === 0) {
      setTimeline(EMPTY_TIMELINE);
      return;
    }
    fetch(`/api/timeline?agents=${sessionIds.map(encodeURIComponent).join(',')}`)
      .then(r => r.json())
      .then((data: TimelineResponse) => setTimeline({
        ...data,
        compactionBoundaries: data.compactionBoundaries ?? [],
      }))
      .catch(() => {});
  }, []);

  useEffect(() => {
    loadTimeline(Array.from(selectedSessionIds));
  }, [selectedSessionIds, loadTimeline]);

  useSSE('/api/events', {
    onMessage: (entry) => {
      if (entry.sessionId && selectedSessionIds.has(entry.sessionId)) {
        loadTimeline(Array.from(selectedSessionIds));
      }
    },
    onDebugDump: (entry) => {
      if (entry.sessionId && selectedSessionIds.has(entry.sessionId)) {
        loadTimeline(Array.from(selectedSessionIds));
      }
    },
    onAgentOnline: (agent: Partial<AgentInfo>) => {
      if (!agent.name || !agent.sessionId || !agent.principalId) {
        loadAgents();
        return;
      }
      setAgents(prev => {
        const existingIndex = prev.findIndex(a => a.sessionId === agent.sessionId);
        if (existingIndex >= 0) {
          const next = [...prev];
          next[existingIndex] = { ...next[existingIndex], ...agent } as AgentInfo;
          return next;
        }
        return [...prev, agent as AgentInfo];
      });
      if (agent.controlMode === 'managed' && agent.sessionId) {
        setSelectedSessionIds(prev => {
          const next = new Set(prev);
          const principalId = agent.principalId || agent.name;
          for (const existing of agents) {
            if ((existing.principalId || existing.name) === principalId) {
              next.delete(existing.sessionId);
            }
          }
          next.add(agent.sessionId!);
          return next;
        });
      }
    },
    onAgentOffline: (agent: any) => {
      const sessionId = agent?.sessionId;
      if (!sessionId) {
        loadAgents();
        return;
      }
      setAgents(prev => prev.map(item =>
        item.sessionId === sessionId ? { ...item, status: 'offline' } : item
      ));
    },
  });

  useDashboardSSE('/api/events', {
    onTurnDone: () => {
      loadAgents();
    },
  });

  const toggleAgent = (sessionId: string) => {
    setSelectedSessionIds(prev => {
      const next = new Set(prev);
      if (next.has(sessionId)) {
        if (next.size > 1) next.delete(sessionId);
      } else {
        next.add(sessionId);
      }
      return next;
    });
  };

  const markAgentStopped = useCallback((sessionId: string) => {
    setAgents(prev => prev.map(item =>
      item.sessionId === sessionId ? { ...item, status: 'offline' } : item
    ));
  }, []);

  const handleSwitchProject = () => {
    setSelectedProjectId(null);
    setSelectedProjectRoot(null);
    setRuntimeError(null);
    setMainView('compose');
    if (typeof window.appfsShell !== 'undefined') {
      window.appfsShell.persistSelectedProjectRoot('');
    }
  };

  const runProjectAction = async (action: 'start' | 'stop') => {
    if (!selectedProjectId) return;

    setRuntimeBusy(true);
    setRuntimeError(null);
    try {
      const res = await fetch(`/api/projects/${selectedProjectId}/${action}`, {
        method: 'POST',
      });
      const data = await res.json().catch(() => ({}));
      if (!res.ok) {
        throw new Error(data.error || `HTTP ${res.status}`);
      }
      upsertProject(data);
    } catch (err: any) {
      setRuntimeError(err.message || String(err));
    } finally {
      setRuntimeBusy(false);
      loadProjects();
    }
  };

  const selectedProject = projects.find(p => p.projectId === selectedProjectId);
  const projectStatus = selectedProject?.status || 'stopped';
  const getProjectFolderName = (root: string) => {
    const parts = root.replace(/\\/g, '/').split('/');
    return parts[parts.length - 1] || root;
  };
  const projectName = selectedProjectRoot ? getProjectFolderName(selectedProjectRoot) : undefined;

  const crossEntryIds = new Set(timeline.interactions.map(i => i.entryId));
  const allKnownAgents = [...agents, ...archivedAgents];
  const selectedAgents = allKnownAgents.filter(a => selectedSessionIds.has(a.sessionId));
  const selectedActiveAgents = agents.filter(a => selectedSessionIds.has(a.sessionId));
  const selectedAgentLabels = displayLabelsForAgents(selectedAgents);
  const filtered = filter === 'all'
    ? timeline.entries
    : timeline.entries.filter(e => {
        if (filter === 'model') return e.role === 'assistant' || e.source === 'debug-dump';
        if (filter === 'tools') return e.role === 'tool' || e.content.includes('tool_use');
        if (filter === 'cross') return crossEntryIds.has(e.id);
        return true;
      });

  if (!selectedProjectId) {
    return (
      <ProjectPicker
        onProjectOpen={(id, root) => {
          setSelectedProjectId(id);
          setSelectedProjectRoot(root);
          setMainView('compose');
          setRuntimeError(null);
          loadProjects();
          loadAgents();
          void bootstrapProject(id);
        }}
      />
    );
  }

  return (
    <>
      <TopBar
        agentCount={agents.filter(a => a.status === 'online').length}
        projectName={projectName}
        projectStatus={projectStatus}
        onSwitchProject={handleSwitchProject}
        onOpenCompose={() => setMainView('compose')}
        onStartRuntime={() => runProjectAction('start')}
        onStopRuntime={() => runProjectAction('stop')}
        runtimeBusy={runtimeBusy}
        runtimeError={runtimeError}
      />
      <div className="main-layout">
        <AgentSidebar
          agents={agents}
          archivedAgents={archivedAgents}
          selected={selectedSessionIds}
          onToggle={toggleAgent}
          onRefreshAgents={loadAgents}
          onAgentStopped={markAgentStopped}
          selectedProjectId={selectedProjectId}
          selectedProjectRoot={selectedProjectRoot}
        />
        <div className="work-area">
          <div className="view-tabs">
            <button className={`view-tab ${mainView === 'timeline' ? 'active' : ''}`} onClick={() => setMainView('timeline')}>Timeline</button>
            <button className={`view-tab ${mainView === 'chat' ? 'active' : ''}`} onClick={() => setMainView('chat')}>💬 Chat</button>
            <button className={`view-tab ${mainView === 'apps' ? 'active' : ''}`} onClick={() => setMainView('apps')}>Apps</button>
            <button className={`view-tab ${mainView === 'compose' ? 'active' : ''}`} onClick={() => setMainView('compose')}>Compose</button>
            <button className={`view-tab ${mainView === 'model' ? 'active' : ''}`} onClick={() => setMainView('model')}>Model</button>
          </div>
          {mainView === 'timeline' ? (
            <TimelinePanel selectedAgents={selectedAgentLabels} entries={filtered} interactions={timeline.interactions} filter={filter} onFilterChange={setFilter} />
          ) : mainView === 'chat' ? (
            <PlaygroundPanel
              agents={agents}
              selectedAgents={selectedActiveAgents}
            />
          ) : mainView === 'apps' ? (
            <AppControlPanel selectedAgents={selectedAgentLabels} entries={timeline.entries} />
          ) : mainView === 'compose' ? (
            <ProjectComposeEditor projectId={selectedProjectId!} />
          ) : (
            <ModelViewPanel selectedAgents={selectedAgentLabels} entries={timeline.entries} compactionBoundaries={timeline.compactionBoundaries ?? []} />
          )}
        </div>
        <InfoPanel agents={selectedAgents} interactions={timeline.interactions} />
      </div>
    </>
  );
}

function displayLabelsForAgents(agents: AgentInfo[]): string[] {
  const labelCounts = new Map<string, number>();
  for (const agent of agents) {
    const label = agent.name || agent.principalId || agent.sessionId;
    labelCounts.set(label, (labelCounts.get(label) ?? 0) + 1);
  }
  return agents.map(agent => {
    const label = agent.name || agent.principalId || agent.sessionId;
    if ((labelCounts.get(label) ?? 0) <= 1) {
      return label;
    }
    return `${label} · ${agent.sessionId}`;
  });
}

function preferredSessionIds(agents: AgentInfo[]): Set<string> {
  const byPrincipal = new Map<string, AgentInfo>();
  for (const agent of agents) {
    const principalId = agent.principalId || agent.name || agent.sessionId;
    const current = byPrincipal.get(principalId);
    if (!current || isPreferredAgent(agent, current)) {
      byPrincipal.set(principalId, agent);
    }
  }
  return new Set(Array.from(byPrincipal.values()).map(agent => agent.sessionId));
}

function isPreferredAgent(candidate: AgentInfo, current: AgentInfo): boolean {
  const candidateScore = agentScore(candidate);
  const currentScore = agentScore(current);
  for (let i = 0; i < candidateScore.length; i += 1) {
    if (candidateScore[i] !== currentScore[i]) {
      return candidateScore[i] > currentScore[i];
    }
  }
  return false;
}

function agentScore(agent: AgentInfo): number[] {
  return [
    agent.status === 'online' ? 1 : 0,
    agent.controlMode === 'managed' ? 1 : 0,
    agent.startedAt || 0,
    agent.messageCount || 0,
  ];
}
