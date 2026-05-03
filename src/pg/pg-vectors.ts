/**
 * PostgreSQL Vector Store (pgvector)
 *
 * Stores and queries embedding vectors for semantic search over
 * messages and summaries. Requires the pgvector extension.
 */
import type { Pool } from "pg";
import type { Embedder } from "../types.js";
import type { SessionId } from "../types.js";

export interface EmbeddingRow {
  id: string;
  sessionId: SessionId;
  sourceType: "message" | "summary";
  sourceId: string;
  content: string;
  similarity?: number;
}

export interface SearchResult extends EmbeddingRow {
  similarity: number;
}

export class PgVectorStore {
  constructor(
    private readonly pool: Pool,
    private readonly embedder: Embedder,
  ) {}

  /** Run the schema migration. Safe to call multiple times. */
  async migrate(): Promise<void> {
    await this.pool.query(`CREATE EXTENSION IF NOT EXISTS vector`);

    const dim = this.embedder.dimensions;
    await this.pool.query(`
      CREATE TABLE IF NOT EXISTS lcm_embeddings (
        id          TEXT        PRIMARY KEY,
        session_id  TEXT        NOT NULL,
        source_type TEXT        NOT NULL CHECK (source_type IN ('message', 'summary')),
        source_id   TEXT        NOT NULL,
        content     TEXT        NOT NULL,
        embedding   vector(${dim}),
        created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

        UNIQUE (source_type, source_id)
      )
    `);

    // HNSW index for cosine similarity
    await this.pool.query(`
      CREATE INDEX IF NOT EXISTS idx_lcm_embeddings_session
        ON lcm_embeddings (session_id)
    `);
    await this.pool.query(`
      CREATE INDEX IF NOT EXISTS idx_lcm_embeddings_vector
        ON lcm_embeddings USING hnsw (embedding vector_cosine_ops)
    `);
  }

  /**
   * Store an embedding for a message or summary.
   * Uses UPSERT so re-embedding the same source is safe.
   */
  async store(
    id: string,
    sessionId: SessionId,
    sourceType: "message" | "summary",
    sourceId: string,
    content: string,
  ): Promise<void> {
    const embedding = await this.embedder.embed(content);
    if (embedding.length === 0) return; // NullEmbedder — skip

    const vectorLiteral = `[${embedding.join(",")}]`;
    await this.pool.query(
      `INSERT INTO lcm_embeddings (id, session_id, source_type, source_id, content, embedding)
       VALUES ($1, $2, $3, $4, $5, $6::vector)
       ON CONFLICT (source_type, source_id) DO UPDATE SET
         content = EXCLUDED.content,
         embedding = EXCLUDED.embedding`,
      [id, sessionId, sourceType, sourceId, content, vectorLiteral],
    );
  }

  /**
   * Semantic search: find the top-k most similar items to a query.
   * Optionally filter by session and/or source type.
   */
  async search(
    query: string,
    opts?: {
      sessionId?: SessionId;
      sourceType?: "message" | "summary";
      limit?: number;
    },
  ): Promise<SearchResult[]> {
    const embedding = await this.embedder.embed(query);
    if (embedding.length === 0) return [];

    const vectorLiteral = `[${embedding.join(",")}]`;
    const limit = opts?.limit ?? 10;

    let sql = `
      SELECT id, session_id, source_type, source_id, content,
             1 - (embedding <=> $1::vector) AS similarity
      FROM lcm_embeddings
      WHERE 1=1
    `;
    const params: unknown[] = [vectorLiteral];
    let paramIdx = 2;

    if (opts?.sessionId) {
      sql += ` AND session_id = $${paramIdx}`;
      params.push(opts.sessionId);
      paramIdx++;
    }
    if (opts?.sourceType) {
      sql += ` AND source_type = $${paramIdx}`;
      params.push(opts.sourceType);
      paramIdx++;
    }

    sql += ` ORDER BY embedding <=> $1::vector LIMIT $${paramIdx}`;
    params.push(limit);

    const { rows } = await this.pool.query(sql, params);
    return rows.map(toSearchResult);
  }

  /** Count total embeddings. */
  async size(): Promise<number> {
    const { rows } = await this.pool.query(
      `SELECT COUNT(*)::int AS count FROM lcm_embeddings`,
    );
    return rows[0].count;
  }
}

function toSearchResult(row: any): SearchResult {
  return {
    id: row.id,
    sessionId: row.session_id as SessionId,
    sourceType: row.source_type,
    sourceId: row.source_id,
    content: row.content,
    similarity: parseFloat(row.similarity),
  };
}
