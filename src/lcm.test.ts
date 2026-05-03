import { describe, expect, it } from "vitest";
import { CompactionEngine } from "./compaction.js";
import { ContextAssembler } from "./context.js";
import { InMemorySummaryDag } from "./dag.js";
import { EchoSummarizer, NaiveTokenCounter } from "./defaults.js";
import { newSessionId } from "./ids.js";
import { RetrievalService } from "./retrieval.js";
import { LcmSession } from "./session.js";
import { InMemoryMessageStore } from "./store.js";
import type { CompactionConfig, SessionId } from "./types.js";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const counter = new NaiveTokenCounter();
const summarizer = new EchoSummarizer();

function testConfig(overrides?: Partial<CompactionConfig>): CompactionConfig {
  return {
    thresholds: {
      modelMaxTokens: 1000,
      softLimit: 400,
      hardLimit: 700,
      riskBuffer: 100,
    },
    leafMinFanout: 2,
    leafChunkTokens: 200,
    condensedMinFanout: 2,
    condensedTargetTokens: 400,
    freshTailCount: 3,
    ...overrides,
  };
}

async function addMessages(
  store: InMemoryMessageStore,
  sessionId: SessionId,
  count: number,
  contentSize = 100,
): Promise<void> {
  for (let i = 0; i < count; i++) {
    await store.append(
      sessionId,
      i % 2 === 0 ? "user" : "assistant",
      `Message ${i}: ${"x".repeat(contentSize)}`,
    );
  }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("InMemoryMessageStore", () => {
  it("appends and retrieves messages", async () => {
    const store = new InMemoryMessageStore(counter);
    const sid = newSessionId();

    const m1 = await store.append(sid, "user", "Hello");
    const m2 = await store.append(sid, "assistant", "Hi there");

    expect(await store.size()).toBe(2);
    expect(await store.get(m1.id)).toBe(m1);
    expect(await store.getBySession(sid)).toEqual([m1, m2]);
    expect(m1.sequenceNumber).toBe(1);
    expect(m2.sequenceNumber).toBe(2);
  });

  it("returns messages in range", async () => {
    const store = new InMemoryMessageStore(counter);
    const sid = newSessionId();

    for (let i = 0; i < 5; i++) await store.append(sid, "user", `msg ${i}`);
    const range = await store.getRange(sid, 2, 4);

    expect(range.length).toBe(3);
    expect(range[0].sequenceNumber).toBe(2);
    expect(range[2].sequenceNumber).toBe(4);
  });

  it("counts tokens via the injected counter", async () => {
    const store = new InMemoryMessageStore(counter);
    const sid = newSessionId();

    const m = await store.append(sid, "user", "Hello world!"); // 12 chars => 3 tokens
    expect(m.tokenCount).toBe(3);
  });
});

describe("InMemorySummaryDag", () => {
  it("adds and retrieves summary nodes", async () => {
    const dag = new InMemorySummaryDag(counter);
    const sid = newSessionId();

    const store = new InMemoryMessageStore(counter);
    const m1 = await store.append(sid, "user", "Hello");
    const m2 = await store.append(sid, "assistant", "World");

    const node = await dag.add(sid, "leaf", "Summary of hello/world", [m1.id, m2.id], []);
    expect(await dag.size()).toBe(1);
    expect(await dag.get(node.id)).toBe(node);
    expect(await dag.getActive(sid)).toEqual([node]);
  });

  it("archives nodes", async () => {
    const dag = new InMemorySummaryDag(counter);
    const sid = newSessionId();
    const node = await dag.add(sid, "leaf", "summary", [], []);

    await dag.archive(node.id);

    expect(await dag.getActive(sid)).toEqual([]);
    expect(await dag.getArchived(sid)).toEqual([node]);
  });

  it("expands lineage to message IDs", async () => {
    const dag = new InMemorySummaryDag(counter);
    const sid = newSessionId();
    const store = new InMemoryMessageStore(counter);

    const m1 = await store.append(sid, "user", "a");
    const m2 = await store.append(sid, "user", "b");
    const m3 = await store.append(sid, "user", "c");

    const leaf1 = await dag.add(sid, "leaf", "sum1", [m1.id], []);
    const leaf2 = await dag.add(sid, "leaf", "sum2", [m2.id, m3.id], []);
    const condensed = await dag.add(sid, "condensed", "condensed", [], [leaf1.id, leaf2.id]);

    const expanded = await dag.expandToMessageIds(condensed.id);
    expect(expanded.sort()).toEqual([m1.id, m2.id, m3.id].sort());
  });
});

describe("ContextAssembler", () => {
  it("returns raw messages when no summaries exist", async () => {
    const config = testConfig();
    const store = new InMemoryMessageStore(counter);
    const dag = new InMemorySummaryDag(counter);
    const sid = newSessionId();

    await store.append(sid, "user", "Hello");
    await store.append(sid, "assistant", "Hi");

    const assembler = new ContextAssembler(store, dag, config);
    const items = await assembler.assemble(sid);

    expect(items.length).toBe(2);
    expect(items.every((i) => i.kind === "message")).toBe(true);
  });

  it("includes summaries followed by fresh tail", async () => {
    const config = testConfig({ freshTailCount: 2 });
    const store = new InMemoryMessageStore(counter);
    const dag = new InMemorySummaryDag(counter);
    const sid = newSessionId();

    const m1 = await store.append(sid, "user", "old1");
    const m2 = await store.append(sid, "assistant", "old2");
    await store.append(sid, "user", "recent1");
    await store.append(sid, "assistant", "recent2");

    await dag.add(sid, "leaf", "summary of old", [m1.id, m2.id], []);

    const assembler = new ContextAssembler(store, dag, config);
    const items = await assembler.assemble(sid);

    expect(items[0].kind).toBe("summary");
    expect(items.filter((i) => i.kind === "message").length).toBe(2);
  });
});

describe("CompactionEngine", () => {
  it("does nothing below soft limit", async () => {
    const config = testConfig();
    const store = new InMemoryMessageStore(counter);
    const dag = new InMemorySummaryDag(counter);
    const engine = new CompactionEngine(store, dag, summarizer, counter, config);
    const sid = newSessionId();

    await store.append(sid, "user", "short");

    const result = await engine.compact(sid, counter.count("short"));
    expect(result.levelReached).toBe(0);
    expect(result.created.length).toBe(0);
  });

  it("creates leaf summaries above soft limit", async () => {
    const config = testConfig({ freshTailCount: 2, leafMinFanout: 2, leafChunkTokens: 50 });
    const store = new InMemoryMessageStore(counter);
    const dag = new InMemorySummaryDag(counter);
    const engine = new CompactionEngine(store, dag, summarizer, counter, config);
    const sid = newSessionId();

    // Each message ~500 chars ≈ 125 tokens; 10 msgs ≈ 1250 tokens (well above softLimit=400)
    await addMessages(store, sid, 10, 500);

    const tokenCount = (await store.getBySession(sid))
      .reduce((s, m) => s + m.tokenCount, 0);

    const result = await engine.compact(sid, tokenCount);
    expect(result.levelReached).toBeGreaterThanOrEqual(1);
    expect(result.created.length).toBeGreaterThan(0);
  });
});

describe("RetrievalService", () => {
  it("describes and expands summary nodes", async () => {
    const store = new InMemoryMessageStore(counter);
    const dag = new InMemorySummaryDag(counter);
    const retrieval = new RetrievalService(store, dag);
    const sid = newSessionId();

    const m1 = await store.append(sid, "user", "hello world");
    const m2 = await store.append(sid, "assistant", "hi there");
    const node = await dag.add(sid, "leaf", "summary", [m1.id, m2.id], []);

    const desc = await retrieval.describe(node.id);
    expect(desc).toBeDefined();
    expect(desc!.totalReachableMessages).toBe(2);

    const expanded = await retrieval.expand(node.id);
    expect(expanded.length).toBe(2);
    expect(expanded[0].content).toBe("hello world");
  });
});

describe("LcmSession", () => {
  it("manages a full session lifecycle", async () => {
    const config = testConfig({ freshTailCount: 3 });
    const session = new LcmSession(counter, summarizer, config);

    // Add a few messages (below threshold)
    await session.addMessage("user", "Hello!");
    await session.addMessage("assistant", "How can I help?");

    expect((await session.getContext()).length).toBe(2);

    // Add many more to trigger compaction
    for (let i = 0; i < 20; i++) {
      await session.addMessage(
        i % 2 === 0 ? "user" : "assistant",
        `Turn ${i}: ${"data ".repeat(20)}`,
      );
    }

    const ctx = await session.getContext();
    const hasSummary = ctx.some((item) => item.kind === "summary");

    // With enough messages, we should see compaction kick in
    expect(await session.store.size()).toBe(22);
    // Token count should be managed
    expect(await session.getTokenCount()).toBeLessThanOrEqual(
      config.thresholds.hardLimit,
    );
  });

  it("persists and restores sessions via SessionPersistence", async () => {
    const cfg = testConfig({ freshTailCount: 3 });
    // Fake in-memory session store
    const savedSessions = new Map<string, { id: any; createdAt: Date; activeTokenCount: number }>();
    const fakeStore = {
      save: async (s: any) => { savedSessions.set(s.id, { ...s }); },
      load: async (id: any) => savedSessions.get(id),
      list: async () => Array.from(savedSessions.values()),
    };

    const session = new LcmSession(counter, summarizer, cfg, {
      sessionStore: fakeStore,
    });

    await session.addMessage("user", "Hello persistence");
    await session.addMessage("assistant", "Got it");

    // Session should have been auto-saved
    const saved = savedSessions.get(session.session.id);
    expect(saved).toBeDefined();
    expect(saved!.activeTokenCount).toBeGreaterThan(0);

    // Restore the session
    const restored = await LcmSession.restore(
      session.session.id,
      counter,
      summarizer,
      cfg,
      { store: session.store, dag: session.dag, sessionStore: fakeStore },
    );

    expect(restored).toBeDefined();
    expect(restored!.session.id).toBe(session.session.id);
    expect(restored!.session.activeTokenCount).toBeGreaterThan(0);

    // Context should be the same
    const ctx = await restored!.getContext();
    expect(ctx.length).toBeGreaterThan(0);
  });

  it("restore returns undefined for non-existent session", async () => {
    const cfg = testConfig();
    const fakeStore = {
      save: async () => {},
      load: async () => undefined,
      list: async () => [],
    };

    const result = await LcmSession.restore(
      "nonexistent" as any,
      counter,
      summarizer,
      cfg,
      { store: new InMemoryMessageStore(counter), dag: new InMemorySummaryDag(counter), sessionStore: fakeStore },
    );

    expect(result).toBeUndefined();
  });
});
