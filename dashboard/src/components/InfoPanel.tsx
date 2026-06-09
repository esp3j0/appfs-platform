import React from 'react';
import type { AgentInfo, CrossAgentInteraction } from '../types';
import { contextUsagePercent, contextUsageTitle } from '../token-usage';

interface MountedApp {
  instance_id: string;
  app_id: string;
  visibility: string;
  principal_id?: string;
  path: string;
}

interface Props {
  agents: AgentInfo[];
  interactions: CrossAgentInteraction[];
}

export function InfoPanel({ agents, interactions }: Props) {
  const [mountedApps, setMountedApps] = React.useState<MountedApp[]>([]);
  const [fetchError, setFetchError] = React.useState<boolean>(false);

  React.useEffect(() => {
    const fetchApps = () => {
      fetch('/api/mounted-apps')
        .then(r => {
          if (!r.ok) throw new Error('API connection error');
          return r.json();
        })
        .then(data => {
          if (data && Array.isArray(data.apps)) {
            setMountedApps(data.apps);
            setFetchError(false);
          } else {
            setFetchError(true);
          }
        })
        .catch(() => {
          setFetchError(true);
        });
    };
    fetchApps();
    const interval = setInterval(fetchApps, 5000);
    return () => clearInterval(interval);
  }, []);

  // Filter and group apps
  // - 'public': Global / shared applications
  // - 'private_instance': Currently used main format for private agent applications
  // - 'private': Transitionary / legacy compatibility value for private agent applications
  const publicApps = mountedApps.filter(app => app.visibility === 'public');

  const selectedPrincipals = new Set(
    agents.map(a => a.principalId || a.name).filter(Boolean)
  );

  const privateApps = mountedApps.filter(app =>
    (app.visibility === 'private_instance' || app.visibility === 'private') &&
    app.principal_id &&
    selectedPrincipals.has(app.principal_id)
  );

  const groupedPrivateApps: Record<string, MountedApp[]> = {};
  for (const app of privateApps) {
    if (app.principal_id) {
      if (!groupedPrivateApps[app.principal_id]) {
        groupedPrivateApps[app.principal_id] = [];
      }
      groupedPrivateApps[app.principal_id].push(app);
    }
  }

  return (
    <div className="info-panel">
      {/* Scrollable Main Content */}
      <div className="info-panel-body">
        {agents.length === 0 ? (
          <div className="info-section">
            <div className="info-title">No agents selected</div>
            <div style={{ color: '#8b949e', fontSize: '11px', fontStyle: 'italic', marginTop: '4px', lineHeight: '1.4' }}>
              Select one or more agents from the sidebar to view detailed metrics and cross-agent event interactions.
            </div>
          </div>
        ) : (
          <>
            {agents.length > 1 && (
              <div className="info-section">
                <div className="info-title">Merged View</div>
                <div className="info-row">
                  <span className="usage-label">Agents</span>
                  <span className="usage-value highlight">{agents.length} selected</span>
                </div>
                <div className="info-row">
                  <span className="usage-label">Interactions</span>
                  <span className="usage-value">{interactions.length} cross-agent</span>
                </div>
              </div>
            )}

            {agents.map(agent => (
              <div className="info-section" key={agent.sessionId}>
                <div className="info-title">{agent.name}</div>
                <div className="info-row">
                  <span className="usage-label">Model</span>
                  <span className="usage-value">{agent.model}</span>
                </div>
                <div className="info-row">
                  <span className="usage-label">Messages</span>
                  <span className="usage-value highlight">{agent.messageCount}</span>
                </div>
                <div className="info-row">
                  <span className="usage-label">Input</span>
                  <span className="usage-value">{agent.totalInputTokens.toLocaleString()} tok</span>
                </div>
                <div className="info-row">
                  <span className="usage-label">Output</span>
                  <span className="usage-value">{agent.totalOutputTokens.toLocaleString()} tok</span>
                </div>
                <div className="info-row">
                  <span className="usage-label">Context</span>
                  <span className="usage-value context-value">
                    <span
                      className="context-usage-dot"
                      style={{
                        ['--context-percent' as string]: `${Math.round(contextUsagePercent(
                          agent.currentContextTokens,
                          agent.contextWindowTokens,
                        ))}%`,
                      }}
                      title={contextUsageTitle(agent.currentContextTokens, agent.contextWindowTokens)}
                      aria-label={contextUsageTitle(agent.currentContextTokens, agent.contextWindowTokens)}
                    />
                    {(agent.currentContextTokens ?? 0).toLocaleString()} tok
                  </span>
                </div>
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
                      {' '}
                      <span style={{ color: '#484f58' }}>({inter.eventType})</span>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </>
        )}
      </div>

      {/* Sticky Bottom Mounted Apps Section */}
      <div className="info-panel-footer">
        <div className="info-title" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: '8px', borderBottom: 'none', paddingBottom: '0' }}>
          <span>Mounted Apps</span>
          {fetchError && (
            <span style={{ fontSize: '9px', color: '#ff7b72', fontWeight: 'normal', textTransform: 'none', letterSpacing: '0px', display: 'inline-flex', alignItems: 'center', gap: '4px' }}>
              <span style={{ width: '6px', height: '6px', borderRadius: '50%', background: '#ff7b72', display: 'inline-block' }}></span>
              stale
            </span>
          )}
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '12px', marginTop: '8px' }}>
          {/* Public Column */}
          <div>
            <div style={{ fontSize: '10px', textTransform: 'uppercase', color: '#8b949e', marginBottom: '6px', fontWeight: 'bold', letterSpacing: '0.5px' }}>
              Public
            </div>
            {publicApps.length === 0 ? (
              <div style={{ fontSize: '11px', color: '#484f58', fontStyle: 'italic', padding: '4px' }}>none</div>
            ) : (
              publicApps.map(app => (
                <div
                  key={app.instance_id}
                  style={{
                    display: 'flex',
                    flexDirection: 'column',
                    padding: '6px 8px',
                    background: '#161b22',
                    border: '1px solid #30363d',
                    borderRadius: '4px',
                    marginBottom: '6px',
                  }}
                >
                  <span style={{ fontSize: '11px', fontWeight: 'bold', color: '#58a6ff' }}>{app.app_id}</span>
                  <span
                    style={{
                      fontSize: '9px',
                      color: '#8b949e',
                      fontFamily: 'monospace',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                      marginTop: '2px',
                    }}
                    title={app.path}
                  >
                    {app.path}
                  </span>
                </div>
              ))
            )}
          </div>

          {/* Private Column */}
          <div>
            <div style={{ fontSize: '10px', textTransform: 'uppercase', color: '#8b949e', marginBottom: '6px', fontWeight: 'bold', letterSpacing: '0.5px' }}>
              Private
            </div>
            {agents.length === 0 ? (
              <div style={{ fontSize: '11px', color: '#8b949e', fontStyle: 'italic', padding: '4px', lineHeight: '1.4' }}>
                Select agent to see private apps
              </div>
            ) : privateApps.length === 0 ? (
              <div style={{ fontSize: '11px', color: '#484f58', fontStyle: 'italic', padding: '4px' }}>none</div>
            ) : (
              Object.entries(groupedPrivateApps).map(([principalId, apps]) => (
                <div key={principalId} style={{ marginBottom: '8px' }}>
                  <div style={{ fontSize: '9px', color: '#ff7b72', marginBottom: '4px', fontFamily: 'monospace', borderBottom: '1px solid #21262d', paddingBottom: '2px' }}>
                    principal: {principalId}
                  </div>
                  {apps.map(app => (
                    <div
                      key={app.instance_id}
                      style={{
                        display: 'flex',
                        flexDirection: 'column',
                        padding: '6px 8px',
                        background: '#161b22',
                        border: '1px solid #30363d',
                        borderRadius: '4px',
                        marginBottom: '4px',
                      }}
                    >
                      <span style={{ fontSize: '11px', fontWeight: 'bold', color: '#3fb950' }}>{app.app_id}</span>
                      <span
                        style={{
                          fontSize: '9px',
                          color: '#8b949e',
                          fontFamily: 'monospace',
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                          marginTop: '2px',
                        }}
                        title={app.path}
                      >
                        {app.path}
                      </span>
                    </div>
                  ))}
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
