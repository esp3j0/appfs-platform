import { describe, it } from 'node:test';
import assert from 'node:assert';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { checkAppfsRuntimeReady, waitForAppfsRuntimeReady } from './appfs-runtime-ready.js';

describe('AppFS runtime ready checks', () => {
  it('requires runtime manifest, principal views, and principal action sinks', () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'appfs-runtime-ready-'));

    try {
      const mountRoot = path.join(tempDir, '.appfs');
      assert.strictEqual(checkAppfsRuntimeReady(mountRoot).ready, false);

      writeReadyMount(mountRoot);

      assert.deepStrictEqual(checkAppfsRuntimeReady(mountRoot), { ready: true });
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it('waits until AppFS files become visible', async () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'appfs-runtime-ready-wait-'));

    try {
      const mountRoot = path.join(tempDir, '.appfs');
      setTimeout(() => writeReadyMount(mountRoot), 20);

      await waitForAppfsRuntimeReady(mountRoot, {
        timeoutMs: 500,
        pollMs: 10,
      });
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });
});

function writeReadyMount(mountRoot: string): void {
  const runtimeDir = path.join(mountRoot, '.well-known', 'appfs');
  const appfsDir = path.join(mountRoot, '_appfs');
  const principalsDir = path.join(appfsDir, 'principals');
  fs.mkdirSync(runtimeDir, { recursive: true });
  fs.mkdirSync(principalsDir, { recursive: true });
  fs.writeFileSync(path.join(runtimeDir, 'runtime.json'), JSON.stringify({ version: 1 }));
  fs.writeFileSync(
    path.join(appfsDir, 'principals.registry.json'),
    JSON.stringify({ version: 1, default_principal_id: 'default', principals: [] }),
  );
  fs.writeFileSync(
    path.join(principalsDir, 'status.res.json'),
    JSON.stringify({ version: 1, principals: [] }),
  );
  fs.writeFileSync(path.join(principalsDir, 'create_principal.act'), '');
  fs.writeFileSync(path.join(principalsDir, 'attach_principal.act'), '');
}
