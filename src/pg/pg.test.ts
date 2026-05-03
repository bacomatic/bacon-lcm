/**
 * PostgreSQL integration tests.
 *
 * These tests require a running Postgres instance. Set the DATABASE_URL
 * environment variable to connect (defaults to localhost:5432/bacon_lcm_test).
 *
 * Run:  DATABASE_URL=postgres://localhost:5432/bacon_lcm_test npm test -- src/pg/pg.test.ts
 *
 * The tests use a unique schema per run to avoid collisions and clean up after.
 */
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import pg from "pg";
import { PgMessageStore } from "./pg-store.js";
import { PgSummaryDag } from "./pg-dag.js";
import { PgSessionStore } from "./pg-session.js";
import { PgVectorStore } from "./pg-vectors.js";
import { NaiveTokenCounter } from "../defaults.js";
import { newSessionId } from "../ids.js";
import type { Embedder, MessageId, SessionId, SummaryId } from "../types.js";

const DATABASE_URL =
  process.env.DATABASE_URL ?? "postgres://localhost:5432/bacon_lcm_test";

// Use a unique schema name per test run to allow parallel CI
const schemaName = `test_${Date.now()}`;

let pool: pg.Pool;
let store: PgMessageStore;
let dag: PgSummaryDag;
let sessionStore: PgSessionStore;
const tokenCounter = new NaiveTokenCounter();

const canConnect = async (): Promise<boolean> => {
  const p = new pg.Pool({ connectionString: DATABASE_URL, max: 1 });
  try {
    const client = await p.connect();
    client.release();
    await p.end();
    return true;
  } catch {
    await p.end();
    return false;
  }
};

// Skip if no database is available
const hasDb = await canConnect();

describe.skipIf(!hasDb)("PostgreSQL persistence", () => {
  beforeAll(async () => {
    pool = new pg.Pool({ connectionString: DATABASE_URL });
    await pool.query(`CREATE SCHEMA IF NOT EXISTS ${schemaName}`);
    await pool.query(`SET search_path TO ${schemaName}, public`);

    store = new PgMessageStore(pool, tokenCounter);
    dag = new PgSummaryDag(pool, tokenCounter);
    sessionStore = new PgSessionStore(pool);

    await store.migrate();
    await dag.migrate();
    await sessionStore.migrate();
  });

  afterAll(async () => {
    await pool.query(`DROP SCHEMA IF EXISTS ${schemaName} CASCADE`);
    await pool.end();
  });

  // -- PgMessageStore --------------------------------------------------------

  describe("PgMessageStore", () => {
    const sessionId = newSessionId();

    it("appends and retrieves a message", async () => {
      const msg = await store.append(sessionId, "user", "Hello from Postgres");

      expect(msg.id).toBeDefined();
      expect(msg.sequenceNumber).toBe(1);
      expect(msg.role).toBe("user");
      expect(msg.tokenCount).toBeGreaterThan(0);

      const fetched = await store.get(msg.id);
      expect(fetched).toBeDefined();
      expect(fetched!.content).toBe("Hello from Postgres");
    });

    it("auto-increments sequence numbers", async () => {
      const msg2 = await store.append(sessionId, "assistant", "Hi there!");
      expect(msg2.sequenceNumber).toBe(2);

      const msg3 = await store.append(sessionId, "user", "Follow-up question");
      expect(msg3.sequenceNumber).toBe(3);
    });

    it("retrieves by session", async () => {
      const msgs = await store.getBySession(sessionId);
      expect(msgs.length).toBe(3);
      expect(msgs[0].sequenceNumber).toBe(1);
      expect(msgs[2].sequenceNumber).toBe(3);
    });

    it("retrieves a range", async () => {
      const range = await store.getRange(sessionId, 2, 3);
      expect(range.length).toBe(2);
      expect(range[0].sequenceNumber).toBe(2);
    });

    it("retrieves many by IDs", async () => {
      const all = await store.getBySession(sessionId);
      const ids = [all[0].id, all[2].id];
      const result = await store.getMany(ids);
      expect(result.length).toBe(2);
    });

    it("counts total messages", async () => {
      const count = await store.size();
      expect(count).toBeGreaterThanOrEqual(3);
    });
  });

  // -- PgSummaryDag ----------------------------------------------------------

  describe("PgSummaryDag", () => {
    const sessionId = newSessionId();

    it("adds and retrieves a summary node", async () => {
      const node = await dag.add(
        sessionId,
        "leaf",
        "Summary of first few messages",
        ["msg-1" as MessageId, "msg-2" as MessageId],
        [],
      );

      expect(node.id).toBeDefined();
      expect(node.level).toBe("leaf");
      expect(node.isActive).toBe(true);
      expect(node.isArchived).toBe(false);

      const fetched = await dag.get(node.id);
      expect(fetched).toBeDefined();
      expect(fetched!.content).toBe("Summary of first few messages");
    });

    it("lists active nodes", async () => {
      const active = await dag.getActive(sessionId);
      expect(active.length).toBe(1);
    });

    it("archives a node", async () => {
      const active = await dag.getActive(sessionId);
      await dag.archive(active[0].id);

      const stillActive = await dag.getActive(sessionId);
      expect(stillActive.length).toBe(0);

      const archived = await dag.getArchived(sessionId);
      expect(archived.length).toBe(1);
    });

    it("expands to message IDs via recursive CTE", async () => {
      const leaf = await dag.add(
        sessionId,
        "leaf",
        "Leaf summary",
        ["msg-10" as MessageId, "msg-11" as MessageId],
        [],
      );

      const condensed = await dag.add(
        sessionId,
        "condensed",
        "Condensed summary",
        ["msg-12" as MessageId],
        [leaf.id],
      );

      const msgIds = await dag.expandToMessageIds(condensed.id);
      expect(msgIds).toContain("msg-10");
      expect(msgIds).toContain("msg-11");
      expect(msgIds).toContain("msg-12");
      expect(msgIds.length).toBe(3);
    });

    it("counts total nodes", async () => {
      const count = await dag.size();
      expect(count).toBeGreaterThanOrEqual(3);
    });
  });

  // -- PgSessionStore ----------------------------------------------------------

  describe("PgSessionStore", () => {
    const sessionId = newSessionId();
    const createdAt = new Date("2025-01-15T10:00:00Z");

    it("saves a session", async () => {
      await sessionStore.save({ id: sessionId, createdAt, activeTokenCount: 500 });
      const loaded = await sessionStore.load(sessionId);
      expect(loaded).toBeDefined();
      expect(loaded!.id).toBe(sessionId);
      expect(loaded!.createdAt.toISOString()).toBe(createdAt.toISOString());
      expect(loaded!.activeTokenCount).toBe(500);
    });

    it("updates active_token_count on re-save", async () => {
      await sessionStore.save({ id: sessionId, createdAt, activeTokenCount: 1200 });
      const loaded = await sessionStore.load(sessionId);
      expect(loaded!.activeTokenCount).toBe(1200);
    });

    it("lists sessions ordered by created_at desc", async () => {
      const id2 = newSessionId();
      await sessionStore.save({ id: id2, createdAt: new Date("2025-06-01T00:00:00Z"), activeTokenCount: 0 });

      const rows = await sessionStore.list();
      expect(rows.length).toBeGreaterThanOrEqual(2);
      // Most recent first
      expect(rows[0].id).toBe(id2);
    });

    it("returns undefined for non-existent session", async () => {
      const loaded = await sessionStore.load("non-existent" as SessionId);
      expect(loaded).toBeUndefined();
    });

    it("deletes a session", async () => {
      await sessionStore.delete(sessionId);
      const loaded = await sessionStore.load(sessionId);
      expect(loaded).toBeUndefined();
    });
  });

  // -- PgVectorStore -----------------------------------------------------------

  describe("PgVectorStore", () => {
    // Deterministic fake embedder: embeds text as a 3-dimensional vector based on char codes
    const fakeEmbedder: Embedder = {
      dimensions: 3,
      async embed(text: string): Promise<number[]> {
        const c = text.charCodeAt(0) || 0;
        return [c / 255, (c * 2) / 255, (c * 3) / 255];
      },
      async embedBatch(texts: string[]): Promise<number[][]> {
        return Promise.all(texts.map((t) => this.embed(t)));
      },
    };

    let vectorStore: PgVectorStore;
    const sid = newSessionId();

    it("migrates the vector table", async () => {
      vectorStore = new PgVectorStore(pool, fakeEmbedder);
      await vectorStore.migrate();
      // If no error, migration succeeded
    });

    it("stores an embedding", async () => {
      await vectorStore.store("emb-1", sid, "message", "msg-100", "Hello world");
      const count = await vectorStore.size();
      expect(count).toBeGreaterThanOrEqual(1);
    });

    it("stores another embedding", async () => {
      await vectorStore.store("emb-2", sid, "message", "msg-101", "Goodbye world");
      const count = await vectorStore.size();
      expect(count).toBeGreaterThanOrEqual(2);
    });

    it("upserts on duplicate source", async () => {
      await vectorStore.store("emb-1-v2", sid, "message", "msg-100", "Updated hello");
      const count = await vectorStore.size();
      // Should still be 2 due to UPSERT on (source_type, source_id)
      expect(count).toBe(2);
    });

    it("searches by similarity", async () => {
      const results = await vectorStore.search("Hello", { sessionId: sid, limit: 5 });
      expect(results.length).toBeGreaterThan(0);
      expect(results[0].similarity).toBeGreaterThan(0);
      expect(results[0].sourceType).toBe("message");
    });

    it("filters by source type", async () => {
      await vectorStore.store("emb-3", sid, "summary", "sum-1", "Summary of conversation");
      const results = await vectorStore.search("Summary", { sessionId: sid, sourceType: "summary" });
      expect(results.every((r) => r.sourceType === "summary")).toBe(true);
    });

    it("respects limit", async () => {
      const results = await vectorStore.search("text", { limit: 1 });
      expect(results.length).toBeLessThanOrEqual(1);
    });
  });
});
