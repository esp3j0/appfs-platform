import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

export class PersistentLog {
  constructor(public readonly filePath: string) {
    fs.mkdirSync(path.dirname(filePath), { recursive: true });
    this.appendLine(`--- log opened pid=${process.pid} cwd=${process.cwd()} ---`);
  }

  appendLine(message: string): void {
    try {
      fs.appendFileSync(this.filePath, `${new Date().toISOString()} ${message}\n`, 'utf8');
    } catch (err) {
      console.error(`[PersistentLog] Failed to write ${this.filePath}:`, err);
    }
  }

  appendChunk(prefix: string, chunk: Buffer | string): void {
    const text = chunk.toString().replace(/\r\n/g, '\n').replace(/\r/g, '\n');
    const lines = text.split('\n');
    for (const line of lines) {
      if (line.length > 0) {
        this.appendLine(`${prefix} ${line}`);
      }
    }
  }
}

export function resolveDashboardLogDir(logDirOverride?: string): string {
  const configured = logDirOverride?.trim() || process.env.APPFS_LOG_DIR?.trim();
  if (configured) {
    return path.resolve(configured);
  }

  const base = process.env.LOCALAPPDATA
    || process.env.APPDATA
    || path.join(os.homedir(), '.appfs');
  return path.join(base, 'AppFS', 'logs');
}

export function safeLogFileSegment(value: string): string {
  return value
    .trim()
    .replace(/[^A-Za-z0-9_.-]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 128) || 'unknown';
}
