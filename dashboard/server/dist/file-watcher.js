import fs from 'node:fs';
import chokidar from 'chokidar';
import { parseNewLines } from './jsonl-parser.js';
export class FileWatcher {
    registry;
    watcher = null;
    lineCounts = new Map();
    onChangeCallback = null;
    constructor(registry) {
        this.registry = registry;
    }
    start(onChange) {
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
        this.watcher.on('change', (filePath) => {
            this.handleFileChange(filePath);
        });
    }
    addPath(filePath) {
        if (!this.watcher)
            return;
        if (this.lineCounts.has(filePath))
            return;
        this.lineCounts.set(filePath, this.countLines(filePath));
        this.watcher.add(filePath);
    }
    removePath(filePath) {
        if (!this.watcher)
            return;
        this.lineCounts.delete(filePath);
        this.watcher.unwatch(filePath);
    }
    handleFileChange(filePath) {
        const sessionId = this.findAgentSessionIdByPath(filePath);
        if (!sessionId)
            return;
        try {
            const content = fs.readFileSync(filePath, 'utf-8');
            const prevLines = this.lineCounts.get(filePath) ?? 0;
            const newRecords = parseNewLines(content, prevLines).filter((r) => r.type === 'message');
            this.lineCounts.set(filePath, content.split('\n').length);
            if (newRecords.length > 0) {
                this.registry.reloadAgent(sessionId);
                if (this.onChangeCallback) {
                    this.onChangeCallback(sessionId, newRecords);
                }
            }
        }
        catch (err) {
            console.error(`Error reading watched file ${filePath}:`, err);
        }
    }
    async stop() {
        if (this.watcher) {
            await this.watcher.close();
            this.watcher = null;
        }
        this.lineCounts.clear();
        this.onChangeCallback = null;
    }
    countLines(filePath) {
        try {
            const content = fs.readFileSync(filePath, 'utf-8');
            return content.split('\n').length;
        }
        catch {
            return 0;
        }
    }
    findAgentSessionIdByPath(filePath) {
        for (const agent of this.registry.getAgents()) {
            if (agent.sessionJsonlPath === filePath) {
                return agent.sessionId;
            }
        }
        return undefined;
    }
}
