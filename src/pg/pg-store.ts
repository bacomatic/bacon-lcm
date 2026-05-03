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

  async append(
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

  async get(id: MessageId): Promise<Message | undefined> {
    const { rows } = await this.pool.query<MessageRow>(
      "SELECT * FROM lcm_messages WHERE id = $1",
      [id],
    );
    return rows.length > 0 ? rowToMessage(rows[0]) : undefined;
  }

  async getBySession(sessionId: SessionId): Promise<Message[]> {
    const { rows } = await this.pool.query<MessageRow>(
      "SELECT * FROM lcm_messages WHERE session_id = $1 ORDER BY sequence_number",
      [sessionId],
    );
    return rows.map(rowToMessage);
  }

  async getRange(
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

  async getMany(ids: MessageId[]): Promise<Message[]> {
    if (ids.length === 0) return [];
    const { rows } = await this.pool.query<MessageRow>(
      "SELECT * FROM lcm_messages WHERE id = ANY($1) ORDER BY sequence_number",
      [ids],
    );
    return rows.map(rowToMessage);
  }

  async size(): Promise<number> {
    const { rows } = await this.pool.query<{ count: string }>(
      "SELECT COUNT(*) as count FROM lcm_messages",
    );
    return parseInt(rows[0].count, 10);
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
