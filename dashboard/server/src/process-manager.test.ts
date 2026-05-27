import { describe, it } from 'node:test';
import assert from 'node:assert';
import { buildManagedAppfsAttachId } from './process-manager.js';

describe('buildManagedAppfsAttachId', () => {
  it('returns a stable attach id scoped by principal id', () => {
    assert.strictEqual(buildManagedAppfsAttachId('default'), 'dashboard-default');
    assert.strictEqual(buildManagedAppfsAttachId('coder'), 'dashboard-coder');
  });

  it('sanitizes attach ids for AppFS lifecycle actions', () => {
    const attachId = buildManagedAppfsAttachId(' coder/main:1 ');

    assert.strictEqual(attachId, 'dashboard-coder-main-1');
    assert.match(attachId, /^[A-Za-z0-9_.-]+$/);
    assert.ok(attachId.length <= 160);
  });
});
