import { useEffect, useRef, useCallback } from 'react';

export interface DashboardSSEEvent {
  type: string;
  timestamp: number;
  payload: unknown;
}

export interface DashboardSSECallbacks {
  onMessage?: (payload: { id?: string; sessionId?: string; agentName: string; role: 'system' | 'user' | 'assistant' | 'tool'; content: string; timestamp: number }) => void;
  onAssistantDelta?: (payload: { sessionId: string; requestId: string; turnId: string; text: string }) => void;
  onToolStart?: (payload: { sessionId: string; requestId?: string; turnId?: string; id?: string; toolName: string }) => void;
  onToolResult?: (payload: { sessionId: string; requestId?: string; turnId?: string; id?: string; toolName: string; isError?: boolean }) => void;
  onTurnStart?: (payload: { sessionId: string; requestId: string; turnId: string }) => void;
  onTurnDone?: (payload: { sessionId: string; requestId: string; turnId: string; status: string; usage?: { input_tokens?: number; output_tokens?: number } }) => void;
  onAgentError?: (payload: { sessionId: string; requestId?: string; turnId?: string; message: string }) => void;
  onAgentOnline?: (payload: { sessionId: string; spawnId: string; controlMode: string }) => void;
  onAgentOffline?: (payload: { sessionId: string; spawnId: string; code: number; signal: string }) => void;
  onProcessLog?: (payload: { agentId: string; spawnId: string; stream: string; text: string }) => void;
}

/**
 * Hook that subscribes to the unified `dashboard-event` SSE stream.
 * Unlike `useSSE` which listens to legacy event names, this hook listens
 * exclusively to the `dashboard-event` envelope and dispatches by `type`.
 */
export function useDashboardSSE(url: string, callbacks: DashboardSSECallbacks) {
  const callbacksRef = useRef(callbacks);
  callbacksRef.current = callbacks;

  useEffect(() => {
    const source = new EventSource(url);

    source.addEventListener('dashboard-event', (e: MessageEvent) => {
      try {
        const envelope: DashboardSSEEvent = JSON.parse(e.data);
        const payload = envelope.payload as any;
        const cbs = callbacksRef.current;

        switch (envelope.type) {
          case 'message':
            cbs.onMessage?.(payload);
            break;
          case 'assistant-delta':
            cbs.onAssistantDelta?.(payload);
            break;
          case 'tool-start':
            cbs.onToolStart?.(payload);
            break;
          case 'tool-result':
            cbs.onToolResult?.(payload);
            break;
          case 'turn-start':
            cbs.onTurnStart?.(payload);
            break;
          case 'turn-done':
            cbs.onTurnDone?.(payload);
            break;
          case 'agent-error':
            cbs.onAgentError?.(payload);
            break;
          case 'agent-online':
            cbs.onAgentOnline?.(payload);
            break;
          case 'agent-offline':
            cbs.onAgentOffline?.(payload);
            break;
          case 'process-log':
            cbs.onProcessLog?.(payload);
            break;
        }
      } catch { /* skip malformed events */ }
    });

    return () => source.close();
  }, [url]);
}
