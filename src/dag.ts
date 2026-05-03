/**
 * Summary DAG
 *
 * A directed acyclic graph of summary nodes.  Each node is a compressed
 * representation derived from either raw messages (leaf) or other summary
 * nodes (condensed / emergency).
 *
 * The DAG preserves full lineage: every summary can be traced back to the
 * original verbatim messages via `sourceMessageIds` and `sourceNodeIds`.
 */
import type {
  MessageId,
  SessionId,
  SummaryId,
  SummaryLevel,
  SummaryNode,
  TokenCounter,
} from "./types.js";
import { newSummaryId } from "./ids.js";

// ---------------------------------------------------------------------------
// Interface
// ---------------------------------------------------------------------------

export interface SummaryDag {
  /** Create a new summary node and add it to the DAG. */
  add(
    sessionId: SessionId,
    level: SummaryLevel,
    content: string,
    sourceMessageIds: MessageId[],
    sourceNodeIds: SummaryId[],
  ): SummaryNode;

  /** Retrieve a node by ID. */
  get(id: SummaryId): SummaryNode | undefined;

  /** All active (in-context) summary nodes for a session, oldest first. */
  getActive(sessionId: SessionId): SummaryNode[];

  /** All archived (off-context) summary nodes for a session. */
  getArchived(sessionId: SessionId): SummaryNode[];

  /** All nodes for a session regardless of status. */
  getBySession(sessionId: SessionId): SummaryNode[];

  /** Mark a node as archived (no longer in active context). */
  archive(id: SummaryId): void;

  /** Recursively collect every source message ID reachable from a node. */
  expandToMessageIds(id: SummaryId): MessageId[];

  /** Total nodes stored. */
  size(): number;
}

// ---------------------------------------------------------------------------
// In-memory implementation
// ---------------------------------------------------------------------------

export class InMemorySummaryDag implements SummaryDag {
  private readonly nodes = new Map<SummaryId, SummaryNode>();
  private readonly sessionIndex = new Map<SessionId, SummaryNode[]>();

  constructor(private readonly tokenCounter: TokenCounter) {}

  add(
    sessionId: SessionId,
    level: SummaryLevel,
    content: string,
    sourceMessageIds: MessageId[],
    sourceNodeIds: SummaryId[],
  ): SummaryNode {
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

    this.nodes.set(node.id, node);

    let list = this.sessionIndex.get(sessionId);
    if (!list) {
      list = [];
      this.sessionIndex.set(sessionId, list);
    }
    list.push(node);

    return node;
  }

  get(id: SummaryId): SummaryNode | undefined {
    return this.nodes.get(id);
  }

  getActive(sessionId: SessionId): SummaryNode[] {
    return (this.sessionIndex.get(sessionId) ?? []).filter(
      (n) => n.isActive && !n.isArchived,
    );
  }

  getArchived(sessionId: SessionId): SummaryNode[] {
    return (this.sessionIndex.get(sessionId) ?? []).filter((n) => n.isArchived);
  }

  getBySession(sessionId: SessionId): SummaryNode[] {
    return this.sessionIndex.get(sessionId) ?? [];
  }

  archive(id: SummaryId): void {
    const node = this.nodes.get(id);
    if (node) {
      node.isActive = false;
      node.isArchived = true;
    }
  }

  expandToMessageIds(id: SummaryId): MessageId[] {
    const node = this.nodes.get(id);
    if (!node) return [];

    const messageIds = new Set<MessageId>(node.sourceMessageIds);

    for (const childId of node.sourceNodeIds) {
      for (const mid of this.expandToMessageIds(childId)) {
        messageIds.add(mid);
      }
    }

    return [...messageIds];
  }

  size(): number {
    return this.nodes.size;
  }
}
