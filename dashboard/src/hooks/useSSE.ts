import { useEffect, useRef } from 'react';
import type { TimelineEntry } from '../types';

interface SSECallbacks {
  onMessage?: (entry: TimelineEntry) => void;
  onDebugDump?: (entry: TimelineEntry) => void;
  onAgentOnline?: (agent: unknown) => void;
  onAgentOffline?: (agent: unknown) => void;
}

export function useSSE(url: string, callbacks: SSECallbacks) {
  const callbacksRef = useRef(callbacks);
  callbacksRef.current = callbacks;

  useEffect(() => {
    const source = new EventSource(url);

    source.addEventListener('message', (e: MessageEvent) => {
      try {
        const entry: TimelineEntry = JSON.parse(e.data);
        callbacksRef.current.onMessage?.(entry);
      } catch { /* skip */ }
    });

    source.addEventListener('debug-dump', (e: MessageEvent) => {
      try {
        const entry: TimelineEntry = JSON.parse(e.data);
        callbacksRef.current.onDebugDump?.(entry);
      } catch { /* skip */ }
    });

    source.addEventListener('agent-online', (e: MessageEvent) => {
      try {
        callbacksRef.current.onAgentOnline?.(JSON.parse(e.data));
      } catch { /* skip */ }
    });

    source.addEventListener('agent-offline', (e: MessageEvent) => {
      try {
        callbacksRef.current.onAgentOffline?.(JSON.parse(e.data));
      } catch { /* skip */ }
    });

    return () => source.close();
  }, [url]);
}
