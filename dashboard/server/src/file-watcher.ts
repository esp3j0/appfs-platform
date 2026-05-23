import fs from 'node:fs';
import chokidar from 'chokidar';
import type { AgentRegistry } from './agent-registry.js';
import { parseNewLines } from './jsonl-parser.js';
import type { MessageRecord } from './types.js';

export type FileChangeHandler = (sessionId: string, newRecords: MessageRecord[]) => void;

export class FileWatcher {
  private watcher: ReturnType<typeof chokidar.watch> | null = null;
  private lineCounts = new Map<string, number>();
  private onChangeCallback: FileChangeHandler | null = null;

  constructor(private registry: AgentRegistry) {}

  start(onChange: FileChangeHandler): void {
    this.onChangeCallback = onChange;
    const paths = this.registry.getSessionPaths();

    // Track initial line counts
    for (const p of paths) {
      this.lineCounts.set(p, this.countLines(p));
    }

    // Initialize watcher. Even if paths is empty, we initialize so we can dynamically add later.
    this.watcher = chokidar.watch(paths, {
      ignoreInitial: true,
      persistent: true,
    });

    this.watcher.on('change', (filePath: string) => {
      this.handleFileChange(filePath);
    });
  }

  addPath(filePath: string): void {
    if (!this.watcher) return;
    if (this.lineCounts.has(filePath)) return;

    this.lineCounts.set(filePath, this.countLines(filePath));
    this.watcher.add(filePath);
  }

  removePath(filePath: string): void {
    if (!this.watcher) return;
    this.lineCounts.delete(filePath);
    this.watcher.unwatch(filePath);
  }

  private handleFileChange(filePath: string): void {
    const sessionId = this.findAgentSessionIdByPath(filePath);
    if (!sessionId) return;

    try {
      const content = fs.readFileSync(filePath, 'utf-8');
      const prevLines = this.lineCounts.get(filePath) ?? 0;
      const newRecords = parseNewLines(content, prevLines).filter(
        (r): r is MessageRecord => r.type === 'message',
      );
      this.lineCounts.set(filePath, content.split('\n').length);

      if (newRecords.length > 0) {
        this.registry.reloadAgent(sessionId);
        if (this.onChangeCallback) {
          this.onChangeCallback(sessionId, newRecords);
        }
      }
    } catch (err) {
      console.error(`Error reading watched file ${filePath}:`, err);
    }
  }

  async stop(): Promise<void> {
    if (this.watcher) {
      await this.watcher.close();
      this.watcher = null;
    }
    this.lineCounts.clear();
    this.onChangeCallback = null;
  }

  private countLines(filePath: string): number {
    try {
      const content = fs.readFileSync(filePath, 'utf-8');
      return content.split('\n').length;
    } catch {
      return 0;
    }
  }

  private findAgentSessionIdByPath(filePath: string): string | undefined {
    for (const agent of this.registry.getAgents()) {
      if (agent.sessionJsonlPath === filePath) {
        return agent.sessionId;
      }
    }
    return undefined;
  }
}
