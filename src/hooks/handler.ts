/**
 * Unified Hook Handler
 *
 * Platform-agnostic logic for capturing messages from agent hooks into
 * the LCM immutable store.  Platform adapters (Windsurf, Copilot CLI)
 * normalize their input into the common HookEvent type and call this handler.
 *
 * When DATABASE_URL is set, sessions are persisted to PostgreSQL so they
 * survive across process invocations (each hook call is a new process).
 * Without it, falls back to in-memory (ephemeral, useful only for testing).
 */
import pg from "pg";
import {
  DEFAULT_COMPACTION_CONFIG,
  EchoSummarizer,
  NaiveTokenCounter,
} from "../defaults.js";
import { loadConfig, getCompactionConfig, getSummarizerConfig } from "../config.js";
import { createTokenCounter } from "../tokenizers/index.js";
import { createSummarizer } from "../summarizers/index.js";
import { createEmbedder } from "../embedders/index.js";
import { PgMessageStore } from "../pg/pg-store.js";
import { PgSummaryDag } from "../pg/pg-dag.js";
import { PgSessionStore } from "../pg/pg-session.js";
import { PgVectorStore } from "../pg/pg-vectors.js";
import { LcmSession } from "../session.js";
import type { CompactionConfig, MessageRole, SessionId, TokenCounter, Summarizer } from "../types.js";
import type { VectorStore } from "../session.js";

// ---------------------------------------------------------------------------
// Common event type that all platform adapters produce
// ---------------------------------------------------------------------------

export interface HookEvent {
  /** Which platform produced this event */
  platform: "windsurf" | "copilot-cli" | "unknown";
  /** Event kind */
  kind:
    | "user_prompt"
    | "assistant_response"
    | "session_start"
    | "session_end"
    | "tool_use"
    | "transcript";
  /** Message content (if applicable) */
  content?: string;
  /** Timestamp */
  timestamp: Date;
  /** Platform-specific raw payload (preserved for debugging) */
  raw: unknown;
}

// ---------------------------------------------------------------------------
// Session bootstrap — Postgres-backed when DATABASE_URL is set
// ---------------------------------------------------------------------------

let session: LcmSession | null = null;
let pool: pg.Pool | null = null;

interface PgStores {
  store: PgMessageStore;
  dag: PgSummaryDag;
  sessionStore: PgSessionStore;
  vectorStore?: VectorStore;
  tokenCounter: TokenCounter;
  summarizer: Summarizer;
  config: CompactionConfig;
}

let pgStores: PgStores | null = null;

async function initPgStores(): Promise<PgStores | null> {
  if (pgStores) return pgStores;

  const cfg = loadConfig();
  const dbUrl = cfg.databaseUrl ?? process.env.DATABASE_URL;
  if (!dbUrl) return null;

  const compactionCfg = getCompactionConfig();
  const summarizerCfg = getSummarizerConfig();
  const tokenCounter = createTokenCounter(cfg);
  const summarizer = createSummarizer(summarizerCfg);

  pool = new pg.Pool({ connectionString: dbUrl });
  const store = new PgMessageStore(pool, tokenCounter);
  const dag = new PgSummaryDag(pool, tokenCounter);
  const sessionStore = new PgSessionStore(pool);

  let vectorStore: VectorStore | undefined;
  const embedder = createEmbedder(cfg);
  if (embedder.dimensions > 0) {
    vectorStore = new PgVectorStore(pool, embedder);
  }

  pgStores = { store, dag, sessionStore, vectorStore, tokenCounter, summarizer, config: compactionCfg };
  return pgStores;
}

/**
 * Get or restore the current session. When Postgres is available:
 * 1. Resume the most recently created session, OR
 * 2. Create a new Postgres-backed session.
 */
async function getSession(): Promise<LcmSession> {
  if (session) return session;

  const stores = await initPgStores();

  if (!stores) {
    // Fallback: ephemeral in-memory session
    const config: CompactionConfig = { ...DEFAULT_COMPACTION_CONFIG };
    session = new LcmSession(new NaiveTokenCounter(), new EchoSummarizer(), config);
    return session;
  }

  // Try to resume the most recent session
  const rows = await stores.sessionStore.list();
  if (rows.length > 0) {
    const restored = await LcmSession.restore(
      rows[0].id as SessionId,
      stores.tokenCounter,
      stores.summarizer,
      stores.config,
      {
        store: stores.store,
        dag: stores.dag,
        sessionStore: stores.sessionStore,
        vectorStore: stores.vectorStore,
      },
    );
    if (restored) {
      session = restored;
      return session;
    }
  }

  // No existing session — create a new Postgres-backed one
  session = new LcmSession(stores.tokenCounter, stores.summarizer, stores.config, {
    store: stores.store,
    dag: stores.dag,
    sessionStore: stores.sessionStore,
    vectorStore: stores.vectorStore,
  });
  return session;
}

export function resetSession(): void {
  session = null;
}

/** Clean up the connection pool (call on process exit) */
export async function shutdownPool(): Promise<void> {
  if (pool) {
    await pool.end();
    pool = null;
  }
  pgStores = null;
}

// ---------------------------------------------------------------------------
// Handle an event
// ---------------------------------------------------------------------------

export interface HookResult {
  stored: boolean;
  message_id?: string;
  session_id: string;
  active_tokens: number;
  compacted: boolean;
}

export async function handleHookEvent(event: HookEvent): Promise<HookResult> {
  const s = await getSession();

  let role: MessageRole;
  let content: string | undefined = event.content;

  switch (event.kind) {
    case "user_prompt":
      role = "user";
      break;
    case "assistant_response":
      role = "assistant";
      break;
    case "tool_use":
      role = "tool";
      break;
    case "session_start":
      resetSession();
      {
        const fresh = await getSession();
        return {
          stored: false,
          session_id: fresh.session.id,
          active_tokens: 0,
          compacted: false,
        };
      }
    case "session_end":
      return {
        stored: false,
        session_id: s.session.id,
        active_tokens: await s.getTokenCount(),
        compacted: false,
      };
    case "transcript":
      role = "system";
      break;
    default:
      return {
        stored: false,
        session_id: s.session.id,
        active_tokens: await s.getTokenCount(),
        compacted: false,
      };
  }

  if (!content) {
    return {
      stored: false,
      session_id: s.session.id,
      active_tokens: await s.getTokenCount(),
      compacted: false,
    };
  }

  const { message, compacted } = await s.addMessage(role, content, {
    platform: event.platform,
    hook_kind: event.kind,
    timestamp: event.timestamp.toISOString(),
  });

  return {
    stored: true,
    message_id: message.id,
    session_id: s.session.id,
    active_tokens: await s.getTokenCount(),
    compacted,
  };
}
