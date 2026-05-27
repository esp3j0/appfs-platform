import { contextBridge, ipcRenderer } from 'electron';

contextBridge.exposeInMainWorld('appfsShell', {
  chooseFolder: () => ipcRenderer.invoke('choose-folder'),
  getRecentProjects: () => ipcRenderer.invoke('get-recent-projects'),
  removeRecentProject: (projectRoot: string) => ipcRenderer.invoke('remove-recent-project', projectRoot),
  persistSelectedProjectRoot: (projectRoot: string) => ipcRenderer.invoke('persist-selected-project-root', projectRoot),
  getShellMetadata: () => ipcRenderer.invoke('get-shell-metadata'),
});
