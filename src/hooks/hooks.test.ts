import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { handleHookEvent, resetSession, shutdownPool, type HookEvent } from "./handler.js";
import { parseWindsurfHook } from "./windsurf.js";
import { parseCopilotHook } from "./copilot.js";
import { resetConfig } from "../config.js";

let savedDbUrl: string | undefined;

beforeEach(() => {
  savedDbUrl = process.env.DATABASE_URL;
  delete process.env.DATABASE_URL;
  resetConfig();
  resetSession();
});

afterEach(async () => {
  await shutdownPool();
  if (savedDbUrl !== undefined) process.env.DATABASE_URL = savedDbUrl;
  else delete process.env.DATABASE_URL;
});

// ---------------------------------------------------------------------------
// Unified handler
// ---------------------------------------------------------------------------

describe("handleHookEvent", () => {
  it("stores a user prompt event", async () => {
    const event: HookEvent = {
      platform: "windsurf",
      kind: "user_prompt",
      content: "Hello, world!",
      timestamp: new Date(),
      raw: {},
    };

    const result = await handleHookEvent(event);
    expect(result.stored).toBe(true);
    expect(result.message_id).toBeDefined();
    expect(result.active_tokens).toBeGreaterThan(0);
  });

  it("stores an assistant response event", async () => {
    const event: HookEvent = {
      platform: "copilot-cli",
      kind: "assistant_response",
      content: "Here is my response",
      timestamp: new Date(),
      raw: {},
    };

    const result = await handleHookEvent(event);
    expect(result.stored).toBe(true);
  });

  it("resets session on session_start", async () => {
    // Store something first
    await handleHookEvent({
      platform: "copilot-cli",
      kind: "user_prompt",
      content: "first session msg",
      timestamp: new Date(),
      raw: {},
    });

    const startResult = await handleHookEvent({
      platform: "copilot-cli",
      kind: "session_start",
      content: "new session",
      timestamp: new Date(),
      raw: {},
    });

    expect(startResult.stored).toBe(false);
    expect(startResult.active_tokens).toBe(0);
  });

  it("does not store events without content", async () => {
    const result = await handleHookEvent({
      platform: "windsurf",
      kind: "user_prompt",
      content: undefined,
      timestamp: new Date(),
      raw: {},
    });

    expect(result.stored).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Windsurf adapter
// ---------------------------------------------------------------------------

describe("parseWindsurfHook", () => {
  it("parses pre_user_prompt", () => {
    const event = parseWindsurfHook({
      agent_action_name: "pre_user_prompt",
      timestamp: "2025-01-01T00:00:00Z",
      tool_info: { user_prompt: "Hello!" },
    });

    expect(event.platform).toBe("windsurf");
    expect(event.kind).toBe("user_prompt");
    expect(event.content).toBe("Hello!");
  });

  it("parses post_cascade_response", () => {
    const event = parseWindsurfHook({
      agent_action_name: "post_cascade_response",
      tool_info: { response: "I created the file." },
    });

    expect(event.kind).toBe("assistant_response");
    expect(event.content).toBe("I created the file.");
  });

  it("parses post_cascade_response_with_transcript", () => {
    const event = parseWindsurfHook({
      agent_action_name: "post_cascade_response_with_transcript",
      tool_info: { transcript_path: "/tmp/transcript.jsonl" },
    });

    expect(event.kind).toBe("transcript");
    expect(event.content).toContain("/tmp/transcript.jsonl");
  });

  it("parses tool-related hooks", () => {
    const event = parseWindsurfHook({
      agent_action_name: "post_write_code",
      tool_info: { file_path: "/foo/bar.ts", edits: [] },
    });

    expect(event.kind).toBe("tool_use");
    expect(event.content).toContain("post_write_code");
  });
});

// ---------------------------------------------------------------------------
// Copilot CLI adapter
// ---------------------------------------------------------------------------

describe("parseCopilotHook", () => {
  it("parses sessionStart", () => {
    const event = parseCopilotHook("sessionStart", {
      timestamp: 1704614400000,
      cwd: "/path/to/project",
      source: "new",
      initialPrompt: "Create a feature",
    });

    expect(event.platform).toBe("copilot-cli");
    expect(event.kind).toBe("session_start");
    expect(event.content).toBe("Create a feature");
  });

  it("parses sessionEnd", () => {
    const event = parseCopilotHook("sessionEnd", {
      timestamp: 1704618000000,
      cwd: "/path/to/project",
      reason: "complete",
    });

    expect(event.kind).toBe("session_end");
    expect(event.content).toContain("complete");
  });

  it("parses userPromptSubmitted", () => {
    const event = parseCopilotHook("userPromptSubmitted", {
      timestamp: 1704614500000,
      cwd: "/path/to/project",
      prompt: "Fix the auth bug",
    });

    expect(event.kind).toBe("user_prompt");
    expect(event.content).toBe("Fix the auth bug");
  });

  it("parses preToolUse", () => {
    const event = parseCopilotHook("preToolUse", {
      timestamp: 1704614500000,
      cwd: "/path/to/project",
      toolName: "edit_file",
    });

    expect(event.kind).toBe("tool_use");
    expect(event.content).toContain("edit_file");
  });

  it("parses errorOccurred", () => {
    const event = parseCopilotHook("errorOccurred", {
      timestamp: 1704614500000,
      cwd: "/path/to/project",
      error: "Something went wrong",
    });

    expect(event.kind).toBe("tool_use");
    expect(event.content).toContain("Something went wrong");
  });
});
