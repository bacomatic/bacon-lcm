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
export type { LcmSessionOptions } from "./session.js";
export { LcmSession } from "./session.js";

// PostgreSQL persistence
export { PgMessageStore } from "./pg/index.js";
export { PgSummaryDag } from "./pg/index.js";

// Defaults
export {
  DEFAULT_COMPACTION_CONFIG,
  DEFAULT_THRESHOLDS,
  EchoSummarizer,
  NaiveTokenCounter,
} from "./defaults.js";

// Config
export { loadConfig, resetConfig, getCompactionConfig, getSummarizerConfig } from "./config.js";
export type { LcmConfig, SummarizerConfig } from "./config.js";

// Summarizers
export { createSummarizer, OpenAISummarizer, AnthropicSummarizer } from "./summarizers/index.js";

// Dashboard
export { registry, startDashboard } from "./dashboard/index.js";
export type { SessionSnapshot, DashboardOverview, DashboardOptions } from "./dashboard/index.js";

// Hooks
export type { HookEvent, HookResult } from "./hooks/index.js";
export { handleHookEvent, resetSession } from "./hooks/index.js";
export { parseWindsurfHook } from "./hooks/index.js";
export { parseCopilotHook } from "./hooks/index.js";
export type { CopilotHookType } from "./hooks/index.js";
