/**
 * Parse a complete JSONL file into records.
 * Skips blank lines and records with unknown `type`.
 */
export function parseJsonl(content) {
    const records = [];
    for (const line of content.split('\n')) {
        const trimmed = line.trim();
        if (!trimmed)
            continue;
        try {
            const record = JSON.parse(trimmed);
            if (record && typeof record === 'object' && typeof record.type === 'string') {
                records.push(record);
            }
        }
        catch {
            // Skip malformed lines
        }
    }
    return records;
}
/**
 * Parse only message records from JSONL content.
 */
export function parseMessages(content) {
    return parseJsonl(content).filter((r) => r.type === 'message');
}
/**
 * Extract session_meta record if present.
 */
export function parseMeta(content) {
    return parseJsonl(content).find((r) => r.type === 'session_meta');
}
/**
 * Parse only debug-dump (message_request) records from JSONL content.
 */
export function parseDebugDumps(content) {
    return parseJsonl(content).filter((r) => r.type === 'message_request');
}
/**
 * Parse only compaction archive records from JSONL content.
 * These contain messages that were removed during session compaction.
 */
export function parseCompactionArchives(content) {
    return parseJsonl(content).filter((r) => r.type === 'compaction_archive');
}
/**
 * Parse only compaction boundary records from JSONL content.
 * These mark the point where a session compaction removed older messages.
 */
export function parseCompactionBoundaries(content) {
    return parseJsonl(content).filter((r) => r.type === 'compaction_boundary');
}
/**
 * Parse only new lines appended since the last known line count.
 * Returns parsed records from the new lines only.
 */
export function parseNewLines(content, previousLineCount) {
    const lines = content.split('\n');
    const newLines = lines.slice(previousLineCount);
    return parseJsonl(newLines.join('\n'));
}
