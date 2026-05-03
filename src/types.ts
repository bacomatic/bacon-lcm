/**
 * Core types for the Lossless Context Memory system.
 *
 * The LCM architecture maintains two key structures:
 *   1. Immutable Store — verbatim record of every message, never modified
 *   2. Summary DAG    — a directed acyclic graph of compressed summary nodes
 *                       that act as materialized views over the immutable history
 */

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/** Opaque branded ID types for type safety */
export type MessageId = string & { readonly __brand: "MessageId" };
export type SummaryId = string & { readonly __brand: "SummaryId" };
export type SessionId = string & { readonly __brand: "SessionId" };

// ---------------------------------------------------------------------------
// Messages (Immutable Store)
// ---------------------------------------------------------------------------

export type MessageRole = "user" | "assistant" | "tool" | "system";

/**
 * A single immutable message exactly as produced during a session.
 * Once persisted, a Message is never modified or deleted.
 */
export interface Message {
  readonly id: MessageId;
  readonly sessionId: SessionId;
  readonly role: MessageRole;
  readonly content: string;
  /** Monotonically increasing within a session */
  readonly sequenceNumber: number;
  /** Token count as estimated by the configured tokenizer */
  readonly tokenCount: number;
  readonly createdAt: Date;
  /** Optional metadata (tool name, tool call id, etc.) */
  readonly metadata?: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// Summary Nodes (DAG)
// ---------------------------------------------------------------------------

/**
 * Summarization level, following the LCM three-level escalation protocol:
 *   - leaf        : direct summary of a contiguous chunk of raw messages
 *   - condensed   : summary of multiple leaf (or lower condensed) nodes
 *   - emergency   : deterministic fallback that requires no LLM inference
 */
export type SummaryLevel = "leaf" | "condensed" | "emergency";

/**
 * A node in the summary DAG.  Each node is a compressed representation of
 * one or more source items (raw messages or lower-level summary nodes).
 *
 * Lineage pointers (`sourceMessageIds` / `sourceNodeIds`) allow any summary
 * to be expanded back to the original verbatim messages.
 */
export interface SummaryNode {
  readonly id: SummaryId;
  readonly sessionId: SessionId;
  readonly level: SummaryLevel;
  readonly content: string;
  readonly tokenCount: number;
  readonly createdAt: Date;

  /** IDs of the raw messages this node was derived from (leaf nodes) */
  readonly sourceMessageIds: MessageId[];
  /** IDs of child summary nodes this was derived from (condensed nodes) */
  readonly sourceNodeIds: SummaryId[];

  /** Whether this node is currently part of the active context window */
  isActive: boolean;
  /** Whether this node has been archived (pushed off-context) */
  isArchived: boolean;
}

// ---------------------------------------------------------------------------
// Active Context Window
// ---------------------------------------------------------------------------

/**
 * An item in the active context window sent to the LLM.
 * Can be either a raw message or a summary node standing in for
 * a group of older messages.
 */
export type ContextItem =
  | { kind: "message"; message: Message }
  | { kind: "summary"; summary: SummaryNode };

// ---------------------------------------------------------------------------
// Compaction Thresholds
// ---------------------------------------------------------------------------

/**
 * Token-based thresholds that drive the deterministic control loop.
 *
 *  - Below `softLimit`: no summarization, raw latency only.
 *  - Between `softLimit` and `hardLimit`: async compaction between turns.
 *  - At `hardLimit`: synchronous compaction before the next LLM call.
 *  - `riskBuffer`: headroom kept below the model's absolute max to avoid
 *    truncation on the response side.
 */
export interface ThresholdConfig {
  /** Model's absolute maximum context length in tokens */
  modelMaxTokens: number;
  /** Soft threshold — triggers async background compaction */
  softLimit: number;
  /** Hard threshold — triggers synchronous compaction */
  hardLimit: number;
  /** Tokens reserved for the model's response */
  riskBuffer: number;
}

/**
 * Configuration for the compaction engine.
 */
export interface CompactionConfig {
  thresholds: ThresholdConfig;
  /** Minimum number of messages to group into a single leaf summary */
  leafMinFanout: number;
  /** Target token count for a leaf summary chunk */
  leafChunkTokens: number;
  /** Minimum number of leaves to merge into a condensed summary */
  condensedMinFanout: number;
  /** Target token count for a condensed summary */
  condensedTargetTokens: number;
  /** Number of most-recent raw messages to keep un-summarized (the "fresh tail") */
  freshTailCount: number;
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

export interface Session {
  readonly id: SessionId;
  readonly createdAt: Date;
  /** Running total of tokens currently in the active context */
  activeTokenCount: number;
}

// ---------------------------------------------------------------------------
// Summarizer interface (pluggable)
// ---------------------------------------------------------------------------

/**
 * Abstraction over the LLM call that produces summaries.
 * Consumers provide an implementation backed by their LLM provider of choice.
 */
export interface Summarizer {
  /** Summarize a list of raw message contents into a single compressed text. */
  summarize(texts: string[], level: SummaryLevel): Promise<string>;
}

// ---------------------------------------------------------------------------
// Token counter interface (pluggable)
// ---------------------------------------------------------------------------

/**
 * Abstraction over token counting so callers can plug in tiktoken, llama
 * tokenizer, or a simple heuristic.
 */
export interface TokenCounter {
  count(text: string): number;
}
