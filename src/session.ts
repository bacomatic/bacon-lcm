/**
 * LCM Session Manager
 *
 * Top-level orchestrator that ties together the immutable store, summary DAG,
 * compaction engine, context assembler, and retrieval service into a single
 * coherent API for managing an LLM conversation session.
 */
import { CompactionEngine } from "./compaction.js";
import { ContextAssembler } from "./context.js";
import { InMemorySummaryDag } from "./dag.js";
import { newSessionId } from "./ids.js";
import { RetrievalService } from "./retrieval.js";
import { InMemoryMessageStore } from "./store.js";
import type {
  CompactionConfig,
  ContextItem,
  Message,
  MessageRole,
  Session,
  SessionId,
  Summarizer,
  SummaryId,
  TokenCounter,
} from "./types.js";

export class LcmSession {
  readonly session: Session;
  readonly store: InMemoryMessageStore;
  readonly dag: InMemorySummaryDag;
  readonly compaction: CompactionEngine;
  readonly context: ContextAssembler;
  readonly retrieval: RetrievalService;

  constructor(
    private readonly tokenCounter: TokenCounter,
    private readonly summarizer: Summarizer,
    private readonly config: CompactionConfig,
    sessionId?: SessionId,
  ) {
    this.session = {
      id: sessionId ?? newSessionId(),
      createdAt: new Date(),
      activeTokenCount: 0,
    };

    this.store = new InMemoryMessageStore(tokenCounter);
    this.dag = new InMemorySummaryDag(tokenCounter);
    this.compaction = new CompactionEngine(
      this.store,
      this.dag,
      summarizer,
      tokenCounter,
      config,
    );
    this.context = new ContextAssembler(this.store, this.dag, config);
    this.retrieval = new RetrievalService(this.store, this.dag);
  }

  // -----------------------------------------------------------------------
  // Message lifecycle
  // -----------------------------------------------------------------------

  /**
   * Append a message to the session and evaluate compaction.
   *
   * If the soft threshold is exceeded, compaction runs asynchronously.
   * If the hard threshold is exceeded, compaction runs synchronously
   * before this method returns.
   */
  async addMessage(
    role: MessageRole,
    content: string,
    metadata?: Record<string, unknown>,
  ): Promise<{ message: Message; compacted: boolean }> {
    const message = this.store.append(
      this.session.id,
      role,
      content,
      metadata,
    );

    this.session.activeTokenCount = this.context.totalTokens(this.session.id);

    let compacted = false;

    if (this.compaction.mustCompactNow(this.session.activeTokenCount)) {
      // Synchronous compaction — hard limit exceeded
      await this.compaction.compact(
        this.session.id,
        this.session.activeTokenCount,
      );
      this.session.activeTokenCount = this.context.totalTokens(this.session.id);
      compacted = true;
    } else if (this.compaction.shouldCompact(this.session.activeTokenCount)) {
      // Async compaction — fire and forget (in a real system this would be
      // dispatched to a background worker; here we await for simplicity)
      await this.compaction.compact(
        this.session.id,
        this.session.activeTokenCount,
      );
      this.session.activeTokenCount = this.context.totalTokens(this.session.id);
      compacted = true;
    }

    return { message, compacted };
  }

  // -----------------------------------------------------------------------
  // Context window
  // -----------------------------------------------------------------------

  /** Build the active context window to send to the LLM. */
  getContext(): ContextItem[] {
    return this.context.assemble(this.session.id);
  }

  /** Current token count of the active context. */
  getTokenCount(): number {
    return this.context.totalTokens(this.session.id);
  }

  // -----------------------------------------------------------------------
  // Retrieval
  // -----------------------------------------------------------------------

  /** Describe a summary node's lineage metadata. */
  describe(summaryId: SummaryId) {
    return this.retrieval.describe(summaryId);
  }

  /** Expand a summary back to its original messages. */
  expand(summaryId: SummaryId) {
    return this.retrieval.expand(summaryId);
  }
}
