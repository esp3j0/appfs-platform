import React from 'react';
import type { AgentInfo, SpawnConfig } from '../types';
import { AgentItem } from './AgentItem';

interface Props {
  agents: AgentInfo[];
  selected: Set<string>;
  onToggle: (name: string) => void;
  onRefreshAgents?: () => void;
}

export function AgentSidebar({ agents, selected, onToggle, onRefreshAgents }: Props) {
  const totalInput = agents.reduce((s, a) => s + a.totalInputTokens, 0);
  const totalOutput = agents.reduce((s, a) => s + a.totalOutputTokens, 0);
  const maxTokens = Math.max(totalInput, totalOutput, 1);

  return (
    <div className="sidebar">
      <div className="sidebar-header">Agents (multi-select)</div>
      <div className="select-hint">Click to toggle. Select 2+ for merged timeline.</div>
      <div className="agent-list">
        {agents.map((agent, i) => (
          <AgentItem key={agent.sessionId || agent.name} agent={agent} checked={selected.has(agent.name)} colorIndex={i} onToggle={() => onToggle(agent.name)} />
        ))}
      </div>
      <SpawnAgentPanel
        agents={agents}
        selected={selected}
        onSpawnAccepted={onRefreshAgents}
      />
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
