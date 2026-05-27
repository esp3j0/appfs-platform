import type { FastifyInstance } from 'fastify';
import type { ProjectRegistry, ProjectRecord } from '../project-registry.js';
import { checkMountpointConflict } from '../project-registry.js';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ensureProjectComposeMountpoint, normalizeComposeMountpointContent } from '../compose-policy.js';
import type { AgentRegistry } from '../agent-registry.js';
import type { AgentProcessManager, ProjectAgentResumeResult } from '../process-manager.js';

function resolvePlatformRoot(): string {
  const moduleDir = path.dirname(fileURLToPath(import.meta.url));
  const candidates = [
    path.resolve(moduleDir, '..', '..', '..'),
    path.resolve(process.cwd(), '..', '..'),
    process.cwd(),
  ];
  for (const candidate of candidates) {
    if (fs.existsSync(path.join(candidate, 'appfs-agent', 'rust', 'Cargo.toml'))) {
      return candidate;
    }
  }
  return process.cwd();
}

function validateComposeContent(projectRoot: string, content: string): Promise<{ valid: boolean; error?: string }> {
  return new Promise(async (resolve) => {
    const tempFileName = `.appfs-compose-candidate-${crypto.randomUUID().slice(0, 8)}.tmp.yaml`;
    const tempFilePath = path.join(projectRoot, tempFileName);

    try {
      fs.writeFileSync(tempFilePath, content, 'utf-8');
    } catch (err: any) {
      return resolve({ valid: false, error: `Failed to write temporary validation file: ${err.message}` });
    }

    const platformRoot = resolvePlatformRoot();
    let cmd = 'cargo';
    let args = [
      'run',
      '--manifest-path',
      path.join(platformRoot, 'appfs', 'cli', 'Cargo.toml'),
      '--',
      'appfs',
      'compose',
      'validate',
      '-f',
      tempFileName
    ];

    if (process.env.APPFS_CLI_BIN) {
      cmd = process.env.APPFS_CLI_BIN;
      args = ['appfs', 'compose', 'validate', '-f', tempFileName];
    }

    const child = spawn(cmd, args, {
      cwd: projectRoot,
      env: { ...process.env },
      windowsHide: true,
    });

    let stderr = '';
    let stdout = '';

    child.stderr?.on('data', chunk => stderr += chunk);
    child.stdout?.on('data', chunk => stdout += chunk);

    child.on('close', (code) => {
      try {
        if (fs.existsSync(tempFilePath)) {
          fs.unlinkSync(tempFilePath);
        }
      } catch (err) {
        console.error('Failed to cleanup temp validation file:', err);
      }

      if (code === 0) {
        resolve({ valid: true });
      } else {
        const errorMsg = stderr.trim() || stdout.trim() || `Validation exited with code ${code}`;
        resolve({ valid: false, error: errorMsg });
      }
    });

    child.on('error', (err) => {
      try {
        if (fs.existsSync(tempFilePath)) {
          fs.unlinkSync(tempFilePath);
        }
      } catch (cleanupErr) {
        console.error('Failed to cleanup temp validation file on spawn error:', cleanupErr);
      }

      resolve({ valid: false, error: `Failed to spawn validator: ${err.message}` });
    });
  });
}

export interface ProjectRuntimeController {
  start(projectId: string): Promise<ProjectRecord>;
  stop(projectId: string): Promise<ProjectRecord>;
  status(projectId: string): ProjectRecord | undefined;
}

export interface ProjectBootstrapResult {
  project: ProjectRecord;
  runtime: { status: 'started' | 'already-running' | 'skipped' | 'error'; error?: string };
  resume: ProjectAgentResumeResult;
}

interface ProjectRouteDependencies {
  agentRegistry?: AgentRegistry;
  processManager?: AgentProcessManager;
}

export function registerProjectsRoute(
  app: FastifyInstance,
  projectRegistry: ProjectRegistry,
  runtimeController: ProjectRuntimeController,
  deps: ProjectRouteDependencies = {},
): void {
  // GET /api/projects - list all registered projects
  app.get('/api/projects', async () => {
    return {
      projects: projectRegistry.getProjects(),
    };
  });

  // POST /api/projects/open - open/register a project root
  app.post<{ Body: { projectRoot: string } }>('/api/projects/open', async (request, reply) => {
    const { projectRoot } = request.body || {};
    if (!projectRoot) {
      return reply.status(400).send({ error: 'Missing projectRoot parameter' });
    }

    if (!fs.existsSync(projectRoot)) {
      return reply.status(400).send({ error: `Project root directory does not exist: ${projectRoot}` });
    }

    try {
      ensureProjectComposeMountpoint(projectRoot);
      const record = projectRegistry.registerProject(projectRoot);
      deps.agentRegistry?.discoverProject(record.projectRoot);
      return record;
    } catch (err: any) {
      return reply.status(400).send({ error: err.message });
    }
  });

  // GET /api/projects/:projectId - get project status/info
  app.get<{ Params: { projectId: string } }>('/api/projects/:projectId', async (request, reply) => {
    const { projectId } = request.params;
    const project = projectRegistry.getProject(projectId);
    if (!project) {
      return reply.status(404).send({ error: 'Project not found' });
    }
    return project;
  });

  // GET /api/projects/:projectId/status - get project status/info specifically
  app.get<{ Params: { projectId: string } }>('/api/projects/:projectId/status', async (request, reply) => {
    const { projectId } = request.params;
    const project = projectRegistry.getProject(projectId);
    if (!project) {
      return reply.status(404).send({ error: 'Project not found' });
    }
    return project;
  });

  // POST /api/projects/:projectId/start - start project
  app.post<{ Params: { projectId: string } }>('/api/projects/:projectId/start', async (request, reply) => {
    const { projectId } = request.params;
    const project = projectRegistry.getProject(projectId);
    if (!project) {
      return reply.status(404).send({ error: 'Project not found' });
    }

    if (!fs.existsSync(project.composeFilePath)) {
      return reply.status(400).send({ error: `Missing compose file: ${project.composeFilePath}` });
    }

    try {
      checkMountpointConflict(project.mountRoot);
    } catch (err: any) {
      return reply.status(400).send({ error: `Conflict detected: ${err.message}` });
    }

    project.status = 'starting';

    try {
      const updated = await runtimeController.start(projectId);
      return updated;
    } catch (err: any) {
      project.status = 'error';
      return reply.status(500).send({ error: err.message });
    }
  });

  // POST /api/projects/:projectId/bootstrap - start runtime and resume persisted sessions
  app.post<{ Params: { projectId: string } }>('/api/projects/:projectId/bootstrap', async (request, reply) => {
    const { projectId } = request.params;
    const project = projectRegistry.getProject(projectId);
    if (!project) {
      return reply.status(404).send({ error: 'Project not found' });
    }

    deps.agentRegistry?.discoverProject(project.projectRoot);

    const resumeEmpty: ProjectAgentResumeResult = {
      resumed: [],
      skipped: [],
      errors: [],
    };
    const result: ProjectBootstrapResult = {
      project,
      runtime: { status: 'skipped' },
      resume: resumeEmpty,
    };

    if (project.status === 'running' || project.status === 'starting') {
      result.runtime = { status: 'already-running' };
    } else if (!fs.existsSync(project.composeFilePath)) {
      result.runtime = {
        status: 'skipped',
        error: `Missing compose file: ${project.composeFilePath}`,
      };
    } else {
      try {
        checkMountpointConflict(project.mountRoot);
        project.status = 'starting';
        result.project = await runtimeController.start(projectId);
        result.runtime = { status: 'started' };
      } catch (err: any) {
        project.status = 'error';
        result.project = project;
        result.runtime = {
          status: 'error',
          error: err.message || String(err),
        };
      }
    }

    deps.agentRegistry?.discoverProject(project.projectRoot);
    if (
      deps.processManager &&
      (result.runtime.status === 'started' || result.runtime.status === 'already-running')
    ) {
      result.resume = deps.processManager.resumeProjectAgents(projectId);
    }

    return result;
  });

  // POST /api/projects/:projectId/stop - stop project
  app.post<{ Params: { projectId: string } }>('/api/projects/:projectId/stop', async (request, reply) => {
    const { projectId } = request.params;
    const project = projectRegistry.getProject(projectId);
    if (!project) {
      return reply.status(404).send({ error: 'Project not found' });
    }

    try {
      const updated = await runtimeController.stop(projectId);
      return updated;
    } catch (err: any) {
      project.status = 'error';
      return reply.status(500).send({ error: err.message });
    }
  });

  // GET /api/projects/:projectId/compose - read project compose file
  app.get<{ Params: { projectId: string } }>('/api/projects/:projectId/compose', async (request, reply) => {
    const { projectId } = request.params;
    const project = projectRegistry.getProject(projectId);
    if (!project) {
      return reply.status(404).send({ error: 'Project not found' });
    }

    const yamlPath = path.join(project.projectRoot, '.appfs-compose.yaml');
    const ymlPath = path.join(project.projectRoot, '.appfs-compose.yml');
    
    let activePath = yamlPath;
    if (!fs.existsSync(yamlPath) && fs.existsSync(ymlPath)) {
      activePath = ymlPath;
    }

    if (!fs.existsSync(activePath)) {
      return reply.status(404).send({ 
        error: `Compose file not found at ${activePath}`,
        isMissing: true
      });
    }

    try {
      const content = fs.readFileSync(activePath, 'utf-8');
      return {
        composeFilePath: activePath,
        content
      };
    } catch (err: any) {
      return reply.status(500).send({ error: `Failed to read compose file: ${err.message}` });
    }
  });

  // POST /api/projects/:projectId/compose/validate - validate proposed compose content
  app.post<{ Params: { projectId: string }; Body: { content: string } }>('/api/projects/:projectId/compose/validate', async (request, reply) => {
    const { projectId } = request.params;
    const { content } = request.body || {};
    const project = projectRegistry.getProject(projectId);
    if (!project) {
      return reply.status(404).send({ error: 'Project not found' });
    }

    if (content === undefined) {
      return reply.status(400).send({ error: 'Missing content parameter' });
    }

    const normalized = normalizeComposeMountpointContent(content);
    const result = await validateComposeContent(project.projectRoot, normalized.content);
    return result;
  });

  // PUT /api/projects/:projectId/compose - save project compose file atomically with validation
  app.put<{ Params: { projectId: string }; Body: { content: string } }>('/api/projects/:projectId/compose', async (request, reply) => {
    const { projectId } = request.params;
    const { content } = request.body || {};
    const project = projectRegistry.getProject(projectId);
    if (!project) {
      return reply.status(404).send({ error: 'Project not found' });
    }

    if (content === undefined) {
      return reply.status(400).send({ error: 'Missing content parameter' });
    }

    const normalized = normalizeComposeMountpointContent(content);

    // 1. Perform AppFS schema and syntax validation first
    const validation = await validateComposeContent(project.projectRoot, normalized.content);
    if (!validation.valid) {
      return reply.status(400).send({ error: `Validation failed: ${validation.error}` });
    }

    // 2. Determine target file path (preserve existing extension or fallback to .appfs-compose.yaml)
    const yamlPath = path.join(project.projectRoot, '.appfs-compose.yaml');
    const ymlPath = path.join(project.projectRoot, '.appfs-compose.yml');
    
    let activePath = yamlPath;
    if (!fs.existsSync(yamlPath) && fs.existsSync(ymlPath)) {
      activePath = ymlPath;
    }

    // 3. Atomically write to a temp file and rename it
    const tempFileName = `.appfs-compose-save-${crypto.randomUUID().slice(0, 8)}.tmp.yaml`;
    const tempFilePath = path.join(project.projectRoot, tempFileName);

    try {
      fs.writeFileSync(tempFilePath, normalized.content, 'utf-8');
      fs.renameSync(tempFilePath, activePath);
      
      // Update composeFilePath in project record
      project.composeFilePath = activePath;

      return {
        success: true,
        composeFilePath: activePath,
        content: normalized.content,
        mountpointNormalized: normalized.changed,
      };
    } catch (err: any) {
      try {
        if (fs.existsSync(tempFilePath)) {
          fs.unlinkSync(tempFilePath);
        }
      } catch {}
      return reply.status(500).send({ error: `Failed to save compose file atomically: ${err.message}` });
    }
  });
}
