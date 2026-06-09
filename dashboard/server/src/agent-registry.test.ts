import { describe, it } from 'node:test';
import assert from 'node:assert';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { AgentRegistry, calculateSessionUsage } from './agent-registry.js';
import { ProjectRegistry } from './project-registry.js';
import type { AgentInfo, CompactionArchiveRecord, ConversationMessage, MessageRecord } from './types.js';

describe('AgentRegistry discovery', () => {
  it('does not downgrade an existing managed agent when rediscovering its session file', () => {
    const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'agent-registry-managed-'));
    const projectRegistry = new ProjectRegistry();
    const project = projectRegistry.registerProject(temp);
    const registry = new AgentRegistry(temp, projectRegistry);
    const sessionPath = path.join(temp, '.claw', 'sessions', 'fingerprint-a', 'session-default.jsonl');

    try {
      fs.mkdirSync(path.dirname(sessionPath), { recursive: true });
      fs.writeFileSync(
        sessionPath,
        [
          JSON.stringify({
            type: 'session_meta',
            version: 1,
            session_id: 'session-default',
            created_at_ms: 1000,
            updated_at_ms: 1000,
            appfs_principal_id: 'default',
            model: 'claude-opus-4-6',
          }),
          JSON.stringify(messageRecord(message('live-1', 42, 7))),
        ].join('\n'),
        'utf8',
      );

      registry.registerAgent({
        name: 'default',
        principalId: 'default',
        sessionId: 'session-default',
        model: 'claude-opus-4-6',
        pid: 321,
        startedAt: 2000,
        sessionJsonlPath: sessionPath,
        status: 'online',
        controlMode: 'managed',
        messageCount: 0,
        totalInputTokens: 0,
        totalOutputTokens: 0,
        projectId: project.projectId,
      });

      registry.discoverProject(temp);

      const rediscovered = registry.getAgent('session-default');
      assert.strictEqual(rediscovered?.status, 'online');
      assert.strictEqual(rediscovered?.controlMode, 'managed');
      assert.strictEqual(rediscovered?.pid, 321);
      assert.strictEqual(rediscovered?.startedAt, 2000);
      assert.strictEqual(rediscovered?.principalId, 'default');
      assert.strictEqual(rediscovered?.projectId, project.projectId);
      assert.strictEqual(rediscovered?.messageCount, 1);
      assert.strictEqual(rediscovered?.totalInputTokens, 42);
      assert.strictEqual(rediscovered?.totalOutputTokens, 7);
    } finally {
      fs.rmSync(temp, { recursive: true, force: true });
    }
  });
});

describe('calculateSessionUsage', () => {
  it('sums live input and output tokens for total usage', () => {
    const usage = calculateSessionUsage([
      messageRecord(message('live-1', 120, 12)),
      messageRecord(message('live-2', 260, 20)),
      messageRecord(message('live-3', 430, 24)),
    ]);

    assert.deepStrictEqual(usage, {
      totalInputTokens: 120 + 260 + 430,
      totalOutputTokens: 56,
      currentContextTokens: 430,
    });
  });

  it('counts cached tokens as effective input when providers split cache usage', () => {
    const usage = calculateSessionUsage([
      messageRecord(message('live-1', 508, 143, { cacheRead: 6080 })),
      messageRecord(message('live-2', 155, 185, { cacheRead: 6784 })),
    ]);

    assert.deepStrictEqual(usage, {
      totalInputTokens: (508 + 6080) + (155 + 6784),
      totalOutputTokens: 143 + 185,
      currentContextTokens: 155 + 6784,
    });
  });

  it('includes compacted archive usage and dedupes messages by uuid', () => {
    const duplicate = message('shared', 999, 40);
    const usage = calculateSessionUsage(
      [
        messageRecord(duplicate),
        messageRecord(message('live-1', 180, 9)),
        messageRecord(message('live-2', 320, 11)),
      ],
      [
        archive(message('archive-1', 100, 3), 1, 10),
        archive(message('archive-2', 220, 5), 1, 10),
        archive(message('archive-3', 80, 7), 2, 20),
        archive(message('archive-4', 140, 13), 2, 20),
        archive(duplicate, 2, 20),
      ],
    );

    assert.deepStrictEqual(usage, {
      totalInputTokens: 100 + 220 + 80 + 140 + 999 + 180 + 320,
      totalOutputTokens: 3 + 5 + 7 + 13 + 40 + 9 + 11,
      currentContextTokens: 320,
    });
  });

  it('uses the latest live input for current context even when archive has duplicates', () => {
    const duplicate = message('shared', 420, 40);
    const usage = calculateSessionUsage(
      [
        messageRecord(message('live-1', 90, 6)),
        messageRecord(duplicate),
      ],
      [
        archive(message('archive-1', 100, 3), 1, 10),
        archive(duplicate, 1, 10),
      ],
    );

    assert.deepStrictEqual(usage, {
      totalInputTokens: 100 + 420 + 90,
      totalOutputTokens: 3 + 40 + 6,
      currentContextTokens: 420,
    });
  });
});

function messageRecord(message: ConversationMessage): MessageRecord {
  return { type: 'message', message };
}

function archive(
  message: ConversationMessage,
  compactionCount: number,
  timestampMs: number,
): CompactionArchiveRecord {
  return {
    type: 'compaction_archive',
    timestamp_ms: timestampMs,
    compaction_count: compactionCount,
    message,
  };
}

function message(
  uuid: string,
  inputTokens: number,
  outputTokens: number,
  options: { cacheCreate?: number; cacheRead?: number } = {},
): ConversationMessage {
  return {
    uuid,
    role: 'assistant',
    blocks: [{ type: 'text', text: uuid }],
    usage: {
      input_tokens: inputTokens,
      output_tokens: outputTokens,
      cache_creation_input_tokens: options.cacheCreate ?? 0,
      cache_read_input_tokens: options.cacheRead ?? 0,
    },
  };
}
