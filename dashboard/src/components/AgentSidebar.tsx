import React from 'react';
import type {
  AgentInfo,
  DashboardModelConfig,
  ModelCatalogEntry,
  ModelConfigResponse,
  ModelProviderConfig,
  ProjectRecord,
  SpawnConfig,
} from '../types';
import { AgentItem } from './AgentItem';
import { useDashboardSSE } from '../hooks/useDashboardSSE';

function defaultProvider(config: DashboardModelConfig): ModelProviderConfig | null {
  return config.providers.find(provider => provider.id === config.defaultProviderId)
    ?? config.providers[0]
    ?? null;
}

function defaultModel(provider: ModelProviderConfig, config?: DashboardModelConfig): ModelCatalogEntry | null {
  return provider.models.find(model => model.id === config?.defaultModelId || model.name === config?.defaultModelId)
    ?? provider.models[0]
    ?? null;
}

function applyModelConfigDefaults(spawnConfig: SpawnConfig, modelConfig: DashboardModelConfig): SpawnConfig {
  const globalModelMatch = !spawnConfig.modelProviderId && spawnConfig.model
    ? modelConfig.providers
        .map(provider => ({
          provider,
          model: provider.models.find(model => model.name === spawnConfig.model || model.id === spawnConfig.model),
        }))
        .find(match => Boolean(match.model))
    : undefined;
  const provider = spawnConfig.modelProviderId
    ? modelConfig.providers.find(item => item.id === spawnConfig.modelProviderId)
    : globalModelMatch?.provider ?? defaultProvider(modelConfig);
  const fallbackProvider = provider ?? defaultProvider(modelConfig);
  if (!fallbackProvider) return spawnConfig;

  const exactModelMatch = globalModelMatch?.model
    ?? fallbackProvider.models.find(item => item.name === spawnConfig.model || item.id === spawnConfig.model);
  const model = spawnConfig.modelId
    ? fallbackProvider.models.find(item => item.id === spawnConfig.modelId || item.name === spawnConfig.modelId)
    : exactModelMatch ?? defaultModel(fallbackProvider, modelConfig);
  const fallbackModel = model ?? defaultModel(fallbackProvider, modelConfig);
  if (!fallbackModel) return spawnConfig;
  const explicitModelName = spawnConfig.model?.trim();
  const shouldPreserveCustomModelName = Boolean(explicitModelName && !exactModelMatch && !spawnConfig.modelId);

  return {
    ...spawnConfig,
    modelProviderId: fallbackProvider.id,
    modelId: fallbackModel.id,
    model: shouldPreserveCustomModelName ? explicitModelName : fallbackModel.name,
    contextWindowTokens: spawnConfig.contextWindowTokens ?? fallbackModel.contextWindowTokens,
    maxOutputTokens: spawnConfig.maxOutputTokens ?? fallbackModel.maxOutputTokens,
  };
}

interface Props {
  agents: AgentInfo[];
  selected: Set<string>;
  onToggle: (name: string) => void;
  onRefreshAgents?: () => void;
  onAgentStopped?: (sessionId: string) => void;
  selectedProjectId?: string | null;
  selectedProjectRoot?: string | null;
}

export function AgentSidebar({
  agents,
  selected,
  onToggle,
  onRefreshAgents,
  onAgentStopped,
  selectedProjectId,
  selectedProjectRoot,
}: Props) {
  const [projects, setProjects] = React.useState<ProjectRecord[]>([]);
  const [expandedProjects, setExpandedProjects] = React.useState<Record<string, boolean>>({});
  const [spawnConfig, setSpawnConfig] = React.useState<SpawnConfig | null>(null);
  const [spawnStatus, setSpawnStatus] = React.useState<{ kind: 'idle' | 'loading' | 'success' | 'error'; text?: string }>({ kind: 'loading' });
  const [spawnDefaultsLoaded, setSpawnDefaultsLoaded] = React.useState(false);
  const [modelConfig, setModelConfig] = React.useState<DashboardModelConfig | null>(null);
  const [modelConfigPath, setModelConfigPath] = React.useState('');
  const [modelConfigStatus, setModelConfigStatus] = React.useState<{ kind: 'idle' | 'loading' | 'error'; text?: string }>({ kind: 'loading' });
  const [stoppingAgents, setStoppingAgents] = React.useState<Set<string>>(new Set());

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

  React.useEffect(() => {
    fetch('/api/process/default-spawn-config')
      .then(res => res.ok ? res.json() : Promise.reject(new Error(`HTTP ${res.status}`)))
      .then((data: SpawnConfig) => {
        setSpawnConfig(data);
        setSpawnDefaultsLoaded(true);
        setSpawnStatus(current => current.kind === 'loading' ? { kind: 'idle' } : current);
      })
      .catch((err: unknown) => {
        setSpawnStatus({
          kind: 'error',
          text: `Failed to load spawn defaults: ${err instanceof Error ? err.message : String(err)}`,
        });
      });
  }, []);

  React.useEffect(() => {
    fetch('/api/model-configs')
      .then(res => res.ok ? res.json() : Promise.reject(new Error(`HTTP ${res.status}`)))
      .then((data: ModelConfigResponse) => {
        setModelConfig(data.config);
        setModelConfigPath(data.path || '');
        setModelConfigStatus({ kind: 'idle' });
      })
      .catch((err: unknown) => {
        setModelConfigStatus({
          kind: 'error',
          text: err instanceof Error ? err.message : String(err),
        });
      });
  }, []);

  React.useEffect(() => {
    if (!modelConfig || !spawnDefaultsLoaded) return;
    setSpawnConfig(current => current ? applyModelConfigDefaults(current, modelConfig) : current);
  }, [modelConfig, spawnDefaultsLoaded]);

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

  const postSpawn = async (spawnConfig: SpawnConfig, successLabel: string) => {
    const principalId = spawnConfig.principalId.trim();
    if (!principalId) {
      setSpawnStatus({ kind: 'error', text: 'principalId is required.' });
      return;
    }

    const projectRoot = selectedProjectRoot?.trim();
    const body: SpawnConfig = {
      ...spawnConfig,
      projectId: selectedProjectId || undefined,
      projectRoot: projectRoot || undefined,
      principalId,
      model: spawnConfig.model.trim(),
      cwd: projectRoot || spawnConfig.cwd.trim(),
      appfsMountRoot: projectRoot || spawnConfig.appfsMountRoot.trim(),
      permissionMode: spawnConfig.permissionMode.trim() || 'dangerous',
      env: spawnConfig.env ?? {},
      sessionPath: spawnConfig.sessionPath?.trim() || undefined,
    };

    setSpawnStatus({ kind: 'loading', text: 'Spawning...' });
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
      setSpawnStatus({ kind: 'success', text: `${successLabel}: ${data.spawnId ?? 'pending'}` });
      handleRefresh();
    } catch (err: unknown) {
      setSpawnStatus({
        kind: 'error',
        text: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const resumeAgent = async (agent: AgentInfo) => {
    if (!spawnConfig) return;
    const baseConfig: SpawnConfig = {
      ...spawnConfig,
      principalId: agent.principalId || agent.name,
      model: agent.model || spawnConfig.model,
      sessionPath: agent.sessionJsonlPath,
    };
    const resolvedConfig = modelConfig
      ? applyModelConfigDefaults(
          agent.model
            ? {
                ...baseConfig,
                modelProviderId: undefined,
                modelId: undefined,
                contextWindowTokens: undefined,
                maxOutputTokens: undefined,
              }
            : baseConfig,
          modelConfig,
        )
      : baseConfig;
    await postSpawn(resolvedConfig, `Resume accepted for ${agent.name}`);
  };

  const stopAgent = async (agent: AgentInfo) => {
    if (!agent.sessionId) return;

    setStoppingAgents(prev => new Set(prev).add(agent.sessionId));
    setSpawnStatus({ kind: 'loading', text: `Stopping ${agent.name}...` });
    try {
      const res = await fetch(`/api/agents/${encodeURIComponent(agent.sessionId)}/stop`, {
        method: 'POST',
      });
      const data = await res.json().catch(() => ({}));
      if (!res.ok) {
        throw new Error(data.error ?? `HTTP ${res.status}`);
      }
      setSpawnStatus({ kind: 'success', text: `Stop requested for ${agent.name}` });
      onAgentStopped?.(agent.sessionId);
    } catch (err: unknown) {
      setSpawnStatus({
        kind: 'error',
        text: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setStoppingAgents(prev => {
        const next = new Set(prev);
        next.delete(agent.sessionId);
        return next;
      });
    }
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
                      onResume={resumeAgent}
                      onStop={stopAgent}
                      resumeDisabled={!spawnConfig || spawnStatus.kind === 'loading'}
                      stopDisabled={stoppingAgents.has(agent.sessionId)}
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
                  onResume={resumeAgent}
                  onStop={stopAgent}
                  resumeDisabled={!spawnConfig || spawnStatus.kind === 'loading'}
                  stopDisabled={stoppingAgents.has(agent.sessionId)}
                />
              ))}
            </div>
          </div>
        )}
      </div>

      <SpawnAgentPanel
        onSpawnAccepted={handleRefresh}
        selectedProjectId={selectedProjectId}
        selectedProjectRoot={selectedProjectRoot}
        config={spawnConfig}
        setConfig={setSpawnConfig}
        status={spawnStatus}
        setStatus={setSpawnStatus}
        postSpawn={postSpawn}
        modelConfig={modelConfig}
        modelConfigPath={modelConfigPath}
        modelConfigStatus={modelConfigStatus}
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
  onSpawnAccepted,
  selectedProjectId,
  selectedProjectRoot,
  config,
  setConfig,
  status,
  setStatus,
  postSpawn,
  modelConfig,
  modelConfigPath,
  modelConfigStatus,
}: {
  onSpawnAccepted?: () => void;
  selectedProjectId?: string | null;
  selectedProjectRoot?: string | null;
  config: SpawnConfig | null;
  setConfig: React.Dispatch<React.SetStateAction<SpawnConfig | null>>;
  status: { kind: 'idle' | 'loading' | 'success' | 'error'; text?: string };
  setStatus: React.Dispatch<React.SetStateAction<{ kind: 'idle' | 'loading' | 'success' | 'error'; text?: string }>>;
  postSpawn: (spawnConfig: SpawnConfig, successLabel: string) => Promise<void>;
  modelConfig: DashboardModelConfig | null;
  modelConfigPath: string;
  modelConfigStatus: { kind: 'idle' | 'loading' | 'error'; text?: string };
}) {
  const [expanded, setExpanded] = React.useState(false);
  const [advancedOpen, setAdvancedOpen] = React.useState(false);
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

  const setField = <K extends keyof SpawnConfig>(field: K, value: SpawnConfig[K]) => {
    setConfig(current => current ? { ...current, [field]: value } : current);
  };

  const selectedProvider = React.useMemo(() => {
    if (!modelConfig) return null;
    return modelConfig.providers.find(provider => provider.id === config?.modelProviderId)
      ?? modelConfig.providers.find(provider =>
        provider.models.some(model => model.name === config?.model || model.id === config?.model)
      )
      ?? defaultProvider(modelConfig);
  }, [config?.model, config?.modelProviderId, modelConfig]);

  const selectedModel = React.useMemo(() => {
    if (!selectedProvider) return null;
    return selectedProvider.models.find(model => model.id === config?.modelId || model.name === config?.model)
      ?? defaultModel(selectedProvider, modelConfig ?? undefined);
  }, [config?.model, config?.modelId, modelConfig, selectedProvider]);
  const customModelName = Boolean(
    config?.model?.trim()
      && selectedProvider
      && !selectedProvider.models.some(model => model.id === config.model || model.name === config.model)
  );

  const applyModelSelection = (
    provider: ModelProviderConfig,
    model: ModelCatalogEntry,
    preserveTokenOverrides = false,
  ) => {
    setConfig(current => current ? {
      ...current,
      modelProviderId: provider.id,
      modelId: model.id,
      model: model.name,
      contextWindowTokens: preserveTokenOverrides
        ? (current.contextWindowTokens ?? model.contextWindowTokens)
        : model.contextWindowTokens,
      maxOutputTokens: preserveTokenOverrides
        ? (current.maxOutputTokens ?? model.maxOutputTokens)
        : model.maxOutputTokens,
    } : current);
  };

  const handleProviderChange = (providerId: string) => {
    if (!modelConfig) return;
    const provider = modelConfig.providers.find(item => item.id === providerId);
    if (!provider) return;
    const model = defaultModel(provider, modelConfig);
    if (model) {
      applyModelSelection(provider, model);
    }
  };

  const handleModelChange = (modelId: string) => {
    if (!selectedProvider) return;
    const model = selectedProvider.models.find(item => item.id === modelId || item.name === modelId);
    if (model) {
      applyModelSelection(selectedProvider, model);
    }
  };

  const setPositiveNumberField = (field: 'contextWindowTokens' | 'maxOutputTokens', value: string) => {
    const parsed = Number(value);
    if (Number.isInteger(parsed) && parsed > 0) {
      setField(field, parsed);
    } else if (!value.trim()) {
      setField(field, undefined);
    }
  };

  const cargoSpec = config?.launchSpec.kind === 'cargo' ? config.launchSpec : null;
  const binarySpec = config?.launchSpec.kind === 'binary' ? config.launchSpec : null;
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
  const setBinaryField = <K extends keyof NonNullable<typeof binarySpec>>(field: K, value: NonNullable<typeof binarySpec>[K]) => {
    setConfig(current => {
      if (!current || current.launchSpec.kind !== 'binary') return current;
      return {
        ...current,
        launchSpec: {
          ...current.launchSpec,
          [field]: value,
        },
      };
    });
  };

  const spawnAgent = async () => {
    if (!config) return;
    await postSpawn(config, 'Spawn accepted');
    onSpawnAccepted?.();
  };

  return (
    <div className="spawn-agent-panel">
      <button type="button" className="spawn-agent-toggle" onClick={() => setExpanded(value => !value)}>
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

          {modelConfig ? (
            <>
              <label>
                Provider
                <select
                  value={selectedProvider?.id ?? ''}
                  onChange={e => handleProviderChange(e.target.value)}
                >
                  {modelConfig.providers.map(provider => (
                    <option key={provider.id} value={provider.id}>
                      {provider.providerName}
                    </option>
                  ))}
                </select>
              </label>

              <label>
                Model
                <select
                  value={customModelName ? '__custom__' : selectedModel?.id ?? ''}
                  onChange={e => handleModelChange(e.target.value)}
                >
                  {customModelName && (
                    <option value="__custom__">
                      Custom: {config?.model}
                    </option>
                  )}
                  {(selectedProvider?.models ?? []).map(model => (
                    <option key={model.id} value={model.id}>
                      {model.displayName || model.name}
                    </option>
                  ))}
                </select>
              </label>

              <div className="spawn-agent-token-grid">
                <label>
                  Context
                  <input
                    type="number"
                    min={1}
                    step={1024}
                    value={config?.contextWindowTokens ?? selectedModel?.contextWindowTokens ?? ''}
                    onChange={e => setPositiveNumberField('contextWindowTokens', e.target.value)}
                  />
                </label>
                <label>
                  Max output
                  <input
                    type="number"
                    min={1}
                    step={1024}
                    value={config?.maxOutputTokens ?? selectedModel?.maxOutputTokens ?? ''}
                    onChange={e => setPositiveNumberField('maxOutputTokens', e.target.value)}
                  />
                </label>
              </div>
            </>
          ) : (
            <label>
              Model
              <input
                value={config?.model ?? ''}
                onChange={e => setField('model', e.target.value)}
                placeholder="claude-opus-4-6"
              />
            </label>
          )}

          {modelConfigStatus.kind === 'error' && (
            <div className="spawn-agent-status error">
              Model config unavailable: {modelConfigStatus.text}
            </div>
          )}

          <label>
            Workspace
            <input
              value={selectedProjectRoot || config?.cwd || ''}
              onChange={e => {
                setField('cwd', e.target.value);
                setField('appfsMountRoot', e.target.value);
              }}
              placeholder="C:\\mnt\\appfs-compose-tinode"
              disabled={Boolean(selectedProjectId && selectedProjectRoot)}
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

          <button type="button" className="spawn-agent-advanced" onClick={() => setAdvancedOpen(value => !value)}>
            {advancedOpen ? 'Hide advanced' : 'Advanced launch config'}
          </button>

          {advancedOpen && config && (
            <div className="spawn-agent-advanced-fields">
              <label>
                Model name
                <input
                  value={config.model ?? ''}
                  onChange={e => setField('model', e.target.value)}
                  placeholder="Provider model id"
                />
              </label>
              <label>
                Permission mode
                <input
                  value={config.permissionMode ?? ''}
                  onChange={e => setField('permissionMode', e.target.value)}
                />
              </label>
              <label>
                Session path
                <input
                  value={config.sessionPath ?? ''}
                  onChange={e => setField('sessionPath', e.target.value || undefined)}
                  placeholder="Resume from a saved session file"
                />
              </label>
              <label>
                Model config file
                <input
                  value={modelConfigPath}
                  disabled
                  title={modelConfigPath}
                />
              </label>
              {binarySpec && (
                <label>
                  Binary path
                  <input
                    value={binarySpec.binaryPath}
                    onChange={e => setBinaryField('binaryPath', e.target.value)}
                  />
                </label>
              )}
              {cargoSpec && (
                <>
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
                </>
              )}
            </div>
          )}

          <button
            type="button"
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
