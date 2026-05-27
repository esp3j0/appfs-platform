import { spawn, type ChildProcess } from 'node:child_process';

interface TerminateOptions {
  label: string;
  gracefulTimeoutMs?: number;
  forceTimeoutMs?: number;
  closeStdin?: boolean;
}

export async function terminateChildProcessTree(
  child: ChildProcess,
  {
    label,
    gracefulTimeoutMs = 3000,
    forceTimeoutMs = 2000,
    closeStdin = true,
  }: TerminateOptions,
): Promise<void> {
  if (hasExited(child)) {
    return;
  }

  const pid = child.pid;
  const exitPromise = waitForExit(child);

  if (closeStdin) {
    try {
      child.stdin?.end();
    } catch {
      // Best-effort shutdown path.
    }
  }

  try {
    child.kill('SIGTERM');
  } catch (err) {
    console.warn(`[ProcessShutdown] Failed to send SIGTERM to ${label}:`, err);
  }

  if (await waitWithTimeout(exitPromise, gracefulTimeoutMs)) {
    return;
  }

  if (hasExited(child)) {
    return;
  }

  console.warn(`[ProcessShutdown] ${label} did not exit after ${gracefulTimeoutMs}ms; forcing shutdown`);
  if (process.platform === 'win32' && pid) {
    await taskkillTree(pid, label, forceTimeoutMs);
  } else {
    try {
      child.kill('SIGKILL');
    } catch (err) {
      console.warn(`[ProcessShutdown] Failed to send SIGKILL to ${label}:`, err);
    }
  }

  await waitWithTimeout(exitPromise, forceTimeoutMs);
}

function hasExited(child: ChildProcess): boolean {
  return child.exitCode !== null || child.signalCode !== null;
}

function waitForExit(child: ChildProcess): Promise<void> {
  if (hasExited(child)) {
    return Promise.resolve();
  }
  return new Promise(resolve => {
    child.once('exit', () => resolve());
  });
}

function waitWithTimeout(promise: Promise<void>, timeoutMs: number): Promise<boolean> {
  return new Promise(resolve => {
    const timer = setTimeout(() => resolve(false), timeoutMs);
    promise.then(() => {
      clearTimeout(timer);
      resolve(true);
    });
  });
}

function taskkillTree(pid: number, label: string, timeoutMs: number): Promise<void> {
  return new Promise(resolve => {
    const taskkill = spawn('taskkill', ['/PID', String(pid), '/T', '/F'], {
      stdio: 'ignore',
      windowsHide: true,
    });

    const timer = setTimeout(() => {
      try {
        taskkill.kill('SIGKILL');
      } catch {
        // Best-effort cleanup.
      }
      resolve();
    }, timeoutMs);

    taskkill.once('close', () => {
      clearTimeout(timer);
      resolve();
    });
    taskkill.once('error', err => {
      clearTimeout(timer);
      console.warn(`[ProcessShutdown] taskkill failed for ${label}:`, err);
      resolve();
    });
  });
}
