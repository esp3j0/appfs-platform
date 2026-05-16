import type { JsonlRecord, MessageRecord, SessionMetaRecord } from './types.js';

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
 * Parse only new lines appended since the last known line count.
 * Returns parsed records from the new lines only.
 */
export function parseNewLines(content: string, previousLineCount: number): JsonlRecord[] {
  const lines = content.split('\n');
  const newLines = lines.slice(previousLineCount);
  return parseJsonl(newLines.join('\n'));
}
