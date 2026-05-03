/**
 * Retrieval Tools — lcm_describe & lcm_expand
 *
 * These mirror the Dolt retrieval traversal from the LCM paper:
 *
 *   lcm_describe  → surfaces lineage metadata for a summary node
 *                    (level, archived status, source pointers)
 *
 *   lcm_expand    → follows lineage pointers and returns the original
 *                    verbatim messages that a summary was derived from
 */
import type { SummaryDag } from "./dag.js";
import type { MessageStore } from "./store.js";
import type { Message, SummaryId, SummaryNode } from "./types.js";

// ---------------------------------------------------------------------------
// Describe result
// ---------------------------------------------------------------------------

export interface DescribeResult {
  id: SummaryId;
  level: SummaryNode["level"];
  tokenCount: number;
  isActive: boolean;
  isArchived: boolean;
  sourceMessageCount: number;
  sourceNodeCount: number;
  /** Total number of original messages reachable through the full lineage */
  totalReachableMessages: number;
  createdAt: Date;
}

// ---------------------------------------------------------------------------
// Retrieval service
// ---------------------------------------------------------------------------

export class RetrievalService {
  constructor(
    private readonly store: MessageStore,
    private readonly dag: SummaryDag,
  ) {}

  /**
   * Describe a summary node's lineage metadata without expanding it.
   */
  describe(id: SummaryId): DescribeResult | undefined {
    const node = this.dag.get(id);
    if (!node) return undefined;

    const reachableIds = this.dag.expandToMessageIds(id);

    return {
      id: node.id,
      level: node.level,
      tokenCount: node.tokenCount,
      isActive: node.isActive,
      isArchived: node.isArchived,
      sourceMessageCount: node.sourceMessageIds.length,
      sourceNodeCount: node.sourceNodeIds.length,
      totalReachableMessages: reachableIds.length,
      createdAt: node.createdAt,
    };
  }

  /**
   * Expand a summary node to its original verbatim messages.
   * Follows the full lineage chain through the DAG.
   */
  expand(id: SummaryId): Message[] {
    const messageIds = this.dag.expandToMessageIds(id);
    return this.store
      .getMany(messageIds)
      .sort((a, b) => a.sequenceNumber - b.sequenceNumber);
  }

  /**
   * List all archived summary nodes for a session, suitable for
   * presenting as retrieval cues in pre-response hooks.
   */
  listArchived(sessionId: string): DescribeResult[] {
    const archived = this.dag.getArchived(sessionId as any);
    return archived
      .map((node) => this.describe(node.id))
      .filter((d): d is DescribeResult => d !== undefined);
  }
}
