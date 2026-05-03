/**
 * PostgreSQL Session Persistence
 *
 * Saves and restores LCM session metadata to the lcm_sessions table.
 * Messages and summary nodes are already persisted by PgMessageStore and
 * PgSummaryDag — this module handles only the session row itself.
 */
import type { Pool } from "pg";
import type { SessionId } from "../types.js";

export interface SessionRow {
  id: SessionId;
  createdAt: Date;
  activeTokenCount: number;
}

export class PgSessionStore {
  constructor(private readonly pool: Pool) {}

  /** Insert or update a session row. */
  async save(session: SessionRow): Promise<void> {
    await this.pool.query(
      `INSERT INTO lcm_sessions (id, created_at, active_token_count)
       VALUES ($1, $2, $3)
       ON CONFLICT (id) DO UPDATE SET active_token_count = EXCLUDED.active_token_count`,
      [session.id, session.createdAt, session.activeTokenCount],
    );
  }

  /** Load a single session by ID. */
  async load(id: SessionId): Promise<SessionRow | undefined> {
    const { rows } = await this.pool.query(
      `SELECT id, created_at, active_token_count FROM lcm_sessions WHERE id = $1`,
      [id],
    );
    if (rows.length === 0) return undefined;
    return toSessionRow(rows[0]);
  }

  /** List all sessions, most recent first. */
  async list(): Promise<SessionRow[]> {
    const { rows } = await this.pool.query(
      `SELECT id, created_at, active_token_count
       FROM lcm_sessions
       ORDER BY created_at DESC`,
    );
    return rows.map(toSessionRow);
  }

  /** Delete a session row (does NOT delete messages/summaries). */
  async delete(id: SessionId): Promise<void> {
    await this.pool.query(`DELETE FROM lcm_sessions WHERE id = $1`, [id]);
  }

  /** Check if the lcm_sessions table exists (migration already run). */
  async exists(): Promise<boolean> {
    const { rows } = await this.pool.query(
      `SELECT 1 FROM information_schema.tables WHERE table_name = 'lcm_sessions' LIMIT 1`,
    );
    return rows.length > 0;
  }

  /** Run the schema migration. Safe to call multiple times (uses IF NOT EXISTS). */
  async migrate(): Promise<void> {
    await this.pool.query(`
      CREATE TABLE IF NOT EXISTS lcm_sessions (
        id                  TEXT        PRIMARY KEY,
        created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        active_token_count  INTEGER     NOT NULL DEFAULT 0
      )
    `);
  }
}

function toSessionRow(row: any): SessionRow {
  return {
    id: row.id as SessionId,
    createdAt: new Date(row.created_at),
    activeTokenCount: parseInt(row.active_token_count, 10),
  };
}
