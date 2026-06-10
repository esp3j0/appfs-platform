import type { FastifyInstance, FastifyReply } from 'fastify';
import {
  PrincipalLifecycleError,
  type PrincipalCreateRequest,
  type PrincipalResumeRequest,
  type PrincipalStartRequest,
} from '../principal-lifecycle.js';

interface ProjectParams {
  projectId: string;
}

interface PrincipalParams extends ProjectParams {
  principalId: string;
}

export interface PrincipalLifecycleRouteService {
  listPrincipals(projectId: string): unknown;
  createPrincipal(projectId: string, input: PrincipalCreateRequest): Promise<unknown>;
  deletePrincipal(projectId: string, principalId: string): Promise<unknown>;
  startPrincipal(projectId: string, principalId: string, input?: PrincipalStartRequest): Promise<unknown>;
  stopPrincipal(projectId: string, principalId: string): Promise<unknown>;
  resumePrincipal(projectId: string, principalId: string, input?: PrincipalResumeRequest): Promise<unknown>;
}

export function registerPrincipalsRoute(
  app: FastifyInstance,
  lifecycle: PrincipalLifecycleRouteService,
): void {
  app.get<{ Params: ProjectParams }>('/api/projects/:projectId/principals', async (request, reply) => {
    return handlePrincipalRoute(reply, () => lifecycle.listPrincipals(request.params.projectId));
  });

  app.post<{ Params: ProjectParams; Body: PrincipalCreateRequest }>(
    '/api/projects/:projectId/principals',
    async (request, reply) => {
      return handlePrincipalRoute(reply, async () => {
        const result = await lifecycle.createPrincipal(request.params.projectId, request.body);
        return reply.status(201).send(result);
      });
    },
  );

  app.delete<{ Params: PrincipalParams }>(
    '/api/projects/:projectId/principals/:principalId',
    async (request, reply) => {
      return handlePrincipalRoute(reply, () => (
        lifecycle.deletePrincipal(request.params.projectId, request.params.principalId)
      ));
    },
  );

  app.post<{ Params: PrincipalParams; Body: PrincipalStartRequest }>(
    '/api/projects/:projectId/principals/:principalId/start',
    async (request, reply) => {
      return handlePrincipalRoute(reply, () => (
        lifecycle.startPrincipal(request.params.projectId, request.params.principalId, request.body ?? {})
      ));
    },
  );

  app.post<{ Params: PrincipalParams }>(
    '/api/projects/:projectId/principals/:principalId/stop',
    async (request, reply) => {
      return handlePrincipalRoute(reply, () => (
        lifecycle.stopPrincipal(request.params.projectId, request.params.principalId)
      ));
    },
  );

  app.post<{ Params: PrincipalParams; Body: PrincipalResumeRequest }>(
    '/api/projects/:projectId/principals/:principalId/resume',
    async (request, reply) => {
      return handlePrincipalRoute(reply, () => (
        lifecycle.resumePrincipal(request.params.projectId, request.params.principalId, request.body ?? {})
      ));
    },
  );

  app.get('/api/principals', async (_request, reply) => {
    return reply.status(400).send({
      error: 'Use /api/projects/:projectId/principals',
    });
  });
}

async function handlePrincipalRoute<T>(
  reply: FastifyReply,
  handler: () => T | Promise<T>,
): Promise<T | FastifyReply> {
  try {
    return await handler();
  } catch (err: unknown) {
    if (err instanceof PrincipalLifecycleError) {
      return reply.status(err.statusCode).send({ error: err.message });
    }
    const message = err instanceof Error ? err.message : String(err);
    console.error('[PrincipalsRoute] Error:', message);
    return reply.status(500).send({ error: message });
  }
}
