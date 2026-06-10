import type { CompactionArchiveRecord, CompactionBoundaryRecord, DebugDumpRecord, JsonlRecord, MessageRecord, SessionMetaRecord, TurnErrorRecord } from './types.js';

/**
 * Parse a complete JSONL file into records.
 * Skips blank lines and records with unknown `type`.
 */
export function parseJsonl(content: string): JsonlRecord[] {
  const records: JsonlRecord[] = [];
  for (const line of content.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    try {
      const record = JSON.parse(trimmed);
      if (record && typeof record === 'object' && typeof record.type === 'string') {
        records.push(record);
      }
    } catch {
      // Skip malformed lines
    }
  }
  return records;
}

/**
 * Parse only message records from JSONL content.
 */
export function parseMessages(content: string): MessageRecord[] {
  return parseJsonl(content).filter(
    (r): r is MessageRecord => r.type === 'message',
  );
}

/**
 * Extract session_meta record if present.
 */
export function parseMeta(content: string): SessionMetaRecord | undefined {
  return parseJsonl(content).find(
    (r): r is SessionMetaRecord => r.type === 'session_meta',
  );
}

/**
 * Parse only debug-dump (message_request) records from JSONL content.
 */
export function parseDebugDumps(content: string): DebugDumpRecord[] {
  return parseJsonl(content).filter(
    (r): r is DebugDumpRecord => r.type === 'message_request',
  );
}

/**
 * Parse persisted headless turn errors from debug JSONL content.
 */
export function parseTurnErrors(content: string): TurnErrorRecord[] {
  return parseJsonl(content).filter(
    (r): r is TurnErrorRecord => r.type === 'turn_error',
  );
}

/**
 * Parse only compaction archive records from JSONL content.
 * These contain messages that were removed during session compaction.
 */
export function parseCompactionArchives(content: string): CompactionArchiveRecord[] {
  return parseJsonl(content).filter(
    (r): r is CompactionArchiveRecord => r.type === 'compaction_archive',
  );
}

/**
 * Parse only compaction boundary records from JSONL content.
 * These mark the point where a session compaction removed older messages.
 */
export function parseCompactionBoundaries(content: string): CompactionBoundaryRecord[] {
  return parseJsonl(content).filter(
    (r): r is CompactionBoundaryRecord => r.type === 'compaction_boundary',
  );
}

/**
 * Parse only new lines appended since the last known line count.
 * Returns parsed records from the new lines only.
 */
export function parseNewLines(content: string, previousLineCount: number): JsonlRecord[] {
  const lines = content.split('\n');
  const newLines = lines.slice(previousLineCount);
  return parseJsonl(newLines.join('\n'));
}
