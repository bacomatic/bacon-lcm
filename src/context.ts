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
  assemble(sessionId: SessionId): ContextItem[] {
    const items: ContextItem[] = [];

    // 1. Active summary nodes (oldest first)
    const summaries = this.dag.getActive(sessionId);
    for (const summary of summaries) {
      items.push({ kind: "summary", summary });
    }

    // 2. Fresh tail: recent raw messages not covered by any active summary
    const allMessages = this.store.getBySession(sessionId);
    const summarizedIds = new Set(
      summaries.flatMap((s) => this.dag.expandToMessageIds(s.id)),
    );

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
  totalTokens(sessionId: SessionId): number {
    const items = this.assemble(sessionId);
    return items.reduce((sum, item) => {
      if (item.kind === "message") return sum + item.message.tokenCount;
      return sum + item.summary.tokenCount;
    }, 0);
  }
}
