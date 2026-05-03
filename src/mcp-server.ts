#!/usr/bin/env node
/**
 * bacon-lcm MCP Server
 *
 * Exposes LCM tools via the Model Context Protocol (stdio transport).
 * Works with Windsurf, Devin, Copilot CLI, and any other MCP-compatible host.
 *
 * Tools:
 *   lcm_store          — persist a message and trigger compaction if needed
 *   lcm_recall         — retrieve the current active context window
 *   lcm_describe       — inspect a summary node's lineage metadata
 *   lcm_expand         — expand a summary back to original messages
 *   lcm_session_new    — start a new LCM session
 *   lcm_session_list   — list all persisted sessions
 *   lcm_session_resume — resume a previously persisted session
 *   lcm_session_info   — get current session stats
 */
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import pg from "pg";
import { z } from "zod";
import { createTokenCounter } from "./tokenizers/index.js";
import { PgMessageStore } from "./pg/pg-store.js";
import { PgSummaryDag } from "./pg/pg-dag.js";
import { PgSessionStore } from "./pg/pg-session.js";
import { LcmSession } from "./session.js";
import type { SessionPersistence } from "./session.js";
import type { MessageStore } from "./store.js";
import type { SummaryDag } from "./dag.js";
import type { SessionId, SummaryId } from "./types.js";
import { loadConfig, type LcmConfig } from "./config.js";
import { createSummarizer } from "./summarizers/index.js";
import { registry } from "./dashboard/registry.js";
import { startDashboard } from "./dashboard/server.js";

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

let config: LcmConfig;
let tokenCounter: import("./types.js").TokenCounter;

let pool: pg.Pool | null = null;
let sharedStore: MessageStore | undefined;
let sharedDag: SummaryDag | undefined;
let sharedSessionStore: SessionPersistence | undefined;

async function init(): Promise<void> {
  config = loadConfig();
  tokenCounter = createTokenCounter(config);

  // Log provider info
  console.error(
    `bacon-lcm MCP: summarizer=${config.summarizer.provider}` +
      (config.summarizer.model ? ` model=${config.summarizer.model}` : ""),
  );
  console.error(
    `bacon-lcm MCP: tokenizer=${config.tokenizer?.type ?? "auto"}` +
      ` (resolved=${tokenCounter.constructor.name})`,
  );

  // Postgres persistence
  if (config.databaseUrl) {
    pool = new pg.Pool({ connectionString: config.databaseUrl });
    const pgStore = new PgMessageStore(pool, tokenCounter);
    const pgDag = new PgSummaryDag(pool, tokenCounter);
    await pgStore.migrate();
    await pgDag.migrate();
    sharedStore = pgStore;
    sharedDag = pgDag;
    sharedSessionStore = new PgSessionStore(pool);
    console.error(`bacon-lcm MCP: using Postgres (sessions persisted)`);
  } else {
    console.error("bacon-lcm MCP: using in-memory storage (set DATABASE_URL for persistence)");
  }

  // Dashboard
  if (config.dashboard?.enabled) {
    startDashboard({
      port: config.dashboard.port,
      host: config.dashboard.host,
    });
  }
}

// ---------------------------------------------------------------------------
// Session registry
// ---------------------------------------------------------------------------

const sessions = new Map<string, LcmSession>();

let activeSessionId: string | null = null;

function createNewSession(): LcmSession {
  const summarizer = createSummarizer(config.summarizer);
  const session = new LcmSession(
    tokenCounter,
    summarizer,
    config.compaction,
    { store: sharedStore, dag: sharedDag, sessionStore: sharedSessionStore },
  );
  activeSessionId = session.session.id;
  sessions.set(activeSessionId, session);
  registry.register(session);
  registry.setActive(activeSessionId);
  // Persist initial session row
  session.save().catch(() => {});
  return session;
}

function getActiveSession(): LcmSession {
  if (!activeSessionId || !sessions.has(activeSessionId)) {
    return createNewSession();
  }
  return sessions.get(activeSessionId)!;
}

// ---------------------------------------------------------------------------
// MCP Server
// ---------------------------------------------------------------------------

const server = new McpServer({
  name: "bacon-lcm",
  version: "0.1.0",
});

// -- lcm_store ---------------------------------------------------------------

server.tool(
  "lcm_store",
  "Persist a message to the LCM immutable store. Compaction runs automatically when thresholds are exceeded.",
  {
    role: z.enum(["user", "assistant", "tool", "system"]).describe("Message role"),
    content: z.string().describe("Message content"),
    session_id: z.string().optional().describe("Session ID (uses active session if omitted)"),
  },
  async ({ role, content, session_id }) => {
    const session = session_id && sessions.has(session_id)
      ? sessions.get(session_id)!
      : getActiveSession();

    const { message, compacted } = await session.addMessage(role, content);

    return {
      content: [
        {
          type: "text" as const,
          text: JSON.stringify({
            message_id: message.id,
            sequence_number: message.sequenceNumber,
            token_count: message.tokenCount,
            compacted,
            active_context_tokens: await session.getTokenCount(),
          }),
        },
      ],
    };
  },
);

// -- lcm_recall --------------------------------------------------------------

server.tool(
  "lcm_recall",
  "Retrieve the current active context window (summaries + fresh tail messages).",
  {
    session_id: z.string().optional().describe("Session ID (uses active session if omitted)"),
  },
  async ({ session_id }) => {
    const session = session_id && sessions.has(session_id)
      ? sessions.get(session_id)!
      : getActiveSession();

    const ctx = await session.getContext();
    const items = ctx.map((item) => {
      if (item.kind === "message") {
        return {
          kind: "message",
          id: item.message.id,
          role: item.message.role,
          content: item.message.content,
          sequence_number: item.message.sequenceNumber,
          token_count: item.message.tokenCount,
        };
      }
      return {
        kind: "summary",
        id: item.summary.id,
        level: item.summary.level,
        content: item.summary.content,
        token_count: item.summary.tokenCount,
        source_message_count: item.summary.sourceMessageIds.length,
        is_archived: item.summary.isArchived,
      };
    });

    return {
      content: [
        {
          type: "text" as const,
          text: JSON.stringify({
            session_id: session.session.id,
            total_tokens: await session.getTokenCount(),
            item_count: items.length,
            items,
          }),
        },
      ],
    };
  },
);

// -- lcm_describe ------------------------------------------------------------

server.tool(
  "lcm_describe",
  "Inspect a summary node's lineage metadata without expanding it.",
  {
    summary_id: z.string().describe("The summary node ID to describe"),
    session_id: z.string().optional().describe("Session ID (uses active session if omitted)"),
  },
  async ({ summary_id, session_id }) => {
    const session = session_id && sessions.has(session_id)
      ? sessions.get(session_id)!
      : getActiveSession();

    const desc = await session.describe(summary_id as SummaryId);
    if (!desc) {
      return {
        content: [{ type: "text" as const, text: JSON.stringify({ error: "Summary node not found" }) }],
      };
    }

    return {
      content: [{ type: "text" as const, text: JSON.stringify(desc) }],
    };
  },
);

// -- lcm_expand --------------------------------------------------------------

server.tool(
  "lcm_expand",
  "Expand a summary node to its original verbatim messages via lineage traversal.",
  {
    summary_id: z.string().describe("The summary node ID to expand"),
    session_id: z.string().optional().describe("Session ID (uses active session if omitted)"),
  },
  async ({ summary_id, session_id }) => {
    const session = session_id && sessions.has(session_id)
      ? sessions.get(session_id)!
      : getActiveSession();

    const messages = await session.expand(summary_id as SummaryId);
    if (messages.length === 0) {
      return {
        content: [{ type: "text" as const, text: JSON.stringify({ error: "No messages found for summary" }) }],
      };
    }

    const items = messages.map((m) => ({
      id: m.id,
      role: m.role,
      content: m.content,
      sequence_number: m.sequenceNumber,
      token_count: m.tokenCount,
    }));

    return {
      content: [{ type: "text" as const, text: JSON.stringify({ count: items.length, messages: items }) }],
    };
  },
);

// -- lcm_session_new ---------------------------------------------------------

server.tool(
  "lcm_session_new",
  "Create a new LCM session and set it as the active session.",
  {},
  async () => {
    const session = createNewSession();

    return {
      content: [
        {
          type: "text" as const,
          text: JSON.stringify({
            session_id: session.session.id,
            created_at: session.session.createdAt.toISOString(),
          }),
        },
      ],
    };
  },
);

// -- lcm_session_list --------------------------------------------------------

server.tool(
  "lcm_session_list",
  "List all persisted LCM sessions. Requires DATABASE_URL for Postgres persistence.",
  {},
  async () => {
    if (!sharedSessionStore) {
      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              error: "Session persistence not configured (set DATABASE_URL)",
              in_memory_sessions: Array.from(sessions.keys()),
            }),
          },
        ],
      };
    }

    const rows = await sharedSessionStore.list();
    return {
      content: [
        {
          type: "text" as const,
          text: JSON.stringify({
            sessions: rows.map((r) => ({
              id: r.id,
              created_at: r.createdAt.toISOString(),
              active_token_count: r.activeTokenCount,
            })),
          }),
        },
      ],
    };
  },
);

// -- lcm_session_resume ------------------------------------------------------

server.tool(
  "lcm_session_resume",
  "Resume a previously persisted session by its ID. The session becomes the active session.",
  {
    session_id: z.string().describe("The session ID to resume"),
  },
  async ({ session_id }) => {
    // Already in memory?
    if (sessions.has(session_id)) {
      activeSessionId = session_id;
      registry.setActive(session_id);
      const session = sessions.get(session_id)!;
      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              session_id: session.session.id,
              created_at: session.session.createdAt.toISOString(),
              active_token_count: session.session.activeTokenCount,
              source: "memory",
            }),
          },
        ],
      };
    }

    // Try to restore from Postgres
    if (!sharedSessionStore || !sharedStore || !sharedDag) {
      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              error: "Session persistence not configured (set DATABASE_URL)",
            }),
          },
        ],
      };
    }

    const summarizer = createSummarizer(config.summarizer);
    const restored = await LcmSession.restore(
      session_id as SessionId,
      tokenCounter,
      summarizer,
      config.compaction,
      { store: sharedStore, dag: sharedDag, sessionStore: sharedSessionStore },
    );

    if (!restored) {
      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({ error: `Session '${session_id}' not found` }),
          },
        ],
      };
    }

    activeSessionId = restored.session.id;
    sessions.set(activeSessionId, restored);
    registry.register(restored);
    registry.setActive(activeSessionId);

    return {
      content: [
        {
          type: "text" as const,
          text: JSON.stringify({
            session_id: restored.session.id,
            created_at: restored.session.createdAt.toISOString(),
            active_token_count: restored.session.activeTokenCount,
            source: "postgres",
          }),
        },
      ],
    };
  },
);

// -- lcm_session_info --------------------------------------------------------

server.tool(
  "lcm_session_info",
  "Get current session statistics.",
  {
    session_id: z.string().optional().describe("Session ID (uses active session if omitted)"),
  },
  async ({ session_id }) => {
    const session = session_id && sessions.has(session_id)
      ? sessions.get(session_id)!
      : getActiveSession();

    const ctx = await session.getContext();
    const summaryCount = ctx.filter((i) => i.kind === "summary").length;
    const messageCount = ctx.filter((i) => i.kind === "message").length;
    const archivedCount = (await session.dag.getArchived(session.session.id)).length;

    return {
      content: [
        {
          type: "text" as const,
          text: JSON.stringify({
            session_id: session.session.id,
            total_messages_stored: await session.store.size(),
            active_context_tokens: await session.getTokenCount(),
            active_summaries: summaryCount,
            active_raw_messages: messageCount,
            archived_summaries: archivedCount,
            total_summary_nodes: await session.dag.size(),
          }),
        },
      ],
    };
  },
);

// ---------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------

async function main() {
  await init();
  const transport = new StdioServerTransport();
  await server.connect(transport);
}

main().catch((err) => {
  console.error("bacon-lcm MCP server fatal error:", err);
  process.exit(1);
});
