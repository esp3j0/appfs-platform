import fs from 'node:fs';
import path from 'node:path';
import type { FastifyInstance } from 'fastify';
import type { AgentRegistry } from '../agent-registry.js';
import type { AppEventRenderOverridesDoc } from '../types.js';

const OVERRIDES_REL_PATH = path.join('.claw', 'appfs-event-render-overrides.json');

export function registerAppEventOverridesRoute(app: FastifyInstance, registry: AgentRegistry): void {
  app.get('/api/app-event-overrides', async () => {
    const overrides = readOverrides(registry.dumpDirectory);
    const discoveredApps = discoverStaticAppEvents(registry.dumpDirectory);
    return {
      ...overrides,
      discoveredApps,
    };
  });

  app.put('/api/app-event-overrides', async (request, reply) => {
    const body = request.body;
    if (!isOverrideDoc(body)) {
      reply.code(400);
      return { error: 'invalid override document' };
    }

    const written = writeOverrides(registry.dumpDirectory, normalizeOverrides(body));
    const discoveredApps = discoverStaticAppEvents(registry.dumpDirectory);
    return {
      ...written,
      discoveredApps,
    };
  });
}

function discoverStaticAppEvents(dumpDir: string): Record<string, {
  appId: string;
  principalId: string;
  events: Record<string, unknown>;
}> {
  const discovered: Record<string, {
    appId: string;
    principalId: string;
    events: Record<string, unknown>;
  }> = {};

  const privateDir = path.join(dumpDir, 'private');
  if (!fs.existsSync(privateDir)) {
    return discovered;
  }

  try {
    const principals = fs.readdirSync(privateDir);
    for (const principalId of principals) {
      const principalPath = path.join(privateDir, principalId);
      if (!fs.statSync(principalPath).isDirectory()) {
        continue;
      }

      const apps = fs.readdirSync(principalPath);
      for (const appId of apps) {
        const appPath = path.join(principalPath, appId);
        if (!fs.statSync(appPath).isDirectory()) {
          continue;
        }

        const eventsJsonPath = path.join(appPath, '_app', 'events.res.json');
        if (fs.existsSync(eventsJsonPath)) {
          try {
            const content = fs.readFileSync(eventsJsonPath, 'utf-8');
            const parsed = JSON.parse(content);
            if (parsed && typeof parsed === 'object' && parsed.events && typeof parsed.events === 'object') {
              const streamId = `app:${appId}--${principalId}`;
              discovered[streamId] = {
                appId,
                principalId,
                events: parsed.events,
              };
            }
          } catch (e) {
            // Ignore parse errors for individual files
          }
        }
      }
    }
  } catch (e) {
    // Ignore overall dir read errors
  }

  return discovered;
}

function overridesPath(root: string): string {
  return path.join(root, OVERRIDES_REL_PATH);
}

function defaultOverrides(): AppEventRenderOverridesDoc {
  return { version: 1, streams: {} };
}

function readOverrides(root: string): AppEventRenderOverridesDoc {
  const file = overridesPath(root);
  if (!fs.existsSync(file)) {
    return defaultOverrides();
  }
  try {
    const parsed = JSON.parse(fs.readFileSync(file, 'utf-8'));
    if (isOverrideDoc(parsed)) {
      return normalizeOverrides(parsed);
    }
  } catch {
    // Fall through to a safe empty document.
  }
  return defaultOverrides();
}

function writeOverrides(root: string, doc: AppEventRenderOverridesDoc): AppEventRenderOverridesDoc {
  const file = overridesPath(root);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify(doc, null, 2)}\n`, 'utf-8');
  return doc;
}

function normalizeOverrides(doc: AppEventRenderOverridesDoc): AppEventRenderOverridesDoc {
  return {
    version: 1,
    streams: normalizeScopes(doc.streams),
    apps: normalizeScopes(doc.apps),
    platform: normalizeScope(doc.platform),
  };
}

function normalizeScopes(
  scopes: Record<string, unknown> | undefined,
): Record<string, { events: Record<string, unknown> }> | undefined {
  if (!scopes || typeof scopes !== 'object' || Array.isArray(scopes)) {
    return undefined;
  }
  const normalized: Record<string, { events: Record<string, unknown> }> = {};
  for (const [key, value] of Object.entries(scopes)) {
    const scope = normalizeScope(value);
    if (scope) {
      normalized[key] = scope;
    }
  }
  return Object.keys(normalized).length > 0 ? normalized : undefined;
}

function normalizeScope(value: unknown): { events: Record<string, unknown> } | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return undefined;
  }
  const events = (value as { events?: unknown }).events;
  if (!events || typeof events !== 'object' || Array.isArray(events)) {
    return undefined;
  }
  return { events: events as Record<string, unknown> };
}

function isOverrideDoc(value: unknown): value is AppEventRenderOverridesDoc {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return false;
  }
  const doc = value as { version?: unknown; streams?: unknown; apps?: unknown; platform?: unknown };
  if (doc.version !== undefined && typeof doc.version !== 'number') {
    return false;
  }
  return optionalScopesAreValid(doc.streams) &&
    optionalScopesAreValid(doc.apps) &&
    (doc.platform === undefined || normalizeScope(doc.platform) !== undefined);
}

function optionalScopesAreValid(value: unknown): boolean {
  if (value === undefined) {
    return true;
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return false;
  }
  return Object.values(value as Record<string, unknown>).every(scope => normalizeScope(scope) !== undefined);
}
