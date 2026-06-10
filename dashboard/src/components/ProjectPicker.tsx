import React, { useEffect, useState } from 'react';

declare global {
  interface Window {
    appfsShell?: {
      chooseFolder(): Promise<string | null>;
      getRecentProjects(): Promise<RecentProject[]>;
      removeRecentProject(projectRoot: string): Promise<RecentProject[]>;
      persistSelectedProjectRoot(projectRoot: string): Promise<RecentProject[]>;
      getShellMetadata(): Promise<{ launchProfile: 'dev' | 'packaged'; serverPort: number; lastSelectedProjectRoot?: string }>;
    };
  }
}

interface ProjectPickerProps {
  onProjectOpen: (projectId: string, projectRoot: string) => void;
}

interface RecentProject {
  projectRoot: string;
  displayName: string;
  lastOpenedAt: number;
  isMissing?: boolean;
}

export function ProjectPicker({ onProjectOpen }: ProjectPickerProps) {
  const [recents, setRecents] = useState<RecentProject[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState<boolean>(false);
  const [webPathInput, setWebPathInput] = useState<string>('');

  const isElectron = typeof window.appfsShell !== 'undefined';

  const loadRecents = async () => {
    if (isElectron && window.appfsShell) {
      try {
        const list = await window.appfsShell.getRecentProjects();
        setRecents(list || []);
      } catch (err) {
        console.error('Failed to load recent projects:', err);
      }
    }
  };

  useEffect(() => {
    loadRecents();
  }, []);

  const openProject = async (projectRoot: string) => {
    setError(null);
    setLoading(true);

    try {
      const res = await fetch('/api/projects/open', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ projectRoot }),
      });

      const data = await res.json();
      if (!res.ok) {
        throw new Error(data.error || `HTTP ${res.status}`);
      }

      if (data && data.projectId) {
        // Track successfully opened project in ShellStore
        if (isElectron && window.appfsShell) {
          const updated = await window.appfsShell.persistSelectedProjectRoot(data.projectRoot || projectRoot);
          setRecents(updated || []);
        }
        onProjectOpen(data.projectId, data.projectRoot || projectRoot);
      }
    } catch (err: any) {
      setError(err.message || String(err));
      // Mark as missing in recents for visual recovery if it was in recent list
      setRecents(prev =>
        prev.map(p => (p.projectRoot === projectRoot ? { ...p, isMissing: true } : p))
      );
    } finally {
      setLoading(false);
    }
  };

  const handleChooseFolder = async () => {
    if (!isElectron || !window.appfsShell) return;

    try {
      const folderPath = await window.appfsShell.chooseFolder();
      if (folderPath) {
        await openProject(folderPath);
      }
    } catch (err: any) {
      setError(`Native file dialog error: ${err.message}`);
    }
  };

  const handleWebOpen = (e: React.FormEvent) => {
    e.preventDefault();
    const path = webPathInput.trim();
    if (path) {
      openProject(path);
    }
  };

  const handleRemoveRecent = async (e: React.MouseEvent, projectRoot: string) => {
    e.stopPropagation();
    if (isElectron && window.appfsShell) {
      try {
        const updated = await window.appfsShell.removeRecentProject(projectRoot);
        setRecents(updated || []);
      } catch (err) {
        console.error('Failed to remove recent project:', err);
      }
    }
  };

  return (
    <div className="project-picker-overlay">
      <div className="project-picker">
        <div className="project-picker-header">
          <h2>Select an AppFS Project</h2>
          <p className="project-picker-desc">
            Open a workspace folder to manage runtimes, agents, and compose scripts.
          </p>
        </div>

        <div className="project-picker-actions">
          <button
            type="button"
            className="project-picker-btn"
            onClick={isElectron ? handleChooseFolder : () => undefined}
            disabled={loading || !isElectron}
          >
            {loading ? 'Opening Project...' : 'Browse Folder'}
          </button>

          {!isElectron && (
            <form className="project-picker-path-form" onSubmit={handleWebOpen}>
              <label>
                Project Root Path
                <input
                  type="text"
                  value={webPathInput}
                  onChange={e => setWebPathInput(e.target.value)}
                  placeholder="e.g. C:\\Users\\you\\workspace"
                  disabled={loading}
                />
              </label>
              <button
                type="submit"
                className="project-picker-secondary-btn"
                disabled={loading || !webPathInput.trim()}
              >
                Open Path
              </button>
            </form>
          )}
        </div>

        {!isElectron && (
          <div className="project-picker-note">
            Native folder browsing and recent projects are available in the Electron desktop shell.
          </div>
        )}

        {error && <div className="project-picker-error">{error}</div>}

        <div className="recent-projects-section">
          <div className="recent-projects-title">Recent Projects</div>
          {isElectron && recents.length > 0 ? (
            <div className="recent-projects-list">
              {recents.map(project => (
                <div
                  key={project.projectRoot}
                  role="button"
                  tabIndex={0}
                  className={`recent-project-item ${project.isMissing ? 'missing' : ''}`}
                  onClick={() => {
                    if (!loading) {
                      void openProject(project.projectRoot);
                    }
                  }}
                  onKeyDown={event => {
                    if (loading) return;
                    if (event.key === 'Enter' || event.key === ' ') {
                      event.preventDefault();
                      void openProject(project.projectRoot);
                    }
                  }}
                  aria-disabled={loading}
                >
                  <div className="recent-project-info">
                    <span className="recent-project-name">
                      {project.displayName}
                    </span>
                    <span className="recent-project-path" title={project.projectRoot}>
                      {project.projectRoot}
                    </span>
                    {project.isMissing && (
                      <span className="recent-project-warning">
                        Directory not found / unreachable
                      </span>
                    )}
                  </div>
                  <button
                    type="button"
                    className="recent-project-remove"
                    onClick={e => handleRemoveRecent(e, project.projectRoot)}
                    title="Remove from recents"
                  >
                    ✕
                  </button>
                </div>
              ))}
            </div>
          ) : (
            <div className="recent-projects-empty">
              {isElectron
                ? 'No recent projects yet. Choose a workspace folder to start.'
                : 'This browser view can open a typed path only.'}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
