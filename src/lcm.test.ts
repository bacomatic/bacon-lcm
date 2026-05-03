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

function addMessages(
  store: InMemoryMessageStore,
  sessionId: SessionId,
  count: number,
  contentSize = 100,
): void {
  for (let i = 0; i < count; i++) {
    store.append(
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
  it("appends and retrieves messages", () => {
    const store = new InMemoryMessageStore(counter);
    const sid = newSessionId();

    const m1 = store.append(sid, "user", "Hello");
    const m2 = store.append(sid, "assistant", "Hi there");

    expect(store.size()).toBe(2);
    expect(store.get(m1.id)).toBe(m1);
    expect(store.getBySession(sid)).toEqual([m1, m2]);
    expect(m1.sequenceNumber).toBe(1);
    expect(m2.sequenceNumber).toBe(2);
  });

  it("returns messages in range", () => {
    const store = new InMemoryMessageStore(counter);
    const sid = newSessionId();

    for (let i = 0; i < 5; i++) store.append(sid, "user", `msg ${i}`);
    const range = store.getRange(sid, 2, 4);

    expect(range.length).toBe(3);
    expect(range[0].sequenceNumber).toBe(2);
    expect(range[2].sequenceNumber).toBe(4);
  });

  it("counts tokens via the injected counter", () => {
    const store = new InMemoryMessageStore(counter);
    const sid = newSessionId();

    const m = store.append(sid, "user", "Hello world!"); // 12 chars => 3 tokens
    expect(m.tokenCount).toBe(3);
  });
});

describe("InMemorySummaryDag", () => {
  it("adds and retrieves summary nodes", () => {
    const dag = new InMemorySummaryDag(counter);
    const sid = newSessionId();

    const store = new InMemoryMessageStore(counter);
    const m1 = store.append(sid, "user", "Hello");
    const m2 = store.append(sid, "assistant", "World");

    const node = dag.add(sid, "leaf", "Summary of hello/world", [m1.id, m2.id], []);
    expect(dag.size()).toBe(1);
    expect(dag.get(node.id)).toBe(node);
    expect(dag.getActive(sid)).toEqual([node]);
  });

  it("archives nodes", () => {
    const dag = new InMemorySummaryDag(counter);
    const sid = newSessionId();
    const node = dag.add(sid, "leaf", "summary", [], []);

    dag.archive(node.id);

    expect(dag.getActive(sid)).toEqual([]);
    expect(dag.getArchived(sid)).toEqual([node]);
  });

  it("expands lineage to message IDs", () => {
    const dag = new InMemorySummaryDag(counter);
    const sid = newSessionId();
    const store = new InMemoryMessageStore(counter);

    const m1 = store.append(sid, "user", "a");
    const m2 = store.append(sid, "user", "b");
    const m3 = store.append(sid, "user", "c");

    const leaf1 = dag.add(sid, "leaf", "sum1", [m1.id], []);
    const leaf2 = dag.add(sid, "leaf", "sum2", [m2.id, m3.id], []);
    const condensed = dag.add(sid, "condensed", "condensed", [], [leaf1.id, leaf2.id]);

    const expanded = dag.expandToMessageIds(condensed.id);
    expect(expanded.sort()).toEqual([m1.id, m2.id, m3.id].sort());
  });
});

describe("ContextAssembler", () => {
  it("returns raw messages when no summaries exist", () => {
    const config = testConfig();
    const store = new InMemoryMessageStore(counter);
    const dag = new InMemorySummaryDag(counter);
    const sid = newSessionId();

    store.append(sid, "user", "Hello");
    store.append(sid, "assistant", "Hi");

    const assembler = new ContextAssembler(store, dag, config);
    const items = assembler.assemble(sid);

    expect(items.length).toBe(2);
    expect(items.every((i) => i.kind === "message")).toBe(true);
  });

  it("includes summaries followed by fresh tail", () => {
    const config = testConfig({ freshTailCount: 2 });
    const store = new InMemoryMessageStore(counter);
    const dag = new InMemorySummaryDag(counter);
    const sid = newSessionId();

    const m1 = store.append(sid, "user", "old1");
    const m2 = store.append(sid, "assistant", "old2");
    store.append(sid, "user", "recent1");
    store.append(sid, "assistant", "recent2");

    dag.add(sid, "leaf", "summary of old", [m1.id, m2.id], []);

    const assembler = new ContextAssembler(store, dag, config);
    const items = assembler.assemble(sid);

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

    store.append(sid, "user", "short");

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
    addMessages(store, sid, 10, 500);

    const tokenCount = store
      .getBySession(sid)
      .reduce((s, m) => s + m.tokenCount, 0);

    const result = await engine.compact(sid, tokenCount);
    expect(result.levelReached).toBeGreaterThanOrEqual(1);
    expect(result.created.length).toBeGreaterThan(0);
  });
});

describe("RetrievalService", () => {
  it("describes and expands summary nodes", () => {
    const store = new InMemoryMessageStore(counter);
    const dag = new InMemorySummaryDag(counter);
    const retrieval = new RetrievalService(store, dag);
    const sid = newSessionId();

    const m1 = store.append(sid, "user", "hello world");
    const m2 = store.append(sid, "assistant", "hi there");
    const node = dag.add(sid, "leaf", "summary", [m1.id, m2.id], []);

    const desc = retrieval.describe(node.id);
    expect(desc).toBeDefined();
    expect(desc!.totalReachableMessages).toBe(2);

    const expanded = retrieval.expand(node.id);
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

    expect(session.getContext().length).toBe(2);

    // Add many more to trigger compaction
    for (let i = 0; i < 20; i++) {
      await session.addMessage(
        i % 2 === 0 ? "user" : "assistant",
        `Turn ${i}: ${"data ".repeat(20)}`,
      );
    }

    const ctx = session.getContext();
    const hasSummary = ctx.some((item) => item.kind === "summary");

    // With enough messages, we should see compaction kick in
    expect(session.store.size()).toBe(22);
    // Token count should be managed
    expect(session.getTokenCount()).toBeLessThanOrEqual(
      config.thresholds.hardLimit,
    );
  });
});
