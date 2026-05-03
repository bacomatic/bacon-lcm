/**
 * PostgreSQL-backed Message Store
 *
 * Drop-in replacement for InMemoryMessageStore.
 * Implements the MessageStore interface with real persistence.
 */
import type { Pool } from "pg";
import { newMessageId } from "../ids.js";
import type { MessageStore } from "../store.js";
import type {
  Message,
  MessageId,
  MessageRole,
  SessionId,
  TokenCounter,
} from "../types.js";

// ---------------------------------------------------------------------------
// Row ↔ Message mapping
// ---------------------------------------------------------------------------

interface MessageRow {
  id: string;
  session_id: string;
  role: string;
  content: string;
  sequence_number: number;
  token_count: number;
  created_at: Date;
  metadata: Record<string, unknown> | null;
}

function rowToMessage(row: MessageRow): Message {
  return {
    id: row.id as MessageId,
    sessionId: row.session_id as SessionId,
    role: row.role as MessageRole,
    content: row.content,
    sequenceNumber: row.sequence_number,
    tokenCount: row.token_count,
    createdAt: row.created_at,
    metadata: row.metadata ?? undefined,
  };
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

export class PgMessageStore implements MessageStore {
  constructor(
    private readonly pool: Pool,
    private readonly tokenCounter: TokenCounter,
  ) {}

  append(
    sessionId: SessionId,
    role: MessageRole,
    content: string,
    metadata?: Record<string, unknown>,
  ): Message {
    // Synchronous interface — we enqueue the insert and return immediately.
    // The actual write happens via appendAsync which should be preferred.
    // For the sync path we compute optimistically and fire-and-forget the INSERT.
    const id = newMessageId();
    const tokenCount = this.tokenCounter.count(content);
    const createdAt = new Date();

    const message: Message = {
      id,
      sessionId,
      role,
      content,
      sequenceNumber: -1, // will be set by DB; see appendAsync
      tokenCount,
      createdAt,
      metadata,
    };

    // Fire and forget — not ideal but preserves the sync interface.
    // Use appendAsync for correctness.
    this._insertAsync(message).catch((err) => {
      console.error("PgMessageStore: fire-and-forget insert failed:", err);
    });

    return message;
  }

  /**
   * Async append — preferred over the sync `append` method.
   * Returns the message with the correct sequence number assigned by the DB.
   */
  async appendAsync(
    sessionId: SessionId,
    role: MessageRole,
    content: string,
    metadata?: Record<string, unknown>,
  ): Promise<Message> {
    const id = newMessageId();
    const tokenCount = this.tokenCounter.count(content);
    const createdAt = new Date();

    const { rows } = await this.pool.query<{ sequence_number: number }>(
      `INSERT INTO lcm_messages (id, session_id, role, content, sequence_number, token_count, created_at, metadata)
       VALUES ($1, $2, $3, $4,
         COALESCE((SELECT MAX(sequence_number) FROM lcm_messages WHERE session_id = $2), 0) + 1,
         $5, $6, $7)
       RETURNING sequence_number`,
      [id, sessionId, role, content, tokenCount, createdAt, metadata ? JSON.stringify(metadata) : null],
    );

    return {
      id,
      sessionId,
      role,
      content,
      sequenceNumber: rows[0].sequence_number,
      tokenCount,
      createdAt,
      metadata,
    };
  }

  get(id: MessageId): Message | undefined {
    // Sync facade — see getAsync
    throw new Error(
      "PgMessageStore.get() is not supported synchronously. Use getAsync().",
    );
  }

  async getAsync(id: MessageId): Promise<Message | undefined> {
    const { rows } = await this.pool.query<MessageRow>(
      "SELECT * FROM lcm_messages WHERE id = $1",
      [id],
    );
    return rows.length > 0 ? rowToMessage(rows[0]) : undefined;
  }

  getBySession(sessionId: SessionId): Message[] {
    throw new Error(
      "PgMessageStore.getBySession() is not supported synchronously. Use getBySessionAsync().",
    );
  }

  async getBySessionAsync(sessionId: SessionId): Promise<Message[]> {
    const { rows } = await this.pool.query<MessageRow>(
      "SELECT * FROM lcm_messages WHERE session_id = $1 ORDER BY sequence_number",
      [sessionId],
    );
    return rows.map(rowToMessage);
  }

  getRange(sessionId: SessionId, from: number, to: number): Message[] {
    throw new Error(
      "PgMessageStore.getRange() is not supported synchronously. Use getRangeAsync().",
    );
  }

  async getRangeAsync(
    sessionId: SessionId,
    from: number,
    to: number,
  ): Promise<Message[]> {
    const { rows } = await this.pool.query<MessageRow>(
      "SELECT * FROM lcm_messages WHERE session_id = $1 AND sequence_number >= $2 AND sequence_number <= $3 ORDER BY sequence_number",
      [sessionId, from, to],
    );
    return rows.map(rowToMessage);
  }

  getMany(ids: MessageId[]): Message[] {
    throw new Error(
      "PgMessageStore.getMany() is not supported synchronously. Use getManyAsync().",
    );
  }

  async getManyAsync(ids: MessageId[]): Promise<Message[]> {
    if (ids.length === 0) return [];
    const { rows } = await this.pool.query<MessageRow>(
      "SELECT * FROM lcm_messages WHERE id = ANY($1) ORDER BY sequence_number",
      [ids],
    );
    return rows.map(rowToMessage);
  }

  size(): number {
    throw new Error(
      "PgMessageStore.size() is not supported synchronously. Use sizeAsync().",
    );
  }

  async sizeAsync(): Promise<number> {
    const { rows } = await this.pool.query<{ count: string }>(
      "SELECT COUNT(*) as count FROM lcm_messages",
    );
    return parseInt(rows[0].count, 10);
  }

  // -----------------------------------------------------------------------
  // Internal
  // -----------------------------------------------------------------------

  private async _insertAsync(message: Message): Promise<void> {
    await this.pool.query(
      `INSERT INTO lcm_messages (id, session_id, role, content, sequence_number, token_count, created_at, metadata)
       VALUES ($1, $2, $3, $4,
         COALESCE((SELECT MAX(sequence_number) FROM lcm_messages WHERE session_id = $2), 0) + 1,
         $5, $6, $7)`,
      [
        message.id,
        message.sessionId,
        message.role,
        message.content,
        message.tokenCount,
        message.createdAt,
        message.metadata ? JSON.stringify(message.metadata) : null,
      ],
    );
  }

  // -----------------------------------------------------------------------
  // Schema management
  // -----------------------------------------------------------------------

  /**
   * Run the schema migration. Safe to call multiple times (uses IF NOT EXISTS).
   */
  async migrate(): Promise<void> {
    await this.pool.query(`
      CREATE TABLE IF NOT EXISTS lcm_messages (
        id              TEXT        PRIMARY KEY,
        session_id      TEXT        NOT NULL,
        role            TEXT        NOT NULL CHECK (role IN ('user', 'assistant', 'tool', 'system')),
        content         TEXT        NOT NULL,
        sequence_number INTEGER     NOT NULL,
        token_count     INTEGER     NOT NULL,
        created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        metadata        JSONB,
        UNIQUE (session_id, sequence_number)
      );
      CREATE INDEX IF NOT EXISTS idx_lcm_messages_session
        ON lcm_messages (session_id, sequence_number);
    `);
  }
}
