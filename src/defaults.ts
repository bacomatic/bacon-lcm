/**
 * Default / reference implementations for pluggable interfaces.
 */
import type {
  CompactionConfig,
  Summarizer,
  SummaryLevel,
  ThresholdConfig,
  TokenCounter,
} from "./types.js";

// ---------------------------------------------------------------------------
// Naive token counter (≈ 4 chars per token heuristic)
// ---------------------------------------------------------------------------

export class NaiveTokenCounter implements TokenCounter {
  count(text: string): number {
    return Math.ceil(text.length / 4);
  }
}

// ---------------------------------------------------------------------------
// Echo summarizer (for testing — just concatenates with a header)
// ---------------------------------------------------------------------------

export class EchoSummarizer implements Summarizer {
  async summarize(texts: string[], level: SummaryLevel): Promise<string> {
    const header = `[${level} summary of ${texts.length} items]`;
    const body = texts.map((t, i) => `  (${i + 1}) ${t.slice(0, 120)}`).join("\n");
    return `${header}\n${body}`;
  }
}

// ---------------------------------------------------------------------------
// Default configuration presets
// ---------------------------------------------------------------------------

export const DEFAULT_THRESHOLDS: ThresholdConfig = {
  modelMaxTokens: 128_000,
  softLimit: 80_000,
  hardLimit: 110_000,
  riskBuffer: 18_000,
};

export const DEFAULT_COMPACTION_CONFIG: CompactionConfig = {
  thresholds: DEFAULT_THRESHOLDS,
  leafMinFanout: 4,
  leafChunkTokens: 8_000,
  condensedMinFanout: 3,
  condensedTargetTokens: 16_000,
  freshTailCount: 10,
};
