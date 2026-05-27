import React from 'react';

interface Props {
  agentCount: number;
  projectName?: string;
  projectStatus?: string;
  onSwitchProject?: () => void;
  onOpenCompose?: () => void;
  onStartRuntime?: () => void;
  onStopRuntime?: () => void;
  runtimeBusy?: boolean;
  runtimeError?: string | null;
}

export function TopBar({
  agentCount,
  projectName,
  projectStatus,
  onSwitchProject,
  onOpenCompose,
  onStartRuntime,
  onStopRuntime,
  runtimeBusy = false,
  runtimeError,
}: Props) {
  const canStart = projectStatus === 'stopped' || projectStatus === 'error' || !projectStatus;
  const canStop = projectStatus === 'running' || projectStatus === 'starting';

  return (
    <div className="topbar-shell">
      <div className="topbar">
        <div className="topbar-left">
          <h1>AppFS Debug Dashboard</h1>
          {projectName && (
            <div className="project-toolbar">
              <span className="project-toolbar-label">Project:</span>
              <span className="project-toolbar-name">{projectName}</span>
              {projectStatus && (
                <span className={`project-badge ${projectStatus}`}>
                  {projectStatus}
                </span>
              )}
              {onOpenCompose && (
                <button type="button" className="project-toolbar-btn" onClick={onOpenCompose}>
                  Compose
                </button>
              )}
              {onStartRuntime && (
                <button
                  type="button"
                  className="project-toolbar-btn start"
                  onClick={onStartRuntime}
                  disabled={runtimeBusy || !canStart}
                >
                  {runtimeBusy && canStart ? 'Starting...' : 'Start Runtime'}
                </button>
              )}
              {onStopRuntime && (
                <button
                  type="button"
                  className="project-toolbar-btn stop"
                  onClick={onStopRuntime}
                  disabled={runtimeBusy || !canStop}
                >
                  {runtimeBusy && canStop ? 'Stopping...' : 'Stop Runtime'}
                </button>
              )}
              {onSwitchProject && (
                <button type="button" className="project-toolbar-btn switch" onClick={onSwitchProject}>
                  Switch / Recent
                </button>
              )}
            </div>
          )}
        </div>
        <div className="status">
          <span style={{ color: agentCount > 0 ? '#3fb950' : '#484f58' }}>
            {agentCount} agent{agentCount !== 1 ? 's' : ''} online
          </span>
        </div>
      </div>
      {runtimeError && <div className="topbar-runtime-error">{runtimeError}</div>}
    </div>
  );
}
