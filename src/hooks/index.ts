/**
 * Hooks — public API surface
 */
export type { HookEvent, HookResult } from "./handler.js";
export { handleHookEvent, resetSession, shutdownPool } from "./handler.js";
export { parseWindsurfHook } from "./windsurf.js";
export { parseCopilotHook, type CopilotHookType } from "./copilot.js";
export { startDaemon, DEFAULT_SOCKET_PATH } from "./daemon.js";
export { daemonRequest, isDaemonRunning, ensureDaemon } from "./daemon-client.js";
