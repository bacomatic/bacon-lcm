/**
 * PostgreSQL-backed Summary DAG
 *
 * Drop-in replacement for InMemorySummaryDag.
 * Implements the SummaryDag interface with real persistence.
 */
import type { Pool } from "pg";
import type { SummaryDag } from "../dag.js";
import { newSummaryId } from "../ids.js";
import type {
  MessageId,
  SessionId,
  SummaryId,
  SummaryLevel,
  SummaryNode,
  TokenCounter,
} from "../types.js";

// ---------------------------------------------------------------------------
// Row ↔ SummaryNode mapping
// ---------------------------------------------------------------------------

interface SummaryRow {
  id: string;
  session_id: string;
  level: string;
  content: string;
  token_count: number;
  created_at: Date;
  source_message_ids: string[];
  source_node_ids: string[];
  is_active: boolean;
  is_archived: boolean;
}

function rowToNode(row: SummaryRow): SummaryNode {
  return {
    id: row.id as SummaryId,
    sessionId: row.session_id as SessionId,
    level: row.level as SummaryLevel,
    content: row.content,
    tokenCount: row.token_count,
    createdAt: row.created_at,
    sourceMessageIds: row.source_message_ids as MessageId[],
    sourceNodeIds: row.source_node_ids as SummaryId[],
    isActive: row.is_active,
    isArchived: row.is_archived,
  };
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

export class PgSummaryDag implements SummaryDag {
  constructor(
    private readonly pool: Pool,
    private readonly tokenCounter: TokenCounter,
  ) {}

  add(
    sessionId: SessionId,
    level: SummaryLevel,
    content: string,
    sourceMessageIds: MessageId[],
    sourceNodeIds: SummaryId[],
  ): SummaryNode {
    // Sync facade — fire-and-forget write. Use addAsync for correctness.
    const node: SummaryNode = {
      id: newSummaryId(),
      sessionId,
      level,
      content,
      tokenCount: this.tokenCounter.count(content),
      createdAt: new Date(),
      sourceMessageIds: [...sourceMessageIds],
      sourceNodeIds: [...sourceNodeIds],
      isActive: true,
      isArchived: false,
    };

    this._insertAsync(node).catch((err) => {
      console.error("PgSummaryDag: fire-and-forget insert failed:", err);
    });

    return node;
  }

  /**
   * Async add — preferred over the sync `add` method.
   */
  async addAsync(
    sessionId: SessionId,
    level: SummaryLevel,
    content: string,
    sourceMessageIds: MessageId[],
    sourceNodeIds: SummaryId[],
  ): Promise<SummaryNode> {
    const node: SummaryNode = {
      id: newSummaryId(),
      sessionId,
      level,
      content,
      tokenCount: this.tokenCounter.count(content),
      createdAt: new Date(),
      sourceMessageIds: [...sourceMessageIds],
      sourceNodeIds: [...sourceNodeIds],
      isActive: true,
      isArchived: false,
    };

    await this._insertAsync(node);
    return node;
  }

  get(id: SummaryId): SummaryNode | undefined {
    throw new Error(
      "PgSummaryDag.get() is not supported synchronously. Use getAsync().",
    );
  }

  async getAsync(id: SummaryId): Promise<SummaryNode | undefined> {
    const { rows } = await this.pool.query<SummaryRow>(
      "SELECT * FROM lcm_summary_nodes WHERE id = $1",
      [id],
    );
    return rows.length > 0 ? rowToNode(rows[0]) : undefined;
  }

  getActive(sessionId: SessionId): SummaryNode[] {
    throw new Error(
      "PgSummaryDag.getActive() is not supported synchronously. Use getActiveAsync().",
    );
  }

  async getActiveAsync(sessionId: SessionId): Promise<SummaryNode[]> {
    const { rows } = await this.pool.query<SummaryRow>(
      "SELECT * FROM lcm_summary_nodes WHERE session_id = $1 AND is_active = TRUE AND is_archived = FALSE ORDER BY created_at",
      [sessionId],
    );
    return rows.map(rowToNode);
  }

  getArchived(sessionId: SessionId): SummaryNode[] {
    throw new Error(
      "PgSummaryDag.getArchived() is not supported synchronously. Use getArchivedAsync().",
    );
  }

  async getArchivedAsync(sessionId: SessionId): Promise<SummaryNode[]> {
    const { rows } = await this.pool.query<SummaryRow>(
      "SELECT * FROM lcm_summary_nodes WHERE session_id = $1 AND is_archived = TRUE ORDER BY created_at",
      [sessionId],
    );
    return rows.map(rowToNode);
  }

  getBySession(sessionId: SessionId): SummaryNode[] {
    throw new Error(
      "PgSummaryDag.getBySession() is not supported synchronously. Use getBySessionAsync().",
    );
  }

  async getBySessionAsync(sessionId: SessionId): Promise<SummaryNode[]> {
    const { rows } = await this.pool.query<SummaryRow>(
      "SELECT * FROM lcm_summary_nodes WHERE session_id = $1 ORDER BY created_at",
      [sessionId],
    );
    return rows.map(rowToNode);
  }

  archive(id: SummaryId): void {
    // Fire-and-forget for sync interface
    this.archiveAsync(id).catch((err) => {
      console.error("PgSummaryDag: fire-and-forget archive failed:", err);
    });
  }

  async archiveAsync(id: SummaryId): Promise<void> {
    await this.pool.query(
      "UPDATE lcm_summary_nodes SET is_active = FALSE, is_archived = TRUE WHERE id = $1",
      [id],
    );
  }

  expandToMessageIds(id: SummaryId): MessageId[] {
    throw new Error(
      "PgSummaryDag.expandToMessageIds() is not supported synchronously. Use expandToMessageIdsAsync().",
    );
  }

  /**
   * Recursively collect every source message ID reachable from a node.
   * Uses a recursive CTE for efficiency.
   */
  async expandToMessageIdsAsync(id: SummaryId): Promise<MessageId[]> {
    const { rows } = await this.pool.query<{ message_id: string }>(
      `WITH RECURSIVE lineage AS (
        SELECT id, source_message_ids, source_node_ids
        FROM lcm_summary_nodes WHERE id = $1
        UNION ALL
        SELECT n.id, n.source_message_ids, n.source_node_ids
        FROM lcm_summary_nodes n
        INNER JOIN lineage l ON n.id = ANY(l.source_node_ids)
      )
      SELECT DISTINCT UNNEST(source_message_ids) AS message_id
      FROM lineage`,
      [id],
    );
    return rows.map((r) => r.message_id as MessageId);
  }

  size(): number {
    throw new Error(
      "PgSummaryDag.size() is not supported synchronously. Use sizeAsync().",
    );
  }

  async sizeAsync(): Promise<number> {
    const { rows } = await this.pool.query<{ count: string }>(
      "SELECT COUNT(*) as count FROM lcm_summary_nodes",
    );
    return parseInt(rows[0].count, 10);
  }

  // -----------------------------------------------------------------------
  // Internal
  // -----------------------------------------------------------------------

  private async _insertAsync(node: SummaryNode): Promise<void> {
    await this.pool.query(
      `INSERT INTO lcm_summary_nodes
        (id, session_id, level, content, token_count, created_at, source_message_ids, source_node_ids, is_active, is_archived)
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)`,
      [
        node.id,
        node.sessionId,
        node.level,
        node.content,
        node.tokenCount,
        node.createdAt,
        node.sourceMessageIds,
        node.sourceNodeIds,
        node.isActive,
        node.isArchived,
      ],
    );
  }

  // -----------------------------------------------------------------------
  // Schema management
  // -----------------------------------------------------------------------

  async migrate(): Promise<void> {
    await this.pool.query(`
      CREATE TABLE IF NOT EXISTS lcm_summary_nodes (
        id                  TEXT        PRIMARY KEY,
        session_id          TEXT        NOT NULL,
        level               TEXT        NOT NULL CHECK (level IN ('leaf', 'condensed', 'emergency')),
        content             TEXT        NOT NULL,
        token_count         INTEGER     NOT NULL,
        created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        source_message_ids  TEXT[]      NOT NULL DEFAULT '{}',
        source_node_ids     TEXT[]      NOT NULL DEFAULT '{}',
        is_active           BOOLEAN     NOT NULL DEFAULT TRUE,
        is_archived         BOOLEAN     NOT NULL DEFAULT FALSE
      );
      CREATE INDEX IF NOT EXISTS idx_lcm_summary_nodes_session
        ON lcm_summary_nodes (session_id);
      CREATE INDEX IF NOT EXISTS idx_lcm_summary_nodes_active
        ON lcm_summary_nodes (session_id) WHERE is_active = TRUE AND is_archived = FALSE;
      CREATE INDEX IF NOT EXISTS idx_lcm_summary_nodes_archived
        ON lcm_summary_nodes (session_id) WHERE is_archived = TRUE;
    `);
  }
}
