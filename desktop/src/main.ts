import { app, BrowserWindow, dialog, ipcMain, type MessageBoxSyncOptions } from 'electron';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import http from 'node:http';
import fs from 'node:fs';
import { ShellStore, RecentProject } from './shell-store.js';
import { ServerLauncher } from './server-launcher.js';

let mainWindow: BrowserWindow | null = null;
let shellStore: ShellStore;
let serverLauncher: ServerLauncher;
let isQuitting = false;
let closePromptOpen = false;
let cleanupDone = false;
let quitInProgress: Promise<void> | null = null;

const gotTheLock = app.requestSingleInstanceLock();
if (!gotTheLock) {
  app.quit();
} else {
  app.on('second-instance', () => {
    if (mainWindow) {
      if (!mainWindow.isVisible()) mainWindow.show();
      if (mainWindow.isMinimized()) mainWindow.restore();
      mainWindow.focus();
    }
  });

  app.on('ready', async () => {
    shellStore = new ShellStore();
    serverLauncher = new ServerLauncher();

    // Dynamically update profile in state
    shellStore.save({
      launchProfile: app.isPackaged ? 'packaged' : 'dev'
    });

    try {
      await serverLauncher.launch();
    } catch (err) {
      dialog.showErrorBox(
        'Server Launch Failed',
        `Failed to launch the local AppFS server backend:\n${err instanceof Error ? err.message : String(err)}`
      );
      app.quit();
      return;
    }

    createWindow();
  });
}

function createWindow() {
  const state = shellStore.getState();
  const bounds = state.windowBounds || { width: 1024, height: 768 };

  const currentDir = path.dirname(fileURLToPath(import.meta.url));
  const preloadPath = path.join(currentDir, 'preload.js');

  mainWindow = new BrowserWindow({
    width: bounds.width,
    height: bounds.height,
    x: bounds.x,
    y: bounds.y,
    minWidth: 800,
    minHeight: 600,
    backgroundColor: '#0d1117',
    webPreferences: {
      preload: preloadPath,
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false
    }
  });

  // Load the unified HTTP local server URL
  mainWindow.loadURL(serverLauncher.getOrigin());

  mainWindow.on('close', (e) => {
    if (isQuitting && cleanupDone) {
      return;
    }

    e.preventDefault();
    void quitAfterCleanup({ promptForActiveProjects: true });
  });

  mainWindow.on('closed', () => {
    mainWindow = null;
  });

  // Track window resizing and bounds
  const saveBounds = () => {
    if (!mainWindow) return;
    const b = mainWindow.getBounds();
    shellStore.save({ windowBounds: b });
  };
  mainWindow.on('resize', saveBounds);
  mainWindow.on('move', saveBounds);
}

app.on('before-quit', (e) => {
  if (cleanupDone) return;
  e.preventDefault();

  void quitAfterCleanup({ promptForActiveProjects: !isQuitting });
});

async function quitAfterCleanup(options: { promptForActiveProjects: boolean }): Promise<void> {
  if (quitInProgress) {
    return quitInProgress;
  }

  quitInProgress = (async () => {
    if (options.promptForActiveProjects) {
      if (closePromptOpen) {
        return;
      }
      closePromptOpen = true;
    }

    try {
      if (options.promptForActiveProjects) {
        try {
          const activeProjects = await getActiveProjectsFromServer();
          if (activeProjects.length > 0) {
            const messageBoxOptions: MessageBoxSyncOptions = {
              type: 'warning',
              buttons: ['Quit & Stop All', 'Cancel'],
              defaultId: 0,
              cancelId: 1,
              title: 'Project Running',
              message: 'An AppFS project runtime is currently active.',
              detail: 'Quitting the application will stop running projects and managed agents.'
            };
            const choice = mainWindow
              ? dialog.showMessageBoxSync(mainWindow, messageBoxOptions)
              : dialog.showMessageBoxSync(messageBoxOptions);

            if (choice !== 0) {
              return;
            }
          }
        } catch {
          // Proceed to exit if the server is unreachable.
        }
      }

      isQuitting = true;
      cleanupDone = true;

      // Perform clean server process tree shutdown.
      if (serverLauncher) {
        await serverLauncher.stop();
      }

      const windowToClose = mainWindow;
      if (windowToClose && !windowToClose.isDestroyed()) {
        windowToClose.destroy();
      }
      app.quit();
    } finally {
      closePromptOpen = false;
      quitInProgress = null;
    }
  })();

  return quitInProgress;
}

app.on('activate', () => {
  if (mainWindow) {
    mainWindow.show();
  } else {
    createWindow();
  }
});

// IPC Handler Registrations
ipcMain.handle('choose-folder', async () => {
  if (!mainWindow) return null;
  const result = await dialog.showOpenDialog(mainWindow, {
    properties: ['openDirectory']
  });
  if (result.canceled || result.filePaths.length === 0) {
    return null;
  }
  return result.filePaths[0];
});

ipcMain.handle('get-recent-projects', () => {
  return getRecentProjectView();
});

ipcMain.handle('remove-recent-project', (_event, projectRoot: string) => {
  const state = shellStore.getState();
  const targetKey = projectRootKey(projectRoot);
  const filtered = normalizeRecentProjects(state.recentProjects)
    .filter(p => projectRootKey(p.projectRoot) !== targetKey);
  shellStore.save({ recentProjects: filtered });
  return getRecentProjectView();
});

ipcMain.handle('persist-selected-project-root', (_event, projectRoot: string) => {
  if (!projectRoot || !projectRoot.trim()) {
    shellStore.save({ lastSelectedProjectRoot: undefined });
    return getRecentProjectView();
  }

  const normalizedRoot = normalizeProjectRoot(projectRoot);
  shellStore.save({ lastSelectedProjectRoot: normalizedRoot });
  
  // Also push to recent list or update lastOpenedAt
  const state = shellStore.getState();
  const folderName = path.basename(normalizedRoot) || normalizedRoot;
  
  const rootKey = projectRootKey(normalizedRoot);
  const updatedRecents: RecentProject[] = normalizeRecentProjects(state.recentProjects);
  const existingIndex = updatedRecents.findIndex(p => projectRootKey(p.projectRoot) === rootKey);
  
  if (existingIndex >= 0) {
    updatedRecents[existingIndex].projectRoot = normalizedRoot;
    updatedRecents[existingIndex].displayName = folderName;
    updatedRecents[existingIndex].lastOpenedAt = Date.now();
  } else {
    updatedRecents.push({
      projectRoot: normalizedRoot,
      displayName: folderName,
      lastOpenedAt: Date.now()
    });
  }

  // Sort recents by lastOpenedAt descending
  updatedRecents.sort((a, b) => b.lastOpenedAt - a.lastOpenedAt);

  shellStore.save({ recentProjects: updatedRecents });
  return getRecentProjectView();
});

ipcMain.handle('get-shell-metadata', () => {
  const state = shellStore.getState();
  return {
    launchProfile: state.launchProfile,
    serverPort: serverLauncher.getPort(),
    lastSelectedProjectRoot: state.lastSelectedProjectRoot
  };
});

async function getActiveProjectsFromServer(): Promise<any[]> {
  return new Promise<any[]>((resolve) => {
    const url = `${serverLauncher.getOrigin()}/api/projects`;
    http.get(url, (res) => {
      let body = '';
      res.on('data', chunk => body += chunk);
      res.on('end', () => {
        try {
          const data = JSON.parse(body);
          if (data && Array.isArray(data.projects)) {
            const active = data.projects.filter((p: any) => p.status === 'running' || p.status === 'starting');
            resolve(active);
          } else {
            resolve([]);
          }
        } catch {
          resolve([]);
        }
      });
    }).on('error', () => {
      resolve([]);
    });
  });
}

function normalizeProjectRoot(projectRoot: string): string {
  return path.resolve(path.normalize(projectRoot.trim()));
}

function projectRootKey(projectRoot: string): string {
  const normalized = normalizeProjectRoot(projectRoot);
  return process.platform === 'win32' ? normalized.toLowerCase() : normalized;
}

function normalizeRecentProjects(recentProjects: RecentProject[]): RecentProject[] {
  const seen = new Set<string>();
  const normalized: RecentProject[] = [];

  for (const project of recentProjects) {
    if (!project || typeof project.projectRoot !== 'string' || project.projectRoot.trim() === '') {
      continue;
    }
    const projectRoot = normalizeProjectRoot(project.projectRoot);
    const key = projectRootKey(projectRoot);
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    normalized.push({
      ...project,
      projectRoot,
      displayName: project.displayName || path.basename(projectRoot) || projectRoot,
    });
  }

  normalized.sort((a, b) => b.lastOpenedAt - a.lastOpenedAt);
  return normalized;
}

function getRecentProjectView() {
  const normalized = normalizeRecentProjects(shellStore.getState().recentProjects);
  shellStore.save({ recentProjects: normalized });
  return normalized.map(p => ({
    ...p,
    isMissing: !fs.existsSync(p.projectRoot)
  }));
}
