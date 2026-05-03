/**
 * LCM Session Manager
 *
 * Top-level orchestrator that ties together the immutable store, summary DAG,
 * compaction engine, context assembler, and retrieval service into a single
 * coherent API for managing an LLM conversation session.
 */
import { CompactionEngine } from "./compaction.js";
import { ContextAssembler } from "./context.js";
import type { SummaryDag } from "./dag.js";
import { InMemorySummaryDag } from "./dag.js";
import { newSessionId } from "./ids.js";
import { RetrievalService } from "./retrieval.js";
import type { MessageStore } from "./store.js";
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

export interface LcmSessionOptions {
  sessionId?: SessionId;
  store?: MessageStore;
  dag?: SummaryDag;
  /** If provided, session metadata is persisted on every addMessage(). */
  sessionStore?: SessionPersistence;
  /** If provided, messages are auto-embedded for semantic search. */
  vectorStore?: VectorStore;
}

/**
 * Minimal interface for vector storage (semantic search).
 * Implemented by PgVectorStore — keeps LcmSession decoupled from pg.
 */
export interface VectorStore {
  store(id: string, sessionId: SessionId, sourceType: "message" | "summary", sourceId: string, content: string): Promise<void>;
  search(query: string, opts?: { sessionId?: SessionId; sourceType?: "message" | "summary"; limit?: number }): Promise<Array<{ sourceType: string; sourceId: string; content: string; similarity: number }>>;
}

/**
 * Minimal interface for session persistence.
 * Implemented by PgSessionStore — keeps LcmSession decoupled from pg.
 */
export interface SessionPersistence {
  save(session: { id: SessionId; createdAt: Date; activeTokenCount: number }): Promise<void>;
  load(id: SessionId): Promise<{ id: SessionId; createdAt: Date; activeTokenCount: number } | undefined>;
  list(): Promise<Array<{ id: SessionId; createdAt: Date; activeTokenCount: number }>>;
}

export class LcmSession {
  readonly session: Session;
  readonly store: MessageStore;
  readonly dag: SummaryDag;
  readonly compaction: CompactionEngine;
  readonly context: ContextAssembler;
  readonly retrieval: RetrievalService;
  private readonly sessionStore?: SessionPersistence;
  private readonly vectorStore?: VectorStore;

  constructor(
    private readonly tokenCounter: TokenCounter,
    private readonly summarizer: Summarizer,
    readonly config: CompactionConfig,
    opts?: LcmSessionOptions | SessionId,
  ) {
    // Backwards-compatible: accept bare SessionId or options object
    const options: LcmSessionOptions =
      typeof opts === "string" || opts === undefined
        ? { sessionId: opts as SessionId | undefined }
        : opts;

    this.session = {
      id: options.sessionId ?? newSessionId(),
      createdAt: new Date(),
      activeTokenCount: 0,
    };

    this.sessionStore = options.sessionStore;
    this.vectorStore = options.vectorStore;
    this.store = options.store ?? new InMemoryMessageStore(tokenCounter);
    this.dag = options.dag ?? new InMemorySummaryDag(tokenCounter);
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
  // Persistence
  // -----------------------------------------------------------------------

  /**
   * Save session metadata to the session store.
   * No-op if no sessionStore was provided.
   */
  async save(): Promise<void> {
    if (!this.sessionStore) return;
    await this.sessionStore.save(this.session);
  }

  /**
   * Restore a previously persisted session.
   *
   * The session row is loaded from the sessionStore, and the existing
   * messages/summaries in the shared store/dag are reused (they already
   * reference this session's ID). The activeTokenCount is recomputed
   * from the context assembler for accuracy.
   */
  static async restore(
    sessionId: SessionId,
    tokenCounter: TokenCounter,
    summarizer: Summarizer,
    config: CompactionConfig,
    opts: Required<Pick<LcmSessionOptions, "store" | "dag" | "sessionStore">> & Pick<LcmSessionOptions, "vectorStore">,
  ): Promise<LcmSession | undefined> {
    const row = await opts.sessionStore.load(sessionId);
    if (!row) return undefined;

    const session = new LcmSession(tokenCounter, summarizer, config, {
      sessionId: row.id,
      store: opts.store,
      dag: opts.dag,
      sessionStore: opts.sessionStore,
      vectorStore: opts.vectorStore,
    });

    // Restore metadata from the persisted row
    (session.session as { createdAt: Date }).createdAt = row.createdAt;

    // Recompute active token count from the actual context
    session.session.activeTokenCount = await session.context.totalTokens(row.id);

    return session;
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
    const message = await this.store.append(
      this.session.id,
      role,
      content,
      metadata,
    );

    this.session.activeTokenCount = await this.context.totalTokens(this.session.id);

    let compacted = false;

    if (this.compaction.mustCompactNow(this.session.activeTokenCount)) {
      // Synchronous compaction — hard limit exceeded
      await this.compaction.compact(
        this.session.id,
        this.session.activeTokenCount,
      );
      this.session.activeTokenCount = await this.context.totalTokens(this.session.id);
      compacted = true;
    } else if (this.compaction.shouldCompact(this.session.activeTokenCount)) {
      // Async compaction — fire and forget (in a real system this would be
      // dispatched to a background worker; here we await for simplicity)
      await this.compaction.compact(
        this.session.id,
        this.session.activeTokenCount,
      );
      this.session.activeTokenCount = await this.context.totalTokens(this.session.id);
      compacted = true;
    }

    // Auto-embed the message for semantic search
    if (this.vectorStore) {
      this.vectorStore
        .store(message.id, this.session.id, "message", message.id, content)
        .catch(() => {}); // fire-and-forget
    }

    // Auto-save session metadata if a session store is configured
    if (this.sessionStore) {
      await this.save();
    }

    return { message, compacted };
  }

  // -----------------------------------------------------------------------
  // Context window
  // -----------------------------------------------------------------------

  /** Build the active context window to send to the LLM. */
  async getContext(): Promise<ContextItem[]> {
    return this.context.assemble(this.session.id);
  }

  /** Current token count of the active context. */
  async getTokenCount(): Promise<number> {
    return this.context.totalTokens(this.session.id);
  }

  // -----------------------------------------------------------------------
  // Retrieval
  // -----------------------------------------------------------------------

  /** Describe a summary node's lineage metadata. */
  async describe(summaryId: SummaryId) {
    return this.retrieval.describe(summaryId);
  }

  /** Expand a summary back to its original messages. */
  async expand(summaryId: SummaryId) {
    return this.retrieval.expand(summaryId);
  }

  // -----------------------------------------------------------------------
  // Semantic search
  // -----------------------------------------------------------------------

  /**
   * Search messages and summaries by semantic similarity.
   * Returns empty array if no vectorStore is configured.
   */
  async search(
    query: string,
    opts?: { limit?: number; sourceType?: "message" | "summary" },
  ): Promise<Array<{ sourceType: string; sourceId: string; content: string; similarity: number }>> {
    if (!this.vectorStore) return [];
    return this.vectorStore.search(query, {
      sessionId: this.session.id,
      ...opts,
    });
  }
}
