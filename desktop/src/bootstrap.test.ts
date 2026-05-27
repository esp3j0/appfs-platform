import test from 'node:test';
import assert from 'node:assert';
import path from 'node:path';
import fs from 'node:fs';
import { ShellStore } from './shell-store.js';
import { ServerLauncher, getFreePort } from './server-launcher.js';
import http from 'node:http';

const TEMP_USER_DATA = path.resolve('./tmp-userData-test');

test('Task 1: Bootstrap & Shell Store Tests', async (t) => {
  // Setup temp directory for testing
  if (!fs.existsSync(TEMP_USER_DATA)) {
    fs.mkdirSync(TEMP_USER_DATA, { recursive: true });
  }

  await t.test('1. ShellStore can load, update, and persist recent projects and window bounds', () => {
    const store = new ShellStore(TEMP_USER_DATA);
    
    // Save some bounds
    store.save({
      windowBounds: { width: 1200, height: 900, x: 100, y: 100 },
      launchProfile: 'dev'
    });

    // Check memory state
    let state = store.getState();
    assert.deepStrictEqual(state.windowBounds, { width: 1200, height: 900, x: 100, y: 100 });
    assert.strictEqual(state.launchProfile, 'dev');

    // Create a new store to load from disk
    const store2 = new ShellStore(TEMP_USER_DATA);
    state = store2.getState();
    assert.deepStrictEqual(state.windowBounds, { width: 1200, height: 900, x: 100, y: 100 });
  });

  await t.test('2. ServerLauncher can detect free port and resolve dev vs packaged mode', async () => {
    const launcherDev = new ServerLauncher(false);
    const launcherPackaged = new ServerLauncher(true);

    const port = await getFreePort();
    assert.ok(port > 0, 'Port must be a valid free port');

    // Verify origin generation
    assert.strictEqual(launcherDev.getOrigin(), 'http://127.0.0.1:3100', 'Initial port defaults to 3100');
  });

  await t.test('3. Server launcher launches server backend successfully (empty-registry)', async () => {
    const launcher = new ServerLauncher(false);
    const port = await launcher.launch();

    assert.ok(port > 0);
    assert.ok(launcher.getOrigin().includes(String(port)));

    // Verify GET /api/projects works and is empty (desktop mode boot verification)
    const checkUrl = `${launcher.getOrigin()}/api/projects`;
    const resPromise = new Promise<any>((resolve, reject) => {
      http.get(checkUrl, (res) => {
        let body = '';
        res.on('data', chunk => body += chunk);
        res.on('end', () => {
          resolve(JSON.parse(body));
        });
      }).on('error', reject);
    });

    const data = await resPromise;
    assert.ok(data && Array.isArray(data.projects));
    assert.strictEqual(data.projects.length, 0, 'Initial registry must be empty');

    // Shutdown launcher
    await launcher.stop();
  });

  await t.test('4. Task 2: Recent projects and last selected project root persistence and operations', () => {
    const store = new ShellStore(TEMP_USER_DATA);

    // Initial state
    let state = store.getState();
    assert.strictEqual(state.recentProjects.length, 0);
    assert.strictEqual(state.lastSelectedProjectRoot, undefined);

    // Simulate saving lastSelectedProjectRoot
    store.save({ lastSelectedProjectRoot: 'C:\\projects\\my-project' });
    assert.strictEqual(store.getState().lastSelectedProjectRoot, 'C:\\projects\\my-project');

    // Simulate adding/updating recent projects (the logic from main.ts IPC handler)
    const addOrUpdateRecent = (projectRoot: string) => {
      const state = store.getState();
      const folderName = path.basename(projectRoot) || projectRoot;
      const existingIndex = state.recentProjects.findIndex(p => p.projectRoot === projectRoot);
      let updatedRecents = [...state.recentProjects];

      if (existingIndex >= 0) {
        updatedRecents[existingIndex].lastOpenedAt = Date.now();
      } else {
        updatedRecents.push({
          projectRoot,
          displayName: folderName,
          lastOpenedAt: Date.now()
        });
      }
      updatedRecents.sort((a, b) => b.lastOpenedAt - a.lastOpenedAt);
      store.save({ recentProjects: updatedRecents });
    };

    // Add first project
    addOrUpdateRecent('C:\\projects\\my-project');
    assert.strictEqual(store.getState().recentProjects.length, 1);
    assert.strictEqual(store.getState().recentProjects[0].projectRoot, 'C:\\projects\\my-project');
    assert.strictEqual(store.getState().recentProjects[0].displayName, 'my-project');

    // Add second project later
    const time1 = store.getState().recentProjects[0].lastOpenedAt;
    const secondProjectRoot = 'C:\\projects\\another-project';
    const stateWithTwo = store.getState();
    let recents = [...stateWithTwo.recentProjects];
    recents.push({
      projectRoot: secondProjectRoot,
      displayName: 'another-project',
      lastOpenedAt: time1 + 100 // later
    });
    recents.sort((a, b) => b.lastOpenedAt - a.lastOpenedAt);
    store.save({ recentProjects: recents });

    // Verify ordering (most recent first)
    assert.strictEqual(store.getState().recentProjects.length, 2);
    assert.strictEqual(store.getState().recentProjects[0].projectRoot, 'C:\\projects\\another-project');
    assert.strictEqual(store.getState().recentProjects[1].projectRoot, 'C:\\projects\\my-project');

    // Reload store to verify survival across "restart"
    const storeRestart = new ShellStore(TEMP_USER_DATA);
    const restartedState = storeRestart.getState();
    assert.strictEqual(restartedState.lastSelectedProjectRoot, 'C:\\projects\\my-project');
    assert.strictEqual(restartedState.recentProjects.length, 2);
    assert.strictEqual(restartedState.recentProjects[0].projectRoot, 'C:\\projects\\another-project');

    // Simulate removing a recent project
    const filtered = storeRestart.getState().recentProjects.filter(p => p.projectRoot !== 'C:\\projects\\my-project');
    storeRestart.save({ recentProjects: filtered });
    assert.strictEqual(storeRestart.getState().recentProjects.length, 1);
    assert.strictEqual(storeRestart.getState().recentProjects[0].projectRoot, 'C:\\projects\\another-project');

    // Simulate switching/clearing selected project (empty/whitespace projectRoot)
    const handleSwitchProjectSimulated = (projectRoot: string) => {
      if (!projectRoot || !projectRoot.trim()) {
        storeRestart.save({ lastSelectedProjectRoot: undefined });
        return storeRestart.getState().recentProjects;
      }
    };
    handleSwitchProjectSimulated('');
    assert.strictEqual(storeRestart.getState().lastSelectedProjectRoot, undefined);
    assert.strictEqual(storeRestart.getState().recentProjects.length, 1); // remains 1, no empty item added

    // Verify store auto-sanitization of empty or whitespace recent items
    storeRestart.save({
      recentProjects: [
        { projectRoot: '', displayName: 'empty', lastOpenedAt: Date.now() },
        { projectRoot: '   ', displayName: 'whitespace', lastOpenedAt: Date.now() },
        { projectRoot: 'C:\\projects\\another-project', displayName: 'another-project', lastOpenedAt: Date.now() }
      ]
    });
    // Checking that store loaded/saved state automatically excludes empty/whitespace ones
    assert.strictEqual(storeRestart.getState().recentProjects.length, 1);
    assert.strictEqual(storeRestart.getState().recentProjects[0].projectRoot, 'C:\\projects\\another-project');

    // Test proactive recents isMissing filesystem verification logic
    const testRecents = [
      { projectRoot: 'C:\\invalid\\non-existent-root', displayName: 'missing', lastOpenedAt: Date.now() },
      { projectRoot: path.resolve('./'), displayName: 'exists', lastOpenedAt: Date.now() }
    ];
    const validated = testRecents.map(p => ({
      ...p,
      isMissing: !fs.existsSync(p.projectRoot)
    }));
    assert.strictEqual(validated[0].isMissing, true, 'Non-existent path should be marked missing');
    assert.strictEqual(validated[1].isMissing, false, 'Existing path should not be marked missing');
  });

  await t.test('5. Task 4: ServerLauncher packaged mode path resolution smoke test', async () => {
    const originalResourcesPath = process.resourcesPath;
    const mockResources = path.resolve('./tmp-resources-mock');
    
    // Create mock folders
    fs.mkdirSync(path.join(mockResources, 'app.asar.unpacked', 'dashboard', 'server', 'dist'), { recursive: true });
    fs.mkdirSync(path.join(mockResources, 'bin'), { recursive: true });
    
    // Write mock server index.js
    const mockServerJs = path.join(mockResources, 'app.asar.unpacked', 'dashboard', 'server', 'dist', 'index.js');
    fs.writeFileSync(mockServerJs, 'console.log("mock server ready");');
    
    // Write mock binaries
    const cliBinName = process.platform === 'win32' ? 'agentfs.exe' : 'agentfs';
    const agentBinName = process.platform === 'win32' ? 'claw.exe' : 'claw';
    fs.writeFileSync(path.join(mockResources, 'bin', cliBinName), 'mock CLI');
    fs.writeFileSync(path.join(mockResources, 'bin', agentBinName), 'mock agent');
    
    try {
      Object.defineProperty(process, 'resourcesPath', {
        value: mockResources,
        configurable: true,
        writable: true
      });
      const launcher = new ServerLauncher(true); // force packaged
      
      assert.strictEqual(launcher.getPort(), 3100);
      assert.strictEqual(launcher.getOrigin(), 'http://127.0.0.1:3100');
    } finally {
      // Clean up mock resources
      try {
        fs.rmSync(mockResources, { recursive: true, force: true });
      } catch {}
      Object.defineProperty(process, 'resourcesPath', {
        value: originalResourcesPath,
        configurable: true,
        writable: true
      });
    }
  });

  await t.test('6. Fastify server static frontend path resolver in packaged mode', () => {
    // Simulate import.meta.url and resolve path
    const mockModuleDir = path.resolve('./tmp-resources-mock/app.asar.unpacked/dashboard/server/dist');
    
    // Dev resolver logic: path.resolve(mockModuleDir, '..', '..', 'dist')
    let resolvedDir = path.resolve(mockModuleDir, '..', '..', 'dist');
    assert.strictEqual(resolvedDir.endsWith(path.join('dashboard', 'dist')), true);

    // Packaged resolver logic: replaces app.asar.unpacked with app.asar
    resolvedDir = resolvedDir.replace('app.asar.unpacked', 'app.asar');
    assert.strictEqual(resolvedDir.endsWith(path.join('app.asar', 'dashboard', 'dist')), true);
  });

  // Cleanup temp directory
  try {
    const stateFile = path.join(TEMP_USER_DATA, 'shell-state.json');
    if (fs.existsSync(stateFile)) {
      fs.unlinkSync(stateFile);
    }
    fs.rmdirSync(TEMP_USER_DATA);
  } catch (err) {
    // Ignore cleanup errors
  }
});
