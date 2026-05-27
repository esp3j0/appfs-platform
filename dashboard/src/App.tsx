import React, { useEffect, useState, useCallback } from 'react';
import type { AgentInfo, TimelineResponse } from './types';
import { useSSE } from './hooks/useSSE';
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
  const [selectedAgents, setSelectedAgents] = useState<Set<string>>(new Set());
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
    fetch('/api/agents')
      .then(r => r.json())
      .then((data: AgentInfo[]) => {
        setAgents(data);
        if (data.length > 0) {
          setSelectedAgents(prev => prev.size > 0 ? prev : new Set([data[0].name]));
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

  const loadTimeline = useCallback((names: string[]) => {
    if (names.length === 0) {
      setTimeline(EMPTY_TIMELINE);
      return;
    }
    fetch(`/api/timeline?agents=${names.map(encodeURIComponent).join(',')}`)
      .then(r => r.json())
      .then((data: TimelineResponse) => setTimeline({
        ...data,
        compactionBoundaries: data.compactionBoundaries ?? [],
      }))
      .catch(() => {});
  }, []);

  useEffect(() => {
    loadTimeline(Array.from(selectedAgents));
  }, [selectedAgents, loadTimeline]);

  useSSE('/api/events', {
    onMessage: (entry) => {
      if (selectedAgents.has(entry.agentName)) {
        loadTimeline(Array.from(selectedAgents));
      } else {
        setTimeline(prev => ({ ...prev, entries: [...prev.entries, entry] }));
      }
    },
    onDebugDump: (entry) => {
      if (selectedAgents.has(entry.agentName)) {
        loadTimeline(Array.from(selectedAgents));
      } else {
        setTimeline(prev => ({ ...prev, entries: [...prev.entries, entry] }));
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
      if (agent.controlMode === 'managed' && agent.name) {
        setSelectedAgents(prev => new Set(prev).add(agent.name!));
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
          selected={selectedAgents}
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
            <TimelinePanel selectedAgents={Array.from(selectedAgents)} entries={filtered} interactions={timeline.interactions} filter={filter} onFilterChange={setFilter} />
          ) : mainView === 'chat' ? (
            <PlaygroundPanel
              agents={agents}
              selectedAgents={agents.filter(a => selectedAgents.has(a.name))}
            />
          ) : mainView === 'apps' ? (
            <AppControlPanel selectedAgents={Array.from(selectedAgents)} entries={timeline.entries} />
          ) : mainView === 'compose' ? (
            <ProjectComposeEditor projectId={selectedProjectId!} />
          ) : (
            <ModelViewPanel selectedAgents={Array.from(selectedAgents)} entries={timeline.entries} compactionBoundaries={timeline.compactionBoundaries ?? []} />
          )}
        </div>
        <InfoPanel agents={agents.filter(a => selectedAgents.has(a.name))} interactions={timeline.interactions} />
      </div>
    </>
  );
}
