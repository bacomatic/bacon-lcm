/**
 * Context Assembler
 *
 * Builds the active context window that is sent to the LLM on each turn.
 * The window is a mix of:
 *   - Active summary nodes (standing in for groups of older messages)
 *   - Raw "fresh tail" messages (most recent, un-summarized)
 *
 * Items are ordered chronologically: summaries first, then the fresh tail.
 */
import type { SummaryDag } from "./dag.js";
import type { MessageStore } from "./store.js";
import type {
  CompactionConfig,
  ContextItem,
  SessionId,
} from "./types.js";

export class ContextAssembler {
  constructor(
    private readonly store: MessageStore,
    private readonly dag: SummaryDag,
    private readonly config: CompactionConfig,
  ) {}

  /**
   * Assemble the active context window for a session.
   * Returns items ordered chronologically for injection into the LLM prompt.
   */
  async assemble(sessionId: SessionId): Promise<ContextItem[]> {
    const items: ContextItem[] = [];

    // 1. Active summary nodes (oldest first)
    const summaries = await this.dag.getActive(sessionId);
    for (const summary of summaries) {
      items.push({ kind: "summary", summary });
    }

    // 2. Fresh tail: recent raw messages not covered by any active summary
    const allMessages = await this.store.getBySession(sessionId);
    const expandedIds = await Promise.all(
      summaries.map((s) => this.dag.expandToMessageIds(s.id)),
    );
    const summarizedIds = new Set(expandedIds.flat());

    const unsummarized = allMessages.filter((m) => !summarizedIds.has(m.id));

    // Take only the freshTailCount most recent unsummarized messages
    const tail = unsummarized.slice(
      Math.max(0, unsummarized.length - this.config.freshTailCount),
    );

    for (const message of tail) {
      items.push({ kind: "message", message });
    }

    return items;
  }

  /**
   * Calculate the total token count of the current active context.
   */
  async totalTokens(sessionId: SessionId): Promise<number> {
    const items = await this.assemble(sessionId);
    return items.reduce((sum, item) => {
      if (item.kind === "message") return sum + item.message.tokenCount;
      return sum + item.summary.tokenCount;
    }, 0);
  }
}
