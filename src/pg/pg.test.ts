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
import { NaiveTokenCounter } from "../defaults.js";
import { newSessionId } from "../ids.js";
import type { MessageId, SessionId, SummaryId } from "../types.js";

const DATABASE_URL =
  process.env.DATABASE_URL ?? "postgres://localhost:5432/bacon_lcm_test";

// Use a unique schema name per test run to allow parallel CI
const schemaName = `test_${Date.now()}`;

let pool: pg.Pool;
let store: PgMessageStore;
let dag: PgSummaryDag;
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
    await pool.query(`SET search_path TO ${schemaName}`);

    store = new PgMessageStore(pool, tokenCounter);
    dag = new PgSummaryDag(pool, tokenCounter);

    await store.migrate();
    await dag.migrate();
  });

  afterAll(async () => {
    await pool.query(`DROP SCHEMA IF EXISTS ${schemaName} CASCADE`);
    await pool.end();
  });

  // -- PgMessageStore --------------------------------------------------------

  describe("PgMessageStore", () => {
    const sessionId = newSessionId();

    it("appends and retrieves a message", async () => {
      const msg = await store.appendAsync(sessionId, "user", "Hello from Postgres");

      expect(msg.id).toBeDefined();
      expect(msg.sequenceNumber).toBe(1);
      expect(msg.role).toBe("user");
      expect(msg.tokenCount).toBeGreaterThan(0);

      const fetched = await store.getAsync(msg.id);
      expect(fetched).toBeDefined();
      expect(fetched!.content).toBe("Hello from Postgres");
    });

    it("auto-increments sequence numbers", async () => {
      const msg2 = await store.appendAsync(sessionId, "assistant", "Hi there!");
      expect(msg2.sequenceNumber).toBe(2);

      const msg3 = await store.appendAsync(sessionId, "user", "Follow-up question");
      expect(msg3.sequenceNumber).toBe(3);
    });

    it("retrieves by session", async () => {
      const msgs = await store.getBySessionAsync(sessionId);
      expect(msgs.length).toBe(3);
      expect(msgs[0].sequenceNumber).toBe(1);
      expect(msgs[2].sequenceNumber).toBe(3);
    });

    it("retrieves a range", async () => {
      const range = await store.getRangeAsync(sessionId, 2, 3);
      expect(range.length).toBe(2);
      expect(range[0].sequenceNumber).toBe(2);
    });

    it("retrieves many by IDs", async () => {
      const all = await store.getBySessionAsync(sessionId);
      const ids = [all[0].id, all[2].id];
      const result = await store.getManyAsync(ids);
      expect(result.length).toBe(2);
    });

    it("counts total messages", async () => {
      const count = await store.sizeAsync();
      expect(count).toBeGreaterThanOrEqual(3);
    });
  });

  // -- PgSummaryDag ----------------------------------------------------------

  describe("PgSummaryDag", () => {
    const sessionId = newSessionId();

    it("adds and retrieves a summary node", async () => {
      const node = await dag.addAsync(
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

      const fetched = await dag.getAsync(node.id);
      expect(fetched).toBeDefined();
      expect(fetched!.content).toBe("Summary of first few messages");
    });

    it("lists active nodes", async () => {
      const active = await dag.getActiveAsync(sessionId);
      expect(active.length).toBe(1);
    });

    it("archives a node", async () => {
      const active = await dag.getActiveAsync(sessionId);
      await dag.archiveAsync(active[0].id);

      const stillActive = await dag.getActiveAsync(sessionId);
      expect(stillActive.length).toBe(0);

      const archived = await dag.getArchivedAsync(sessionId);
      expect(archived.length).toBe(1);
    });

    it("expands to message IDs via recursive CTE", async () => {
      const leaf = await dag.addAsync(
        sessionId,
        "leaf",
        "Leaf summary",
        ["msg-10" as MessageId, "msg-11" as MessageId],
        [],
      );

      const condensed = await dag.addAsync(
        sessionId,
        "condensed",
        "Condensed summary",
        ["msg-12" as MessageId],
        [leaf.id],
      );

      const msgIds = await dag.expandToMessageIdsAsync(condensed.id);
      expect(msgIds).toContain("msg-10");
      expect(msgIds).toContain("msg-11");
      expect(msgIds).toContain("msg-12");
      expect(msgIds.length).toBe(3);
    });

    it("counts total nodes", async () => {
      const count = await dag.sizeAsync();
      expect(count).toBeGreaterThanOrEqual(3);
    });
  });
});
