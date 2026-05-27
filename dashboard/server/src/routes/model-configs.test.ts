import { describe, it, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import Fastify from 'fastify';
import { ModelConfigStore, type DashboardModelConfig } from '../model-config-store.js';
import { registerModelConfigsRoute } from './model-configs.js';

describe('Model Config Routes', () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'appfs-model-route-'));
  });

  afterEach(() => {
    fs.rmSync(tempDir, { recursive: true, force: true });
  });

  it('returns the persisted model config', async () => {
    const app = Fastify({ logger: false });
    const store = new ModelConfigStore(tempDir);
    registerModelConfigsRoute(app, store);

    try {
      const res = await app.inject({ method: 'GET', url: '/api/model-configs' });
      const body = JSON.parse(res.payload);

      assert.strictEqual(res.statusCode, 200);
      assert.strictEqual(body.config.version, 1);
      assert.match(body.path, /models\.json$/);
    } finally {
      await app.close();
    }
  });

  it('updates model config with validation', async () => {
    const app = Fastify({ logger: false });
    const store = new ModelConfigStore(tempDir);
    registerModelConfigsRoute(app, store);

    const payload: DashboardModelConfig = {
      version: 1,
      defaultProviderId: 'local',
      defaultModelId: 'local-model',
      providers: [
        {
          id: 'local',
          providerName: 'Local OpenAI',
          type: 'openai',
          baseUrl: 'http://127.0.0.1:11434/v1',
          credential: { mode: 'env', apiKeyEnv: 'LOCAL_API_KEY' },
          models: [
            {
              id: 'local-model',
              name: 'local-model',
              contextWindowTokens: 32_768,
              maxOutputTokens: 8_192,
            },
          ],
        },
      ],
    };

    try {
      const res = await app.inject({
        method: 'PUT',
        url: '/api/model-configs',
        payload,
      });

      assert.strictEqual(res.statusCode, 200);
      assert.strictEqual(JSON.parse(res.payload).config.defaultProviderId, 'local');
      assert.strictEqual(store.load().providers[0].baseUrl, 'http://127.0.0.1:11434/v1');
    } finally {
      await app.close();
    }
  });
});

