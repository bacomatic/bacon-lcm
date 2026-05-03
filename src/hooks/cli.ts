#!/usr/bin/env node
/**
 * bacon-lcm hook CLI
 *
 * Unified entry point for Windsurf and Copilot CLI hooks.
 * Reads JSON from stdin, detects the platform, and persists the event
 * to the LCM store.
 *
 * Tries the daemon first (fast path — no Postgres connect per call).
 * Falls back to direct handler if daemon is unreachable.
 *
 * Usage:
 *   echo '{"agent_action_name":"pre_user_prompt",...}' | bacon-lcm-hook --platform windsurf
 *   echo '{"timestamp":123,...}'                       | bacon-lcm-hook --platform copilot --hook userPromptSubmitted
 *   bacon-lcm-hook --no-daemon --platform copilot ...   # skip daemon, always direct
 */
import { handleHookEvent, shutdownPool } from "./handler.js";
import { parseWindsurfHook } from "./windsurf.js";
import { parseCopilotHook, type CopilotHookType } from "./copilot.js";
import { ensureDaemon, daemonRequest } from "./daemon-client.js";

async function readStdin(): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of process.stdin) {
    chunks.push(chunk as Buffer);
  }
  return Buffer.concat(chunks).toString("utf-8");
}

async function main() {
  const args = process.argv.slice(2);
  const platformIdx = args.indexOf("--platform");
  const platform = platformIdx >= 0 ? args[platformIdx + 1] : "auto";

  const hookIdx = args.indexOf("--hook");
  const hookType = hookIdx >= 0 ? args[hookIdx + 1] : undefined;

  const noDaemon = args.includes("--no-daemon");

  let raw: string;
  try {
    raw = await readStdin();
  } catch {
    process.stderr.write("bacon-lcm-hook: failed to read stdin\n");
    process.exit(1);
  }

  if (!raw.trim()) {
    process.stderr.write("bacon-lcm-hook: empty stdin\n");
    process.exit(0);
  }

  let input: Record<string, unknown>;
  try {
    input = JSON.parse(raw);
  } catch {
    process.stderr.write("bacon-lcm-hook: invalid JSON on stdin\n");
    process.exit(1);
  }

  // Detect platform
  const detectedPlatform =
    platform !== "auto"
      ? platform
      : "agent_action_name" in input
        ? "windsurf"
        : "copilot";

  // --- Fast path: try daemon ---
  if (!noDaemon) {
    const daemonAvailable = await ensureDaemon();
    if (daemonAvailable) {
      const resp = await daemonRequest({
        action: "hook",
        platform: detectedPlatform,
        hookType,
        payload: input,
      });

      if (resp && resp.ok) {
        process.stdout.write(JSON.stringify(resp.result) + "\n");
        return;
      }
      if (resp && !resp.ok) {
        process.stderr.write(`bacon-lcm-hook: daemon error: ${resp.error}\n`);
        // Fall through to direct mode
      }
      // resp === null means daemon went away, fall through
    }
  }

  // --- Fallback: direct handler (opens its own Postgres connection) ---
  try {
    let event;
    if (detectedPlatform === "windsurf") {
      event = parseWindsurfHook(input as any);
    } else if (detectedPlatform === "copilot") {
      if (!hookType) {
        process.stderr.write(
          "bacon-lcm-hook: --hook <hookType> is required for Copilot CLI\n",
        );
        process.exit(1);
      }
      event = parseCopilotHook(hookType as CopilotHookType, input);
    } else {
      process.stderr.write(
        `bacon-lcm-hook: unknown platform '${detectedPlatform}'\n`,
      );
      process.exit(1);
    }

    const result = await handleHookEvent(event);

    // Output result as JSON to stdout (some hook systems inspect output)
    process.stdout.write(JSON.stringify(result) + "\n");
  } catch (err) {
    process.stderr.write(`bacon-lcm-hook: error: ${err}\n`);
    await shutdownPool();
    process.exit(1);
  }

  await shutdownPool();
}

main();
