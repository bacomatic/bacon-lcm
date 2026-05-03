/**
 * Windsurf Cascade Hooks Adapter
 *
 * Parses the JSON that Windsurf pipes to hook scripts via stdin and
 * converts it to a common HookEvent for the unified handler.
 *
 * Windsurf hook events:
 *   pre_user_prompt                       → user_prompt
 *   post_cascade_response                 → assistant_response
 *   post_cascade_response_with_transcript → transcript
 */
import type { HookEvent } from "./handler.js";

// ---------------------------------------------------------------------------
// Windsurf input shapes
// ---------------------------------------------------------------------------

interface WindsurfHookInput {
  agent_action_name: string;
  trajectory_id?: string;
  execution_id?: string;
  timestamp?: string;
  model_name?: string;
  tool_info: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

export function parseWindsurfHook(input: WindsurfHookInput): HookEvent {
  const actionName = input.agent_action_name;
  const ts = input.timestamp ? new Date(input.timestamp) : new Date();

  switch (actionName) {
    case "pre_user_prompt":
      return {
        platform: "windsurf",
        kind: "user_prompt",
        content: input.tool_info.user_prompt as string | undefined,
        timestamp: ts,
        raw: input,
      };

    case "post_cascade_response":
      return {
        platform: "windsurf",
        kind: "assistant_response",
        content: input.tool_info.response as string | undefined,
        timestamp: ts,
        raw: input,
      };

    case "post_cascade_response_with_transcript":
      return {
        platform: "windsurf",
        kind: "transcript",
        content: input.tool_info.transcript_path
          ? `[Transcript saved to ${input.tool_info.transcript_path}]`
          : undefined,
        timestamp: ts,
        raw: input,
      };

    case "pre_write_code":
    case "post_write_code":
    case "pre_read_code":
    case "post_read_code":
    case "pre_run_command":
    case "post_run_command":
    case "pre_mcp_tool_use":
    case "post_mcp_tool_use": {
      const toolContent = JSON.stringify(input.tool_info);
      return {
        platform: "windsurf",
        kind: "tool_use",
        content: `[${actionName}] ${toolContent}`,
        timestamp: ts,
        raw: input,
      };
    }

    default:
      return {
        platform: "windsurf",
        kind: "tool_use",
        content: `[${actionName}] ${JSON.stringify(input.tool_info)}`,
        timestamp: ts,
        raw: input,
      };
  }
}
