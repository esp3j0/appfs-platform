import fs from 'node:fs';
import chokidar from 'chokidar';
import type { AgentRegistry } from './agent-registry.js';
import { parseNewLines } from './jsonl-parser.js';
import type { MessageRecord } from './types.js';

export type FileChangeHandler = (agentName: string, newRecords: MessageRecord[]) => void;

export class FileWatcher {
  private watcher: chokidar.FSWatcher | null = null;
  private lineCounts = new Map<string, number>();

  constructor(private registry: AgentRegistry) {}

  start(onChange: FileChangeHandler): void {
    const paths = this.registry.getSessionPaths();
    if (paths.length === 0) return;

    // Track initial line counts
    for (const p of paths) {
      this.lineCounts.set(p, this.countLines(p));
    }

    this.watcher = chokidar.watch(paths, {
      ignoreInitial: true,
      persistent: true,
    });

    this.watcher.on('change', (filePath: string) => {
      const agentName = this.findAgentByPath(filePath);
      if (!agentName) return;

      const content = fs.readFileSync(filePath, 'utf-8');
      const prevLines = this.lineCounts.get(filePath) ?? 0;
      const newRecords = parseNewLines(content, prevLines).filter(
        (r): r is MessageRecord => r.type === 'message',
      );
      this.lineCounts.set(filePath, content.split('\n').length);

      if (newRecords.length > 0) {
        this.registry.reloadAgent(agentName);
        onChange(agentName, newRecords);
      }
    });
  }

  async stop(): Promise<void> {
    if (this.watcher) {
      await this.watcher.close();
    }
  }

  private countLines(filePath: string): number {
    try {
      const content = fs.readFileSync(filePath, 'utf-8');
      return content.split('\n').length;
    } catch {
      return 0;
    }
  }

  private findAgentByPath(filePath: string): string | undefined {
    for (const agent of this.registry.getAgents()) {
      if (agent.sessionJsonlPath === filePath) {
        return agent.name;
      }
    }
    return undefined;
  }
}
