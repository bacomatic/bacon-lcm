#!/usr/bin/env node
/**
 * bacon-lcm daemon
 *
 * Long-running background process that holds the Postgres connection pool
 * and active sessions.  Hook CLIs connect over a Unix domain socket to
 * avoid per-invocation startup cost.
 *
 * Self-terminates after a configurable idle period (default 5 minutes).
 *
 * Protocol: newline-delimited JSON over a Unix socket.
 *   Request:  { "action": "hook", "platform": "copilot", "hookType": "userPromptSubmitted", "payload": {...} }
 *   Response: { "ok": true, "result": {...} }  or  { "ok": false, "error": "..." }
 *
 *   Request:  { "action": "ping" }
 *   Response: { "ok": true, "uptime": <seconds> }
 *
 *   Request:  { "action": "shutdown" }
 *   Response: { "ok": true }  (then process exits)
 *
 * Usage:
 *   bacon-lcm-daemon                          # foreground
 *   bacon-lcm-daemon --idle-timeout 600       # 10 min idle
 *   BACON_LCM_SOCKET=/tmp/my.sock bacon-lcm-daemon
 */
import net from "node:net";
import fs from "node:fs";
import pg from "pg";
import { loadConfig, resetConfig, getCompactionConfig, getSummarizerConfig } from "../config.js";
import { createTokenCounter } from "../tokenizers/index.js";
import { createSummarizer } from "../summarizers/index.js";
import { createEmbedder } from "../embedders/index.js";
import { PgMessageStore } from "../pg/pg-store.js";
import { PgSummaryDag } from "../pg/pg-dag.js";
import { PgSessionStore } from "../pg/pg-session.js";
import { PgVectorStore } from "../pg/pg-vectors.js";
import { LcmSession } from "../session.js";
import { handleHookEvent } from "./handler.js";
import { parseWindsurfHook } from "./windsurf.js";
import { parseCopilotHook, type CopilotHookType } from "./copilot.js";
import type { CompactionConfig, SessionId, TokenCounter, Summarizer } from "../types.js";
import type { VectorStore } from "../session.js";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

export const DEFAULT_SOCKET_PATH =
  process.env.BACON_LCM_SOCKET ?? "/tmp/bacon-lcm.sock";
const DEFAULT_IDLE_TIMEOUT_MS = 5 * 60 * 1000; // 5 minutes

// ---------------------------------------------------------------------------
// Stores (initialized once on first request)
// ---------------------------------------------------------------------------

let pool: pg.Pool | null = null;
let pgStore: PgMessageStore | null = null;
let pgDag: PgSummaryDag | null = null;
let pgSessionStore: PgSessionStore | null = null;
let vectorStore: VectorStore | undefined;
let tokenCounter: TokenCounter | null = null;
let summarizer: Summarizer | null = null;
let compactionConfig: CompactionConfig | null = null;

const sessions = new Map<string, LcmSession>();

function initStores(): void {
  if (pool) return; // already initialized

  resetConfig();
  const cfg = loadConfig();
  const dbUrl = cfg.databaseUrl ?? process.env.DATABASE_URL;

  compactionConfig = getCompactionConfig();
  const summarizerCfg = getSummarizerConfig();
  tokenCounter = createTokenCounter(cfg);
  summarizer = createSummarizer(summarizerCfg);

  if (dbUrl) {
    pool = new pg.Pool({ connectionString: dbUrl });
    pgStore = new PgMessageStore(pool, tokenCounter);
    pgDag = new PgSummaryDag(pool, tokenCounter);
    pgSessionStore = new PgSessionStore(pool);

    const embedder = createEmbedder(cfg);
    if (embedder.dimensions > 0) {
      vectorStore = new PgVectorStore(pool, embedder);
    }
    log("Postgres stores initialized");
  } else {
    log("No DATABASE_URL — using in-memory stores");
  }
}

// ---------------------------------------------------------------------------
// Session management (mirrors MCP server pattern)
// ---------------------------------------------------------------------------

async function getOrCreateSession(): Promise<LcmSession> {
  initStores();

  // Try to resume the most recent persisted session
  if (pgSessionStore) {
    const rows = await pgSessionStore.list();
    if (rows.length > 0) {
      const id = rows[0].id as SessionId;
      if (sessions.has(id)) return sessions.get(id)!;

      const restored = await LcmSession.restore(
        id,
        tokenCounter!,
        summarizer!,
        compactionConfig!,
        {
          store: pgStore!,
          dag: pgDag!,
          sessionStore: pgSessionStore,
          vectorStore,
        },
      );
      if (restored) {
        sessions.set(id, restored);
        return restored;
      }
    }
  }

  // Create a new session
  const s = new LcmSession(tokenCounter!, summarizer!, compactionConfig!, {
    store: pgStore ?? undefined,
    dag: pgDag ?? undefined,
    sessionStore: pgSessionStore ?? undefined,
    vectorStore,
  });
  sessions.set(s.session.id, s);
  return s;
}

async function createNewSession(): Promise<LcmSession> {
  initStores();
  const s = new LcmSession(tokenCounter!, summarizer!, compactionConfig!, {
    store: pgStore ?? undefined,
    dag: pgDag ?? undefined,
    sessionStore: pgSessionStore ?? undefined,
    vectorStore,
  });
  sessions.set(s.session.id, s);
  return s;
}

// ---------------------------------------------------------------------------
// Request dispatch
// ---------------------------------------------------------------------------

interface DaemonRequest {
  action: "hook" | "ping" | "shutdown";
  platform?: string;
  hookType?: string;
  payload?: Record<string, unknown>;
}

interface DaemonResponse {
  ok: boolean;
  result?: unknown;
  error?: string;
  uptime?: number;
}

const startTime = Date.now();

async function dispatch(req: DaemonRequest): Promise<DaemonResponse> {
  switch (req.action) {
    case "ping":
      return { ok: true, uptime: (Date.now() - startTime) / 1000 };

    case "shutdown":
      // Caller gets a response, then we exit (handled by server loop)
      return { ok: true };

    case "hook": {
      try {
        let event;
        if (req.platform === "windsurf") {
          event = parseWindsurfHook(req.payload as any);
        } else if (req.platform === "copilot") {
          if (!req.hookType) {
            return { ok: false, error: "hookType required for copilot" };
          }
          event = parseCopilotHook(req.hookType as CopilotHookType, req.payload ?? {});
        } else {
          return { ok: false, error: `Unknown platform: ${req.platform}` };
        }

        // session_start → create a fresh session
        if (event.kind === "session_start") {
          const s = await createNewSession();
          return {
            ok: true,
            result: {
              stored: false,
              session_id: s.session.id,
              active_tokens: 0,
              compacted: false,
            },
          };
        }

        const s = await getOrCreateSession();
        const result = await handleHookEventDirect(s, event);
        return { ok: true, result };
      } catch (err) {
        return { ok: false, error: String(err) };
      }
    }

    default:
      return { ok: false, error: `Unknown action: ${(req as any).action}` };
  }
}

/**
 * Handle a hook event directly with a given session, bypassing the
 * standalone handler (which has its own session management).
 */
async function handleHookEventDirect(
  s: LcmSession,
  event: { kind: string; content?: string; platform: string; timestamp: Date },
): Promise<Record<string, unknown>> {
  type MessageRole = "user" | "assistant" | "system" | "tool";
  let role: MessageRole;

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
    case "transcript":
      role = "system";
      break;
    case "session_end":
      return {
        stored: false,
        session_id: s.session.id,
        active_tokens: await s.getTokenCount(),
        compacted: false,
      };
    default:
      return {
        stored: false,
        session_id: s.session.id,
        active_tokens: await s.getTokenCount(),
        compacted: false,
      };
  }

  if (!event.content) {
    return {
      stored: false,
      session_id: s.session.id,
      active_tokens: await s.getTokenCount(),
      compacted: false,
    };
  }

  const { message, compacted } = await s.addMessage(role, event.content, {
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

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

function log(msg: string): void {
  process.stderr.write(`[bacon-lcm-daemon] ${msg}\n`);
}

export async function startDaemon(opts?: {
  socketPath?: string;
  idleTimeoutMs?: number;
}): Promise<net.Server> {
  const socketPath = opts?.socketPath ?? DEFAULT_SOCKET_PATH;
  const idleTimeoutMs = opts?.idleTimeoutMs ?? DEFAULT_IDLE_TIMEOUT_MS;

  // Clean up stale socket
  if (fs.existsSync(socketPath)) {
    try {
      // Test if another daemon is alive
      await new Promise<void>((resolve, reject) => {
        const c = net.createConnection(socketPath, () => {
          c.end();
          reject(new Error("Another daemon is already running"));
        });
        c.on("error", () => resolve()); // Dead socket, safe to remove
      });
    } catch (err) {
      if ((err as Error).message.includes("already running")) throw err;
    }
    fs.unlinkSync(socketPath);
  }

  let idleTimer: ReturnType<typeof setTimeout>;

  function resetIdleTimer(): void {
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => {
      log(`Idle for ${idleTimeoutMs / 1000}s — shutting down`);
      shutdown();
    }, idleTimeoutMs);
  }

  const server = net.createServer((conn) => {
    resetIdleTimer();

    let buffer = "";
    conn.on("data", (chunk) => {
      buffer += chunk.toString();

      // Process complete lines
      let newlineIdx: number;
      while ((newlineIdx = buffer.indexOf("\n")) !== -1) {
        const line = buffer.slice(0, newlineIdx).trim();
        buffer = buffer.slice(newlineIdx + 1);

        if (!line) continue;

        let req: DaemonRequest;
        try {
          req = JSON.parse(line);
        } catch {
          conn.write(JSON.stringify({ ok: false, error: "Invalid JSON" }) + "\n");
          continue;
        }

        dispatch(req)
          .then((resp) => {
            conn.write(JSON.stringify(resp) + "\n");
            if (req.action === "shutdown") {
              setTimeout(() => shutdown(), 100);
            }
          })
          .catch((err) => {
            conn.write(JSON.stringify({ ok: false, error: String(err) }) + "\n");
          });
      }
    });
  });

  async function shutdown(): Promise<void> {
    clearTimeout(idleTimer);
    server.close();
    if (pool) {
      await pool.end();
      pool = null;
    }
    try {
      fs.unlinkSync(socketPath);
    } catch {
      // ignore
    }
    process.exit(0);
  }

  // Handle signals gracefully
  process.on("SIGTERM", () => shutdown());
  process.on("SIGINT", () => shutdown());

  return new Promise((resolve, reject) => {
    server.on("error", reject);
    server.listen(socketPath, () => {
      log(`Listening on ${socketPath} (idle timeout: ${idleTimeoutMs / 1000}s)`);
      resetIdleTimer();
      resolve(server);
    });
  });
}

// ---------------------------------------------------------------------------
// CLI entry point
// ---------------------------------------------------------------------------

if (process.argv[1]?.endsWith("daemon.js") || process.argv[1]?.endsWith("daemon.ts")) {
  const args = process.argv.slice(2);
  const idleIdx = args.indexOf("--idle-timeout");
  const idleTimeoutMs = idleIdx >= 0
    ? parseInt(args[idleIdx + 1], 10) * 1000
    : DEFAULT_IDLE_TIMEOUT_MS;

  const socketIdx = args.indexOf("--socket");
  const socketPath = socketIdx >= 0 ? args[socketIdx + 1] : DEFAULT_SOCKET_PATH;

  startDaemon({ socketPath, idleTimeoutMs }).catch((err) => {
    process.stderr.write(`bacon-lcm-daemon: ${err}\n`);
    process.exit(1);
  });
}
