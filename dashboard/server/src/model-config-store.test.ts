import { describe, it, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { ModelConfigStore, type DashboardModelConfig } from './model-config-store.js';

describe('ModelConfigStore', () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'appfs-model-config-'));
  });

  afterEach(() => {
    fs.rmSync(tempDir, { recursive: true, force: true });
  });

  it('creates a default model config when models.json is missing', () => {
    const store = new ModelConfigStore(tempDir);

    const config = store.load();

    assert.strictEqual(config.version, 1);
    assert.ok(config.providers.some(provider => provider.type === 'anthropic'));
    assert.ok(fs.existsSync(path.join(tempDir, 'models.json')));
  });

  it('saves and resolves a selected provider model into runtime config', () => {
    const store = new ModelConfigStore(tempDir);
    const config: DashboardModelConfig = {
      version: 1,
      defaultProviderId: 'gateway',
      defaultModelId: 'qwen-plus',
      providers: [
        {
          id: 'gateway',
          providerName: 'Gateway',
          type: 'openai',
          baseUrl: 'https://gateway.example/v1',
          credential: { mode: 'env', apiKeyEnv: 'GATEWAY_API_KEY' },
          models: [
            {
              id: 'qwen-plus',
              name: 'qwen-plus',
              contextWindowTokens: 131_072,
              maxOutputTokens: 16_384,
            },
          ],
        },
      ],
    };

    store.save(config);
    const resolved = store.resolveSelection({
      providerId: 'gateway',
      modelId: 'qwen-plus',
      contextWindowTokens: 100_000,
    });

    assert.strictEqual(resolved.provider.type, 'openai');
    assert.strictEqual(resolved.provider.apiKeyEnv, 'GATEWAY_API_KEY');
    assert.strictEqual(resolved.provider.baseUrl, 'https://gateway.example/v1');
    assert.strictEqual(resolved.model.name, 'qwen-plus');
    assert.strictEqual(resolved.model.contextWindowTokens, 100_000);
    assert.strictEqual(resolved.model.maxOutputTokens, 16_384);
  });
});

