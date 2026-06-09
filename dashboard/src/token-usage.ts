export interface TokenUsageLike {
  input_tokens?: number;
  output_tokens?: number;
  cache_creation_input_tokens?: number;
  cache_read_input_tokens?: number;
}

export function effectiveInputTokens(usage: TokenUsageLike | undefined): number {
  if (!usage) return 0;
  return positiveTokenCount(usage.input_tokens)
    + positiveTokenCount(usage.cache_creation_input_tokens)
    + positiveTokenCount(usage.cache_read_input_tokens);
}

export function cachedInputTokens(usage: TokenUsageLike | undefined): number {
  if (!usage) return 0;
  return positiveTokenCount(usage.cache_creation_input_tokens)
    + positiveTokenCount(usage.cache_read_input_tokens);
}

export function contextUsagePercent(tokens: number | undefined, contextWindowTokens: number | undefined): number {
  if (!tokens || !contextWindowTokens || contextWindowTokens <= 0) return 0;
  return Math.min(100, Math.max(0, (tokens / contextWindowTokens) * 100));
}

export function contextUsageTitle(tokens: number | undefined, contextWindowTokens: number | undefined): string {
  if (!tokens || !contextWindowTokens || contextWindowTokens <= 0) {
    return 'Context usage unavailable';
  }
  return `${tokens.toLocaleString()} / ${contextWindowTokens.toLocaleString()} context tokens`;
}

function positiveTokenCount(value: number | undefined): number {
  return typeof value === 'number' && value > 0 ? value : 0;
}
