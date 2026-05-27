import React, { useEffect, useState } from 'react';

declare global {
  interface Window {
    appfsShell?: {
      chooseFolder(): Promise<string | null>;
      getRecentProjects(): Promise<Array<{ projectRoot: string; displayName: string; lastOpenedAt: number }>>;
      removeRecentProject(projectRoot: string): Promise<Array<{ projectRoot: string; displayName: string; lastOpenedAt: number }>>;
      persistSelectedProjectRoot(projectRoot: string): Promise<Array<{ projectRoot: string; displayName: string; lastOpenedAt: number }>>;
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
          await window.appfsShell.persistSelectedProjectRoot(projectRoot);
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
        <h2>Select an AppFS Project</h2>
        <p className="project-picker-desc">
          AppFS operates inside designated workspace folders containing compose deployment scripts.
          Open a project directory to manage runtimes and agents.
        </p>

        {isElectron ? (
          <button
            type="button"
            className="project-picker-btn"
            onClick={handleChooseFolder}
            disabled={loading}
          >
            {loading ? 'Opening Project…' : 'Open Project Folder…'}
          </button>
        ) : (
          <form onSubmit={handleWebOpen} style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
            <label style={{ fontSize: '11px', color: '#8b949e', textTransform: 'uppercase', fontWeight: 700 }}>
              Project Root Path
              <input
                type="text"
                value={webPathInput}
                onChange={e => setWebPathInput(e.target.value)}
                placeholder="e.g. C:\mnt\appfs-compose-tinode"
                style={{
                  width: '100%',
                  border: '1px solid #30363d',
                  borderRadius: '6px',
                  background: '#0d1117',
                  color: '#c9d1d9',
                  padding: '10px',
                  marginTop: '4px',
                  fontSize: '13px',
                  outline: 'none',
                }}
                disabled={loading}
              />
            </label>
            <button
              type="submit"
              className="project-picker-btn"
              disabled={loading || !webPathInput.trim()}
              style={{ marginTop: '4px' }}
            >
              {loading ? 'Opening Project…' : 'Open Workspace Path'}
            </button>
          </form>
        )}

        {error && <div className="project-picker-error">{error}</div>}

        {isElectron && recents.length > 0 && (
          <div className="recent-projects-section">
            <div className="recent-projects-title">Recent Projects</div>
            <div className="recent-projects-list">
              {recents.map(project => (
                <div
                  key={project.projectRoot}
                  className="recent-project-item"
                  onClick={() => openProject(project.projectRoot)}
                  style={project.isMissing ? { borderColor: '#f8514980', background: '#2a121533' } : undefined}
                >
                  <div className="recent-project-info">
                    <span className="recent-project-name">
                      {project.displayName}
                    </span>
                    <span className="recent-project-path" title={project.projectRoot}>
                      {project.projectRoot}
                    </span>
                    {project.isMissing && (
                      <span style={{ fontSize: '9px', color: '#ff7b72', marginTop: '2px' }}>
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
          </div>
        )}
      </div>
    </div>
  );
}
