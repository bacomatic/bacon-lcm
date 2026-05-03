/**
 * Hooks — public API surface
 */
export type { HookEvent, HookResult } from "./handler.js";
export { handleHookEvent, resetSession } from "./handler.js";
export { parseWindsurfHook } from "./windsurf.js";
export { parseCopilotHook, type CopilotHookType } from "./copilot.js";
