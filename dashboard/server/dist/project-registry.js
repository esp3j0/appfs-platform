import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { projectMountRoot, resolveProjectComposePath } from './compose-policy.js';
function generateProjectId(normalizedRoot) {
    return crypto.createHash('sha256').update(normalizedRoot).digest('hex').slice(0, 12);
}
export function checkMountpointConflict(mountRoot) {
    if (fs.existsSync(mountRoot)) {
        const stats = fs.statSync(mountRoot);
        if (!stats.isDirectory()) {
            throw new Error(`Conflict detected: .appfs exists and is a file at ${mountRoot}`);
        }
        const files = fs.readdirSync(mountRoot);
        if (files.length > 0) {
            throw new Error(`Conflict detected: .appfs directory is not empty at ${mountRoot}`);
        }
    }
}
export function walkProjectDirectory(dir) {
    const results = [];
    if (!fs.existsSync(dir))
        return results;
    const entries = fs.readdirSync(dir, { withFileTypes: true });
    for (const entry of entries) {
        const name = entry.name;
        if (name === '.appfs' || name === '.claw') {
            continue;
        }
        const fullPath = path.join(dir, name);
        if (entry.isDirectory()) {
            results.push(...walkProjectDirectory(fullPath));
        }
        else {
            results.push(fullPath);
        }
    }
    return results;
}
export class ProjectRegistry {
    projects = new Map();
    resolveComposePath(projectRoot) {
        return resolveProjectComposePath(projectRoot);
    }
    registerProject(projectRoot) {
        const normalizedRoot = path.resolve(path.normalize(projectRoot));
        const projectId = generateProjectId(normalizedRoot);
        const existing = this.projects.get(projectId);
        if (existing) {
            existing.composeFilePath = this.resolveComposePath(normalizedRoot);
            return existing;
        }
        const mountRoot = projectMountRoot(normalizedRoot);
        const composeFilePath = this.resolveComposePath(normalizedRoot);
        const record = {
            projectId,
            projectRoot: normalizedRoot,
            composeFilePath,
            mountRoot,
            status: 'stopped',
            agentSessionIds: [],
            managedAgentSessionIds: [],
        };
        this.projects.set(projectId, record);
        return record;
    }
    getProject(projectId) {
        const project = this.projects.get(projectId);
        if (project) {
            project.composeFilePath = this.resolveComposePath(project.projectRoot);
        }
        return project;
    }
    getProjectByRoot(projectRoot) {
        const normalizedRoot = path.resolve(path.normalize(projectRoot));
        const projectId = generateProjectId(normalizedRoot);
        const project = this.projects.get(projectId);
        if (project) {
            project.composeFilePath = this.resolveComposePath(project.projectRoot);
        }
        return project;
    }
    getProjects() {
        const list = Array.from(this.projects.values());
        for (const project of list) {
            project.composeFilePath = this.resolveComposePath(project.projectRoot);
        }
        return list;
    }
    removeProject(projectId) {
        return this.projects.delete(projectId);
    }
    attachAgent(projectId, sessionId, controlMode) {
        const project = this.projects.get(projectId);
        if (!project) {
            return false;
        }
        if (!project.agentSessionIds.includes(sessionId)) {
            project.agentSessionIds.push(sessionId);
        }
        if (controlMode === 'managed') {
            if (!project.managedAgentSessionIds.includes(sessionId)) {
                project.managedAgentSessionIds.push(sessionId);
            }
        }
        else {
            project.managedAgentSessionIds = project.managedAgentSessionIds.filter(id => id !== sessionId);
        }
        return true;
    }
    detachAgent(projectId, sessionId) {
        const project = this.projects.get(projectId);
        if (project) {
            project.agentSessionIds = project.agentSessionIds.filter(id => id !== sessionId);
            project.managedAgentSessionIds = project.managedAgentSessionIds.filter(id => id !== sessionId);
        }
    }
    clear() {
        this.projects.clear();
    }
}
