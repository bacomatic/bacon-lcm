/**
 * Unified Hook Handler
 *
 * Platform-agnostic logic for capturing messages from agent hooks into
 * the LCM immutable store.  Platform adapters (Windsurf, Copilot CLI)
 * normalize their input into the common HookEvent type and call this handler.
 */
import {
  DEFAULT_COMPACTION_CONFIG,
  EchoSummarizer,
  NaiveTokenCounter,
} from "../defaults.js";
import { LcmSession } from "../session.js";
import type { CompactionConfig, MessageRole } from "../types.js";

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
// Persistent session (file-backed in the future; in-memory for now)
// ---------------------------------------------------------------------------

let session: LcmSession | null = null;

function getSession(): LcmSession {
  if (!session) {
    const config: CompactionConfig = { ...DEFAULT_COMPACTION_CONFIG };
    session = new LcmSession(new NaiveTokenCounter(), new EchoSummarizer(), config);
  }
  return session;
}

export function resetSession(): void {
  session = null;
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
  const s = getSession();

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
      return {
        stored: false,
        session_id: getSession().session.id,
        active_tokens: 0,
        compacted: false,
      };
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
