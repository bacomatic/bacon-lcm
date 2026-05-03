/**
 * GitHub Copilot CLI Hooks Adapter
 *
 * Parses the JSON that Copilot CLI pipes to hook scripts via stdin and
 * converts it to a common HookEvent for the unified handler.
 *
 * Copilot CLI hook events:
 *   sessionStart          → session_start
 *   sessionEnd            → session_end
 *   userPromptSubmitted   → user_prompt
 *   preToolUse            → tool_use
 *   postToolUse           → tool_use
 *   errorOccurred         → (logged but not stored)
 */
import type { HookEvent } from "./handler.js";

// ---------------------------------------------------------------------------
// Copilot CLI input shapes
// ---------------------------------------------------------------------------

interface CopilotSessionStartInput {
  timestamp: number;
  cwd: string;
  source: "new" | "resume" | "startup";
  initialPrompt?: string;
}

interface CopilotSessionEndInput {
  timestamp: number;
  cwd: string;
  reason: "complete" | "error" | "abort" | "timeout" | "user_exit";
}

interface CopilotPromptInput {
  timestamp: number;
  cwd: string;
  prompt: string;
}

interface CopilotToolInput {
  timestamp: number;
  cwd: string;
  toolName?: string;
  [key: string]: unknown;
}

interface CopilotErrorInput {
  timestamp: number;
  cwd: string;
  error: string;
  [key: string]: unknown;
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

export type CopilotHookType =
  | "sessionStart"
  | "sessionEnd"
  | "userPromptSubmitted"
  | "preToolUse"
  | "postToolUse"
  | "errorOccurred";

export function parseCopilotHook(
  hookType: CopilotHookType,
  input: Record<string, unknown>,
): HookEvent {
  const ts = typeof input.timestamp === "number"
    ? new Date(input.timestamp)
    : new Date();

  switch (hookType) {
    case "sessionStart": {
      const data = input as unknown as CopilotSessionStartInput;
      return {
        platform: "copilot-cli",
        kind: "session_start",
        content: data.initialPrompt ?? `[Session started: source=${data.source}]`,
        timestamp: ts,
        raw: input,
      };
    }

    case "sessionEnd": {
      const data = input as unknown as CopilotSessionEndInput;
      return {
        platform: "copilot-cli",
        kind: "session_end",
        content: `[Session ended: reason=${data.reason}]`,
        timestamp: ts,
        raw: input,
      };
    }

    case "userPromptSubmitted": {
      const data = input as unknown as CopilotPromptInput;
      return {
        platform: "copilot-cli",
        kind: "user_prompt",
        content: data.prompt,
        timestamp: ts,
        raw: input,
      };
    }

    case "preToolUse":
    case "postToolUse": {
      const data = input as unknown as CopilotToolInput;
      return {
        platform: "copilot-cli",
        kind: "tool_use",
        content: `[${hookType}: ${data.toolName ?? "unknown"}] ${JSON.stringify(input)}`,
        timestamp: ts,
        raw: input,
      };
    }

    case "errorOccurred": {
      const data = input as unknown as CopilotErrorInput;
      return {
        platform: "copilot-cli",
        kind: "tool_use",
        content: `[error] ${data.error}`,
        timestamp: ts,
        raw: input,
      };
    }

    default:
      return {
        platform: "copilot-cli",
        kind: "tool_use",
        content: JSON.stringify(input),
        timestamp: ts,
        raw: input,
      };
  }
}
