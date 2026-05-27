import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

export type ModelProviderType = 'anthropic' | 'openai' | 'xai';

export interface ModelCredentialConfig {
  mode: 'env';
  apiKeyEnv?: string;
  authTokenEnv?: string;
}

export interface ModelCatalogEntry {
  id: string;
  name: string;
  displayName?: string;
  contextWindowTokens: number;
  maxOutputTokens: number;
}

export interface ModelProviderConfig {
  id: string;
  providerName: string;
  type: ModelProviderType;
  baseUrl?: string;
  credential: ModelCredentialConfig;
  models: ModelCatalogEntry[];
}

export interface DashboardModelConfig {
  version: 1;
  defaultProviderId: string;
  defaultModelId: string;
  providers: ModelProviderConfig[];
}

export interface ResolvedRuntimeModelConfig {
  version: 1;
  providerId: string;
  modelId: string;
  provider: {
    type: ModelProviderType;
    providerName: string;
    baseUrl?: string;
    apiKeyEnv?: string;
    authTokenEnv?: string;
  };
  model: {
    name: string;
    displayName?: string;
    contextWindowTokens: number;
    maxOutputTokens: number;
  };
}

export interface ResolveModelSelectionOptions {
  providerId?: string;
  modelId?: string;
  modelName?: string;
  contextWindowTokens?: number;
  maxOutputTokens?: number;
}

export function defaultAppfsPlatformHome(): string {
  return process.env.APPFS_PLATFORM_HOME
    ? path.resolve(process.env.APPFS_PLATFORM_HOME)
    : path.join(os.homedir(), '.appfs-platform');
}

export class ModelConfigValidationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'ModelConfigValidationError';
  }
}

export class ModelConfigStore {
  private configPath: string;
  private runtimeConfigDir: string;

  constructor(private rootDir: string = defaultAppfsPlatformHome()) {
    this.configPath = path.join(rootDir, 'models.json');
    this.runtimeConfigDir = path.join(rootDir, 'runtime', 'model-configs');
  }

  getConfigPath(): string {
    return this.configPath;
  }

  getRuntimeConfigDir(): string {
    return this.runtimeConfigDir;
  }

  load(): DashboardModelConfig {
    if (!fs.existsSync(this.configPath)) {
      const defaults = defaultModelConfig();
      this.save(defaults);
      return defaults;
    }

    const parsed = JSON.parse(fs.readFileSync(this.configPath, 'utf8')) as DashboardModelConfig;
    return normalizeDashboardModelConfig(parsed);
  }

  save(config: DashboardModelConfig): DashboardModelConfig {
    const normalized = normalizeDashboardModelConfig(config);
    fs.mkdirSync(path.dirname(this.configPath), { recursive: true });
    fs.writeFileSync(this.configPath, `${JSON.stringify(normalized, null, 2)}\n`, 'utf8');
    return normalized;
  }

  resolveSelection(options: ResolveModelSelectionOptions = {}): ResolvedRuntimeModelConfig {
    const config = this.load();
    const providerId = options.providerId || config.defaultProviderId;
    const provider = config.providers.find(item => item.id === providerId);
    if (!provider) {
      throw new ModelConfigValidationError(`Model provider not found: ${providerId}`);
    }

    const model = resolveModelEntry(provider, options.modelId, options.modelName || config.defaultModelId);
    const contextWindowTokens = positiveIntegerOr(options.contextWindowTokens, model.contextWindowTokens);
    const maxOutputTokens = positiveIntegerOr(options.maxOutputTokens, model.maxOutputTokens);

    return {
      version: 1,
      providerId: provider.id,
      modelId: model.id,
      provider: {
        type: provider.type,
        providerName: provider.providerName,
        baseUrl: nonEmpty(provider.baseUrl),
        apiKeyEnv: nonEmpty(provider.credential.apiKeyEnv),
        authTokenEnv: nonEmpty(provider.credential.authTokenEnv),
      },
      model: {
        name: options.modelName?.trim() || model.name,
        displayName: nonEmpty(model.displayName),
        contextWindowTokens,
        maxOutputTokens,
      },
    };
  }

  writeRuntimeConfig(config: ResolvedRuntimeModelConfig, label = 'spawn'): string {
    fs.mkdirSync(this.runtimeConfigDir, { recursive: true });
    const safeLabel = label.replace(/[^a-zA-Z0-9_-]/g, '-').slice(0, 80) || 'spawn';
    const filePath = path.join(this.runtimeConfigDir, `${safeLabel}-${Date.now()}.json`);
    fs.writeFileSync(filePath, `${JSON.stringify(config, null, 2)}\n`, 'utf8');
    return filePath;
  }
}

function defaultModelConfig(): DashboardModelConfig {
  return {
    version: 1,
    defaultProviderId: 'anthropic-default',
    defaultModelId: 'claude-opus-4-6',
    providers: [
      {
        id: 'anthropic-default',
        providerName: 'Anthropic',
        type: 'anthropic',
        baseUrl: process.env.ANTHROPIC_BASE_URL || undefined,
        credential: {
          mode: 'env',
          apiKeyEnv: 'ANTHROPIC_API_KEY',
          authTokenEnv: 'ANTHROPIC_AUTH_TOKEN',
        },
        models: [
          {
            id: 'claude-opus-4-6',
            name: 'claude-opus-4-6',
            displayName: 'Claude Opus 4.6',
            contextWindowTokens: 200_000,
            maxOutputTokens: 32_000,
          },
          {
            id: 'claude-sonnet-4-6',
            name: 'claude-sonnet-4-6',
            displayName: 'Claude Sonnet 4.6',
            contextWindowTokens: 200_000,
            maxOutputTokens: 64_000,
          },
        ],
      },
      {
        id: 'openai-default',
        providerName: 'OpenAI Compatible',
        type: 'openai',
        baseUrl: process.env.OPENAI_BASE_URL || undefined,
        credential: {
          mode: 'env',
          apiKeyEnv: 'OPENAI_API_KEY',
        },
        models: [
          {
            id: 'gpt-4.1',
            name: 'gpt-4.1',
            displayName: 'GPT-4.1',
            contextWindowTokens: 1_047_576,
            maxOutputTokens: 32_768,
          },
        ],
      },
    ],
  };
}

function normalizeDashboardModelConfig(config: DashboardModelConfig): DashboardModelConfig {
  if (!config || config.version !== 1) {
    throw new ModelConfigValidationError('models.json must have version 1');
  }
  if (!Array.isArray(config.providers) || config.providers.length === 0) {
    throw new ModelConfigValidationError('models.json must include at least one provider');
  }

  const providers = config.providers.map(normalizeProvider);
  const defaultProviderId = config.defaultProviderId?.trim() || providers[0].id;
  const defaultProvider = providers.find(provider => provider.id === defaultProviderId);
  if (!defaultProvider) {
    throw new ModelConfigValidationError(`defaultProviderId not found: ${defaultProviderId}`);
  }

  const defaultModelId = config.defaultModelId?.trim() || defaultProvider.models[0].id;
  if (!defaultProvider.models.some(model => model.id === defaultModelId || model.name === defaultModelId)) {
    throw new ModelConfigValidationError(`defaultModelId not found in default provider: ${defaultModelId}`);
  }

  return {
    version: 1,
    defaultProviderId,
    defaultModelId,
    providers,
  };
}

function normalizeProvider(provider: ModelProviderConfig): ModelProviderConfig {
  const id = requiredString(provider.id, 'provider.id');
  const providerName = requiredString(provider.providerName, `provider ${id}.providerName`);
  if (!['anthropic', 'openai', 'xai'].includes(provider.type)) {
    throw new ModelConfigValidationError(`provider ${id}.type must be anthropic, openai, or xai`);
  }
  if (!Array.isArray(provider.models) || provider.models.length === 0) {
    throw new ModelConfigValidationError(`provider ${id} must include at least one model`);
  }
  const credential = provider.credential ?? { mode: 'env' as const };
  if (credential.mode !== 'env') {
    throw new ModelConfigValidationError(`provider ${id}.credential.mode must be env`);
  }

  return {
    id,
    providerName,
    type: provider.type,
    baseUrl: nonEmpty(provider.baseUrl),
    credential: {
      mode: 'env',
      apiKeyEnv: nonEmpty(credential.apiKeyEnv),
      authTokenEnv: provider.type === 'anthropic' ? nonEmpty(credential.authTokenEnv) : undefined,
    },
    models: provider.models.map(model => normalizeModel(model, id)),
  };
}

function normalizeModel(model: ModelCatalogEntry, providerId: string): ModelCatalogEntry {
  const id = requiredString(model.id, `provider ${providerId}.models[].id`);
  const name = requiredString(model.name, `provider ${providerId}.models.${id}.name`);
  return {
    id,
    name,
    displayName: nonEmpty(model.displayName),
    contextWindowTokens: positiveInteger(model.contextWindowTokens, `model ${id}.contextWindowTokens`),
    maxOutputTokens: positiveInteger(model.maxOutputTokens, `model ${id}.maxOutputTokens`),
  };
}

function resolveModelEntry(provider: ModelProviderConfig, modelId?: string, modelName?: string): ModelCatalogEntry {
  const trimmedModelId = modelId?.trim();
  const trimmedModelName = modelName?.trim();
  const model = provider.models.find(item =>
    item.id === trimmedModelId ||
    item.name === trimmedModelId ||
    item.id === trimmedModelName ||
    item.name === trimmedModelName
  );
  if (!model) {
    throw new ModelConfigValidationError(
      `Model not found for provider ${provider.id}: ${trimmedModelId || trimmedModelName || '<empty>'}`,
    );
  }
  return model;
}

function requiredString(value: string | undefined, label: string): string {
  const trimmed = value?.trim();
  if (!trimmed) {
    throw new ModelConfigValidationError(`${label} is required`);
  }
  return trimmed;
}

function nonEmpty(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed || undefined;
}

function positiveInteger(value: number | undefined, label: string): number {
  if (!Number.isInteger(value) || (value ?? 0) <= 0) {
    throw new ModelConfigValidationError(`${label} must be a positive integer`);
  }
  return value as number;
}

function positiveIntegerOr(value: number | undefined, fallback: number): number {
  if (value === undefined || value === null) {
    return fallback;
  }
  return positiveInteger(value, 'token override');
}

