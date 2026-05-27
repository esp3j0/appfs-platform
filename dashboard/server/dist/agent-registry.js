import fs from 'node:fs';
import path from 'node:path';
import { parseCompactionArchives, parseCompactionBoundaries, parseDebugDumps, parseMessages, parseMeta } from './jsonl-parser.js';
export class AgentRegistry {
    agents = new Map();
    messages = new Map();
    debugDumps = new Map();
    compactionArchives = new Map();
    compactionBoundaries = new Map();
    dumpDir;
    fileWatcher = null;
    projectRegistry;
    constructor(dumpDir, projectRegistry) {
        this.dumpDir = dumpDir;
        this.projectRegistry = projectRegistry;
    }
    setFileWatcher(watcher) {
        this.fileWatcher = watcher;
    }
    getFileWatcher() {
        return this.fileWatcher;
    }
    /**
     * Helper to resolve the correct sessionId key from a lookup key
     * which could be a sessionId or legacy agent name.
     */
    resolveSessionId(key) {
        if (this.agents.has(key))
            return key;
        // Fallback: search by name
        const agent = Array.from(this.agents.values()).find(a => a.name === key);
        if (agent)
            return agent.sessionId;
        return null;
    }
    /**
     * External wrapper to perform on-demand directory scan and automatically
     * add any newly found session paths to the active FileWatcher.
     */
    rediscover() {
        this.discover();
    }
    /** Scan a project root for persisted claw sessions. Used by desktop mode,
     * where the dashboard starts with an empty registry and projects are opened
     * after the backend is already running. */
    discoverProject(projectRoot) {
        this.discoverSessionRoot(projectRoot, { flatFallback: false });
    }
    /** Scan for agents. Discovery order:
     *  1. agent-meta-*.json files in dump dir (Phase 3 debug-dump)
     *  2. .claw/sessions/<fingerprint>/*.jsonl (real claw session layout)
     *  3. *.jsonl directly in dump dir (flat fixture mode) */
    discover() {
        if (!fs.existsSync(this.dumpDir))
            return;
        const oldPaths = new Set(this.getSessionPaths());
        // Phase 3 path: agent-meta.json files exist in dump dir
        const metaFiles = fs.readdirSync(this.dumpDir).filter(f => f.startsWith('agent-meta'));
        if (metaFiles.length > 0) {
            for (const file of metaFiles) {
                try {
                    const meta = JSON.parse(fs.readFileSync(path.join(this.dumpDir, file), 'utf-8'));
                    this.registerFromMeta(meta);
                }
                catch {
                    // Skip invalid meta files
                }
            }
        }
        else {
            // Phase 1b: .claw/sessions/<fingerprint>/*.jsonl
            const clawSessionsDir = path.join(this.dumpDir, '.claw', 'sessions');
            if (fs.existsSync(clawSessionsDir)) {
                this.discoverClawSessions(clawSessionsDir);
            }
            // Phase 1a fallback: flat *.jsonl files in dump dir if no sessions were discovered in .claw
            if (this.agents.size === 0) {
                const jsonlFiles = fs.readdirSync(this.dumpDir).filter(f => f.endsWith('.jsonl') && !f.endsWith('.debug.jsonl'));
                for (const file of jsonlFiles) {
                    const fullPath = path.join(this.dumpDir, file);
                    this.registerFromSessionFile(fullPath);
                }
            }
        }
        // Proactively register new paths in the file watcher if it exists
        if (this.fileWatcher) {
            const newPaths = this.getSessionPaths();
            for (const p of newPaths) {
                if (!oldPaths.has(p)) {
                    this.fileWatcher.addPath(p);
                }
            }
        }
    }
    discoverSessionRoot(root, options) {
        if (!root || !fs.existsSync(root))
            return;
        const oldPaths = new Set(this.getSessionPaths());
        const oldAgentCount = this.agents.size;
        const clawSessionsDir = path.join(root, '.claw', 'sessions');
        if (fs.existsSync(clawSessionsDir)) {
            this.discoverClawSessions(clawSessionsDir);
        }
        if (options.flatFallback && this.agents.size === oldAgentCount) {
            let jsonlFiles;
            try {
                jsonlFiles = fs.readdirSync(root).filter(f => f.endsWith('.jsonl') && !f.endsWith('.debug.jsonl'));
            }
            catch {
                jsonlFiles = [];
            }
            for (const file of jsonlFiles) {
                this.registerFromSessionFile(path.join(root, file));
            }
        }
        if (this.fileWatcher) {
            for (const p of this.getSessionPaths()) {
                if (!oldPaths.has(p)) {
                    this.fileWatcher.addPath(p);
                }
            }
        }
    }
    /** Recursively scan .claw/sessions/ for session-*.jsonl files.
     *  Layout: .claw/sessions/<fingerprint>/<session-id>.jsonl */
    discoverClawSessions(sessionsDir) {
        let entries;
        try {
            entries = fs.readdirSync(sessionsDir);
        }
        catch {
            return;
        }
        for (const fingerprint of entries) {
            const fpDir = path.join(sessionsDir, fingerprint);
            if (!fs.statSync(fpDir).isDirectory())
                continue;
            let sessionFiles;
            try {
                sessionFiles = fs.readdirSync(fpDir).filter(f => f.endsWith('.jsonl') && !f.endsWith('.debug.jsonl'));
            }
            catch {
                continue;
            }
            for (const file of sessionFiles) {
                const fullPath = path.join(fpDir, file);
                this.registerFromSessionFile(fullPath);
            }
        }
    }
    registerFromMeta(meta) {
        const sessionId = meta.session_id;
        let sessionContent = '';
        if (meta.session_jsonl_path && fs.existsSync(meta.session_jsonl_path)) {
            sessionContent = fs.readFileSync(meta.session_jsonl_path, 'utf-8');
        }
        const msgs = parseMessages(sessionContent);
        // Look for companion .debug.jsonl
        const debugPath = (meta.session_jsonl_path ?? '').replace(/\.jsonl$/, '.debug.jsonl');
        let dumps = [];
        let archives = [];
        let boundaries = [];
        if (debugPath && fs.existsSync(debugPath)) {
            const debugContent = fs.readFileSync(debugPath, 'utf-8');
            dumps = parseDebugDumps(debugContent);
            archives = parseCompactionArchives(debugContent);
            boundaries = parseCompactionBoundaries(debugContent);
        }
        const name = meta.agent_name;
        const agentInfo = {
            name,
            principalId: meta.principal_id,
            sessionId,
            workspaceFingerprint: workspaceFingerprintFromSessionPath(meta.session_jsonl_path),
            model: meta.model,
            pid: meta.pid,
            startedAt: meta.started_at_ms,
            sessionJsonlPath: meta.session_jsonl_path,
            status: 'online',
            controlMode: 'external', // Managed agents will override this explicitly
            messageCount: msgs.length,
            ...this.sumUsage(msgs),
        };
        this.fillProjectInfo(agentInfo);
        this.agents.set(sessionId, agentInfo);
        this.messages.set(sessionId, msgs);
        this.debugDumps.set(sessionId, dumps);
        this.compactionArchives.set(sessionId, archives);
        this.compactionBoundaries.set(sessionId, boundaries);
    }
    registerFromSessionFile(fullPath) {
        const content = fs.readFileSync(fullPath, 'utf-8');
        const sess = parseMeta(content);
        const msgs = parseMessages(content);
        // Look for companion .debug.jsonl file
        const debugPath = fullPath.replace(/\.jsonl$/, '.debug.jsonl');
        let dumps = [];
        let archives = [];
        let boundaries = [];
        if (fs.existsSync(debugPath)) {
            const debugContent = fs.readFileSync(debugPath, 'utf-8');
            dumps = parseDebugDumps(debugContent);
            archives = parseCompactionArchives(debugContent);
            boundaries = parseCompactionBoundaries(debugContent);
        }
        const principalId = sess?.appfs_principal_id;
        const name = principalId ?? sess?.session_id ?? path.basename(fullPath, '.jsonl');
        const sessionId = sess?.session_id ?? name;
        const agentInfo = {
            name,
            principalId: principalId ?? name,
            sessionId,
            workspaceFingerprint: workspaceFingerprintFromSessionPath(fullPath),
            model: sess?.model ?? 'unknown',
            pid: 0,
            startedAt: sess?.created_at_ms ?? Date.now(),
            sessionJsonlPath: fullPath,
            status: 'offline',
            controlMode: 'external',
            messageCount: msgs.length,
            ...this.sumUsage(msgs),
        };
        this.fillProjectInfo(agentInfo);
        this.agents.set(sessionId, agentInfo);
        this.messages.set(sessionId, msgs);
        this.debugDumps.set(sessionId, dumps);
        this.compactionArchives.set(sessionId, archives);
        this.compactionBoundaries.set(sessionId, boundaries);
    }
    /**
     * Directly register or update an agent in the registry.
     * Useful when spawning a managed agent process.
     */
    registerAgent(agentInfo, msgs) {
        const sessionId = agentInfo.sessionId;
        const normalizedAgentInfo = { ...agentInfo };
        this.fillProjectInfo(normalizedAgentInfo);
        this.agents.set(sessionId, normalizedAgentInfo);
        if (msgs) {
            this.messages.set(sessionId, msgs);
        }
        else if (!this.messages.has(sessionId) && normalizedAgentInfo.sessionJsonlPath && fs.existsSync(normalizedAgentInfo.sessionJsonlPath)) {
            this.reloadAgent(sessionId);
        }
        else if (!this.messages.has(sessionId)) {
            this.messages.set(sessionId, []);
        }
        if (!this.debugDumps.has(sessionId))
            this.debugDumps.set(sessionId, []);
        if (!this.compactionArchives.has(sessionId))
            this.compactionArchives.set(sessionId, []);
        if (!this.compactionBoundaries.has(sessionId))
            this.compactionBoundaries.set(sessionId, []);
        // Dynamically watch the file if a watcher exists
        if (this.fileWatcher && normalizedAgentInfo.sessionJsonlPath) {
            this.fileWatcher.addPath(normalizedAgentInfo.sessionJsonlPath);
        }
    }
    sumUsage(msgs) {
        let totalInputTokens = 0;
        let totalOutputTokens = 0;
        for (const msg of msgs) {
            if (msg.message.usage) {
                totalInputTokens += msg.message.usage.input_tokens;
                totalOutputTokens += msg.message.usage.output_tokens;
            }
        }
        return { totalInputTokens, totalOutputTokens };
    }
    getAgents() {
        for (const sessionId of Array.from(this.agents.keys())) {
            this.reloadAgent(sessionId);
        }
        return Array.from(this.agents.values());
    }
    getAgent(key) {
        const sessionId = this.resolveSessionId(key);
        return sessionId ? this.agents.get(sessionId) : undefined;
    }
    getMessages(key) {
        const sessionId = this.resolveSessionId(key);
        if (sessionId) {
            this.reloadAgent(sessionId);
        }
        return sessionId ? (this.messages.get(sessionId) ?? []) : [];
    }
    getDebugDumps(key) {
        const sessionId = this.resolveSessionId(key);
        if (sessionId) {
            this.reloadAgent(sessionId);
        }
        return sessionId ? (this.debugDumps.get(sessionId) ?? []) : [];
    }
    getCompactionArchives(key) {
        const sessionId = this.resolveSessionId(key);
        if (sessionId) {
            this.reloadAgent(sessionId);
        }
        return sessionId ? (this.compactionArchives.get(sessionId) ?? []) : [];
    }
    getCompactionBoundaries(key) {
        const sessionId = this.resolveSessionId(key);
        if (sessionId) {
            this.reloadAgent(sessionId);
        }
        return sessionId ? (this.compactionBoundaries.get(sessionId) ?? []) : [];
    }
    /** Reload a single agent's messages and debug dumps from its session file. */
    reloadAgent(key) {
        const sessionId = this.resolveSessionId(key);
        if (!sessionId)
            return [];
        const info = this.agents.get(sessionId);
        if (!info || !info.sessionJsonlPath || !fs.existsSync(info.sessionJsonlPath))
            return [];
        try {
            const content = fs.readFileSync(info.sessionJsonlPath, 'utf-8');
            const msgs = parseMessages(content);
            const debugPath = info.sessionJsonlPath.replace(/\.jsonl$/, '.debug.jsonl');
            let dumps = [];
            let archives = [];
            let boundaries = [];
            if (fs.existsSync(debugPath)) {
                const debugContent = fs.readFileSync(debugPath, 'utf-8');
                dumps = parseDebugDumps(debugContent);
                archives = parseCompactionArchives(debugContent);
                boundaries = parseCompactionBoundaries(debugContent);
            }
            this.messages.set(sessionId, msgs);
            this.debugDumps.set(sessionId, dumps);
            this.compactionArchives.set(sessionId, archives);
            this.compactionBoundaries.set(sessionId, boundaries);
            this.agents.set(sessionId, {
                ...info,
                messageCount: msgs.length,
                ...this.sumUsage(msgs),
            });
            return msgs;
        }
        catch (err) {
            console.error(`Error reloading agent ${sessionId}:`, err);
            return [];
        }
    }
    get dumpDirectory() {
        return this.dumpDir;
    }
    getSessionPaths() {
        return Array.from(this.agents.values()).map(a => a.sessionJsonlPath).filter(Boolean);
    }
    fillProjectInfo(agentInfo) {
        const sessionId = agentInfo.sessionId;
        const oldAgent = this.agents.get(sessionId);
        const oldProjectId = oldAgent?.projectId;
        let targetProject = undefined;
        // 1. If projectId exists, check if it exists in projectRegistry
        if (agentInfo.projectId) {
            const proj = this.projectRegistry.getProject(agentInfo.projectId);
            if (proj) {
                targetProject = proj;
            }
        }
        // 2. If targetProject not found, try to infer from projectRoot
        if (!targetProject && agentInfo.projectRoot) {
            try {
                targetProject = this.projectRegistry.registerProject(agentInfo.projectRoot);
            }
            catch {
                targetProject = this.projectRegistry.getProjectByRoot(agentInfo.projectRoot);
            }
        }
        // 3. If targetProject still not found, try to infer from sessionJsonlPath
        if (!targetProject && agentInfo.sessionJsonlPath) {
            const normalizedPath = path.resolve(agentInfo.sessionJsonlPath).replace(/\\/g, '/');
            const clawIndex = normalizedPath.lastIndexOf('/.claw/sessions/');
            if (clawIndex !== -1) {
                const inferredProjectRoot = path.resolve(normalizedPath.substring(0, clawIndex));
                try {
                    targetProject = this.projectRegistry.registerProject(inferredProjectRoot);
                }
                catch {
                    targetProject = this.projectRegistry.getProjectByRoot(inferredProjectRoot);
                }
            }
        }
        // 4. Update the relationship
        if (targetProject) {
            const newProjectId = targetProject.projectId;
            // If project changed, detach from old project
            if (oldProjectId && oldProjectId !== newProjectId) {
                this.projectRegistry.detachAgent(oldProjectId, sessionId);
            }
            agentInfo.projectId = newProjectId;
            agentInfo.projectRoot = targetProject.projectRoot;
            this.projectRegistry.attachAgent(newProjectId, sessionId, agentInfo.controlMode);
        }
        else {
            // If we failed to map, and there was an old project, detach it
            if (oldProjectId) {
                this.projectRegistry.detachAgent(oldProjectId, sessionId);
            }
            // Ensure we don't carry any stale projectId or projectRoot
            delete agentInfo.projectId;
            delete agentInfo.projectRoot;
        }
    }
}
function workspaceFingerprintFromSessionPath(sessionPath) {
    if (!sessionPath)
        return undefined;
    const normalized = sessionPath.replace(/\\/g, '/');
    const parts = normalized.split('/').filter(Boolean);
    const index = parts.lastIndexOf('sessions');
    if (index >= 0 && parts.length > index + 1) {
        return parts[index + 1];
    }
    return undefined;
}
