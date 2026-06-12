import type { FastifyInstance, FastifyReply } from 'fastify';
import {
  PrincipalLifecycleError,
  type PrincipalCreateRequest,
  type PrincipalStartRequest,
} from '../principal-lifecycle.js';

interface ProjectParams {
  projectId: string;
}

export interface ExternalAgentCreateRequest extends PrincipalStartRequest {
  principalId: string;
  displayName?: string;
  description?: string | null;
  teamName?: string;
  taskListId?: string;
}

export interface InternalExternalAgentLifecycleService {
  ensurePrincipalReady(projectId: string, input: PrincipalCreateRequest): Promise<unknown>;
  startPrincipalAndWait(
    projectId: string,
    principalId: string,
    input?: PrincipalStartRequest,
  ): Promise<unknown>;
}

export function registerInternalExternalAgentsRoute(
  app: FastifyInstance,
  lifecycle: InternalExternalAgentLifecycleService,
  controlToken: string,
): void {
  app.post<{ Params: ProjectParams; Body: ExternalAgentCreateRequest }>(
    '/api/internal/projects/:projectId/external-agents',
    async (request, reply) => {
      if (!isAuthorized(request.headers['x-appfs-agent-control-token'], controlToken)) {
        return reply.status(403).send({ error: 'Forbidden' });
      }

      return handleInternalAgentRoute(reply, async () => {
        const body = request.body ?? ({} as ExternalAgentCreateRequest);
        if (!body.principalId?.trim()) {
          return reply.status(400).send({
            error: 'principalId is required',
            code: 'INVALID_ARGUMENT',
          });
        }
        await lifecycle.ensurePrincipalReady(request.params.projectId, {
          principalId: body.principalId,
          displayName: body.displayName ?? body.principalId,
          description: body.description,
          kind: 'agent',
        });
        return lifecycle.startPrincipalAndWait(request.params.projectId, body.principalId, {
          model: body.model,
          modelProviderId: body.modelProviderId,
          modelId: body.modelId,
          contextWindowTokens: body.contextWindowTokens,
          maxOutputTokens: body.maxOutputTokens,
          permissionMode: body.permissionMode,
          teamName: body.teamName,
          taskListId: body.taskListId,
        });
      });
    },
  );
}

function isAuthorized(header: string | string[] | undefined, controlToken: string): boolean {
  const value = Array.isArray(header) ? header[0] : header;
  return Boolean(controlToken) && value === controlToken;
}

async function handleInternalAgentRoute<T>(
  reply: FastifyReply,
  handler: () => T | Promise<T>,
): Promise<T | FastifyReply> {
  try {
    return await handler();
  } catch (err: unknown) {
    if (err instanceof PrincipalLifecycleError) {
      return reply.status(err.statusCode).send({
        error: err.message,
        ...(err.code ? { code: err.code } : {}),
      });
    }
    const message = err instanceof Error ? err.message : String(err);
    console.error('[InternalExternalAgentsRoute] Error:', message);
    return reply.status(500).send({ error: message });
  }
}
