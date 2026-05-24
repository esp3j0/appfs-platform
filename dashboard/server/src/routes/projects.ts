import type { FastifyInstance } from 'fastify';
import type { ProjectRegistry, ProjectRecord } from '../project-registry.js';
import { checkMountpointConflict } from '../project-registry.js';
import fs from 'node:fs';

export interface ProjectRuntimeController {
  start(projectId: string): Promise<ProjectRecord>;
  stop(projectId: string): Promise<ProjectRecord>;
  status(projectId: string): ProjectRecord | undefined;
}

export function registerProjectsRoute(
  app: FastifyInstance,
  projectRegistry: ProjectRegistry,
  runtimeController: ProjectRuntimeController
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
      const record = projectRegistry.registerProject(projectRoot);
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
}
