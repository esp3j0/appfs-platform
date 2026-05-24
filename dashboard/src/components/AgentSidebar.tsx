import React from 'react';
import type { AgentInfo, SpawnConfig, ProjectRecord } from '../types';
import { AgentItem } from './AgentItem';
import { useDashboardSSE } from '../hooks/useDashboardSSE';

interface Props {
  agents: AgentInfo[];
  selected: Set<string>;
  onToggle: (name: string) => void;
  onRefreshAgents?: () => void;
}

export function AgentSidebar({ agents, selected, onToggle, onRefreshAgents }: Props) {
  const [projects, setProjects] = React.useState<ProjectRecord[]>([]);
  const [expandedProjects, setExpandedProjects] = React.useState<Record<string, boolean>>({});

  const totalInput = agents.reduce((s, a) => s + a.totalInputTokens, 0);
  const totalOutput = agents.reduce((s, a) => s + a.totalOutputTokens, 0);
  const maxTokens = Math.max(totalInput, totalOutput, 1);

  const loadProjects = React.useCallback(() => {
    fetch('/api/projects')
      .then(res => {
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        return res.json();
      })
      .then((data: { projects: ProjectRecord[] }) => {
        if (data && data.projects) {
          setProjects(data.projects);
        }
      })
      .catch((err) => {
        console.error('[AgentSidebar] Failed to load projects:', err);
      });
  }, []);

  React.useEffect(() => {
    loadProjects();
    const interval = setInterval(loadProjects, 5000);
    return () => clearInterval(interval);
  }, [loadProjects]);

  const getProjectFolderName = (projectRoot: string) => {
    const parts = projectRoot.replace(/\\/g, '/').split('/');
    return parts[parts.length - 1] || projectRoot;
  };

  const groupedAgents = React.useMemo(() => {
    const groups: Record<string, { agent: AgentInfo; index: number }[]> = {};
    const ungrouped: { agent: AgentInfo; index: number }[] = [];

    agents.forEach((agent, i) => {
      if (agent.projectId) {
        if (!groups[agent.projectId]) {
          groups[agent.projectId] = [];
        }
        groups[agent.projectId].push({ agent, index: i });
      } else {
        ungrouped.push({ agent, index: i });
      }
    });

    return { groups, ungrouped };
  }, [agents]);

  // Union of all projects to render
  const renderedProjects = React.useMemo(() => {
    const list: {
      projectId: string;
      projectRoot: string;
      status: 'stopped' | 'starting' | 'running' | 'error';
      agents: { agent: AgentInfo; index: number }[];
      isFallback: boolean;
    }[] = [];

    const processedIds = new Set<string>();

    // 1. Process known projects from fetched projects list
    projects.forEach(p => {
      processedIds.add(p.projectId);
      const projectAgents = groupedAgents.groups[p.projectId] || [];
      list.push({
        projectId: p.projectId,
        projectRoot: p.projectRoot,
        status: p.status,
        agents: projectAgents,
        isFallback: false,
      });
    });

    // 2. Process any other projectIds that appear in agents but are missing from projects list
    Object.keys(groupedAgents.groups).forEach(pId => {
      if (!processedIds.has(pId)) {
        processedIds.add(pId);
        const projectAgents = groupedAgents.groups[pId] || [];
        const projectRoot = projectAgents[0]?.agent.projectRoot || pId;
        list.push({
          projectId: pId,
          projectRoot,
          status: 'stopped', // fallback status
          agents: projectAgents,
          isFallback: true,
        });
      }
    });

    return list;
  }, [projects, groupedAgents]);

  const toggleProject = (projectId: string) => {
    setExpandedProjects(prev => ({
      ...prev,
      [projectId]: !prev[projectId],
    }));
  };

  const handleRefresh = () => {
    loadProjects();
    onRefreshAgents?.();
  };

  return (
    <div className="sidebar">
      <div className="sidebar-header">Agents (multi-select)</div>
      <div className="select-hint">Click to toggle. Select 2+ for merged timeline.</div>
      
      <div className="agent-list">
        {renderedProjects.map(project => {
          const isExpanded = expandedProjects[project.projectId] ?? true;

          return (
            <div key={project.projectId} className="project-group">
              <button
                type="button"
                className="project-header"
                onClick={() => toggleProject(project.projectId)}
                aria-expanded={isExpanded}
                aria-controls={`project-content-${project.projectId}`}
              >
                <div className="project-header-left">
                  <span className={`project-arrow ${isExpanded ? 'expanded' : ''}`}>▸</span>
                  <div className="project-header-titles">
                    <div className="project-header-row">
                      <span className="project-name">
                        {getProjectFolderName(project.projectRoot)}
                      </span>
                      <span className={`project-badge ${project.status}`}>{project.status}</span>
                    </div>
                    <div className="project-root-subtitle" title={project.projectRoot}>
                      {project.projectRoot}
                    </div>
                  </div>
                </div>
                <div className="project-header-right">
                  <span className="project-agent-count">{project.agents.length}</span>
                </div>
              </button>
              
              <div
                id={`project-content-${project.projectId}`}
                className="project-content"
                hidden={!isExpanded}
              >
                {project.agents.length === 0 ? (
                  <div className="project-empty-agents">No active agents in project</div>
                ) : (
                  project.agents.map(({ agent, index }) => (
                    <AgentItem
                      key={agent.sessionId || agent.name}
                      agent={agent}
                      checked={selected.has(agent.name)}
                      colorIndex={index}
                      onToggle={() => onToggle(agent.name)}
                    />
                  ))
                )}
              </div>
            </div>
          );
        })}

        {groupedAgents.ungrouped.length > 0 && (
          <div className="project-group">
            <div className="legacy-header">
              Legacy / Flat Sessions ({groupedAgents.ungrouped.length})
            </div>
            <div className="project-content">
              {groupedAgents.ungrouped.map(({ agent, index }) => (
                <AgentItem
                  key={agent.sessionId || agent.name}
                  agent={agent}
                  checked={selected.has(agent.name)}
                  colorIndex={index}
                  onToggle={() => onToggle(agent.name)}
                />
              ))}
            </div>
          </div>
        )}
      </div>

      <SpawnAgentPanel
        agents={agents}
        selected={selected}
        onSpawnAccepted={handleRefresh}
      />
      <div className="sidebar-usage">
        <div className="sidebar-header usage-header">Total Usage</div>
        <div className="usage-row"><span className="usage-label">Input</span><span className="usage-value">{totalInput.toLocaleString()} tok</span></div>
        <div className="usage-bar"><div className="usage-fill input" style={{ width: `${(totalInput / maxTokens) * 100}%` }} /></div>
        <div className="usage-row" style={{ marginTop: 6 }}><span className="usage-label">Output</span><span className="usage-value">{totalOutput.toLocaleString()} tok</span></div>
        <div className="usage-bar"><div className="usage-fill output" style={{ width: `${(totalOutput / maxTokens) * 100}%` }} /></div>
      </div>
    </div>
  );
}

function SpawnAgentPanel({
  agents,
  selected,
  onSpawnAccepted,
}: {
  agents: AgentInfo[];
  selected: Set<string>;
  onSpawnAccepted?: () => void;
}) {
  const [config, setConfig] = React.useState<SpawnConfig | null>(null);
  const [expanded, setExpanded] = React.useState(false);
  const [advancedOpen, setAdvancedOpen] = React.useState(false);
  const [status, setStatus] = React.useState<{ kind: 'idle' | 'loading' | 'success' | 'error'; text?: string }>({ kind: 'loading' });
  const [processLogs, setProcessLogs] = React.useState<SpawnLogEntry[]>([]);

  useDashboardSSE('/api/events', {
    onProcessLog: payload => {
      setProcessLogs(prev => [{
        id: `process-log-${Date.now()}-${prev.length}`,
        kind: 'log',
        agentId: payload.agentId,
        spawnId: payload.spawnId,
        stream: payload.stream,
        text: payload.text,
      }, ...prev].slice(0, 12));
    },
    onAgentOnline: payload => {
      setProcessLogs(prev => [{
        id: `agent-online-${Date.now()}-${prev.length}`,
        kind: 'online',
        agentId: payload.sessionId,
        spawnId: payload.spawnId,
        text: `agent online${payload.sessionId ? `: ${payload.sessionId}` : ''}`,
      }, ...prev].slice(0, 12));
    },
    onAgentOffline: payload => {
      setProcessLogs(prev => [{
        id: `agent-offline-${Date.now()}-${prev.length}`,
        kind: 'offline',
        agentId: payload.sessionId,
        spawnId: payload.spawnId,
        text: `agent offline${payload.sessionId ? `: ${payload.sessionId}` : ''}`,
      }, ...prev].slice(0, 12));
    },
  });

  const selectedExternalAgents = agents.filter(agent =>
    selected.has(agent.name) && agent.controlMode === 'external' && agent.sessionJsonlPath
  );

  React.useEffect(() => {
    fetch('/api/process/default-spawn-config')
      .then(res => res.ok ? res.json() : Promise.reject(new Error(`HTTP ${res.status}`)))
      .then((data: SpawnConfig) => {
        setConfig(data);
        setStatus({ kind: 'idle' });
      })
      .catch((err: unknown) => {
        setStatus({
          kind: 'error',
          text: `Failed to load spawn defaults: ${err instanceof Error ? err.message : String(err)}`,
        });
      });
  }, []);

  const setField = <K extends keyof SpawnConfig>(field: K, value: SpawnConfig[K]) => {
    setConfig(current => current ? { ...current, [field]: value } : current);
  };

  const cargoSpec = config?.launchSpec.kind === 'cargo' ? config.launchSpec : null;
  const setCargoField = <K extends keyof NonNullable<typeof cargoSpec>>(field: K, value: NonNullable<typeof cargoSpec>[K]) => {
    setConfig(current => {
      if (!current || current.launchSpec.kind !== 'cargo') return current;
      return {
        ...current,
        launchSpec: {
          ...current.launchSpec,
          [field]: value,
        },
      };
    });
  };

  const postSpawn = async (spawnConfig: SpawnConfig, successLabel: string) => {
    const principalId = spawnConfig.principalId.trim();
    if (!principalId) {
      setStatus({ kind: 'error', text: 'principalId is required.' });
      return;
    }

    const body: SpawnConfig = {
      ...spawnConfig,
      principalId,
      model: spawnConfig.model.trim(),
      cwd: spawnConfig.cwd.trim(),
      appfsMountRoot: spawnConfig.appfsMountRoot.trim(),
      permissionMode: spawnConfig.permissionMode.trim() || 'dangerous',
      env: spawnConfig.env ?? {},
      sessionPath: spawnConfig.sessionPath?.trim() || undefined,
    };

    setStatus({ kind: 'loading', text: 'Spawning…' });
    try {
      const res = await fetch('/api/process/spawn', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      const data = await res.json().catch(() => ({}));
      if (!res.ok) {
        throw new Error(data.error ?? `HTTP ${res.status}`);
      }
      setStatus({ kind: 'success', text: `${successLabel}: ${data.spawnId ?? 'pending'}` });
      onSpawnAccepted?.();
    } catch (err: unknown) {
      setStatus({
        kind: 'error',
        text: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const spawnAgent = async () => {
    if (!config) return;
    await postSpawn(config, 'Spawn accepted');
  };

  const resumeAgent = async (agent: AgentInfo) => {
    if (!config) return;
    await postSpawn({
      ...config,
      principalId: agent.principalId || agent.name,
      model: agent.model || config.model,
      sessionPath: agent.sessionJsonlPath,
    }, `Resume accepted for ${agent.name}`);
  };

  return (
    <div className="spawn-agent-panel">
      <button className="spawn-agent-toggle" onClick={() => setExpanded(value => !value)}>
        <span>{expanded ? 'Hide' : 'New Agent'}</span>
        <span className="spawn-agent-toggle-icon">{expanded ? '−' : '+'}</span>
      </button>

      {expanded && (
        <div className="spawn-agent-form">
          <label>
            Principal
            <input
              value={config?.principalId ?? ''}
              onChange={e => setField('principalId', e.target.value)}
              placeholder="code-implementer"
            />
          </label>

          <label>
            Model
            <input
              value={config?.model ?? ''}
              onChange={e => setField('model', e.target.value)}
              placeholder="claude-opus-4-6"
            />
          </label>

          <label>
            Workspace / mount root
            <input
              value={config?.cwd ?? ''}
              onChange={e => {
                setField('cwd', e.target.value);
                setField('appfsMountRoot', e.target.value);
              }}
              placeholder="C:\\mnt\\appfs-compose-tinode"
            />
          </label>

          <label className="spawn-agent-checkbox">
            <input
              type="checkbox"
              checked={config?.appfsIdleWake ?? true}
              onChange={e => setField('appfsIdleWake', e.target.checked)}
            />
            AppFS idle wake
          </label>

          <button className="spawn-agent-advanced" onClick={() => setAdvancedOpen(value => !value)}>
            {advancedOpen ? 'Hide advanced' : 'Advanced launch config'}
          </button>

          {advancedOpen && cargoSpec && (
            <div className="spawn-agent-advanced-fields">
              <label>
                Cargo.toml
                <input
                  value={cargoSpec.manifestPath}
                  onChange={e => setCargoField('manifestPath', e.target.value)}
                />
              </label>
              <label>
                Target dir
                <input
                  value={cargoSpec.targetDir ?? ''}
                  onChange={e => setCargoField('targetDir', e.target.value || undefined)}
                />
              </label>
              <label>
                Package
                <input
                  value={cargoSpec.package}
                  onChange={e => setCargoField('package', e.target.value)}
                />
              </label>
              <label>
                Features
                <input
                  value={(cargoSpec.features ?? []).join(',')}
                  onChange={e => setCargoField(
                    'features',
                    e.target.value.split(',').map(item => item.trim()).filter(Boolean),
                  )}
                />
              </label>
              <label>
                Permission mode
                <input
                  value={config?.permissionMode ?? ''}
                  onChange={e => setField('permissionMode', e.target.value)}
                />
              </label>
            </div>
          )}

          <button
            className="spawn-agent-submit"
            onClick={spawnAgent}
            disabled={!config || status.kind === 'loading'}
          >
            {status.kind === 'loading' ? 'Working…' : 'Spawn Headless Agent'}
          </button>

          {status.text && (
            <div className={`spawn-agent-status ${status.kind}`}>
              {status.text}
            </div>
          )}

          <div className="spawn-agent-log-panel">
            <div className="spawn-agent-resume-title">Startup logs</div>
            {processLogs.length === 0 ? (
              <div className="spawn-log-empty">
                Spawn an agent to see live stdout / stderr here.
              </div>
            ) : (
              <div className="spawn-agent-log-container">
                {processLogs.map(log => (
                  <div
                    key={log.id}
                    className={`spawn-log-item ${log.kind}`}
                    title={[log.agentId, log.spawnId, log.stream].filter(Boolean).join(' · ')}
                  >
                    <div className="spawn-log-header">
                      <span className={`spawn-log-kind ${log.kind}`}>
                        {log.kind === 'log' ? log.stream : log.kind}
                      </span>
                      <span className="spawn-log-id">
                        {log.spawnId}
                      </span>
                    </div>
                    <div>{log.text}</div>
                  </div>
                ))}
              </div>
            )}
          </div>

          {selectedExternalAgents.length > 0 && (
            <div className="spawn-agent-resume">
              <div className="spawn-agent-resume-title">Resume selected external</div>
              {selectedExternalAgents.map(agent => (
                <button
                  key={agent.sessionId}
                  className="spawn-agent-resume-btn"
                  onClick={() => void resumeAgent(agent)}
                  disabled={!config || status.kind === 'loading'}
                  title={agent.sessionJsonlPath}
                >
                  Resume {agent.name}
                </button>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

interface SpawnLogEntry {
  id: string;
  kind: 'log' | 'online' | 'offline';
  agentId?: string;
  spawnId?: string;
  stream?: string;
  text: string;
}
