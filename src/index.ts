/**
 * bacon-lcm — Lossless Context Memory
 *
 * Public API surface.
 */

// Core types
export type {
  CompactionConfig,
  ContextItem,
  Message,
  MessageId,
  MessageRole,
  Session,
  SessionId,
  Summarizer,
  SummaryId,
  SummaryLevel,
  SummaryNode,
  ThresholdConfig,
  TokenCounter,
} from "./types.js";

// ID factories
export { newMessageId, newSessionId, newSummaryId } from "./ids.js";

// Store
export type { MessageStore } from "./store.js";
export { InMemoryMessageStore } from "./store.js";

// DAG
export type { SummaryDag } from "./dag.js";
export { InMemorySummaryDag } from "./dag.js";

// Compaction
export type { CompactionResult } from "./compaction.js";
export { CompactionEngine } from "./compaction.js";

// Context assembly
export { ContextAssembler } from "./context.js";

// Retrieval
export type { DescribeResult } from "./retrieval.js";
export { RetrievalService } from "./retrieval.js";

// Session manager
export { LcmSession } from "./session.js";

// Defaults
export {
  DEFAULT_COMPACTION_CONFIG,
  DEFAULT_THRESHOLDS,
  EchoSummarizer,
  NaiveTokenCounter,
} from "./defaults.js";
