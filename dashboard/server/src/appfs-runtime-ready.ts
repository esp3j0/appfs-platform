import fs from 'node:fs';
import path from 'node:path';

const DEFAULT_RUNTIME_READY_TIMEOUT_MS = 30_000;
const DEFAULT_RUNTIME_READY_POLL_MS = 100;

export interface AppfsRuntimeReadyOptions {
  timeoutMs?: number;
  pollMs?: number;
}

export interface AppfsRuntimeReadyCheck {
  ready: boolean;
  reason?: string;
}

export function appfsRuntimeReadyTimeoutMs(): number {
  return parsePositiveInt(process.env.DASHBOARD_APPFS_RUNTIME_READY_TIMEOUT_MS)
    ?? DEFAULT_RUNTIME_READY_TIMEOUT_MS;
}

export async function waitForAppfsRuntimeReady(
  mountRoot: string,
  options: AppfsRuntimeReadyOptions = {},
): Promise<void> {
  const timeoutMs = options.timeoutMs ?? appfsRuntimeReadyTimeoutMs();
  const pollMs = options.pollMs ?? DEFAULT_RUNTIME_READY_POLL_MS;
  const deadline = Date.now() + timeoutMs;
  let lastReason = 'not checked';

  while (Date.now() <= deadline) {
    const check = checkAppfsRuntimeReady(mountRoot);
    if (check.ready) {
      return;
    }
    lastReason = check.reason ?? lastReason;
    await sleep(pollMs);
  }

  throw new Error(
    `AppFS runtime did not become ready at ${mountRoot} within ${timeoutMs} ms`
    + (lastReason ? `: ${lastReason}` : ''),
  );
}

export function checkAppfsRuntimeReady(mountRoot: string): AppfsRuntimeReadyCheck {
  const runtimeManifestPath = path.join(mountRoot, '.well-known', 'appfs', 'runtime.json');
  const principalsDir = path.join(mountRoot, '_appfs', 'principals');
  const createPrincipalPath = path.join(principalsDir, 'create_principal.act');
  const attachPrincipalPath = path.join(principalsDir, 'attach_principal.act');
  const principalRegistryPath = path.join(mountRoot, '_appfs', 'principals.registry.json');
  const principalStatusPath = path.join(principalsDir, 'status.res.json');

  const runtimeManifest = readJsonObject(runtimeManifestPath, 'runtime manifest');
  if (!runtimeManifest.ok) {
    return runtimeManifest;
  }

  const principalRegistry = readJsonObject(principalRegistryPath, 'principal registry');
  if (!principalRegistry.ok) {
    return principalRegistry;
  }
  if (!Array.isArray(principalRegistry.value.principals)) {
    return {
      ready: false,
      reason: `principal registry ${principalRegistryPath} does not contain a principals array`,
    };
  }

  const principalStatus = readJsonObject(principalStatusPath, 'principal status view');
  if (!principalStatus.ok) {
    return principalStatus;
  }
  if (!Array.isArray(principalStatus.value.principals)) {
    return {
      ready: false,
      reason: `principal status view ${principalStatusPath} does not contain a principals array`,
    };
  }

  const createAccess = checkAccess(createPrincipalPath, fs.constants.R_OK | fs.constants.W_OK, 'create principal action');
  if (!createAccess.ready) {
    return createAccess;
  }

  const attachAccess = checkAccess(attachPrincipalPath, fs.constants.R_OK | fs.constants.W_OK, 'attach principal action');
  if (!attachAccess.ready) {
    return attachAccess;
  }

  return { ready: true };
}

function readJsonObject(
  filePath: string,
  label: string,
): { ok: true; ready: true; value: Record<string, unknown> } | { ok: false; ready: false; reason: string } {
  const access = checkAccess(filePath, fs.constants.R_OK, label);
  if (!access.ready) {
    return {
      ok: false,
      ready: false,
      reason: access.reason ?? `${label} ${filePath} is not accessible`,
    };
  }

  try {
    const value = JSON.parse(fs.readFileSync(filePath, 'utf8'));
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
      return {
        ok: false,
        ready: false,
        reason: `${label} ${filePath} is not a JSON object`,
      };
    }
    return { ok: true, ready: true, value: value as Record<string, unknown> };
  } catch (err: unknown) {
    return {
      ok: false,
      ready: false,
      reason: `failed to read ${label} ${filePath}: ${err instanceof Error ? err.message : String(err)}`,
    };
  }
}

function checkAccess(filePath: string, mode: number, label: string): AppfsRuntimeReadyCheck {
  try {
    fs.accessSync(filePath, mode);
    return { ready: true };
  } catch (err: unknown) {
    return {
      ready: false,
      reason: `${label} ${filePath} is not accessible: ${err instanceof Error ? err.message : String(err)}`,
    };
  }
}

function parsePositiveInt(value: string | undefined): number | null {
  if (!value) {
    return null;
  }
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
