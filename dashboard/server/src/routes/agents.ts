import type { FastifyInstance } from 'fastify';
import type { AgentRegistry } from '../agent-registry.js';
import type { AgentProcessManager } from '../process-manager.js';
import type { AgentInfo } from '../types.js';

interface AgentsQuery {
  archived?: 'include' | 'only';
}

export function registerAgentsRoute(
  app: FastifyInstance,
  registry: AgentRegistry,
  processManager?: Pick<AgentProcessManager, 'getManagedAgents'>,
): void {
  app.get<{ Querystring: AgentsQuery }>('/api/agents', async (request) => {
    const agents = agentsForArchiveMode(registry, request.query.archived);
    if (!processManager) {
      return agents;
    }

    const managedBySessionId = new Map(
      processManager.getManagedAgents()
        .filter(agent => Boolean(agent.sessionId))
        .map(agent => [agent.sessionId as string, agent]),
    );

    return agents.map(agent => {
      const managed = managedBySessionId.get(agent.sessionId);
      if (!managed) {
        return agent;
      }

      return {
        ...agent,
        name: managed.principalId || agent.name,
        principalId: managed.principalId || agent.principalId,
        pid: managed.pid ?? agent.pid,
        model: managed.model || agent.model,
        status: 'online' as const,
        controlMode: 'managed' as const,
        projectId: managed.projectId ?? agent.projectId,
      };
    });
  });
}

function agentsForArchiveMode(registry: AgentRegistry, mode?: AgentsQuery['archived']): AgentInfo[] {
  if (mode === 'only') {
    return registry.getArchivedAgents();
  }
  if (mode === 'include') {
    return registry.getAgents();
  }
  return registry.getActiveAgents();
}
