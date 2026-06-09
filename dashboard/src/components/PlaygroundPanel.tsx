import React, { useState, useRef, useEffect, useCallback } from 'react';
import type { AgentInfo, ChatItem, ChatThread } from '../types';
import { useDashboardSSE } from '../hooks/useDashboardSSE';
import {
  cachedInputTokens,
  contextUsagePercent,
  contextUsageTitle,
  effectiveInputTokens,
  type TokenUsageLike,
} from '../token-usage';

interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'tool' | 'error';
  text: string;
  title?: string;
  tool?: Extract<ChatItem, { kind: 'tool' }>;
  requestId?: string;
  turnId?: string;
  timestamp: number;
  streaming?: boolean;
  usage?: TokenUsageLike;
  pendingInput?: {
    requestId: string;
    mode: 'queued' | 'guidance';
  };
}

interface AgentChatState {
  messages: ChatMessage[];
  inputValue: string;
  isBusy: boolean;
  isStopping: boolean;
  activeRequestId: string | null;
  isHydrating: boolean;
  hydrateError: string | null;
}

interface AgentProcessStatus {
  status: string;
  currentRequestId: string | null;
}

interface PlaygroundPanelProps {
  agents: AgentInfo[];
  selectedAgents: AgentInfo[];
}

function emptyChatState(): AgentChatState {
  return {
    messages: [],
    inputValue: '',
    isBusy: false,
    isStopping: false,
    activeRequestId: null,
    isHydrating: false,
    hydrateError: null,
  };
}

export function PlaygroundPanel({ agents, selectedAgents }: PlaygroundPanelProps) {
  const [stateBySession, setStateBySession] = useState<Record<string, AgentChatState>>({});
  const [focusedSessionId, setFocusedSessionId] = useState<string | null>(null);
  const selectedSessionKey = selectedAgents.map(agent => agent.sessionId).join('|');
  const managedSessionKey = selectedAgents
    .filter(agent => agent.controlMode === 'managed')
    .map(agent => agent.sessionId)
    .join('|');
  const selectedSessionIds = new Set(selectedAgents.map(agent => agent.sessionId));

  const patchSession = useCallback((
    sessionId: string,
    updater: (state: AgentChatState) => AgentChatState,
  ) => {
    setStateBySession(prev => ({
      ...prev,
      [sessionId]: updater(prev[sessionId] ?? emptyChatState()),
    }));
  }, []);

  const syncAgentStatus = useCallback(async (sessionId: string) => {
    try {
      const res = await fetch(`/api/agents/${encodeURIComponent(sessionId)}/status`);
      if (!res.ok) return;
      const processStatus = await res.json() as AgentProcessStatus;
      patchSession(sessionId, state => applyAgentProcessStatus(state, processStatus));
    } catch {
      // The chat history hydrate path remains the source of visible content;
      // status sync is only a best-effort guard against missed transient SSE.
    }
  }, [patchSession]);

  const hydrateSession = useCallback((sessionId: string, options?: { silent?: boolean }) => {
    if (!options?.silent) {
      patchSession(sessionId, state => ({
        ...state,
        isHydrating: true,
        hydrateError: null,
      }));
    }
    return fetch(`/api/agents/${encodeURIComponent(sessionId)}/chat`)
      .then(res => res.ok ? res.json() : Promise.reject(new Error(`HTTP ${res.status}`)))
      .then((thread: ChatThread) => {
        patchSession(sessionId, state => {
          const hydrated = thread.items.map(chatItemToMessage);
          const liveMessages = state.messages.filter(item =>
            item.streaming || item.pendingInput || item.role === 'error',
          );
          return {
            ...state,
            messages: mergeHydratedWithLive(hydrated, liveMessages),
            isHydrating: false,
            hydrateError: null,
          };
        });
        void syncAgentStatus(sessionId);
      })
      .catch((err: unknown) => {
        patchSession(sessionId, state => ({
          ...state,
          isHydrating: false,
          hydrateError: err instanceof Error ? err.message : String(err),
        }));
        throw err;
      });
  }, [patchSession, syncAgentStatus]);

  useEffect(() => {
    setStateBySession(prev => {
      const next: Record<string, AgentChatState> = {};
      for (const agent of selectedAgents) {
        next[agent.sessionId] = prev[agent.sessionId] ?? emptyChatState();
      }
      return next;
    });
  }, [selectedSessionKey]);

  useEffect(() => {
    if (focusedSessionId && !selectedSessionIds.has(focusedSessionId)) {
      setFocusedSessionId(null);
    }
  }, [focusedSessionId, selectedSessionKey]);

  useEffect(() => {
    const managedSessionIds = selectedAgents
      .filter(agent => agent.controlMode === 'managed')
      .map(agent => agent.sessionId);
    if (managedSessionIds.length === 0) return;

    const syncAll = () => {
      for (const sessionId of managedSessionIds) {
        void syncAgentStatus(sessionId);
      }
    };

    syncAll();
    const interval = window.setInterval(syncAll, 5000);
    return () => window.clearInterval(interval);
  }, [managedSessionKey, syncAgentStatus]);

  useEffect(() => {
    let cancelled = false;

    for (const agent of selectedAgents) {
      hydrateSession(agent.sessionId)
        .catch(() => {
          if (cancelled) return;
          patchSession(agent.sessionId, state => {
            if (state.messages.some(item => item.id.startsWith('hydrate-error-'))) {
              return state;
            }
            return {
              ...state,
              messages: [...state.messages, {
                id: `hydrate-error-${agent.sessionId}`,
                role: 'error',
                text: 'Failed to load chat history for this session.',
                timestamp: Date.now(),
              }],
            };
          });
        });
    }

    return () => {
      cancelled = true;
    };
  }, [hydrateSession, patchSession, selectedSessionKey]);

  useDashboardSSE('/api/events', {
    onMessage: useCallback((payload) => {
      if (!payload.sessionId || !selectedSessionIds.has(payload.sessionId)) return;
      void hydrateSession(payload.sessionId, { silent: true }).catch(() => undefined);
      if (payload.role !== 'assistant' && payload.role !== 'user') return;
      if (payload.role === 'assistant') return;

      patchSession(payload.sessionId, state => {
        if (payload.role === 'user') {
          const pendingIndex = state.messages.findIndex(msg =>
            msg.role === 'user'
            && msg.pendingInput
            && msg.text === payload.content,
          );
          if (pendingIndex >= 0) {
            const messages = [...state.messages];
            messages[pendingIndex] = {
              ...messages[pendingIndex],
              id: payload.id ?? messages[pendingIndex].id,
              timestamp: payload.timestamp,
              pendingInput: undefined,
            };
            return { ...state, messages };
          }

          const duplicate = state.messages.some(msg =>
            msg.role === 'user'
            && msg.text === payload.content
            && Math.abs(msg.timestamp - payload.timestamp) < 2000,
          );
          if (duplicate) return state;
          return {
            ...state,
            messages: [...state.messages, {
              id: payload.id ?? `session-${payload.sessionId}-${payload.timestamp}`,
              role: 'user',
              text: payload.content,
              timestamp: payload.timestamp,
            }],
          };
        }

        return state;
      });
    }, [hydrateSession, patchSession, selectedSessionKey]),

    onTurnStart: useCallback((payload) => {
      if (!selectedSessionIds.has(payload.sessionId)) return;
      patchSession(payload.sessionId, state => ({
        ...state,
        isBusy: true,
        isStopping: false,
        activeRequestId: payload.requestId,
      }));
    }, [patchSession, selectedSessionKey]),

    onAssistantDelta: useCallback((payload) => {
      if (!selectedSessionIds.has(payload.sessionId)) return;
      patchSession(payload.sessionId, state => appendAssistantDelta(state, payload));
    }, [patchSession, selectedSessionKey]),

    onToolStart: useCallback((payload) => {
      if (!selectedSessionIds.has(payload.sessionId)) return;
      const toolId = payload.id
        ? `tool:${payload.id}`
        : `tool-live:${payload.turnId ?? payload.requestId ?? Date.now()}:${payload.toolName}`;
      patchSession(payload.sessionId, state => {
        const messages = closeAssistantSegmentsForTurn(state.messages, payload.turnId);
        if (state.messages.some(msg => msg.id === toolId)) {
          return { ...state, messages };
        }
        return {
          ...state,
          messages: [...messages, {
            id: toolId,
            role: 'tool',
            text: `${payload.toolName} running`,
            title: payload.toolName,
            requestId: payload.requestId,
            turnId: payload.turnId,
            timestamp: Date.now(),
            streaming: true,
            tool: {
              kind: 'tool',
              id: toolId,
              toolCallId: payload.id,
              toolName: payload.toolName,
              status: 'pending',
              summary: `${payload.toolName} running`,
              timestamp: Date.now(),
            },
          }],
        };
      });
    }, [patchSession, selectedSessionKey]),

    onToolResult: useCallback((payload) => {
      if (!selectedSessionIds.has(payload.sessionId)) return;
      patchSession(payload.sessionId, state => {
        const messages = [...state.messages];
        const index = findPendingToolIndex(messages, payload.id, payload.turnId, payload.toolName);
        const status = payload.isError ? 'error' : 'completed';
        if (index >= 0) {
          const existing = messages[index];
          messages[index] = {
            ...existing,
            text: `${payload.toolName} ${status}`,
            streaming: false,
            tool: existing.tool ? {
              ...existing.tool,
              status,
              summary: `${payload.toolName} ${status}`,
              isError: payload.isError,
            } : undefined,
          };
          return { ...state, messages };
        }
        return {
          ...state,
          messages: [...messages, {
            id: `tool-result-live:${payload.turnId ?? payload.requestId ?? Date.now()}:${payload.toolName}`,
            role: 'tool',
            text: `${payload.toolName} ${status}`,
            title: payload.toolName,
            requestId: payload.requestId,
            turnId: payload.turnId,
            timestamp: Date.now(),
            tool: {
              kind: 'tool',
              id: `tool-result-live:${payload.turnId ?? payload.requestId ?? Date.now()}:${payload.toolName}`,
              toolCallId: payload.id,
              toolName: payload.toolName,
              status,
              summary: `${payload.toolName} ${status}`,
              isError: payload.isError,
              timestamp: Date.now(),
            },
          }],
        };
      });
    }, [patchSession, selectedSessionKey]),

    onTurnDone: useCallback((payload) => {
      if (!selectedSessionIds.has(payload.sessionId)) return;
      patchSession(payload.sessionId, state => ({
        ...state,
        isBusy: false,
        isStopping: false,
        activeRequestId: null,
        messages: finalizeTurnMessages(state.messages, payload.turnId, payload.usage, payload.status),
      }));
    }, [patchSession, selectedSessionKey]),

    onAgentError: useCallback((payload) => {
      if (!selectedSessionIds.has(payload.sessionId)) return;
      patchSession(payload.sessionId, state => ({
        ...state,
        isBusy: false,
        isStopping: false,
        activeRequestId: null,
        messages: [...state.messages, {
          id: `error-${Date.now()}`,
          role: 'error',
          text: payload.message,
          requestId: payload.requestId,
          turnId: payload.turnId,
          timestamp: Date.now(),
        }],
      }));
    }, [patchSession, selectedSessionKey]),
  });

  const updateInput = (sessionId: string, inputValue: string) => {
    patchSession(sessionId, state => ({ ...state, inputValue }));
  };

  const handleSend = async (agent: AgentInfo) => {
    const state = stateBySession[agent.sessionId] ?? emptyChatState();
    if (!state.inputValue.trim() || agent.controlMode !== 'managed') return;

    const userText = state.inputValue.trim();
    const sendWasBusy = state.isBusy;
    patchSession(agent.sessionId, current => ({
      ...current,
      inputValue: '',
    }));

    try {
      const res = await fetch(`/api/agents/${encodeURIComponent(agent.sessionId)}/prompt`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json; charset=utf-8' },
        body: JSON.stringify({
          prompt: userText,
          delivery: sendWasBusy ? 'queue' : 'prompt',
        }),
      });

      if (res.status === 409) {
        patchSession(agent.sessionId, current => ({
          ...current,
          messages: [...current.messages, {
            id: `error-busy-${Date.now()}`,
            role: 'error',
            text: 'Agent is currently busy with another turn. Please wait.',
            timestamp: Date.now(),
          }],
        }));
        return;
      }

      if (!res.ok) {
        const data = await res.json().catch(() => ({ error: 'Unknown error' }));
        patchSession(agent.sessionId, current => ({
          ...current,
          messages: [...current.messages, {
            id: `error-${Date.now()}`,
            role: 'error',
            text: data.error || 'Failed to send prompt',
            timestamp: Date.now(),
          }],
        }));
        return;
      }

      const data = await res.json();
      patchSession(agent.sessionId, current => ({
        ...current,
        isBusy: data.status === 'accepted' ? true : current.isBusy,
        isStopping: data.status === 'accepted' ? false : current.isStopping,
        activeRequestId: data.status === 'accepted' ? data.request_id : current.activeRequestId,
        messages: [...current.messages, {
          id: `user-${data.request_id ?? Date.now()}`,
          role: 'user',
          text: userText,
          requestId: data.request_id,
          timestamp: Date.now(),
          pendingInput: data.status === 'queued' || data.status === 'guidance'
            ? {
                requestId: data.request_id,
                mode: data.status,
              }
            : undefined,
        }],
      }));
    } catch (err) {
      patchSession(agent.sessionId, current => ({
        ...current,
        messages: [...current.messages, {
          id: `error-net-${Date.now()}`,
          role: 'error',
          text: `Network error: ${err instanceof Error ? err.message : String(err)}`,
          timestamp: Date.now(),
        }],
      }));
    }
  };

  const handleCancelTurn = async (agent: AgentInfo) => {
    const state = stateBySession[agent.sessionId] ?? emptyChatState();
    if (!state.isBusy || !state.activeRequestId || agent.controlMode !== 'managed') return;

    patchSession(agent.sessionId, current => ({
      ...current,
      isStopping: true,
    }));

    try {
      const res = await fetch(`/api/agents/${encodeURIComponent(agent.sessionId)}/cancel-turn`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json; charset=utf-8' },
        body: JSON.stringify({ request_id: state.activeRequestId }),
      });

      if (!res.ok) {
        const data = await res.json().catch(() => ({ error: 'Unknown error' }));
        throw new Error(data.error || 'Failed to stop current turn');
      }
    } catch (err) {
      patchSession(agent.sessionId, current => ({
        ...current,
        isStopping: false,
        messages: [...current.messages, {
          id: `error-stop-${Date.now()}`,
          role: 'error',
          text: `Failed to stop current turn: ${err instanceof Error ? err.message : String(err)}`,
          timestamp: Date.now(),
        }],
      }));
    }
  };

  const promoteQueuedInput = async (agent: AgentInfo, message: ChatMessage) => {
    const requestId = message.pendingInput?.requestId;
    if (!requestId || message.pendingInput.mode !== 'queued' || agent.controlMode !== 'managed') {
      return;
    }

    patchSession(agent.sessionId, current => ({
      ...current,
      messages: current.messages.map(item => item.id === message.id
        ? { ...item, pendingInput: { requestId, mode: 'guidance' } }
        : item),
    }));

    try {
      const res = await fetch(
        `/api/agents/${encodeURIComponent(agent.sessionId)}/inputs/${encodeURIComponent(requestId)}/guidance`,
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json; charset=utf-8' },
        },
      );
      if (!res.ok) {
        const data = await res.json().catch(() => ({ error: 'Unknown error' }));
        throw new Error(data.error || 'Failed to promote queued input');
      }
    } catch (err) {
      patchSession(agent.sessionId, current => ({
        ...current,
        messages: current.messages.map(item => item.id === message.id
          ? { ...item, pendingInput: { requestId, mode: 'queued' } }
          : item),
        hydrateError: err instanceof Error ? err.message : String(err),
      }));
    }
  };

  if (selectedAgents.length === 0) {
    return (
      <div className="playground">
        <div className="playground-empty">
          <div className="playground-empty-icon">💬</div>
          <h3>Chat Playground</h3>
          <p>Select one or more agents from the sidebar to start a conversation.</p>
        </div>
      </div>
    );
  }

  const visibleAgents = focusedSessionId
    ? selectedAgents.filter(agent => agent.sessionId === focusedSessionId)
    : selectedAgents;

  return (
    <div className={`playground-grid ${visibleAgents.length > 1 ? 'multi' : 'single'} ${focusedSessionId ? 'focused' : ''}`}>
      {visibleAgents.map(agent => (
        <AgentChatPane
          key={agent.sessionId}
          agent={agent}
          state={stateBySession[agent.sessionId] ?? emptyChatState()}
          onInputChange={value => updateInput(agent.sessionId, value)}
          onSend={() => handleSend(agent)}
          onCancelTurn={() => void handleCancelTurn(agent)}
          onPromoteQueuedInput={message => void promoteQueuedInput(agent, message)}
          onRefresh={() => void hydrateSession(agent.sessionId).catch(() => undefined)}
          canFocus={selectedAgents.length > 1}
          isFocused={focusedSessionId === agent.sessionId}
          onToggleFocus={() => setFocusedSessionId(current =>
            current === agent.sessionId ? null : agent.sessionId,
          )}
        />
      ))}
    </div>
  );
}

function AgentChatPane({
  agent,
  state,
  onInputChange,
  onSend,
  onCancelTurn,
  onPromoteQueuedInput,
  onRefresh,
  canFocus,
  isFocused,
  onToggleFocus,
}: {
  agent: AgentInfo;
  state: AgentChatState;
  onInputChange: (value: string) => void;
  onSend: () => void;
  onCancelTurn: () => void;
  onPromoteQueuedInput: (message: ChatMessage) => void;
  onRefresh: () => void;
  canFocus: boolean;
  isFocused: boolean;
  onToggleFocus: () => void;
}) {
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const isManaged = agent.controlMode === 'managed';

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [state.messages]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      onSend();
    }
  };

  return (
    <div className="playground">
      <div className="playground-header">
        <div className="playground-header-left">
          <h2>{agent.name}</h2>
          <div className="playground-agent-info">
            <span className="playground-agent-model">{agent.model ?? 'unknown'}</span>
            <span className={`playground-mode-badge ${isManaged ? 'managed' : 'external'}`}>
              {isManaged ? '⚡ Managed' : '👁 External'}
            </span>
            <AgentContextUsage agent={agent} />
            <span className="playground-session-id">{agent.sessionId}</span>
          </div>
        </div>
        <div className="playground-header-actions">
          {(state.isBusy || state.isHydrating) && (
            <div className="playground-status">
              <span className="playground-thinking-indicator">
                <span className="dot dot1"></span>
                <span className="dot dot2"></span>
                <span className="dot dot3"></span>
              </span>
              <span>{state.isBusy ? 'Thinking…' : 'Refreshing…'}</span>
            </div>
          )}
          <button
            className="playground-header-btn"
            onClick={onRefresh}
            disabled={state.isHydrating}
            title="Reload this session from disk"
          >
            {state.isHydrating ? 'Syncing' : 'Refresh'}
          </button>
          {canFocus && (
            <button
              className="playground-header-btn"
              onClick={onToggleFocus}
              title={isFocused ? 'Show all selected agents' : 'Focus this agent'}
            >
              {isFocused ? 'Show all' : 'Focus'}
            </button>
          )}
        </div>
      </div>

      <div className="playground-messages">
        {state.hydrateError && (
          <div className="playground-msg playground-msg-error">
            <div className="playground-msg-header">
              <span className="playground-msg-role">⚠ History</span>
            </div>
            <div className="playground-msg-content">
              Failed to refresh chat history: {state.hydrateError}
            </div>
          </div>
        )}

        {state.messages.length === 0 && (
          <div className="playground-welcome">
            <div className="playground-welcome-icon">🦀</div>
            <p>
              {isManaged
                ? `Send a message to ${agent.name}.`
                : `${agent.name} is external/read-only.`}
            </p>
          </div>
        )}

        {state.messages.map(msg => (
          <div key={msg.id} className={`playground-msg playground-msg-${msg.role}`}>
            <div className="playground-msg-header">
              <span className="playground-msg-role">
                {msg.role === 'user'
                  ? '👤 You'
                  : msg.role === 'assistant'
                    ? '🤖 Assistant'
                    : msg.role === 'tool'
                      ? '🛠 Tool'
                      : '⚠ Error'}
              </span>
              <span className="playground-msg-time">
                {new Date(msg.timestamp).toLocaleTimeString()}
              </span>
            </div>
            {msg.role === 'tool' && msg.tool ? (
              <ToolCard message={msg} />
            ) : (
              <>
                {msg.title && <div className="playground-msg-title">{msg.title}</div>}
                <div className="playground-msg-content">
                  <MessageContent text={msg.text || (msg.streaming ? '' : '(empty response)')} />
                  {msg.streaming && <span className="playground-cursor">▌</span>}
                </div>
              </>
            )}
            {msg.usage && <MessageUsage usage={msg.usage} contextWindowTokens={agent.contextWindowTokens} />}
            {msg.role === 'user' && msg.pendingInput && (
              <div className="playground-pending-input">
                <span className={`playground-pending-badge ${msg.pendingInput.mode}`}>
                  {msg.pendingInput.mode === 'queued' ? 'Queued for after turn' : 'Guidance at next boundary'}
                </span>
                {msg.pendingInput.mode === 'queued' && (
                  <button
                    className="playground-guidance-btn"
                    type="button"
                    onClick={() => onPromoteQueuedInput(msg)}
                    title="Inject this message at the next model boundary"
                  >
                    引导
                  </button>
                )}
              </div>
            )}
          </div>
        ))}
        <div ref={messagesEndRef} />
      </div>

      {isManaged ? (
        <div className="playground-input-area">
          <textarea
            className="playground-input"
            placeholder={state.isBusy
              ? `Queue a follow-up for ${agent.name}…`
              : `Message ${agent.name}… (Shift+Enter for newline)`}
            value={state.inputValue}
            onChange={e => onInputChange(e.target.value)}
            onKeyDown={handleKeyDown}
            rows={2}
          />
          <button
            type="button"
            className={`playground-send-btn ${state.isBusy ? 'stop' : 'send'}`}
            onClick={state.isBusy ? onCancelTurn : onSend}
            disabled={state.isBusy ? state.isStopping || !state.activeRequestId : !state.inputValue.trim()}
            title={state.isBusy ? 'Stop current response' : 'Send (Enter)'}
          >
            {state.isBusy ? (state.isStopping ? 'Stopping' : 'Stop') : '➤'}
          </button>
        </div>
      ) : (
        <div className="playground-readonly-banner">
          <span>👁 Read Only</span> — This is an external agent. Prompt controls are disabled.
        </div>
      )}
    </div>
  );
}

function chatItemToMessage(item: ChatItem): ChatMessage {
  if (item.kind === 'message') {
    return {
      id: item.id,
      role: item.role,
      text: item.text,
      timestamp: item.timestamp,
      usage: item.usage,
    };
  }

  if (item.kind === 'error') {
    return {
      id: item.id,
      role: 'error',
      text: item.text,
      requestId: item.requestId,
      turnId: item.turnId,
      timestamp: item.timestamp,
    };
  }

  return {
    id: item.id,
    role: 'tool',
    title: item.toolName,
    text: item.summary ?? summarizeToolItem(item),
    tool: item,
    timestamp: item.timestamp,
  };
}

function ToolCard({ message }: { message: ChatMessage }) {
  const tool = message.tool;
  if (!tool) {
    return <div className="playground-msg-content">{message.text}</div>;
  }

  return (
    <div className={`playground-tool-card ${tool.status}`}>
      <span className="playground-tool-name">{tool.toolName}</span>
      <span className={`playground-tool-status ${tool.status}`}>{tool.status}</span>
      <span className="playground-tool-summary">{tool.summary ?? message.text}</span>
    </div>
  );
}

function summarizeToolItem(item: Extract<ChatItem, { kind: 'tool' }>): string {
  return `${item.toolName} ${item.status}`;
}

function findPendingToolIndex(
  messages: ChatMessage[],
  toolCallId: string | undefined,
  turnId: string | undefined,
  toolName: string,
): number {
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i];
    if (msg.role !== 'tool' || !msg.tool || msg.tool.status !== 'pending') {
      continue;
    }
    if (toolCallId && msg.tool.toolCallId === toolCallId) {
      return i;
    }
    if (msg.turnId === turnId && msg.tool.toolName === toolName) {
      return i;
    }
  }
  return -1;
}

function appendAssistantDelta(
  state: AgentChatState,
  payload: { requestId?: string; turnId?: string; text: string },
): AgentChatState {
  if (!payload.text) {
    return state;
  }

  const messages = [...state.messages];
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i];
    if (msg.role === 'assistant' && msg.turnId === payload.turnId && msg.streaming) {
      messages[i] = { ...msg, text: msg.text + payload.text };
      return { ...state, messages };
    }
  }

  return {
    ...state,
    messages: [...messages, {
      id: `assistant-${payload.turnId ?? payload.requestId ?? Date.now()}-${messages.length}`,
      role: 'assistant',
      text: payload.text,
      requestId: payload.requestId,
      turnId: payload.turnId,
      timestamp: Date.now(),
      streaming: true,
    }],
  };
}

function closeAssistantSegmentsForTurn(
  messages: ChatMessage[],
  turnId: string | undefined,
): ChatMessage[] {
  return messages.flatMap(msg => {
    if (msg.role !== 'assistant' || msg.turnId !== turnId || !msg.streaming) {
      return [msg];
    }
    if (!msg.text.trim()) {
      return [];
    }
    return [{ ...msg, streaming: false }];
  });
}

function closeAllLiveMessages(messages: ChatMessage[]): ChatMessage[] {
  return messages.flatMap(msg => {
    if (msg.role === 'assistant' && msg.streaming) {
      if (!msg.text.trim()) {
        return [];
      }
      return [{ ...msg, streaming: false }];
    }

    if (msg.role === 'tool' && msg.tool?.status === 'pending') {
      return [{
        ...msg,
        streaming: false,
        text: `${msg.tool.toolName} completed`,
        tool: {
          ...msg.tool,
          status: 'completed',
          summary: `${msg.tool.toolName} completed`,
        },
      }];
    }

    return [msg];
  });
}

function applyAgentProcessStatus(
  state: AgentChatState,
  processStatus: AgentProcessStatus,
): AgentChatState {
  if (processStatus.status === 'busy') {
    return {
      ...state,
      isBusy: true,
      isStopping: false,
      activeRequestId: processStatus.currentRequestId,
    };
  }

  if (!state.isBusy && !state.isStopping && state.activeRequestId === null) {
    return state;
  }

  return {
    ...state,
    isBusy: false,
    isStopping: false,
    activeRequestId: null,
    messages: closeAllLiveMessages(state.messages),
  };
}

function finalizeTurnMessages(
  messages: ChatMessage[],
  turnId: string | undefined,
  usage?: TokenUsageLike,
  status?: string,
): ChatMessage[] {
  const finalized = closeAssistantSegmentsForTurn(messages, turnId);
  if (status === 'cancelled') {
    return [...finalized, {
      id: `cancelled-${turnId ?? Date.now()}`,
      role: 'error',
      text: 'Current response stopped. You can send a new message when ready.',
      turnId,
      timestamp: Date.now(),
    }];
  }

  let lastAssistantIndex = -1;
  for (let i = finalized.length - 1; i >= 0; i--) {
    if (finalized[i].role === 'assistant' && finalized[i].turnId === turnId) {
      lastAssistantIndex = i;
      break;
    }
  }

  if (lastAssistantIndex < 0 || !usage) {
    return finalized;
  }

  return finalized.map((msg, index) => index === lastAssistantIndex
    ? { ...msg, usage }
    : msg);
}

function MessageContent({ text }: { text: string }) {
  const parts = splitCodeFences(text);
  return (
    <>
      {parts.map((part, index) => part.kind === 'code' ? (
        <div className="playground-code-frame" key={index}>
          {part.language && <div className="playground-code-language">{part.language}</div>}
          <pre className="playground-code-block">
            <code>{part.text}</code>
          </pre>
        </div>
      ) : (
        <span key={index}>{part.text}</span>
      ))}
    </>
  );
}

function splitCodeFences(text: string): Array<{ kind: 'text' | 'code'; text: string; language?: string }> {
  const parts: Array<{ kind: 'text' | 'code'; text: string; language?: string }> = [];
  const pattern = /```([^\n`]*)\n?([\s\S]*?)```/g;
  let cursor = 0;
  let match: RegExpExecArray | null;

  while ((match = pattern.exec(text)) !== null) {
    if (match.index > cursor) {
      parts.push({ kind: 'text', text: text.slice(cursor, match.index) });
    }
    parts.push({
      kind: 'code',
      language: match[1]?.trim() || undefined,
      text: match[2].replace(/\n$/, ''),
    });
    cursor = match.index + match[0].length;
  }

  if (cursor < text.length) {
    parts.push({ kind: 'text', text: text.slice(cursor) });
  }

  return parts.length > 0 ? parts : [{ kind: 'text', text }];
}

function mergeHydratedWithLive(hydrated: ChatMessage[], liveMessages: ChatMessage[]): ChatMessage[] {
  if (liveMessages.length === 0) {
    return hydrated;
  }
  const hydratedIds = new Set(hydrated.map(item => item.id));
  return [
    ...hydrated,
    ...liveMessages.filter(item => !hydratedIds.has(item.id) && !isCoveredByHydrated(item, hydrated)),
  ];
}

function isCoveredByHydrated(live: ChatMessage, hydrated: ChatMessage[]): boolean {
  if (live.role === 'assistant') {
    const liveText = live.text.trim();
    return liveText.length > 0 && hydrated.some(item =>
      item.role === 'assistant' && item.text.trim() === liveText,
    );
  }

  if (live.role === 'tool' && live.tool?.toolCallId) {
    return hydrated.some(item =>
      item.role === 'tool' && item.tool?.toolCallId === live.tool?.toolCallId,
    );
  }

  if (live.role === 'user' && live.pendingInput) {
    return hydrated.some(item =>
      item.role === 'user' && item.text.trim() === live.text.trim(),
    );
  }

  if (live.role === 'error') {
    return hydrated.some(item =>
      item.role === 'error'
      && item.text.trim() === live.text.trim()
      && (!live.turnId || item.turnId === live.turnId),
    );
  }

  return false;
}

function MessageUsage({
  usage,
  contextWindowTokens,
}: {
  usage: TokenUsageLike;
  contextWindowTokens?: number;
}) {
  const input = effectiveInputTokens(usage);
  const cached = cachedInputTokens(usage);
  const output = usage.output_tokens ?? 0;
  const percent = contextUsagePercent(input, contextWindowTokens);

  return (
    <div className="playground-msg-usage">
      {input > 0 && (
        <span>
          <ContextUsageDot
            percent={percent}
            title={contextUsageTitle(input, contextWindowTokens)}
          />
          ↓ {input.toLocaleString()}
          {cached > 0 && <span className="usage-cache-note">(cached {cached.toLocaleString()})</span>}
        </span>
      )}
      {output > 0 && <span>↑ {output.toLocaleString()}</span>}
    </div>
  );
}

function AgentContextUsage({ agent }: { agent: AgentInfo }) {
  const tokens = agent.currentContextTokens ?? 0;
  const percent = contextUsagePercent(tokens, agent.contextWindowTokens);
  const title = contextUsageTitle(tokens, agent.contextWindowTokens);

  return (
    <span className="playground-agent-context" title={title} aria-label={title}>
      <ContextUsageDot percent={percent} title={title} />
      <span>{tokens.toLocaleString()}</span>
      <span className="playground-agent-context-unit">tok</span>
    </span>
  );
}

function ContextUsageDot({ percent, title }: { percent: number; title: string }) {
  const clamped = Math.round(Math.min(100, Math.max(0, percent)));
  return (
    <span
      className="context-usage-dot"
      style={{ ['--context-percent' as string]: `${clamped}%` }}
      title={title}
      aria-label={title}
    />
  );
}
