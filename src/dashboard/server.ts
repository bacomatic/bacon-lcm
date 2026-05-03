#!/usr/bin/env node
/**
 * bacon-lcm Dashboard Server
 *
 * Lightweight HTTP server that exposes a REST API and serves the
 * dashboard UI.  Uses only Node built-ins (http, fs, path) — no
 * Express dependency required.
 *
 * Usage:
 *   import { startDashboard } from "bacon-lcm/dashboard";
 *   startDashboard({ port: 3333 });
 *
 * Or as CLI:
 *   bacon-lcm-dashboard          # port 3333
 *   DASHBOARD_PORT=4000 bacon-lcm-dashboard
 */
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { registry, type SessionSnapshot } from "./registry.js";

// ---------------------------------------------------------------------------
// HTML asset
// ---------------------------------------------------------------------------

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

// In dist/ the HTML lives beside the compiled JS; in src/ it's the same dir.
// We try both locations.
function loadDashboardHtml(): string {
  const candidates = [
    join(__dirname, "dashboard.html"),
    join(__dirname, "..", "..", "src", "dashboard", "dashboard.html"),
  ];
  for (const p of candidates) {
    try {
      return readFileSync(p, "utf-8");
    } catch {
      // try next
    }
  }
  return "<html><body><h1>Dashboard HTML not found</h1></body></html>";
}

// ---------------------------------------------------------------------------
// API handlers
// ---------------------------------------------------------------------------

async function handleOverview(_req: IncomingMessage, res: ServerResponse) {
  const overview = await registry.overview();

  // Also attach compaction thresholds from the active session if available
  const active = registry.getActive();
  const thresholds = active?.config?.thresholds ?? null;

  res.writeHead(200, { "Content-Type": "application/json" });
  res.end(JSON.stringify({ ...overview, thresholds }));
}

async function handleSession(req: IncomingMessage, res: ServerResponse) {
  const url = new URL(req.url!, `http://${req.headers.host}`);
  const sessionId = url.searchParams.get("id");
  if (!sessionId) {
    res.writeHead(400, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ error: "Missing ?id= parameter" }));
    return;
  }

  const snap = await registry.snapshot(sessionId);
  if (!snap) {
    res.writeHead(404, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ error: "Session not found" }));
    return;
  }

  // Get context items for the detail view
  const session = registry.get(sessionId);
  let contextItems: unknown[] = [];
  if (session) {
    const ctx = await session.getContext();
    contextItems = ctx.map((item) => {
      if (item.kind === "message") {
        return {
          kind: "message",
          id: item.message.id,
          role: item.message.role,
          content: item.message.content.slice(0, 200),
          sequenceNumber: item.message.sequenceNumber,
          tokenCount: item.message.tokenCount,
        };
      }
      return {
        kind: "summary",
        id: item.summary.id,
        level: item.summary.level,
        content: item.summary.content.slice(0, 200),
        tokenCount: item.summary.tokenCount,
        sourceMessageCount: item.summary.sourceMessageIds.length,
      };
    });
  }

  res.writeHead(200, { "Content-Type": "application/json" });
  res.end(JSON.stringify({ ...snap, contextItems }));
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

export interface DashboardOptions {
  port?: number;
  host?: string;
}

export function startDashboard(opts?: DashboardOptions): ReturnType<typeof createServer> {
  const port = opts?.port ?? parseInt(process.env.DASHBOARD_PORT ?? "3333", 10);
  const host = opts?.host ?? "127.0.0.1";

  const dashboardHtml = loadDashboardHtml();

  const server = createServer(async (req, res) => {
    // CORS for local dev
    res.setHeader("Access-Control-Allow-Origin", "*");
    res.setHeader("Access-Control-Allow-Methods", "GET, OPTIONS");
    res.setHeader("Access-Control-Allow-Headers", "Content-Type");

    if (req.method === "OPTIONS") {
      res.writeHead(204);
      res.end();
      return;
    }

    const url = new URL(req.url!, `http://${req.headers.host}`);

    try {
      switch (url.pathname) {
        case "/":
          res.writeHead(200, { "Content-Type": "text/html" });
          res.end(dashboardHtml);
          break;
        case "/api/overview":
          await handleOverview(req, res);
          break;
        case "/api/session":
          await handleSession(req, res);
          break;
        default:
          res.writeHead(404, { "Content-Type": "application/json" });
          res.end(JSON.stringify({ error: "Not found" }));
      }
    } catch (err) {
      console.error("Dashboard request error:", err);
      res.writeHead(500, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: "Internal error" }));
    }
  });

  server.listen(port, host, () => {
    console.error(`🥓 bacon-lcm dashboard: http://${host}:${port}`);
  });

  return server;
}

// ---------------------------------------------------------------------------
// CLI entry point
// ---------------------------------------------------------------------------

const isCli =
  process.argv[1] &&
  (process.argv[1].endsWith("dashboard/server.js") ||
   process.argv[1].endsWith("dashboard/server.ts"));

if (isCli) {
  startDashboard();
}
