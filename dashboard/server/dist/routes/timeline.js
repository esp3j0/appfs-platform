import { isModelContextAttachmentMessage } from '../session-message-filters.js';
export function registerTimelineRoute(app, registry) {
    app.get('/api/timeline', async (request) => {
        const query = request.query;
        const agentNames = (query.agents ?? '').split(',').filter(Boolean).map(decodeURIComponent);
        if (agentNames.length === 0) {
            return { entries: [], interactions: [], compactionBoundaries: [] };
        }
        const interactions = [];
        const compactionBoundaries = [];
        const perAgent = new Map();
        for (const name of agentNames) {
            const entries = [];
            for (const rec of registry.getMessages(name)) {
                const msg = rec.message;
                if (isModelContextAttachmentMessage(msg)) {
                    continue;
                }
                const content = extractTextContent(msg.blocks);
                const timestamp = msg.timestamp_ms ?? 0;
                const entryId = `${name}:${msg.uuid}`;
                const appfsEvents = msg.role === 'user'
                    ? extractAppfsEvents(name, msg, content, 'session')
                    : [];
                entries.push({
                    id: entryId,
                    agentName: name,
                    timestamp,
                    source: 'session',
                    role: msg.role,
                    content,
                    raw: msg,
                    usage: msg.usage,
                    appfsEvents: appfsEvents.length > 0 ? appfsEvents : undefined,
                });
                addAppfsInteractions(name, timestamp, entryId, appfsEvents, interactions);
            }
            for (const dump of registry.getDebugDumps(name)) {
                const systemPrompt = dump.system ?? dump.system_prompt ?? '';
                const toolCount = dump.tools?.length ?? dump.tools_count ?? 0;
                const msgCount = dump.messages?.length ?? dump.message_count ?? 0;
                const sysLen = systemPrompt.length;
                entries.push({
                    id: `${name}:debug:${dump.request_index ?? entries.length}:${dump.timestamp_ms ?? 0}`,
                    agentName: name,
                    timestamp: dump.timestamp_ms ?? 0,
                    source: 'debug-dump',
                    role: 'system',
                    content: `model: ${dump.model || '?'} | max_tokens: ${dump.max_tokens} | messages: ${msgCount} | tools: ${toolCount} | system: ${sysLen} chars`,
                    raw: dump,
                });
            }
            for (const archive of registry.getCompactionArchives(name)) {
                const msg = archive.message;
                if (isModelContextAttachmentMessage(msg)) {
                    continue;
                }
                const content = extractTextContent(msg.blocks);
                const timestamp = msg.timestamp_ms ?? archive.timestamp_ms;
                const entryId = `${name}:archive:${msg.uuid}:${archive.timestamp_ms}`;
                const appfsEvents = msg.role === 'user'
                    ? extractAppfsEvents(name, msg, content, 'archive')
                    : [];
                entries.push({
                    id: entryId,
                    agentName: name,
                    timestamp,
                    source: 'compaction-archive',
                    role: msg.role,
                    content,
                    raw: msg,
                    usage: msg.usage,
                    appfsEvents: appfsEvents.length > 0 ? appfsEvents : undefined,
                });
                addAppfsInteractions(name, timestamp, entryId, appfsEvents, interactions);
            }
            for (const boundary of registry.getCompactionBoundaries(name)) {
                compactionBoundaries.push({
                    agentName: name,
                    timestamp: boundary.timestamp_ms,
                    compactionCount: boundary.compaction_count,
                    archivedMessageCount: boundary.archived_message_count,
                });
            }
            entries.sort((a, b) => a.timestamp - b.timestamp);
            perAgent.set(name, entries);
        }
        compactionBoundaries.sort((a, b) => a.timestamp - b.timestamp);
        return { entries: mergeTimelines(perAgent), interactions, compactionBoundaries };
    });
}
function addAppfsInteractions(agentName, timestamp, entryId, events, interactions) {
    for (const event of events) {
        const interaction = toCrossAgentInteraction(event, agentName, timestamp, entryId);
        if (interaction) {
            interactions.push(interaction);
        }
    }
}
function toCrossAgentInteraction(event, agentName, timestamp, entryId) {
    if (event.eventType !== 'message.sent' &&
        event.eventType !== 'message.received' &&
        event.eventType !== 'message.read') {
        return undefined;
    }
    const currentAgent = normalizeAgentName(agentName);
    let fromAgent = normalizeAgentName(event.fromAgent);
    let toAgent = normalizeAgentName(event.toAgent);
    const principal = normalizeAgentName(event.principal);
    const contactKey = normalizeAgentName(event.contactKey);
    if (event.eventType === 'message.received') {
        fromAgent = fromAgent || contactKey || (principal !== currentAgent ? principal : '');
        toAgent = toAgent || principal || currentAgent;
    }
    else if (event.eventType === 'message.sent') {
        fromAgent = fromAgent || principal || currentAgent;
        toAgent = toAgent || contactKey;
    }
    else if (event.eventType === 'message.read') {
        fromAgent = fromAgent || contactKey || (principal !== currentAgent ? principal : '');
        toAgent = toAgent || currentAgent;
    }
    if (!fromAgent || !toAgent || fromAgent === toAgent) {
        return undefined;
    }
    return {
        entryId,
        fromAgent,
        toAgent,
        eventType: event.eventType,
        timestamp,
        seq: event.seq,
        label: `${fromAgent} -> ${toAgent} (${event.eventType}${event.seq !== undefined ? ` #${event.seq}` : ''})`,
    };
}
function extractAppfsEvents(agentName, msg, content, idScope) {
    const structuredRecords = extractStructuredInputRouterEvents(agentName, msg, idScope);
    const kind = msg.attachment_metadata?.kind;
    const looksRelevant = structuredRecords.length > 0 ||
        kind === 'input_router' ||
        kind === 'appfs_events' ||
        content.includes('[appfs_event]') ||
        content.includes('[agent_message]') ||
        content.includes('来源：');
    if (!looksRelevant) {
        return [];
    }
    const records = [...structuredRecords];
    const lines = content.split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
        const line = lines[i].trim();
        const sourceMatch = line.match(/\[(appfs_event|agent_message|user_terminal|system)\]/);
        if (!sourceMatch) {
            const genericEvent = parseGenericAppfsEventLine(agentName, msg, line, idScope, records.length);
            if (genericEvent) {
                records.push(genericEvent);
            }
            continue;
        }
        const eventType = readKeyValue(line, 'type');
        if (!eventType) {
            continue;
        }
        const nextLine = lines[i + 1]?.trim() ?? '';
        const text = readKeyValue(line, 'text');
        const fromDisplay = readKeyValue(text ?? '', 'from_display_name');
        const fromPrincipal = readKeyValue(text ?? '', 'from_principal');
        const toDisplay = readKeyValue(text ?? '', 'to_display_name');
        const contactKey = readKeyValue(line, 'contact_key') ?? readKeyValue(text ?? '', 'contact_key');
        records.push({
            id: `${agentName}:${idScope}:${msg.uuid}:appfs:${records.length}`,
            parentMessageUuid: msg.uuid,
            source: sourceMatch[1],
            eventType,
            principal: readKeyValue(line, 'principal'),
            fromAgent: nextLine.match(/^From:\s*(.+)$/i)?.[1] ??
                readKeyValue(line, 'from') ??
                fromDisplay ??
                fromPrincipal,
            toAgent: readKeyValue(line, 'to_principal') ?? toDisplay,
            app: readKeyValue(line, 'app'),
            stream: readKeyValue(line, 'stream'),
            seq: parseInteger(readKeyValue(line, 'seq')),
            correlationId: readKeyValue(line, 'correlation_id'),
            contactKey,
            text,
            rawLine: line,
        });
    }
    const sourceReminder = lines.find(line => line.includes('来源：') && line.includes('from='));
    if (sourceReminder) {
        const sourceLine = sourceReminder.trim();
        const seq = parseInteger(readKeyValue(sourceLine, 'seq'));
        const alreadyCaptured = records.some(record => record.eventType === 'message.received' && record.seq === seq);
        if (!alreadyCaptured) {
            records.push({
                id: `${agentName}:${idScope}:${msg.uuid}:appfs:${records.length}`,
                parentMessageUuid: msg.uuid,
                source: 'appfs_event',
                eventType: 'message.received',
                principal: readKeyValue(sourceLine, 'to_principal'),
                fromAgent: readKeyValue(sourceLine, 'from'),
                toAgent: readKeyValue(sourceLine, 'to_principal') ?? agentName,
                app: sourceLine.match(/来源：([^\s，]+)/)?.[1],
                seq,
                contactKey: readKeyValue(sourceLine, 'contact_key'),
                text: content.split('<system-reminder>')[0]?.trim(),
                rawLine: sourceLine,
            });
        }
    }
    return records.map(record => ({
        ...record,
        fromAgent: normalizeAgentName(record.fromAgent) || undefined,
        toAgent: normalizeAgentName(record.toAgent) || undefined,
        principal: normalizeAgentName(record.principal) || undefined,
        contactKey: normalizeAgentName(record.contactKey) || undefined,
    }));
}
function extractStructuredInputRouterEvents(agentName, msg, idScope) {
    const records = [];
    for (const block of msg.blocks) {
        if (block.type !== 'input_router') {
            continue;
        }
        for (const input of block.inputs) {
            if (input.source !== 'appfs_event' && input.source !== 'agent_message') {
                continue;
            }
            const payload = asRecord(input.payload);
            const contactKey = stringField(payload, 'contact_key');
            const fromDisplay = stringField(payload, 'from_display_name');
            const fromPrincipal = stringField(payload, 'from_principal');
            const toDisplay = stringField(payload, 'to_display_name');
            records.push({
                id: `${agentName}:${idScope}:${msg.uuid}:input-router:${records.length}`,
                parentMessageUuid: msg.uuid,
                source: input.source,
                eventType: input.input_type,
                principal: input.principal_id,
                fromAgent: fromDisplay ?? fromPrincipal,
                toAgent: toDisplay ?? input.principal_id,
                app: input.app_id,
                stream: input.stream_id,
                seq: input.seq,
                correlationId: input.correlation_id,
                contactKey,
                text: input.text,
                rawLine: JSON.stringify(input),
            });
        }
    }
    return records;
}
function parseGenericAppfsEventLine(agentName, msg, line, idScope, index) {
    const match = line.match(/^-\s+\[[^\]]+\]\s+seq=(\d+)\s+([a-z]+(?:\.[a-z]+)+)\b/);
    if (!match) {
        return undefined;
    }
    const summary = readKeyValue(line, 'summary');
    const fromDisplay = readKeyValue(summary ?? '', 'from_display_name');
    const fromPrincipal = readKeyValue(summary ?? '', 'from_principal');
    const toDisplay = readKeyValue(summary ?? '', 'to_display_name');
    const contactKey = readKeyValue(line, 'contact_key') ?? readKeyValue(summary ?? '', 'contact_key');
    const principal = line.match(/principal `([^`]+)`/)?.[1];
    const app = readKeyValue(line, 'app') ?? line.match(/AppFS app `([^`]+)`/)?.[1];
    return {
        id: `${agentName}:${idScope}:${msg.uuid}:appfs:${index}`,
        parentMessageUuid: msg.uuid,
        source: 'appfs_event',
        eventType: match[2],
        principal,
        fromAgent: readKeyValue(line, 'from') ?? fromDisplay ?? fromPrincipal,
        toAgent: readKeyValue(line, 'to_principal') ?? toDisplay,
        app,
        stream: readKeyValue(line, 'stream'),
        seq: parseInteger(match[1]),
        correlationId: readKeyValue(line, 'request_id') ?? readKeyValue(line, 'correlation_id'),
        contactKey,
        text: summary,
        rawLine: line,
    };
}
function readKeyValue(text, key) {
    const match = new RegExp(`(?:^|[\\s,，;])${escapeRegExp(key)}=`).exec(text);
    if (!match || match.index === undefined) {
        return undefined;
    }
    const start = match.index + match[0].length;
    const rest = text.slice(start);
    const quoted = rest.match(/^(['"])(.*?)\1/);
    if (quoted) {
        return quoted[2].trim();
    }
    const boundary = rest.search(/[,，;\n]|\s+[A-Za-z_][\w.-]*=/);
    const raw = boundary >= 0 ? rest.slice(0, boundary) : rest;
    return raw.trim() || undefined;
}
function parseInteger(value) {
    if (!value) {
        return undefined;
    }
    const parsed = Number.parseInt(value, 10);
    return Number.isFinite(parsed) ? parsed : undefined;
}
function normalizeAgentName(value) {
    if (!value) {
        return '';
    }
    return value
        .trim()
        .replace(/^AppFS Agent\s+/i, '')
        .replace(/^agent:/i, '')
        .replace(/^['"]|['"]$/g, '')
        .replace(/[。.)\]]+$/g, '')
        .trim();
}
function escapeRegExp(value) {
    return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
function asRecord(value) {
    return value !== null && typeof value === 'object' && !Array.isArray(value)
        ? value
        : undefined;
}
function stringField(record, key) {
    const value = record?.[key];
    return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}
/**
 * K-way merge of per-agent timelines.
 *
 * Each agent's timeline is already in chronological JSONL order.
 * We merge them by comparing the current head of each list using
 * their timestamp_ms, preserving intra-agent order when timestamps
 * are equal (batch persistence).
 */
function mergeTimelines(perAgent) {
    const indices = new Map();
    const total = Array.from(perAgent.values()).reduce((s, e) => s + e.length, 0);
    const result = [];
    result.length = total;
    for (let i = 0; i < total; i++) {
        let bestAgent = null;
        let bestTs = Infinity;
        for (const [name, entries] of perAgent) {
            const idx = indices.get(name) ?? 0;
            if (idx < entries.length) {
                const ts = entries[idx].timestamp;
                if (ts < bestTs) {
                    bestTs = ts;
                    bestAgent = name;
                }
            }
        }
        if (bestAgent === null)
            break;
        const idx = indices.get(bestAgent) ?? 0;
        result[i] = perAgent.get(bestAgent)[idx];
        indices.set(bestAgent, idx + 1);
    }
    return result;
}
function extractTextContent(blocks) {
    return blocks
        .map(block => {
        if (block.type === 'text') {
            return block.text;
        }
        if (block.type === 'input_router') {
            return block.inputs.map(input => input.text).filter(Boolean).join('\n');
        }
        return '';
    })
        .filter(Boolean)
        .join('\n');
}
