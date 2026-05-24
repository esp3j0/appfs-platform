import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';

export interface ProjectRecord {
  projectId: string;
  projectRoot: string;
  composeFilePath: string;
  mountRoot: string;
  status: 'stopped' | 'starting' | 'running' | 'error';
  agentSessionIds: string[];
  managedAgentSessionIds: string[];
}

function generateProjectId(normalizedRoot: string): string {
  return crypto.createHash('sha256').update(normalizedRoot).digest('hex').slice(0, 12);
}

export function checkMountpointConflict(mountRoot: string): void {
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

export function walkProjectDirectory(dir: string): string[] {
  const results: string[] = [];
  if (!fs.existsSync(dir)) return results;

  const entries = fs.readdirSync(dir, { withFileTypes: true });
  for (const entry of entries) {
    const name = entry.name;
    if (name === '.appfs' || name === '.claw') {
      continue;
    }
    const fullPath = path.join(dir, name);
    if (entry.isDirectory()) {
      results.push(...walkProjectDirectory(fullPath));
    } else {
      results.push(fullPath);
    }
  }
  return results;
}

export class ProjectRegistry {
  private projects = new Map<string, ProjectRecord>();

  registerProject(projectRoot: string): ProjectRecord {
    const normalizedRoot = path.resolve(path.normalize(projectRoot));
    const projectId = generateProjectId(normalizedRoot);

    const existing = this.projects.get(projectId);
    if (existing) {
      return existing;
    }

    const mountRoot = path.join(normalizedRoot, '.appfs');

    const record: ProjectRecord = {
      projectId,
      projectRoot: normalizedRoot,
      composeFilePath: path.join(normalizedRoot, '.appfs-compose.yaml'),
      mountRoot,
      status: 'stopped',
      agentSessionIds: [],
      managedAgentSessionIds: [],
    };

    this.projects.set(projectId, record);
    return record;
  }

  getProject(projectId: string): ProjectRecord | undefined {
    return this.projects.get(projectId);
  }

  getProjectByRoot(projectRoot: string): ProjectRecord | undefined {
    const normalizedRoot = path.resolve(path.normalize(projectRoot));
    const projectId = generateProjectId(normalizedRoot);
    return this.projects.get(projectId);
  }

  getProjects(): ProjectRecord[] {
    return Array.from(this.projects.values());
  }

  removeProject(projectId: string): boolean {
    return this.projects.delete(projectId);
  }

  attachAgent(projectId: string, sessionId: string, controlMode: 'managed' | 'external'): boolean {
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
    } else {
      project.managedAgentSessionIds = project.managedAgentSessionIds.filter(
        id => id !== sessionId
      );
    }
    return true;
  }

  detachAgent(projectId: string, sessionId: string): void {
    const project = this.projects.get(projectId);
    if (project) {
      project.agentSessionIds = project.agentSessionIds.filter(
        id => id !== sessionId
      );
      project.managedAgentSessionIds = project.managedAgentSessionIds.filter(
        id => id !== sessionId
      );
    }
  }

  clear(): void {
    this.projects.clear();
  }
}
