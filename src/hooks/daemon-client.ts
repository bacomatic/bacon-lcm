/**
 * Daemon client — thin helper for communicating with the bacon-lcm daemon
 * over its Unix domain socket.
 *
 * Usage:
 *   const resp = await daemonRequest({ action: "hook", platform: "copilot", ... });
 *   if (resp) { /* daemon handled it *\/ } else { /* daemon not available *\/ }
 */
import net from "node:net";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { DEFAULT_SOCKET_PATH } from "./daemon.js";

interface DaemonResponse {
  ok: boolean;
  result?: unknown;
  error?: string;
  uptime?: number;
}

/**
 * Send a request to the daemon and return the response.
 * Returns null if the daemon is not reachable.
 */
export async function daemonRequest(
  req: Record<string, unknown>,
  socketPath: string = DEFAULT_SOCKET_PATH,
  timeoutMs: number = 5000,
): Promise<DaemonResponse | null> {
  return new Promise((resolve) => {
    const conn = net.createConnection(socketPath, () => {
      conn.write(JSON.stringify(req) + "\n");
    });

    let buffer = "";
    const timer = setTimeout(() => {
      conn.destroy();
      resolve(null);
    }, timeoutMs);

    conn.on("data", (chunk) => {
      buffer += chunk.toString();
      const newlineIdx = buffer.indexOf("\n");
      if (newlineIdx !== -1) {
        clearTimeout(timer);
        const line = buffer.slice(0, newlineIdx).trim();
        conn.end();
        try {
          resolve(JSON.parse(line));
        } catch {
          resolve(null);
        }
      }
    });

    conn.on("error", () => {
      clearTimeout(timer);
      resolve(null);
    });
  });
}

/**
 * Check if the daemon is running by sending a ping.
 */
export async function isDaemonRunning(
  socketPath: string = DEFAULT_SOCKET_PATH,
): Promise<boolean> {
  const resp = await daemonRequest({ action: "ping" }, socketPath, 2000);
  return resp !== null && resp.ok === true;
}

/**
 * Auto-start the daemon in the background if it is not already running.
 * Returns true if daemon is available (already running or just started).
 */
export async function ensureDaemon(
  socketPath: string = DEFAULT_SOCKET_PATH,
): Promise<boolean> {
  // Already running?
  if (await isDaemonRunning(socketPath)) return true;

  // Find the daemon script (sibling to this file in dist/)
  const daemonScript = path.join(path.dirname(new URL(import.meta.url).pathname), "daemon.js");
  if (!fs.existsSync(daemonScript)) {
    return false;
  }

  // Spawn detached daemon
  const child = spawn("node", [daemonScript, "--socket", socketPath], {
    detached: true,
    stdio: "ignore",
    env: { ...process.env },
  });
  child.unref();

  // Wait a bit for it to start listening
  for (let i = 0; i < 10; i++) {
    await new Promise((r) => setTimeout(r, 200));
    if (await isDaemonRunning(socketPath)) return true;
  }

  return false;
}
