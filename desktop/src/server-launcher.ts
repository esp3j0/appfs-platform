import electron from 'electron';
import { spawn, ChildProcess } from 'node:child_process';
import path from 'node:path';
import fs from 'node:fs';
import net from 'node:net';
import http from 'node:http';
import kill from 'tree-kill';
import { fileURLToPath } from 'node:url';
import { randomUUID } from 'node:crypto';
import { PersistentLog, resolveDesktopLogDir } from './persistent-log.js';

export function getFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const port = (server.address() as net.AddressInfo).port;
      server.close(() => resolve(port));
    });
  });
}

export class ServerLauncher {
  private serverProcess: ChildProcess | null = null;
  private port: number = 3100;
  private isPackaged: boolean = false;
  private shutdownToken: string = randomUUID();
  private logDir: string;
  private log: PersistentLog;

  constructor(isPackagedOverride?: boolean, logDirOverride?: string) {
    try {
      const app = (electron && typeof electron === 'object' && 'app' in electron)
        ? (electron as any).app
        : null;
      this.isPackaged = isPackagedOverride !== undefined ? isPackagedOverride : (app ? app.isPackaged : false);
    } catch {
      this.isPackaged = isPackagedOverride || false;
    }
    this.logDir = resolveDesktopLogDir(logDirOverride);
    this.log = new PersistentLog(path.join(this.logDir, 'electron-main.log'));
  }

  getPort(): number {
    return this.port;
  }

  getOrigin(): string {
    return `http://127.0.0.1:${this.port}`;
  }

  async launch(): Promise<number> {
    this.port = await getFreePort();
    const platformRoot = this.resolvePlatformRoot();

    const env: Record<string, string | undefined> = {
      ...process.env,
      PORT: String(this.port),
      HOST: '127.0.0.1',
      APPFS_DEBUG_DUMP_DIR: '', // Explicit empty dump dir for empty registry start
      APPFS_CLI_BIN: process.env.APPFS_CLI_BIN || '',
      DASHBOARD_AGENT_BIN: process.env.DASHBOARD_AGENT_BIN || '',
      DASHBOARD_SHUTDOWN_TOKEN: this.shutdownToken,
      APPFS_LOG_DIR: this.logDir,
    };

    let cmd = 'npx';
    let args: string[] = [];
    let cwd = platformRoot;

    if (this.isPackaged) {
      // In packaged electron app, resolve the compiled server file in resources
      const resourcesPath = process.resourcesPath;
      // Run the server from inside app.asar (NOT app.asar.unpacked). Only inside
      // the asar do both dashboard/server/package.json ("type": "module", needed
      // for the ESM `import` syntax) AND node_modules/{fastify,...} live on the
      // same resolution tree. Pointing at the unpacked copy made Node walk up the
      // host filesystem for a type:module package.json + node_modules, which only
      // succeeds when the bundle happens to sit under the repo (desktop/), and
      // fails 100% in a clean install (SyntaxError: Cannot use import statement
      // outside a module).
      const serverPath = path.join(resourcesPath, 'app.asar', 'dashboard', 'server', 'dist', 'index.js');
      
      // Also resolve packaged binary overrides if bundled
      const binaryFolder = path.join(resourcesPath, 'bin');
      const cliBinName = process.platform === 'win32' ? 'agentfs.exe' : 'agentfs';
      const agentBinName = process.platform === 'win32' ? 'claw.exe' : 'claw';
      
      const bundledCliPath = path.join(binaryFolder, cliBinName);
      const bundledAgentPath = path.join(binaryFolder, agentBinName);

      if (fs.existsSync(bundledCliPath)) {
        env.APPFS_CLI_BIN = bundledCliPath;
      }
      if (fs.existsSync(bundledAgentPath)) {
        env.DASHBOARD_AGENT_BIN = bundledAgentPath;
      }

      cmd = process.execPath;
      args = [serverPath];
      env.ELECTRON_RUN_AS_NODE = '1';
      // cwd only needs to be a real, existing directory: the server entry and its
      // dependencies resolve from inside app.asar (above), not from cwd. The
      // previous cwd pointed at app.asar.unpacked/dashboard/server, which no
      // longer exists now that the server runs from app.asar.
      cwd = resourcesPath;
    } else {
      // In development
      cmd = process.platform === 'win32' ? 'npx.cmd' : 'npx';
      args = ['tsx', 'src/index.ts'];
      cwd = path.join(platformRoot, 'dashboard', 'server');
    }

    console.log(`[ServerLauncher] Launching server on port ${this.port} (Profile: ${this.isPackaged ? 'packaged' : 'dev'})`);
    console.log(`[ServerLauncher] cmd=${cmd} args=${args.join(' ')} cwd=${cwd}`);
    console.log(`[ServerLauncher] Logs directory: ${this.logDir}`);
    this.log.appendLine(`[ServerLauncher] Launching server on port ${this.port} profile=${this.isPackaged ? 'packaged' : 'dev'}`);
    this.log.appendLine(`[ServerLauncher] cmd=${cmd} args=${args.join(' ')} cwd=${cwd}`);

    this.serverProcess = spawn(cmd, args, {
      cwd,
      env,
      stdio: 'pipe',
      // Only dev mode needs a shell (to resolve npx.cmd). In packaged mode cmd is
      // process.execPath (an absolute .exe) and we must spawn it WITHOUT a shell:
      // shell:true routes through cmd.exe, which splits the exe path at the first
      // space — fatal when installed under "C:\Program Files\..." ('C:\Program'
      // is not recognized). A direct (no-shell) spawn uses CreateProcess with the
      // exe as lpApplicationName and handles spaces correctly in both exe and args.
      shell: process.platform === 'win32' && !this.isPackaged,
      windowsHide: true,
    });

    this.serverProcess.stdout?.on('data', (data) => {
      console.log(`[Server stdout] ${data.toString().trim()}`);
      this.log.appendChunk('[Server stdout]', data);
    });

    this.serverProcess.stderr?.on('data', (data) => {
      console.error(`[Server stderr] ${data.toString().trim()}`);
      this.log.appendChunk('[Server stderr]', data);
    });

    this.serverProcess.on('exit', (code, signal) => {
      console.log(`[ServerLauncher] Server process exited with code=${code}, signal=${signal}`);
      this.log.appendLine(`[ServerLauncher] Server process exited code=${code} signal=${signal}`);
      this.serverProcess = null;
    });

    await this.waitForReadiness();
    return this.port;
  }

  async stop(): Promise<void> {
    if (!this.serverProcess || !this.serverProcess.pid) {
      return;
    }

    const processToStop = this.serverProcess;
    const pid = processToStop.pid!;
    console.log(`[ServerLauncher] Requesting graceful server shutdown for PID ${pid}`);
    this.log.appendLine(`[ServerLauncher] Requesting graceful server shutdown pid=${pid}`);

    const exited = this.waitForServerExit(processToStop);
    const graceful = await this.requestServerShutdown().catch((err) => {
      console.warn('[ServerLauncher] Graceful shutdown request failed:', err);
      return false;
    });

    if (graceful && await this.waitWithTimeout(exited, 10000)) {
      this.serverProcess = null;
      return;
    }

    console.warn(`[ServerLauncher] Graceful shutdown timed out; stopping server process tree under PID ${pid}`);
    this.log.appendLine(`[ServerLauncher] Graceful shutdown timed out; killing pid=${pid}`);
    return new Promise<void>((resolve) => {
      kill(pid, 'SIGTERM', (err) => {
        if (err) {
          console.error(`[ServerLauncher] Tree-kill error:`, err);
        }
        this.serverProcess = null;
        resolve();
      });
    });
  }

  private requestServerShutdown(): Promise<boolean> {
    return new Promise((resolve, reject) => {
      const req = http.request(`${this.getOrigin()}/api/admin/shutdown`, {
        method: 'POST',
        headers: {
          'x-appfs-shutdown-token': this.shutdownToken,
        },
        timeout: 2000,
      }, (res) => {
        res.resume();
        res.on('end', () => {
          resolve(res.statusCode === 202);
        });
      });

      req.on('timeout', () => {
        req.destroy(new Error('Timed out requesting server shutdown'));
      });
      req.on('error', reject);
      req.end();
    });
  }

  private waitForServerExit(processToStop: ChildProcess): Promise<void> {
    if (processToStop.exitCode !== null || processToStop.signalCode !== null) {
      return Promise.resolve();
    }
    return new Promise(resolve => {
      processToStop.once('exit', () => resolve());
    });
  }

  private waitWithTimeout(promise: Promise<void>, timeoutMs: number): Promise<boolean> {
    return new Promise(resolve => {
      const timer = setTimeout(() => resolve(false), timeoutMs);
      promise.then(() => {
        clearTimeout(timer);
        resolve(true);
      });
    });
  }

  private resolvePlatformRoot(): string {
    const currentFile = fileURLToPath(import.meta.url);
    const currentDir = path.dirname(currentFile);
    let dir = currentDir;
    while (dir && dir !== path.parse(dir).root) {
      if (fs.existsSync(path.join(dir, 'appfs-agent'))) {
        return dir;
      }
      dir = path.dirname(dir);
    }
    return process.cwd();
  }

  private waitForReadiness(): Promise<void> {
    const url = `${this.getOrigin()}/api/projects`;
    const maxAttempts = 50;
    const intervalMs = 150;

    return new Promise((resolve, reject) => {
      let attempts = 0;

      const check = () => {
        attempts++;
        http.get(url, (res) => {
          if (res.statusCode === 200) {
            console.log(`[ServerLauncher] Server verified ready at ${url}`);
            resolve();
          } else {
            retry();
          }
        }).on('error', () => {
          retry();
        });
      };

      const retry = () => {
        if (attempts >= maxAttempts) {
          reject(new Error(`Server failed to start ready check at ${url} after ${maxAttempts} attempts`));
        } else {
          setTimeout(check, intervalMs);
        }
      };

      check();
    });
  }
}
