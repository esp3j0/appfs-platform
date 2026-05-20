import React from 'react';
import type {
  AppEventRenderOverridesDoc,
  AppEventRenderScopeOverride,
  ConversationMessage,
  InputRouterBlockInput,
  TimelineEntry,
} from '../types';
import { CollapsibleBlock } from './CollapsibleBlock';

interface Props {
  selectedAgents: string[];
  entries: TimelineEntry[];
}

interface EventRenderMetadata {
  class?: string;
  wake?: boolean;
  running_delivery?: string;
  idle_delivery?: string;
  mode?: string;
  visibility?: string;
  template?: string;
  model_render?: {
    mode?: string;
    visibility?: string;
    template?: string;
    body_template?: string;
    source_template?: string;
    [key: string]: unknown;
  };
  terminal_render?: {
    mode?: string;
    lines?: string[];
    template?: string;
  };
  ui_render?: {
    mode?: string;
    lines?: string[];
    template?: string;
  };
  user_render?: {
    mode?: string;
    lines?: string[];
    template?: string;
  };
  [key: string]: unknown;
}

interface AppEventSample {
  agentName: string;
  appLabel: string;
  appId: string;
  streamId: string;
  eventType: string;
  timestamp: number;
  text: string;
  payload?: unknown;
  rawEvent?: unknown;
  metadata?: EventRenderMetadata;
  delivery?: string;
  requiresAttention?: boolean;
  seq?: number;
  correlationId?: string;
  eventPath?: string;
}

interface AppEventSummary {
  eventType: string;
  count: number;
  lastSeen: number;
  agents: Set<string>;
  deliveries: Set<string>;
  classes: Set<string>;
  samples: AppEventSample[];
}

interface AppSummary {
  key: string;
  appId: string;
  principalId?: string;
  streamId?: string;
  label: string;
  agents: Set<string>;
  totalEvents: number;
  lastSeen: number;
  eventTypes: Map<string, AppEventSummary>;
}

interface EventDraft {
  classValue: string;
  mode: string;
  wake: boolean;
  runningDelivery: string;
  idleDelivery: string;
  template: string;
  bodyTemplate: string;
  sourceTemplate: string;
  terminalLines: string;
}

const DEFAULT_DRAFT: EventDraft = {
  classValue: 'status',
  mode: 'summary',
  wake: false,
  runningDelivery: 'context_only',
  idleDelivery: 'context_only',
  template: '{{app.display_name}}: 操作已完成。',
  bodyTemplate: '{{content.text_preview}}',
  sourceTemplate: '来源：{{app.display_name}} {{type}}，seq={{seq}}',
  terminalLines: '{{ansi.cyan}}{{app.display_name}} · from {{message.sender}}{{ansi.reset}}\n{{message.body}}',
};

export function AppControlPanel({ selectedAgents, entries }: Props) {
  const [overrides, setOverrides] = React.useState<AppEventRenderOverridesDoc>({ version: 1, streams: {} });
  const [loading, setLoading] = React.useState(true);
  const [status, setStatus] = React.useState<string>('');
  const [scopeFilter, setScopeFilter] = React.useState<'session' | 'all'>('session');

  React.useEffect(() => {
    let cancelled = false;
    fetch('/api/app-event-overrides')
      .then(r => r.json())
      .then((data: AppEventRenderOverridesDoc) => {
        if (!cancelled) {
          setOverrides(normalizeOverrides(data));
          setLoading(false);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const apps = React.useMemo(
    () => collectAppSummaries(entries, selectedAgents, overrides, scopeFilter === 'all'),
    [entries, selectedAgents, overrides, scopeFilter],
  );
  const totalEventTypes = apps.reduce((sum, app) => sum + app.eventTypes.size, 0);
  const totalEvents = apps.reduce((sum, app) => sum + app.totalEvents, 0);

  return (
    <div className="app-control">
      <div className="app-control-header">
        <div>
          <h2>App Control</h2>
          <div className="app-control-meta">
            {loading ? 'Loading overrides...' : `Read/write app event policy · ${apps.length} app instance${apps.length === 1 ? '' : 's'}, ${totalEventTypes} event type${totalEventTypes === 1 ? '' : 's'}, ${totalEvents} observed event${totalEvents === 1 ? '' : 's'}`}
          </div>
          {status && <div className="app-control-subtitle">{status}</div>}
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
          <div className="model-view-controls">
            <button
              className={`filter-btn ${scopeFilter === 'session' ? 'active' : ''}`}
              onClick={() => setScopeFilter('session')}
            >
              Session observed
            </button>
            <button
              className={`filter-btn ${scopeFilter === 'all' ? 'active' : ''}`}
              onClick={() => setScopeFilter('all')}
            >
              All configured
            </button>
          </div>
          <div className="app-control-agents">
            {selectedAgents.length === 0 ? (
              <span className="app-control-pill muted">no agents selected</span>
            ) : selectedAgents.map(agent => (
              <span className="app-control-pill" key={agent}>{agent}</span>
            ))}
          </div>
        </div>
      </div>

      {apps.length === 0 ? (
        <div className="model-empty">
          No AppFS event metadata found yet. Trigger an AppFS app action or receive an app event, then reload this view.
        </div>
      ) : (
        <div className="app-control-list">
          {apps.map(app => (
            <AppCard
              key={app.key}
              app={app}
              overrides={overrides}
              onChangeOverrides={setOverrides}
              onStatus={setStatus}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function AppCard({
  app,
  overrides,
  onChangeOverrides,
  onStatus,
}: {
  app: AppSummary;
  overrides: AppEventRenderOverridesDoc;
  onChangeOverrides: (overrides: AppEventRenderOverridesDoc) => void;
  onStatus: (status: string) => void;
}) {
  const events = Array.from(app.eventTypes.values()).sort((a, b) => {
    if (b.lastSeen !== a.lastSeen) return b.lastSeen - a.lastSeen;
    return a.eventType.localeCompare(b.eventType);
  });

  return (
    <section className="app-control-card">
      <div className="app-control-card-head">
        <div>
          <div className="app-control-title">
            <span className="app-control-app-name">{app.label}</span>
            {app.principalId && <span className="app-control-pill">principal {app.principalId}</span>}
            {app.streamId && <span className="app-control-pill muted">{app.streamId}</span>}
          </div>
          <div className="app-control-subtitle">
            Seen by {Array.from(app.agents).sort().join(', ')} · last event {formatTime(app.lastSeen)}
          </div>
        </div>
        <div className="app-control-stats">
          <span>{app.totalEvents} events</span>
          <span>{app.eventTypes.size} types</span>
        </div>
      </div>

      <div className="app-control-event-list">
        {events.map(event => (
          <EventCard
            key={event.eventType}
            app={app}
            event={event}
            overrides={overrides}
            onChangeOverrides={onChangeOverrides}
            onStatus={onStatus}
          />
        ))}
      </div>
    </section>
  );
}

function EventCard({
  app,
  event,
  overrides,
  onChangeOverrides,
  onStatus,
}: {
  app: AppSummary;
  event: AppEventSummary;
  overrides: AppEventRenderOverridesDoc;
  onChangeOverrides: (overrides: AppEventRenderOverridesDoc) => void;
  onStatus: (status: string) => void;
}) {
  const sample = event.samples[0];
  const streamKey = app.streamId ?? `app:${app.appId}`;
  const overrideMetadata = getEventOverrideMetadata(overrides, streamKey, event.eventType);
  const baseMetadata = sample?.metadata;
  const effectiveMetadata = mergeEventRenderMetadata(baseMetadata, overrideMetadata);
  const [draft, setDraft] = React.useState<EventDraft>(() => draftFromMetadata(effectiveMetadata, sample));
  const [saving, setSaving] = React.useState(false);

  React.useEffect(() => {
    setDraft(draftFromMetadata(effectiveMetadata, sample));
  }, [effectiveMetadata, sample]);

  const preview = renderEventPreview(draft, sample, app.label);
  const terminalPreviewLines = renderTerminalPreview(draft, sample, app.label);
  const deliverySummary = draft.wake ? 'wake on' : 'wake off';

  const save = async () => {
    if (!sample) {
      return;
    }
    setSaving(true);
    onStatus(`Saving ${app.label} · ${event.eventType}...`);
    const nextOverrides = upsertOverride(
      overrides,
      streamKey,
      event.eventType,
      draftToOverride(draft),
    );
    try {
      const response = await fetch('/api/app-event-overrides', {
        method: 'PUT',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify(nextOverrides),
      });
      if (!response.ok) {
        throw new Error(`save failed: ${response.status}`);
      }
      const saved: AppEventRenderOverridesDoc = await response.json();
      onChangeOverrides(normalizeOverrides(saved));
      onStatus(`Saved ${app.label} · ${event.eventType}`);
    } catch (error) {
      onStatus(`Save failed for ${app.label} · ${event.eventType}: ${(error as Error).message}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="app-control-event">
      <div className="app-control-event-head">
        <div className="app-control-event-title">
          <span className="app-control-event-type">{event.eventType}</span>
          <span className="app-control-pill">x{event.count}</span>
          <span className="app-control-pill">{draft.classValue}</span>
          <span className="app-control-pill">{humanizeMode(draft.mode)}</span>
          <span className={`app-control-pill ${draft.wake ? 'hot' : ''}`}>{deliverySummary}</span>
          <span className="app-control-pill">{humanizeDelivery(draft.runningDelivery)}</span>
        </div>
        <div className="app-control-subtitle">
          last seen {formatTime(event.lastSeen)} · agents {Array.from(event.agents).sort().join(', ')}
        </div>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(300px, 1fr))', gap: '16px', marginBottom: '12px' }}>
        <div className="app-control-preview" style={{ marginBottom: 0 }}>
          <div className="app-control-preview-label">Model preview</div>
          <div className="app-control-preview-text" style={{ minHeight: '130px' }}>{preview || '(empty)'}</div>
        </div>

        <div className="app-control-preview" style={{ marginBottom: 0 }}>
          <div className="app-control-preview-label" style={{ color: '#cba6f7' }}>Terminal preview</div>
          <TerminalPreviewCard lines={terminalPreviewLines} />
        </div>
      </div>

      <div className="app-control-kv-grid">
        <KeyValue label="Template mode" value={draft.mode} />
        <KeyValue label="Wake" value={draft.wake ? 'yes' : 'no'} />
        <KeyValue label="Delivery" value={draft.runningDelivery} />
        <KeyValue label="Correlation" value={sample?.correlationId ?? 'n/a'} />
      </div>

      <div className="app-control-edit-grid">
        <label className="app-control-field">
          <span>Class</span>
          <select value={draft.classValue} onChange={e => setDraft(prev => ({ ...prev, classValue: e.target.value }))}>
            <option value="attention">attention</option>
            <option value="guidance">guidance</option>
            <option value="receipt">receipt</option>
            <option value="status">status</option>
            <option value="noise">noise</option>
          </select>
        </label>

        <label className="app-control-field">
          <span>Mode</span>
          <select value={draft.mode} onChange={e => setDraft(prev => ({ ...prev, mode: e.target.value }))}>
            <option value="summary">summary</option>
            <option value="body_with_source_reminder">body_with_source_reminder</option>
            <option value="debug_only">debug_only</option>
            <option value="hidden">hidden</option>
            <option value="drop">drop</option>
          </select>
        </label>

        <label className="app-control-field">
          <span>Wake</span>
          <select value={draft.wake ? 'yes' : 'no'} onChange={e => setDraft(prev => ({ ...prev, wake: e.target.value === 'yes' }))}>
            <option value="yes">yes</option>
            <option value="no">no</option>
          </select>
        </label>

        <label className="app-control-field">
          <span>Running delivery</span>
          <select value={draft.runningDelivery} onChange={e => setDraft(prev => ({ ...prev, runningDelivery: e.target.value }))}>
            <option value="inject_at_next_boundary">inject_at_next_boundary</option>
            <option value="queue_after_turn">queue_after_turn</option>
            <option value="context_only">context_only</option>
            <option value="wake_if_idle">wake_if_idle</option>
            <option value="drop">drop</option>
          </select>
        </label>

        <label className="app-control-field">
          <span>Idle delivery</span>
          <select value={draft.idleDelivery} onChange={e => setDraft(prev => ({ ...prev, idleDelivery: e.target.value }))}>
            <option value="wake_if_idle">wake_if_idle</option>
            <option value="context_only">context_only</option>
            <option value="drop">drop</option>
          </select>
        </label>
      </div>

      <div className="app-control-template-grid">
        <label className="app-control-field">
          <span>Summary template</span>
          <textarea
            rows={4}
            value={draft.template}
            onChange={e => setDraft(prev => ({ ...prev, template: e.target.value }))}
          />
        </label>
        <label className="app-control-field">
          <span>Body template</span>
          <textarea
            rows={3}
            value={draft.bodyTemplate}
            onChange={e => setDraft(prev => ({ ...prev, bodyTemplate: e.target.value }))}
          />
        </label>
        <label className="app-control-field">
          <span>Source template</span>
          <textarea
            rows={3}
            value={draft.sourceTemplate}
            onChange={e => setDraft(prev => ({ ...prev, sourceTemplate: e.target.value }))}
          />
        </label>
        <label className="app-control-field" style={{ gridColumn: 'span 3' }}>
          <span>Terminal wake card template (one line per template line)</span>
          <textarea
            rows={4}
            placeholder={`{{ansi.cyan}}{{app.display_name}} · from {{message.sender}}{{ansi.reset}}\n{{message.body}}`}
            value={draft.terminalLines}
            onChange={e => setDraft(prev => ({ ...prev, terminalLines: e.target.value }))}
            style={{ fontFamily: 'Consolas, Monaco, monospace' }}
          />
        </label>
      </div>

      <div className="app-control-actions">
        <button className="view-tab active" onClick={save} disabled={saving}>
          {saving ? 'Saving...' : 'Save override'}
        </button>
      </div>

      {effectiveMetadata && (
        <CollapsibleBlock label="Show effective render metadata">
          {formatJson(effectiveMetadata)}
        </CollapsibleBlock>
      )}
      {(sample?.payload !== undefined || sample?.rawEvent !== undefined) && (
        <CollapsibleBlock label="Show example payload">
          {formatJson({
            payload: sample.payload,
            raw_event: sample.rawEvent,
          })}
        </CollapsibleBlock>
      )}
    </div>
  );
}

function KeyValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="app-control-kv">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function collectAppSummaries(
  entries: TimelineEntry[],
  selectedAgents: string[],
  overrides: AppEventRenderOverridesDoc,
  showAllConfigured: boolean = false,
): AppSummary[] {
  const apps = new Map<string, AppSummary>();
  const selectedAgentSet = new Set(selectedAgents);

  for (const entry of entries) {
    if (entry.source === 'debug-dump') {
      continue;
    }
    if (selectedAgentSet.size > 0 && !selectedAgentSet.has(entry.agentName)) {
      continue;
    }
    const message = entry.raw as ConversationMessage;
    if (!message.blocks) {
      continue;
    }

    for (const block of message.blocks) {
      if (block.type !== 'input_router') {
        continue;
      }
      for (const input of block.inputs) {
        if (input.source !== 'appfs_event') {
          continue;
        }
        const app = ensureAppSummary(apps, input);
        app.agents.add(entry.agentName);
        app.totalEvents += 1;
        app.lastSeen = Math.max(app.lastSeen, entry.timestamp);

        const effectiveMetadata = mergeEventRenderMetadata(
          asEventRenderMetadata(input.event_render_metadata),
          getEventOverrideMetadata(
            overrides,
            app.streamId ?? `app:${app.appId}`,
            input.input_type,
            app.appId,
          ),
        );
        const event = ensureEventSummary(app, input.input_type);
        event.count += 1;
        event.lastSeen = Math.max(event.lastSeen, entry.timestamp);
        event.agents.add(entry.agentName);
        if (input.delivery) event.deliveries.add(input.delivery);
        if (effectiveMetadata?.class) event.classes.add(String(effectiveMetadata.class));
        if (event.samples.length < 3) {
          event.samples.push({
            agentName: entry.agentName,
            appLabel: app.label,
            appId: app.appId,
            streamId: app.streamId ?? `app:${app.appId}`,
            eventType: input.input_type,
            timestamp: entry.timestamp,
            text: input.text,
            payload: input.payload,
            rawEvent: input.raw_event,
            metadata: effectiveMetadata,
            delivery: input.delivery,
            requiresAttention: input.requires_attention,
            seq: input.seq,
            correlationId: input.correlation_id,
            eventPath: input.event_path,
          });
        }
      }
    }
  }

  if (showAllConfigured) {
    if (overrides.streams) {
      for (const [streamId, scope] of Object.entries(overrides.streams)) {
        if (!scope?.events) continue;
        const appId = streamId.startsWith('app:') ? streamId.slice('app:'.length) : streamId;
        const dummyInput: InputRouterBlockInput = {
          stream_id: streamId,
          app_id: appId,
          source: 'appfs_event',
          input_type: '',
          text: '',
        };
        const app = ensureAppSummary(apps, dummyInput);
        
        for (const [eventType, eventMeta] of Object.entries(scope.events)) {
          const event = ensureEventSummary(app, eventType);
          const effectiveMetadata = asEventRenderMetadata(eventMeta);
          if (effectiveMetadata?.class) event.classes.add(String(effectiveMetadata.class));
          if (event.samples.length === 0) {
            event.samples.push({
              agentName: 'system',
              appLabel: app.label,
              appId: app.appId,
              streamId: app.streamId ?? `app:${app.appId}`,
              eventType,
              timestamp: Date.now(),
              text: '(no active event sample in session)',
              metadata: effectiveMetadata,
            });
          }
        }
      }
    }

    if (overrides.apps) {
      for (const [appId, scope] of Object.entries(overrides.apps)) {
        if (!scope?.events) continue;
        const streamId = `app:${appId}`;
        const dummyInput: InputRouterBlockInput = {
          stream_id: streamId,
          app_id: appId,
          source: 'appfs_event',
          input_type: '',
          text: '',
        };
        const app = ensureAppSummary(apps, dummyInput);
        
        for (const [eventType, eventMeta] of Object.entries(scope.events)) {
          const event = ensureEventSummary(app, eventType);
          const effectiveMetadata = asEventRenderMetadata(eventMeta);
          if (effectiveMetadata?.class) event.classes.add(String(effectiveMetadata.class));
          if (event.samples.length === 0) {
            event.samples.push({
              agentName: 'system',
              appLabel: app.label,
              appId: app.appId,
              streamId: app.streamId ?? `app:${app.appId}`,
              eventType,
              timestamp: Date.now(),
              text: '(no active event sample in session)',
              metadata: effectiveMetadata,
            });
          }
        }
      }
    }

    if (overrides.platform?.events) {
      const dummyInput: InputRouterBlockInput = {
        stream_id: 'platform',
        app_id: 'platform',
        source: 'appfs_event',
        input_type: '',
        text: '',
      };
      const app = ensureAppSummary(apps, dummyInput);
      for (const [eventType, eventMeta] of Object.entries(overrides.platform.events)) {
        const event = ensureEventSummary(app, eventType);
        const effectiveMetadata = asEventRenderMetadata(eventMeta);
        if (effectiveMetadata?.class) event.classes.add(String(effectiveMetadata.class));
        if (event.samples.length === 0) {
          event.samples.push({
            agentName: 'system',
            appLabel: app.label,
            appId: app.appId,
            streamId: app.streamId ?? `app:${app.appId}`,
            eventType,
            timestamp: Date.now(),
            text: '(no active event sample in session)',
            metadata: effectiveMetadata,
          });
        }
      }
    }
    if (overrides.discoveredApps) {
      for (const [streamId, appInfo] of Object.entries(overrides.discoveredApps)) {
        if (!appInfo?.events) continue;
        const appId = appInfo.appId;
        const dummyInput: InputRouterBlockInput = {
          stream_id: streamId,
          app_id: appId,
          source: 'appfs_event',
          input_type: '',
          text: '',
          principal_id: appInfo.principalId,
        };
        const app = ensureAppSummary(apps, dummyInput);
        
        for (const [eventType, eventMeta] of Object.entries(appInfo.events)) {
          const event = ensureEventSummary(app, eventType);
          const overrideMetadata = getEventOverrideMetadata(
            overrides,
            streamId,
            eventType,
            appId,
          );
          const baseMetadata = asEventRenderMetadata(eventMeta);
          const effectiveMetadata = mergeEventRenderMetadata(baseMetadata, overrideMetadata);
          
          if (effectiveMetadata?.class) event.classes.add(String(effectiveMetadata.class));
          if (event.samples.length === 0) {
            event.samples.push({
              agentName: 'system',
              appLabel: app.label,
              appId: app.appId,
              streamId: app.streamId ?? `app:${app.appId}`,
              eventType,
              timestamp: Date.now(),
              text: '(no active event sample in session)',
              metadata: effectiveMetadata,
            });
          }
        }
      }
    }
  }

  return Array.from(apps.values()).sort((a, b) => {
    if (b.lastSeen !== a.lastSeen) return b.lastSeen - a.lastSeen;
    return a.label.localeCompare(b.label);
  });
}

function ensureAppSummary(apps: Map<string, AppSummary>, input: InputRouterBlockInput): AppSummary {
  const streamId = input.stream_id ?? `app:${input.app_id ?? 'unknown'}`;
  const appId = input.app_id ?? (streamId === 'platform' ? 'platform' : 'unknown');
  const principalId = input.principal_id;
  const key = streamId;
  let app = apps.get(key);
  if (!app) {
    app = {
      key,
      appId,
      principalId,
      streamId,
      label: formatAppLabel(appId, principalId),
      agents: new Set(),
      totalEvents: 0,
      lastSeen: 0,
      eventTypes: new Map(),
    };
    apps.set(key, app);
  }
  return app;
}

function ensureEventSummary(app: AppSummary, eventType: string): AppEventSummary {
  let event = app.eventTypes.get(eventType);
  if (!event) {
    event = {
      eventType,
      count: 0,
      lastSeen: 0,
      agents: new Set(),
      deliveries: new Set(),
      classes: new Set(),
      samples: [],
    };
    app.eventTypes.set(eventType, event);
  }
  return event;
}

function asEventRenderMetadata(value: unknown): EventRenderMetadata | undefined {
  if (typeof value === 'object' && value !== null && !Array.isArray(value)) {
    return value as EventRenderMetadata;
  }
  return undefined;
}

function getEventOverrideMetadata(
  overrides: AppEventRenderOverridesDoc,
  streamKey: string,
  eventType: string,
  appId?: string,
): EventRenderMetadata | undefined {
  const streamOverride = overrides.streams?.[streamKey];
  const appOverride = appId ? overrides.apps?.[appId] : overrides.apps?.[streamKey];
  const platformOverride = streamKey === 'platform' ? overrides.platform : undefined;
  const eventOverride = streamOverride?.events?.[eventType] ??
    appOverride?.events?.[eventType] ??
    platformOverride?.events?.[eventType];
  return asEventRenderMetadata(eventOverride);
}

function mergeEventRenderMetadata(
  base?: EventRenderMetadata,
  override?: EventRenderMetadata,
): EventRenderMetadata | undefined {
  if (!base && !override) {
    return undefined;
  }
  if (!base) {
    return override;
  }
  if (!override) {
    return base;
  }
  return mergeJson(base, override) as EventRenderMetadata;
}

function mergeJson(base: unknown, override: unknown): unknown {
  if (isPlainObject(base) && isPlainObject(override)) {
    const merged: Record<string, unknown> = { ...base };
    for (const [key, value] of Object.entries(override)) {
      merged[key] = key in merged ? mergeJson(merged[key], value) : value;
    }
    return merged;
  }
  return override;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function draftFromMetadata(
  metadata: EventRenderMetadata | undefined,
  sample?: AppEventSample,
): EventDraft {
  const modelRender = metadata?.model_render ?? {};
  const mode = stringOrDefault(
    modelRender.mode ??
      modelRender.visibility ??
      metadata?.mode ??
      metadata?.visibility,
    DEFAULT_DRAFT.mode,
  );
  const template = stringOrDefault(
    modelRender.template ?? metadata?.template,
    DEFAULT_DRAFT.template,
  );
  const bodyTemplate = stringOrDefault(modelRender.body_template, DEFAULT_DRAFT.bodyTemplate);
  const sourceTemplate = stringOrDefault(modelRender.source_template, DEFAULT_DRAFT.sourceTemplate);
  const wake = metadata?.wake ?? Boolean(sample?.requiresAttention);
  const runningDelivery = stringOrDefault(
    metadata?.running_delivery,
    wake ? 'inject_at_next_boundary' : DEFAULT_DRAFT.runningDelivery,
  );
  const idleDelivery = stringOrDefault(
    metadata?.idle_delivery,
    wake ? 'wake_if_idle' : DEFAULT_DRAFT.idleDelivery,
  );
  const classValue = stringOrDefault(metadata?.class, DEFAULT_DRAFT.classValue);

  // Read terminal render overrides or legacy ui/user renders
  const terminalRender = metadata?.terminal_render ?? metadata?.ui_render ?? metadata?.user_render ?? {};
  const terminalLines = Array.isArray(terminalRender.lines)
    ? terminalRender.lines.filter((line): line is string => typeof line === 'string').join('\n')
    : stringOrDefault(terminalRender.template, DEFAULT_DRAFT.terminalLines);

  return {
    classValue,
    mode,
    wake,
    runningDelivery,
    idleDelivery,
    template,
    bodyTemplate,
    sourceTemplate,
    terminalLines,
  };
}

function draftToOverride(draft: EventDraft): EventRenderMetadata {
  const modelRender: Record<string, unknown> = {
    mode: draft.mode,
    template: draft.template,
  };
  if (draft.mode === 'body_with_source_reminder') {
    modelRender.body_template = draft.bodyTemplate;
    modelRender.source_template = draft.sourceTemplate;
  }

  const terminalLinesArray = draft.terminalLines
    .split('\n')
    .map(line => line.trimEnd())
    .filter(line => line.length > 0);

  const override: EventRenderMetadata = {
    class: draft.classValue,
    wake: draft.wake,
    running_delivery: draft.runningDelivery,
    idle_delivery: draft.idleDelivery,
    model_render: modelRender,
  };

  if (terminalLinesArray.length > 0) {
    override.terminal_render = {
      mode: 'card',
      lines: terminalLinesArray,
    };
  }

  return override;
}

function upsertOverride(
  current: AppEventRenderOverridesDoc,
  streamKey: string,
  eventType: string,
  metadata: EventRenderMetadata,
): AppEventRenderOverridesDoc {
  const next: AppEventRenderOverridesDoc = normalizeOverrides({
    ...current,
    streams: { ...(current.streams ?? {}) },
  });
  const streams = { ...(next.streams ?? {}) };
  const scope: AppEventRenderScopeOverride = {
    events: {
      ...((streams[streamKey]?.events) ?? {}),
      [eventType]: metadata,
    },
  };
  streams[streamKey] = scope;
  return {
    version: 1,
    streams,
    apps: next.apps,
    platform: next.platform,
  };
}

function renderEventPreview(draft: EventDraft, sample?: AppEventSample): string {
  if (!sample) {
    return '(no sample available)';
  }
  if (draft.mode === 'drop') {
    return 'Dropped from model context.';
  }
  if (draft.mode === 'debug_only' || draft.mode === 'hidden') {
    return 'Debug only. Hidden from the model.';
  }
  if (draft.mode === 'body_with_source_reminder') {
    const body = renderTemplate(draft.bodyTemplate, sample);
    const source = renderTemplate(draft.sourceTemplate, sample);
    return [body, `<system-reminder>${source}</system-reminder>`].filter(Boolean).join('\n\n');
  }
  return renderTemplate(draft.template, sample);
}

function renderTemplate(template: string, sample: AppEventSample): string {
  return template.replace(/\{\{\s*([^}]+?)\s*\}\}/g, (_, rawKey: string) => {
    const key = rawKey.trim();
    return resolveTemplateToken(key, sample);
  });
}

function resolveTemplateToken(key: string, sample: AppEventSample): string {
  // Resolve message helpers
  if (key === 'message.sender') {
    const payload = sample.payload as Record<string, unknown> | undefined;
    return String(
      payload?.from_display_name ||
      payload?.from_principal ||
      payload?.contact_key ||
      'unknown'
    );
  }
  if (key === 'message.body') {
    const payload = sample.payload as Record<string, unknown> | undefined;
    return String(
      payload?.text ||
      payload?.text_preview ||
      sample.text ||
      ''
    );
  }

  // Resolve ANSI sequences
  switch (key) {
    case 'ansi.bold':
      return '\x1b[1m';
    case 'ansi.dim':
      return '\x1b[2m';
    case 'ansi.italic':
      return '\x1b[3m';
    case 'ansi.underline':
      return '\x1b[4m';
    case 'ansi.reset':
      return '\x1b[0m';
    case 'ansi.cyan':
      return '\x1b[36m';
    case 'ansi.green':
      return '\x1b[32m';
    case 'ansi.yellow':
      return '\x1b[33m';
    case 'ansi.blue':
      return '\x1b[34m';
    case 'ansi.magenta':
      return '\x1b[35m';
    case 'ansi.red':
      return '\x1b[31m';
    case 'ansi.gray':
      return '\x1b[90m';

    case 'type':
      return sample.eventType;
    case 'path':
      return sample.eventPath ?? '';
    case 'seq':
      return sample.seq !== undefined ? String(sample.seq) : '';
    case 'app.display_name':
      return sample.appLabel;
    case 'app.id':
    case 'app_id':
      return sample.appId;
    case 'stream':
      return sample.streamId;
    default:
      break;
  }
  if (key.startsWith('content.')) {
    return resolveNestedValue(sample.payload, key.slice('content.'.length));
  }
  if (key.startsWith('error.')) {
    return resolveNestedValue(sample.rawEvent, key.slice('error.'.length));
  }
  if (key === 'content.text_preview') {
    return resolveNestedValue(sample.payload, 'text_preview');
  }
  return '';
}

function resolveNestedValue(value: unknown, path: string): string {
  if (!isPlainObject(value)) {
    return '';
  }
  let current: unknown = value;
  for (const part of path.split('.')) {
    if (!isPlainObject(current) && !Array.isArray(current)) {
      return '';
    }
    current = (current as Record<string, unknown>)[part];
    if (current === undefined || current === null) {
      return '';
    }
  }
  if (typeof current === 'string') {
    return current;
  }
  if (typeof current === 'number' || typeof current === 'boolean') {
    return String(current);
  }
  try {
    return JSON.stringify(current);
  } catch {
    return String(current);
  }
}

function normalizeOverrides(doc: AppEventRenderOverridesDoc | undefined): AppEventRenderOverridesDoc {
  const next: AppEventRenderOverridesDoc = {
    version: 1,
    streams: normalizeScopes(doc?.streams),
    apps: normalizeScopes(doc?.apps),
    platform: normalizeScope(doc?.platform),
    discoveredApps: doc?.discoveredApps,
  };
  return next;
}

function normalizeScopes(
  scopes: Record<string, AppEventRenderScopeOverride> | undefined,
): Record<string, AppEventRenderScopeOverride> | undefined {
  if (!scopes) {
    return undefined;
  }
  const normalized: Record<string, AppEventRenderScopeOverride> = {};
  for (const [key, value] of Object.entries(scopes)) {
    const scope = normalizeScope(value);
    if (scope) {
      normalized[key] = scope;
    }
  }
  return Object.keys(normalized).length > 0 ? normalized : undefined;
}

function normalizeScope(value: AppEventRenderScopeOverride | undefined): AppEventRenderScopeOverride | undefined {
  if (!value || typeof value !== 'object') {
    return undefined;
  }
  const events = value.events && typeof value.events === 'object' && !Array.isArray(value.events)
    ? value.events
    : undefined;
  if (!events) {
    return undefined;
  }
  return { events: { ...events } };
}

function formatAppLabel(appId: string, principalId?: string): string {
  if (appId === 'platform') {
    return 'AppFS platform';
  }
  return principalId ? `${appId} / ${principalId}` : appId;
}

function humanizeMode(mode: string): string {
  switch (mode) {
    case 'body_with_source_reminder':
      return 'body + source';
    case 'debug_only':
      return 'debug only';
    case 'drop':
    case 'hidden':
      return 'hidden';
    case 'summary':
    case 'model':
      return 'summary';
    default:
      return mode;
  }
}

function humanizeDelivery(delivery: string): string {
  switch (delivery) {
    case 'inject_at_next_boundary':
      return 'boundary';
    case 'queue_after_turn':
      return 'queued';
    case 'context_only':
      return 'context';
    case 'wake_if_idle':
      return 'wake';
    case 'drop':
      return 'drop';
    default:
      return delivery;
  }
}

function stringOrDefault(value: unknown, fallback: string): string {
  return typeof value === 'string' && value.trim().length > 0 ? value : fallback;
}

function formatJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function formatTime(timestamp: number): string {
  if (!timestamp) {
    return 'unknown time';
  }
  return new Date(timestamp).toLocaleString();
}

function TerminalPreviewCard({ lines }: { lines: string[] }) {
  if (lines.length === 0) {
    return (
      <div style={{
        fontFamily: 'Consolas, Monaco, monospace',
        backgroundColor: '#1e1e2e',
        color: '#585b70',
        padding: '16px',
        borderRadius: '8px',
        fontSize: '13px',
        border: '1px solid #313244',
        minHeight: '130px',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        fontStyle: 'italic',
      }}>
        (card hidden or dropped)
      </div>
    );
  }

  const title = "AppFS Wake";
  const border = "─".repeat(title.length + 10);
  
  const bodyText = lines
    .map(line => `\u001b[38;5;245m│\u001b[0m ${line}`)
    .join('\n');
    
  const fullAnsiText = `\u001b[38;5;245m╭─ \u001b[1;35m${title}\u001b[0;38;5;245m ─╮\u001b[0m\n${bodyText}\n\u001b[38;5;245m╰${border}╯\u001b[0m`;

  return (
    <pre style={{
      fontFamily: 'Consolas, Monaco, monospace',
      backgroundColor: '#1e1e2e',
      color: '#cdd6f4',
      padding: '16px',
      borderRadius: '8px',
      fontSize: '13px',
      border: '1px solid #313244',
      margin: 0,
      minHeight: '130px',
      maxHeight: '220px',
      overflowX: 'auto',
      overflowY: 'auto',
      whiteSpace: 'pre-wrap',
      lineHeight: '1.5',
    }}>
      {ansiToHtml(fullAnsiText)}
    </pre>
  );
}

function renderTerminalPreview(draft: EventDraft, sample?: AppEventSample, appLabel?: string): string[] {
  if (!sample) {
    return [];
  }
  
  const terminalLinesArray = draft.terminalLines
    .split('\n')
    .map(line => line.trimEnd())
    .filter(line => line.length > 0);

  if (terminalLinesArray.length > 0) {
    return terminalLinesArray.map(template => renderTemplate(template, sample));
  }

  const app_label = appLabel || sample.appLabel || 'appfs';
  const lines: string[] = [];

  if (sample.eventType === 'message.received') {
    const payload = sample.payload as Record<string, unknown> | undefined;
    const from = String(
      payload?.from_display_name ||
      payload?.from_principal ||
      payload?.contact_key ||
      'unknown'
    );
    let meta = `${app_label} · message.received · from ${from}`;
    if (sample.requiresAttention) {
      meta += ' · attention required';
    }
    lines.push(`\u001b[1;36m${meta}\u001b[0m`);

    const body = String(
      payload?.text ||
      payload?.text_preview ||
      singleLinePreview(sample.text, 280)
    );
    if (body.trim()) {
      lines.push(body);
    }
    return lines;
  }

  let meta = `${app_label} · ${sample.eventType.trim()}`;
  if (sample.payload && (sample.payload as Record<string, unknown>).principal_id) {
    meta += ` · principal ${(sample.payload as Record<string, unknown>).principal_id}`;
  }
  if (sample.requiresAttention) {
    meta += ' · attention required';
  }
  lines.push(`\u001b[1;36m${meta}\u001b[0m`);

  const preview = singleLinePreview(sample.text, 280);
  if (preview.trim()) {
    lines.push(preview);
  }

  return lines;
}

function singleLinePreview(text: string, maxChars: number): string {
  if (!text) return '';
  const collapsed = text.split(/\s+/).join(' ');
  if (collapsed.length <= maxChars) {
    return collapsed;
  }
  return collapsed.substring(0, maxChars) + '…';
}

function ansiToHtml(text: string): React.ReactNode[] {
  const regex = /\u001b\[([0-9;]*)m/g;
  const parts: React.ReactNode[] = [];
  let lastIndex = 0;
  
  let bold = false;
  let dim = false;
  let italic = false;
  let underline = false;
  let color = '';

  const getColor = (num: number): string => {
    switch (num) {
      case 31: return '#f38ba8'; // red
      case 32: return '#a6e3a1'; // green
      case 33: return '#f9e2af'; // yellow
      case 34: return '#89b4fa'; // blue
      case 35: return '#cba6f7'; // magenta
      case 36: return '#89dceb'; // cyan
      case 90: return '#585b70'; // gray
      default: return '';
    }
  };

  let match;
  let key = 0;
  while ((match = regex.exec(text)) !== null) {
    const textChunk = text.substring(lastIndex, match.index);
    if (textChunk) {
      const style: React.CSSProperties = {};
      if (bold) style.fontWeight = 'bold';
      if (dim) style.opacity = 0.6;
      if (italic) style.fontStyle = 'italic';
      if (underline) style.textDecoration = 'underline';
      if (color) style.color = color;
      parts.push(<span key={key++} style={style}>{textChunk}</span>);
    }

    const codeStr = match[1];
    const codes = codeStr.split(';').map(Number);
    for (let i = 0; i < codes.length; i++) {
      const code = codes[i];
      if (code === 0) {
        bold = false;
        dim = false;
        italic = false;
        underline = false;
        color = '';
      } else if (code === 1) {
        bold = true;
      } else if (code === 2) {
        dim = true;
      } else if (code === 3) {
        italic = true;
      } else if (code === 4) {
        underline = true;
      } else if (code >= 30 && code <= 37) {
        color = getColor(code);
      } else if (code === 90) {
        color = '#585b70';
      } else if (code === 38 && codes[i+1] === 5 && codes[i+2] === 245) {
        color = '#8087a2';
        i += 2; // skip parameters
      }
    }
    lastIndex = regex.lastIndex;
  }

  const remaining = text.substring(lastIndex);
  if (remaining) {
    const style: React.CSSProperties = {};
    if (bold) style.fontWeight = 'bold';
    if (dim) style.opacity = 0.6;
    if (italic) style.fontStyle = 'italic';
    if (underline) style.textDecoration = 'underline';
    if (color) style.color = color;
    parts.push(<span key={key++} style={style}>{remaining}</span>);
  }

  return parts;
}
