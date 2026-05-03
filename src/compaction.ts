/**
 * Compaction Engine
 *
 * Implements the LCM three-level escalation protocol for context compression:
 *
 *   Level 1 — Leaf summaries:     Groups of raw messages → single summary node.
 *   Level 2 — Condensed summaries: Groups of leaf nodes → higher-level summary.
 *   Level 3 — Emergency fallback:  Deterministic truncation requiring no LLM call.
 *
 * Compaction is triggered by a deterministic control loop that monitors token
 * usage against soft and hard thresholds.  The engine never discards data;
 * originals remain in the immutable store and are reachable via DAG lineage.
 */
import type { SummaryDag } from "./dag.js";
import type { MessageStore } from "./store.js";
import type {
  CompactionConfig,
  Message,
  SessionId,
  Summarizer,
  SummaryNode,
  TokenCounter,
} from "./types.js";

// ---------------------------------------------------------------------------
// Compaction result
// ---------------------------------------------------------------------------

export interface CompactionResult {
  /** Summary nodes created during this compaction pass */
  created: SummaryNode[];
  /** Summary nodes archived (pushed off-context) */
  archived: SummaryNode[];
  /** Tokens removed from active context */
  tokensReclaimed: number;
  /** Escalation level that was required (0 = none needed) */
  levelReached: 0 | 1 | 2 | 3;
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

export class CompactionEngine {
  constructor(
    private readonly store: MessageStore,
    private readonly dag: SummaryDag,
    private readonly summarizer: Summarizer,
    private readonly tokenCounter: TokenCounter,
    private readonly config: CompactionConfig,
  ) {}

  // -----------------------------------------------------------------------
  // Public API
  // -----------------------------------------------------------------------

  /**
   * Evaluate whether compaction is needed and perform it if so.
   * Returns a result describing what happened.
   */
  async compact(
    sessionId: SessionId,
    currentTokenCount: number,
  ): Promise<CompactionResult> {
    const { softLimit, hardLimit } = this.config.thresholds;

    // No compaction needed
    if (currentTokenCount <= softLimit) {
      return { created: [], archived: [], tokensReclaimed: 0, levelReached: 0 };
    }

    // Level 1 — Leaf compaction
    let result = await this.leafCompact(sessionId);
    if ((await this.activeTokens(sessionId)) <= hardLimit && result.created.length > 0) {
      return { ...result, levelReached: 1 };
    }

    // Level 2 — Condensed compaction
    const condensedResult = await this.condensedCompact(sessionId);
    result = this.mergeResults(result, condensedResult);
    if ((await this.activeTokens(sessionId)) <= hardLimit) {
      return { ...result, levelReached: 2 };
    }

    // Level 3 — Emergency deterministic fallback
    const emergencyResult = await this.emergencyCompact(sessionId);
    result = this.mergeResults(result, emergencyResult);
    return { ...result, levelReached: 3 };
  }

  /**
   * Check if compaction should run (soft threshold exceeded).
   */
  shouldCompact(currentTokenCount: number): boolean {
    return currentTokenCount > this.config.thresholds.softLimit;
  }

  /**
   * Check if synchronous compaction is required (hard threshold exceeded).
   */
  mustCompactNow(currentTokenCount: number): boolean {
    return currentTokenCount > this.config.thresholds.hardLimit;
  }

  // -----------------------------------------------------------------------
  // Level 1 — Leaf compaction
  // -----------------------------------------------------------------------

  private async leafCompact(sessionId: SessionId): Promise<CompactionResult> {
    const messages = await this.store.getBySession(sessionId);
    const { freshTailCount, leafMinFanout, leafChunkTokens } = this.config;

    // Determine which messages are eligible for compaction:
    // all except the fresh tail and any already summarized
    const activeSummaries = await this.dag.getActive(sessionId);
    const expandedIds = await Promise.all(
      activeSummaries.map((s) => this.dag.expandToMessageIds(s.id)),
    );
    const summarizedMsgIds = new Set(expandedIds.flat());

    const eligible = messages
      .slice(0, Math.max(0, messages.length - freshTailCount))
      .filter((m) => !summarizedMsgIds.has(m.id));

    if (eligible.length < leafMinFanout) {
      return { created: [], archived: [], tokensReclaimed: 0, levelReached: 1 };
    }

    // Group eligible messages into chunks
    const chunks = this.chunkMessages(eligible, leafChunkTokens, leafMinFanout);

    const created: SummaryNode[] = [];
    let tokensReclaimed = 0;

    for (const chunk of chunks) {
      const texts = chunk.map((m) => `[${m.role}] ${m.content}`);
      const summaryText = await this.summarizer.summarize(texts, "leaf");
      const node = await this.dag.add(
        sessionId,
        "leaf",
        summaryText,
        chunk.map((m) => m.id),
        [],
      );
      created.push(node);

      const chunkTokens = chunk.reduce((sum, m) => sum + m.tokenCount, 0);
      tokensReclaimed += chunkTokens - node.tokenCount;
    }

    return { created, archived: [], tokensReclaimed, levelReached: 1 };
  }

  // -----------------------------------------------------------------------
  // Level 2 — Condensed compaction
  // -----------------------------------------------------------------------

  private async condensedCompact(
    sessionId: SessionId,
  ): Promise<CompactionResult> {
    const activeLeaves = (await this.dag.getActive(sessionId))
      .filter((n) => n.level === "leaf");

    const { condensedMinFanout, condensedTargetTokens } = this.config;

    if (activeLeaves.length < condensedMinFanout) {
      return { created: [], archived: [], tokensReclaimed: 0, levelReached: 2 };
    }

    // Group leaves into chunks for condensation
    const chunks = this.chunkNodes(
      activeLeaves,
      condensedTargetTokens,
      condensedMinFanout,
    );

    const created: SummaryNode[] = [];
    const archived: SummaryNode[] = [];
    let tokensReclaimed = 0;

    for (const chunk of chunks) {
      if (chunk.length < condensedMinFanout) continue;

      const texts = chunk.map((n) => n.content);
      const summaryText = await this.summarizer.summarize(texts, "condensed");

      const expandedMsgIds = await Promise.all(
        chunk.map((n) => this.dag.expandToMessageIds(n.id)),
      );
      const sourceMessageIds = expandedMsgIds.flat();

      const node = await this.dag.add(
        sessionId,
        "condensed",
        summaryText,
        sourceMessageIds,
        chunk.map((n) => n.id),
      );
      created.push(node);

      // Archive the consumed leaf nodes
      for (const leaf of chunk) {
        await this.dag.archive(leaf.id);
        archived.push(leaf);
      }

      const chunkTokens = chunk.reduce((sum, n) => sum + n.tokenCount, 0);
      tokensReclaimed += chunkTokens - node.tokenCount;
    }

    return { created, archived, tokensReclaimed, levelReached: 2 };
  }

  // -----------------------------------------------------------------------
  // Level 3 — Emergency deterministic fallback
  // -----------------------------------------------------------------------

  private async emergencyCompact(sessionId: SessionId): Promise<CompactionResult> {
    const activeSummaries = await this.dag.getActive(sessionId);
    if (activeSummaries.length === 0) {
      return { created: [], archived: [], tokensReclaimed: 0, levelReached: 3 };
    }

    // Archive the oldest active summaries until we're under the hard limit
    const archived: SummaryNode[] = [];
    let tokensReclaimed = 0;
    const sorted = [...activeSummaries].sort(
      (a, b) => a.createdAt.getTime() - b.createdAt.getTime(),
    );

    for (const node of sorted) {
      if ((await this.activeTokens(sessionId)) <= this.config.thresholds.hardLimit) {
        break;
      }
      await this.dag.archive(node.id);
      archived.push(node);
      tokensReclaimed += node.tokenCount;
    }

    // Create a terse emergency stub for the archived batch
    const created: SummaryNode[] = [];
    if (archived.length > 0) {
      const stub = `[Emergency compaction: ${archived.length} summary nodes archived. ` +
        `Use lcm_describe / lcm_expand to retrieve original content.]`;
      const expandedIds = await Promise.all(
        archived.map((n) => this.dag.expandToMessageIds(n.id)),
      );
      const node = await this.dag.add(
        sessionId,
        "emergency",
        stub,
        expandedIds.flat(),
        archived.map((n) => n.id),
      );
      created.push(node);
    }

    return { created, archived, tokensReclaimed, levelReached: 3 };
  }

  // -----------------------------------------------------------------------
  // Helpers
  // -----------------------------------------------------------------------

  /** Estimate current active token count for a session. */
  private async activeTokens(sessionId: SessionId): Promise<number> {
    const messages = await this.store.getBySession(sessionId);
    const activeSummaries = await this.dag.getActive(sessionId);
    const expandedIds = await Promise.all(
      activeSummaries.map((s) => this.dag.expandToMessageIds(s.id)),
    );
    const summarizedMsgIds = new Set(expandedIds.flat());

    const rawTokens = messages
      .filter((m) => !summarizedMsgIds.has(m.id))
      .reduce((sum, m) => sum + m.tokenCount, 0);

    const summaryTokens = activeSummaries.reduce(
      (sum, s) => sum + s.tokenCount,
      0,
    );

    return rawTokens + summaryTokens;
  }

  /** Split messages into chunks by token budget and minimum fanout. */
  private chunkMessages(
    messages: Message[],
    targetTokens: number,
    minFanout: number,
  ): Message[][] {
    const chunks: Message[][] = [];
    let current: Message[] = [];
    let currentTokens = 0;

    for (const msg of messages) {
      current.push(msg);
      currentTokens += msg.tokenCount;

      if (currentTokens >= targetTokens && current.length >= minFanout) {
        chunks.push(current);
        current = [];
        currentTokens = 0;
      }
    }

    // Remainder: merge into last chunk if too small, or keep as own chunk
    if (current.length > 0) {
      if (current.length >= minFanout) {
        chunks.push(current);
      } else if (chunks.length > 0) {
        chunks[chunks.length - 1].push(...current);
      }
      // else: not enough messages to form any chunk — skip
    }

    return chunks;
  }

  /** Split summary nodes into chunks by token budget and minimum fanout. */
  private chunkNodes(
    nodes: SummaryNode[],
    targetTokens: number,
    minFanout: number,
  ): SummaryNode[][] {
    const chunks: SummaryNode[][] = [];
    let current: SummaryNode[] = [];
    let currentTokens = 0;

    for (const node of nodes) {
      current.push(node);
      currentTokens += node.tokenCount;

      if (currentTokens >= targetTokens && current.length >= minFanout) {
        chunks.push(current);
        current = [];
        currentTokens = 0;
      }
    }

    if (current.length > 0) {
      if (current.length >= minFanout) {
        chunks.push(current);
      } else if (chunks.length > 0) {
        chunks[chunks.length - 1].push(...current);
      }
    }

    return chunks;
  }

  private mergeResults(
    a: CompactionResult,
    b: CompactionResult,
  ): CompactionResult {
    return {
      created: [...a.created, ...b.created],
      archived: [...a.archived, ...b.archived],
      tokensReclaimed: a.tokensReclaimed + b.tokensReclaimed,
      levelReached: Math.max(a.levelReached, b.levelReached) as
        | 0
        | 1
        | 2
        | 3,
    };
  }
}
