import electron from 'electron';
import fs from 'node:fs';
import path from 'node:path';

export interface RecentProject {
  projectRoot: string;
  displayName: string;
  lastOpenedAt: number;
}

export interface ShellState {
  schemaVersion: number;
  recentProjects: RecentProject[];
  lastSelectedProjectRoot?: string;
  windowBounds?: { width: number; height: number; x?: number; y?: number };
  launchProfile: 'dev' | 'packaged';
}

const DEFAULT_STATE: ShellState = {
  schemaVersion: 1,
  recentProjects: [],
  launchProfile: 'dev'
};

export class ShellStore {
  private filePath: string;
  private state: ShellState = { ...DEFAULT_STATE };

  constructor(userDataPath?: string) {
    let userData = '';
    try {
      const app = (electron && typeof electron === 'object' && 'app' in electron)
        ? (electron as any).app
        : null;
      userData = userDataPath || (app ? app.getPath('userData') : '.');
    } catch {
      userData = userDataPath || '.';
    }
    this.filePath = path.join(userData, 'shell-state.json');
    this.load();
  }

  getState(): ShellState {
    return this.state;
  }

  save(updates: Partial<ShellState>): void {
    this.state = {
      ...this.state,
      ...updates
    };
    if (this.state.recentProjects) {
      this.state.recentProjects = this.state.recentProjects.filter(
        p => p && typeof p.projectRoot === 'string' && p.projectRoot.trim() !== ''
      );
    }
    try {
      fs.writeFileSync(this.filePath, JSON.stringify(this.state, null, 2), 'utf-8');
    } catch (err) {
      console.error('[ShellStore] Failed to save state:', err);
    }
  }

  private load(): void {
    try {
      if (fs.existsSync(this.filePath)) {
        const raw = fs.readFileSync(this.filePath, 'utf-8');
        const parsed = JSON.parse(raw);
        if (parsed && parsed.schemaVersion === 1) {
          if (Array.isArray(parsed.recentProjects)) {
            parsed.recentProjects = parsed.recentProjects.filter(
              (p: any) => p && typeof p.projectRoot === 'string' && p.projectRoot.trim() !== ''
            );
          }
          this.state = parsed;
          return;
        }
      }
    } catch (err) {
      console.error('[ShellStore] Failed to load state, reverting to default:', err);
    }
    this.state = { ...DEFAULT_STATE };
  }
}
