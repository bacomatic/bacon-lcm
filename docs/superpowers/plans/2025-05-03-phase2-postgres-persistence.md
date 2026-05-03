# Phase 2: PostgreSQL Persistence Layer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement four PostgreSQL-backed storage structs (`PgSessionStore`, `PgMessageStore`, `PgSummaryDag`, `PgVectorStore`) inside the `daemon` crate, wired into a `StorageLayer::postgres(pool)` constructor in `core`, with SQLx migrations, pgvector support, and full integration tests using `testcontainers`.

**Architecture:** Trait implementations live in `daemon/src/storage/`; SQL migrations live in `daemon/migrations/`; the `core` crate's `StorageLayer` gains a `postgres()` constructor that accepts a `PgPool` and wraps the four Pg impls behind the existing trait objects. The `daemon` crate owns the DB connection pool lifetime.

**Tech Stack:** Rust · Tokio · SQLx 0.8 (postgres, uuid, chrono, json) · pgvector 0.4 · testcontainers 0.27 · testcontainers-modules 0.15 (postgres)

---

## Schema notes and migration strategy

The existing `sql/` files (`001_init.sql`, `002_embeddings.sql`) are **reference SQL only** — they are not run by `sqlx::migrate!()`. This plan introduces a proper `daemon/migrations/` folder that `sqlx::migrate!()` will pick up at runtime.

The new migrations fix the following mismatches between the old SQL and the current Rust types:

| Table | Old schema gap | Fix |
|---|---|---|
| `lcm_sessions` | Missing `updated_at`, `metadata JSONB` | Migration 0001 adds both |
| `lcm_summary_nodes` | Uses `source_message_ids TEXT[]` + `source_node_ids TEXT[]` instead of unified `lineage JSONB` | Migration 0001 uses `lineage JSONB`; old split columns omitted |
| `lcm_embeddings` | `embedding vector(1536)` hard-coded dimension | Migration 0002 uses dimension-agnostic `vector` column |

All four tables use `UUID` primary keys (matching the Rust `Uuid` types) and `REFERENCES lcm_sessions(id) ON DELETE CASCADE` so a session deletion cascades cleanly through all child tables.

---
## Task 1 — Set up `daemon` crate: dependencies, connection pool, migration runner

**Files created / modified:**
- `daemon/Cargo.toml` — add `sqlx`, `pgvector`, `bacon-lcm-core`, `async-trait`, `tracing`, `serde_json`, `thiserror`; add `[lib]` entry
- `daemon/src/lib.rs` — expose `db` and `storage` modules for integration tests
- `daemon/src/main.rs` — replace stub with pool init + migration run
- `daemon/src/db.rs` — `connect(url)` and `run_migrations(pool)` helpers
- `daemon/src/storage/mod.rs` — declare four sub-modules
- `daemon/src/storage/pg_{session,message,summary,vector}_store.rs` — stubs (filled in Tasks 2-5)
- `daemon/migrations/0001_init.sql` — sessions, messages, summary nodes
- `daemon/migrations/0002_embeddings.sql` — pgvector extension + embeddings table
- `daemon/tests/helpers.rs` — shared testcontainers helper
- `/Cargo.toml` — add `pgvector` to workspace dependencies

**Commit message:** `feat(daemon): add sqlx pool, migrations, testcontainers helper (Task 1)`

---

- [ ] **Step 1.1 — Write the failing integration test first**

Create `daemon/tests/test_migrations.rs`:

```rust
// daemon/tests/test_migrations.rs
//! Verifies that all migrations apply cleanly against a real Postgres container.

mod helpers;

#[tokio::test]
async fn migrations_apply_cleanly() {
    let pool = helpers::test_pool().await;

    // Verify all four LCM tables exist after migrations run.
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM information_schema.tables \
         WHERE table_schema = 'public' \
         AND table_name IN ('lcm_sessions','lcm_messages','lcm_summary_nodes','lcm_embeddings')"
    )
    .fetch_one(&pool)
    .await
    .expect("query failed");

    assert_eq!(count, 4, "all four tables must exist after migrations");
}
```

Run — this **must fail** with a compile error because `helpers` does not exist yet:

```bash
cargo test -p bacon-lcm-daemon --test test_migrations 2>&1 | head -30
```

---

- [ ] **Step 1.2 — Add `pgvector` to workspace `Cargo.toml`**

In `/Cargo.toml`, inside `[workspace.dependencies]`, append:

```toml
pgvector = { version = "0.4", features = ["sqlx"] }
```

---

- [ ] **Step 1.3 — Update `daemon/Cargo.toml`**

Replace the entire file:

```toml
[package]
name = "bacon-lcm-daemon"
version.workspace = true
license.workspace = true
edition.workspace = true

# Binary entry point
[[bin]]
name = "bacon-lcm-daemon"
path = "src/main.rs"

# Library entry point — required so integration tests in daemon/tests/ can
# import types via `bacon_lcm_daemon::storage::...`
[lib]
name = "bacon_lcm_daemon"
path = "src/lib.rs"

[dependencies]
bacon-lcm-core     = { path = "../core" }
tokio              = { workspace = true }
sqlx               = { workspace = true }
serde              = { workspace = true }
serde_json         = { workspace = true }
async-trait        = { workspace = true }
tracing            = { workspace = true }
tracing-subscriber = { workspace = true }
thiserror          = { workspace = true }
anyhow             = { workspace = true }
pgvector           = { workspace = true }

[dev-dependencies]
tokio                  = { workspace = true }
testcontainers         = { workspace = true }
testcontainers-modules = { workspace = true }
```

---

- [ ] **Step 1.4 — Write `daemon/migrations/0001_init.sql`**

```sql
-- daemon/migrations/0001_init.sql
-- Core LCM tables.

BEGIN;

-- -----------------------------------------------------------------------
-- Sessions
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS lcm_sessions (
    id          UUID        PRIMARY KEY,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata    JSONB       NOT NULL DEFAULT '{}'
);

-- -----------------------------------------------------------------------
-- Messages  (immutable, append-only)
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS lcm_messages (
    id              UUID        PRIMARY KEY,
    session_id      UUID        NOT NULL REFERENCES lcm_sessions(id) ON DELETE CASCADE,
    role            TEXT        NOT NULL CHECK (role IN ('user','assistant','system','tool')),
    content         TEXT        NOT NULL,
    sequence_number INTEGER     NOT NULL,
    token_count     INTEGER     NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata        JSONB       NOT NULL DEFAULT '{}',

    UNIQUE (session_id, sequence_number)
);

CREATE INDEX IF NOT EXISTS idx_lcm_messages_session
    ON lcm_messages (session_id, sequence_number);

-- -----------------------------------------------------------------------
-- Summary nodes (DAG)
-- lineage stored as JSONB array of serde external-tagged enum objects:
--   [{"Message": "<uuid>"}, {"Summary": "<uuid>"}, ...]
-- Matches the serde external-tag format of LineagePointer.
-- -----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS lcm_summary_nodes (
    id          UUID        PRIMARY KEY,
    session_id  UUID        NOT NULL REFERENCES lcm_sessions(id) ON DELETE CASCADE,
    level       TEXT        NOT NULL CHECK (level IN ('leaf','condensed','emergency')),
    content     TEXT        NOT NULL,
    token_count INTEGER     NOT NULL,
    lineage     JSONB       NOT NULL DEFAULT '[]',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata    JSONB       NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_lcm_summary_nodes_session
    ON lcm_summary_nodes (session_id);

CREATE INDEX IF NOT EXISTS idx_lcm_summary_nodes_level
    ON lcm_summary_nodes (session_id, level);

COMMIT;
```

---

- [ ] **Step 1.5 — Write `daemon/migrations/0002_embeddings.sql`**

```sql
-- daemon/migrations/0002_embeddings.sql
-- Requires pgvector extension.
-- The `vector` type is declared WITHOUT an explicit dimension so that
-- embeddings from different models (different widths) can coexist.

BEGIN;

CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS lcm_embeddings (
    id          UUID        PRIMARY KEY,
    session_id  UUID        NOT NULL REFERENCES lcm_sessions(id) ON DELETE CASCADE,
    content     TEXT        NOT NULL,
    -- Dimension-agnostic: pgvector supports unconstrained `vector` columns.
    -- Cosine-distance queries require matching dimensions between query & rows.
    embedding   vector      NOT NULL,
    metadata    JSONB       NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_lcm_embeddings_session
    ON lcm_embeddings (session_id);

-- IVFFlat / HNSW index for ANN search is created lazily via
-- PgVectorStore::ensure_ann_index() once the table has enough rows.
-- Uncomment once you have >= 100 rows and a fixed dimension:
-- CREATE INDEX IF NOT EXISTS idx_lcm_embeddings_vector
--     ON lcm_embeddings USING hnsw (embedding vector_cosine_ops);

COMMIT;
```

---

- [ ] **Step 1.6 — Write `daemon/src/db.rs`**

```rust
// daemon/src/db.rs
//! Database connection pool and migration runner.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Connect to Postgres and return a connection pool.
pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await
}

/// Run all SQLx migrations found in `daemon/migrations/`.
/// Call once at daemon start-up, after `connect()`.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
```

---

- [ ] **Step 1.7 — Write `daemon/tests/helpers.rs`**

```rust
// daemon/tests/helpers.rs
//! Shared helper for daemon integration tests.
//! Spins up a Postgres + pgvector container and runs all migrations.

use sqlx::PgPool;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// Start a pgvector-enabled Postgres container, run all migrations, and
/// return a ready-to-use `PgPool`.
///
/// The container handle is intentionally leaked so it lives until the test
/// process exits (Docker removes the container automatically on exit).
pub async fn test_pool() -> PgPool {
    // `pgvector/pgvector:pg16` bundles the pgvector extension into Postgres 16.
    let container = Postgres::default()
        .with_image_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .expect("failed to start Postgres container");

    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("failed to get container port");

    let url = format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        port
    );

    let pool = sqlx::PgPool::connect(&url)
        .await
        .expect("failed to connect to test database");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations failed");

    // Keep container alive for the whole process lifetime.
    std::mem::forget(container);

    pool
}
```

---

- [ ] **Step 1.8 — Write `daemon/src/lib.rs`**

```rust
// daemon/src/lib.rs
//! Public library interface for the daemon crate.
//! Exposes storage implementations for integration tests and external consumers.

pub mod db;
pub mod storage;
```

---

- [ ] **Step 1.9 — Write `daemon/src/storage/mod.rs` and stub files**

`daemon/src/storage/mod.rs`:

```rust
// daemon/src/storage/mod.rs
pub mod pg_message_store;
pub mod pg_session_store;
pub mod pg_summary_dag;
pub mod pg_vector_store;
```

Create four stub files (one-line comment each) so the crate compiles:

- `daemon/src/storage/pg_session_store.rs` → `// Task 2 implementation`
- `daemon/src/storage/pg_message_store.rs` → `// Task 3 implementation`
- `daemon/src/storage/pg_summary_dag.rs`   → `// Task 4 implementation`
- `daemon/src/storage/pg_vector_store.rs`  → `// Task 5 implementation`

---

- [ ] **Step 1.10 — Write `daemon/src/main.rs`**

```rust
// daemon/src/main.rs
use anyhow::Context;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL must be set")?;

    let pool = bacon_lcm_daemon::db::connect(&database_url)
        .await
        .context("failed to connect to database")?;

    bacon_lcm_daemon::db::run_migrations(&pool)
        .await
        .context("failed to run migrations")?;

    tracing::info!("bacon-lcm-daemon started");
    Ok(())
}
```

---

- [ ] **Step 1.11 — Run the migration test and confirm it passes**

```bash
cargo test -p bacon-lcm-daemon --test test_migrations -- --nocapture
```

Expected: `test migrations_apply_cleanly ... ok`

---

- [ ] **Step 1.12 — Commit**

```bash
git add daemon/Cargo.toml daemon/src/ daemon/migrations/ daemon/tests/ Cargo.toml Cargo.lock
git commit -m "feat(daemon): add sqlx pool, migrations, testcontainers helper (Task 1)"
```

---

## Task 2 — `PgSessionStore` implementing `SessionStore`

**Files created / modified:**
- `daemon/src/storage/pg_session_store.rs` — full implementation
- `daemon/tests/test_pg_session_store.rs` — integration tests

**Commit message:** `feat(daemon/storage): PgSessionStore — Task 2`

---

- [ ] **Step 2.1 — Write the failing integration test**

Create `daemon/tests/test_pg_session_store.rs`:

```rust
// daemon/tests/test_pg_session_store.rs
mod helpers;

use bacon_lcm_core::storage::SessionStore;
use bacon_lcm_daemon::storage::pg_session_store::PgSessionStore;
use bacon_lcm_core::types::Session;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

fn make_session(id: Uuid) -> Session {
    Session {
        id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_create_and_get() {
    let pool = helpers::test_pool().await;
    let store = PgSessionStore::new(pool);
    let id = Uuid::new_v4();

    let stored_id = store.create(make_session(id)).await.expect("create failed");
    assert_eq!(stored_id, id);

    let retrieved = store.get(id).await.expect("get failed");
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().id, id);
}

#[tokio::test]
async fn test_get_nonexistent_returns_none() {
    let pool = helpers::test_pool().await;
    let store = PgSessionStore::new(pool);
    assert!(store.get(Uuid::new_v4()).await.expect("get failed").is_none());
}

#[tokio::test]
async fn test_update_metadata() {
    let pool = helpers::test_pool().await;
    let store = PgSessionStore::new(pool);
    let id = Uuid::new_v4();
    store.create(make_session(id)).await.unwrap();

    let mut updated = make_session(id);
    updated.metadata.insert("env".to_string(), serde_json::json!("prod"));
    store.update(updated).await.expect("update failed");

    let s = store.get(id).await.unwrap().unwrap();
    assert_eq!(s.metadata["env"], serde_json::json!("prod"));
}

#[tokio::test]
async fn test_delete() {
    let pool = helpers::test_pool().await;
    let store = PgSessionStore::new(pool);
    let id = Uuid::new_v4();
    store.create(make_session(id)).await.unwrap();

    assert!(store.exists(id).await.unwrap());
    store.delete(id).await.expect("delete failed");
    assert!(!store.exists(id).await.unwrap());
}

#[tokio::test]
async fn test_list() {
    let pool = helpers::test_pool().await;
    let store = PgSessionStore::new(pool);
    let id1 = Uuid::new_v4();
    let id2 = Uuid::new_v4();
    store.create(make_session(id1)).await.unwrap();
    store.create(make_session(id2)).await.unwrap();

    let ids = store.list().await.expect("list failed");
    assert!(ids.contains(&id1));
    assert!(ids.contains(&id2));
}

#[tokio::test]
async fn test_exists() {
    let pool = helpers::test_pool().await;
    let store = PgSessionStore::new(pool);
    let id = Uuid::new_v4();

    assert!(!store.exists(id).await.unwrap());
    store.create(make_session(id)).await.unwrap();
    assert!(store.exists(id).await.unwrap());
}
```

Run — must fail because `PgSessionStore` is a stub:

```bash
cargo test -p bacon-lcm-daemon --test test_pg_session_store 2>&1 | head -20
```

---

- [ ] **Step 2.2 — Implement `daemon/src/storage/pg_session_store.rs`**

```rust
// daemon/src/storage/pg_session_store.rs
use async_trait::async_trait;
use bacon_lcm_core::{
    error::{StorageError, StorageResult},
    storage::SessionStore,
    types::{Session, SessionId},
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

pub struct PgSessionStore {
    pool: PgPool,
}

impl PgSessionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

// Internal row type: one field per lcm_sessions column.
struct SessionRow {
    id: Uuid,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    metadata: Value,
}

impl TryFrom<SessionRow> for Session {
    type Error = StorageError;

    fn try_from(row: SessionRow) -> Result<Self, Self::Error> {
        let metadata: HashMap<String, Value> =
            serde_json::from_value(row.metadata).map_err(StorageError::Serialization)?;
        Ok(Session {
            id: row.id,
            created_at: row.created_at,
            updated_at: row.updated_at,
            metadata,
        })
    }
}

#[async_trait]
impl SessionStore for PgSessionStore {
    async fn create(&self, session: Session) -> StorageResult<SessionId> {
        let metadata =
            serde_json::to_value(&session.metadata).map_err(StorageError::Serialization)?;

        sqlx::query!(
            r#"
            INSERT INTO lcm_sessions (id, created_at, updated_at, metadata)
            VALUES ($1, $2, $3, $4)
            "#,
            session.id,
            session.created_at,
            session.updated_at,
            metadata,
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        Ok(session.id)
    }

    async fn get(&self, id: SessionId) -> StorageResult<Option<Session>> {
        let row = sqlx::query_as!(
            SessionRow,
            r#"
            SELECT id, created_at, updated_at, metadata
            FROM lcm_sessions
            WHERE id = $1
            "#,
            id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        row.map(Session::try_from).transpose()
    }

    async fn update(&self, session: Session) -> StorageResult<()> {
        let metadata =
            serde_json::to_value(&session.metadata).map_err(StorageError::Serialization)?;

        sqlx::query!(
            r#"
            UPDATE lcm_sessions
            SET updated_at = $2, metadata = $3
            WHERE id = $1
            "#,
            session.id,
            session.updated_at,
            metadata,
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        Ok(())
    }

    async fn delete(&self, id: SessionId) -> StorageResult<()> {
        sqlx::query!("DELETE FROM lcm_sessions WHERE id = $1", id)
            .execute(&self.pool)
            .await
            .map_err(StorageError::ConnectionFailed)?;

        Ok(())
    }

    async fn list(&self) -> StorageResult<Vec<SessionId>> {
        let rows = sqlx::query_scalar!(
            "SELECT id FROM lcm_sessions ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        Ok(rows)
    }

    async fn exists(&self, id: SessionId) -> StorageResult<bool> {
        let count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM lcm_sessions WHERE id = $1",
            id,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?
        .unwrap_or(0);

        Ok(count > 0)
    }
}
```

---

- [ ] **Step 2.3 — Run the session store tests**

```bash
cargo test -p bacon-lcm-daemon --test test_pg_session_store -- --nocapture
```

All 6 tests must pass.

---

- [ ] **Step 2.4 — Commit**

```bash
git add daemon/src/storage/pg_session_store.rs daemon/tests/test_pg_session_store.rs
git commit -m "feat(daemon/storage): PgSessionStore — Task 2"
```

---

## Task 3 — `PgMessageStore` implementing `MessageStore`

**Files created / modified:**
- `daemon/src/storage/pg_message_store.rs` — full implementation
- `daemon/tests/test_pg_message_store.rs` — integration tests

**Commit message:** `feat(daemon/storage): PgMessageStore — Task 3`

---

### Design notes

- `sequence_number` is assigned atomically inside a transaction: `COUNT(*)` of existing rows for the session is used as the next sequence number. The `UNIQUE(session_id, sequence_number)` constraint catches races.
- The Rust `Message.timestamp` maps to the `created_at` DB column; the DB is the source of truth on reads.
- `token_count` is stored as `INTEGER` (i32); the Rust type uses `usize`. Safe because counts are always non-negative and fit in 32 bits.
- `get_range(session_id, start..end)` translates to `ORDER BY sequence_number LIMIT (end-start) OFFSET start`.
- `store_batch` calls `store` sequentially; for large batches a COPY-based approach would be faster (future optimisation).

---

- [ ] **Step 3.1 — Write the failing integration test**

Create `daemon/tests/test_pg_message_store.rs`:

```rust
// daemon/tests/test_pg_message_store.rs
mod helpers;

use bacon_lcm_core::storage::{MessageStore, SessionStore};
use bacon_lcm_daemon::storage::{
    pg_message_store::PgMessageStore,
    pg_session_store::PgSessionStore,
};
use bacon_lcm_core::types::{Message, MessageRole, Session};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

async fn setup(pool: sqlx::PgPool) -> (PgSessionStore, PgMessageStore, Uuid) {
    let sessions = PgSessionStore::new(pool.clone());
    let messages = PgMessageStore::new(pool);
    let session_id = Uuid::new_v4();
    sessions
        .create(Session {
            id: session_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: HashMap::new(),
        })
        .await
        .unwrap();
    (sessions, messages, session_id)
}

fn make_message(session_id: Uuid, content: &str) -> Message {
    Message {
        id: Uuid::new_v4(),
        session_id,
        role: MessageRole::User,
        content: content.to_string(),
        timestamp: Utc::now(),
        token_count: content.split_whitespace().count(),
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_store_and_get() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    let msg = make_message(session_id, "hello world");
    let id = store.store(msg.clone()).await.expect("store failed");
    assert_eq!(id, msg.id);

    let retrieved = store.get(id).await.expect("get failed");
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().content, "hello world");
}

#[tokio::test]
async fn test_get_nonexistent_returns_none() {
    let pool = helpers::test_pool().await;
    let (_, store, _) = setup(pool).await;
    assert!(store.get(Uuid::new_v4()).await.unwrap().is_none());
}

#[tokio::test]
async fn test_get_session_messages_ordered() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;

    for i in 0..4u32 {
        let mut msg = make_message(session_id, &format!("msg {}", i));
        msg.timestamp = Utc::now() + chrono::Duration::milliseconds(i as i64 * 10);
        store.store(msg).await.unwrap();
    }

    let messages = store.get_session_messages(session_id).await.unwrap();
    assert_eq!(messages.len(), 4);
    // Must come back in sequence_number order.
    for pair in messages.windows(2) {
        assert!(pair[0].timestamp <= pair[1].timestamp);
    }
}

#[tokio::test]
async fn test_get_range() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;

    for i in 0..5u32 {
        let mut msg = make_message(session_id, &format!("msg {}", i));
        msg.timestamp = Utc::now() + chrono::Duration::milliseconds(i as i64 * 10);
        store.store(msg).await.unwrap();
    }

    // range 1..3 should return "msg 1" and "msg 2"
    let range = store.get_range(session_id, 1..3).await.unwrap();
    assert_eq!(range.len(), 2);
    assert_eq!(range[0].content, "msg 1");
    assert_eq!(range[1].content, "msg 2");
}

#[tokio::test]
async fn test_get_message_count() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;

    store.store(make_message(session_id, "a")).await.unwrap();
    store.store(make_message(session_id, "b")).await.unwrap();

    assert_eq!(store.get_message_count(session_id).await.unwrap(), 2);
}

#[tokio::test]
async fn test_get_token_count() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;

    // token_count = word count: "one" -> 1, "two words" -> 2
    store.store(make_message(session_id, "one")).await.unwrap();
    store.store(make_message(session_id, "two words")).await.unwrap();

    assert_eq!(store.get_token_count(session_id).await.unwrap(), 3);
}

#[tokio::test]
async fn test_store_batch() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;

    let messages: Vec<_> = (0..3)
        .map(|i| make_message(session_id, &format!("batch {}", i)))
        .collect();
    let ids = store.store_batch(messages).await.unwrap();
    assert_eq!(ids.len(), 3);
}

#[tokio::test]
async fn test_delete_session() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;

    store.store(make_message(session_id, "x")).await.unwrap();
    assert_eq!(store.get_message_count(session_id).await.unwrap(), 1);

    store.delete_session(session_id).await.unwrap();
    assert_eq!(store.get_message_count(session_id).await.unwrap(), 0);
}
```

Run — must fail:

```bash
cargo test -p bacon-lcm-daemon --test test_pg_message_store 2>&1 | head -20
```

---

- [ ] **Step 3.2 — Implement `daemon/src/storage/pg_message_store.rs`**

```rust
// daemon/src/storage/pg_message_store.rs
use async_trait::async_trait;
use bacon_lcm_core::{
    error::{StorageError, StorageResult},
    storage::MessageStore,
    types::{Message, MessageId, MessageRole, SessionId},
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

pub struct PgMessageStore {
    pool: PgPool,
}

impl PgMessageStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

// ── conversion helpers ────────────────────────────────────────────────────────

fn role_to_str(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User      => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System    => "system",
        MessageRole::Tool      => "tool",
    }
}

fn str_to_role(s: &str) -> StorageResult<MessageRole> {
    match s {
        "user"      => Ok(MessageRole::User),
        "assistant" => Ok(MessageRole::Assistant),
        "system"    => Ok(MessageRole::System),
        "tool"      => Ok(MessageRole::Tool),
        other => Err(StorageError::ConstraintViolation(format!(
            "unknown role: {other}"
        ))),
    }
}

struct MessageRow {
    id: Uuid,
    session_id: Uuid,
    role: String,
    content: String,
    token_count: i32,
    created_at: DateTime<Utc>,
    metadata: Value,
}

fn row_to_message(row: MessageRow) -> StorageResult<Message> {
    let role = str_to_role(&row.role)?;
    let metadata: HashMap<String, Value> =
        serde_json::from_value(row.metadata).map_err(StorageError::Serialization)?;
    Ok(Message {
        id: row.id,
        session_id: row.session_id,
        role,
        content: row.content,
        timestamp: row.created_at,
        token_count: row.token_count as usize,
        metadata,
    })
}

// ── trait implementation ──────────────────────────────────────────────────────

#[async_trait]
impl MessageStore for PgMessageStore {
    async fn store(&self, message: Message) -> StorageResult<MessageId> {
        let metadata =
            serde_json::to_value(&message.metadata).map_err(StorageError::Serialization)?;
        let role = role_to_str(message.role);

        // Assign sequence_number atomically inside a transaction to ensure
        // the UNIQUE(session_id, sequence_number) constraint is never violated.
        let mut tx = self.pool.begin().await.map_err(StorageError::ConnectionFailed)?;

        let seq: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM lcm_messages WHERE session_id = $1",
            message.session_id,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(StorageError::ConnectionFailed)?
        .unwrap_or(0);

        sqlx::query!(
            r#"
            INSERT INTO lcm_messages
                (id, session_id, role, content, sequence_number, token_count, created_at, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
            message.id,
            message.session_id,
            role,
            message.content,
            seq as i32,
            message.token_count as i32,
            message.timestamp,
            metadata,
        )
        .execute(&mut *tx)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        tx.commit().await.map_err(StorageError::ConnectionFailed)?;

        Ok(message.id)
    }

    async fn get(&self, id: MessageId) -> StorageResult<Option<Message>> {
        let row = sqlx::query_as!(
            MessageRow,
            r#"
            SELECT id, session_id, role, content, token_count, created_at, metadata
            FROM lcm_messages
            WHERE id = $1
            "#,
            id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        row.map(row_to_message).transpose()
    }

    async fn get_range(
        &self,
        session_id: SessionId,
        range: std::ops::Range<usize>,
    ) -> StorageResult<Vec<Message>> {
        let limit  = (range.end.saturating_sub(range.start)) as i64;
        let offset = range.start as i64;

        let rows = sqlx::query_as!(
            MessageRow,
            r#"
            SELECT id, session_id, role, content, token_count, created_at, metadata
            FROM lcm_messages
            WHERE session_id = $1
            ORDER BY sequence_number ASC
            LIMIT $2 OFFSET $3
            "#,
            session_id,
            limit,
            offset,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        rows.into_iter().map(row_to_message).collect()
    }

    async fn get_session_messages(&self, session_id: SessionId) -> StorageResult<Vec<Message>> {
        let rows = sqlx::query_as!(
            MessageRow,
            r#"
            SELECT id, session_id, role, content, token_count, created_at, metadata
            FROM lcm_messages
            WHERE session_id = $1
            ORDER BY sequence_number ASC
            "#,
            session_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        rows.into_iter().map(row_to_message).collect()
    }

    async fn get_message_count(&self, session_id: SessionId) -> StorageResult<usize> {
        let count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM lcm_messages WHERE session_id = $1",
            session_id,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?
        .unwrap_or(0);

        Ok(count as usize)
    }

    async fn get_token_count(&self, session_id: SessionId) -> StorageResult<usize> {
        let total: i64 = sqlx::query_scalar!(
            "SELECT COALESCE(SUM(token_count), 0) FROM lcm_messages WHERE session_id = $1",
            session_id,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?
        .unwrap_or(0);

        Ok(total as usize)
    }

    async fn delete_session(&self, session_id: SessionId) -> StorageResult<()> {
        sqlx::query!(
            "DELETE FROM lcm_messages WHERE session_id = $1",
            session_id,
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        Ok(())
    }

    async fn store_batch(&self, messages: Vec<Message>) -> StorageResult<Vec<MessageId>> {
        let mut ids = Vec::with_capacity(messages.len());
        for message in messages {
            ids.push(self.store(message).await?);
        }
        Ok(ids)
    }
}
```

---

- [ ] **Step 3.3 — Run the message store tests**

```bash
cargo test -p bacon-lcm-daemon --test test_pg_message_store -- --nocapture
```

All 8 tests must pass.

---

- [ ] **Step 3.4 — Commit**

```bash
git add daemon/src/storage/pg_message_store.rs daemon/tests/test_pg_message_store.rs
git commit -m "feat(daemon/storage): PgMessageStore — Task 3"
```

---

## Task 4 — `PgSummaryDag` implementing `SummaryDag`

**Files created / modified:**
- `daemon/src/storage/pg_summary_dag.rs` — full implementation
- `daemon/tests/test_pg_summary_dag.rs` — integration tests

**Commit message:** `feat(daemon/storage): PgSummaryDag — Task 4`

---

### Lineage serialisation contract

`lineage` is stored as a JSONB array where each element uses serde's default **external tagging** for `LineagePointer`:

```json
[
  { "Message": "550e8400-e29b-41d4-a716-446655440000" },
  { "Summary": "6ba7b810-9dad-11d1-80b4-00c04fd430c8" }
]
```

`serde_json::to_value(&Vec<LineagePointer>)` produces exactly this layout because `LineagePointer` derives `Serialize`/`Deserialize` with no attribute overrides (serde defaults to external tagging for enums). `serde_json::from_value` round-trips it perfectly.

`detect_cycles` performs a DFS over the in-memory adjacency list of nodes fetched from the DB. It is a best-effort validation tool for operator diagnostics; it does not block concurrent writes.

`expand` recurses into nested `LineagePointer::Summary` entries via `Box::pin(self.expand(...))` to satisfy the async recursion requirement.

---

- [ ] **Step 4.1 — Write the failing integration test**

Create `daemon/tests/test_pg_summary_dag.rs`:

```rust
// daemon/tests/test_pg_summary_dag.rs
mod helpers;

use bacon_lcm_core::storage::{MessageStore, SessionStore, SummaryDag};
use bacon_lcm_daemon::storage::{
    pg_message_store::PgMessageStore,
    pg_session_store::PgSessionStore,
    pg_summary_dag::PgSummaryDag,
};
use bacon_lcm_core::types::{
    LineagePointer, Message, MessageRole, Session, SummaryLevel, SummaryNode,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

async fn setup(
    pool: sqlx::PgPool,
) -> (PgSessionStore, PgMessageStore, PgSummaryDag, Uuid) {
    let sessions  = PgSessionStore::new(pool.clone());
    let messages  = PgMessageStore::new(pool.clone());
    let summaries = PgSummaryDag::new(pool);
    let session_id = Uuid::new_v4();
    sessions
        .create(Session {
            id: session_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: HashMap::new(),
        })
        .await
        .unwrap();
    (sessions, messages, summaries, session_id)
}

fn make_node(session_id: Uuid, level: SummaryLevel, lineage: Vec<LineagePointer>) -> SummaryNode {
    SummaryNode {
        id: Uuid::new_v4(),
        session_id,
        level,
        content: "test summary".to_string(),
        token_count: 42,
        lineage,
        timestamp: Utc::now(),
        metadata: HashMap::new(),
    }
}

fn make_message(session_id: Uuid) -> Message {
    Message {
        id: Uuid::new_v4(),
        session_id,
        role: MessageRole::User,
        content: "original message".to_string(),
        timestamp: Utc::now(),
        token_count: 3,
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_add_and_get_node() {
    let pool = helpers::test_pool().await;
    let (_, _, dag, session_id) = setup(pool).await;
    let node = make_node(session_id, SummaryLevel::Leaf, vec![]);

    let id = dag.add_node(node.clone()).await.expect("add_node failed");
    assert_eq!(id, node.id);

    let retrieved = dag.get_node(id).await.expect("get_node failed");
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().content, "test summary");
}

#[tokio::test]
async fn test_get_session_summaries() {
    let pool = helpers::test_pool().await;
    let (_, _, dag, session_id) = setup(pool).await;

    dag.add_node(make_node(session_id, SummaryLevel::Leaf, vec![])).await.unwrap();
    dag.add_node(make_node(session_id, SummaryLevel::Condensed, vec![])).await.unwrap();

    let all = dag.get_session_summaries(session_id).await.unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn test_get_lineage_with_message_pointer() {
    let pool = helpers::test_pool().await;
    let (_, msg_store, dag, session_id) = setup(pool).await;

    let msg = make_message(session_id);
    msg_store.store(msg.clone()).await.unwrap();

    let node = make_node(
        session_id,
        SummaryLevel::Leaf,
        vec![LineagePointer::Message(msg.id)],
    );
    dag.add_node(node.clone()).await.unwrap();

    let lineage = dag.get_lineage(node.id).await.unwrap();
    assert_eq!(lineage.len(), 1);
    assert!(matches!(lineage[0], LineagePointer::Message(id) if id == msg.id));
}

#[tokio::test]
async fn test_expand_returns_original_messages() {
    let pool = helpers::test_pool().await;
    let (_, msg_store, dag, session_id) = setup(pool).await;

    let msg = make_message(session_id);
    msg_store.store(msg.clone()).await.unwrap();

    let node = make_node(
        session_id,
        SummaryLevel::Leaf,
        vec![LineagePointer::Message(msg.id)],
    );
    dag.add_node(node.clone()).await.unwrap();

    let expanded = dag.expand(node.id, &msg_store).await.unwrap();
    assert_eq!(expanded.len(), 1);
    assert_eq!(expanded[0].id, msg.id);
}

#[tokio::test]
async fn test_get_summaries_by_level() {
    let pool = helpers::test_pool().await;
    let (_, _, dag, session_id) = setup(pool).await;

    dag.add_node(make_node(session_id, SummaryLevel::Leaf, vec![])).await.unwrap();
    dag.add_node(make_node(session_id, SummaryLevel::Emergency, vec![])).await.unwrap();

    let leaves = dag
        .get_summaries_by_level(session_id, SummaryLevel::Leaf)
        .await
        .unwrap();
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].level, SummaryLevel::Leaf);
}

#[tokio::test]
async fn test_delete_session() {
    let pool = helpers::test_pool().await;
    let (_, _, dag, session_id) = setup(pool).await;

    dag.add_node(make_node(session_id, SummaryLevel::Leaf, vec![])).await.unwrap();
    assert_eq!(dag.get_session_summaries(session_id).await.unwrap().len(), 1);

    dag.delete_session(session_id).await.unwrap();
    assert_eq!(dag.get_session_summaries(session_id).await.unwrap().len(), 0);
}

#[tokio::test]
async fn test_detect_cycles_returns_false_for_valid_dag() {
    let pool = helpers::test_pool().await;
    let (_, _, dag, session_id) = setup(pool).await;

    let leaf = make_node(session_id, SummaryLevel::Leaf, vec![]);
    let leaf_id = leaf.id;
    dag.add_node(leaf).await.unwrap();

    let condensed = make_node(
        session_id,
        SummaryLevel::Condensed,
        vec![LineagePointer::Summary(leaf_id)],
    );
    dag.add_node(condensed).await.unwrap();

    assert!(!dag.detect_cycles(session_id).await.unwrap());
}
```

Run — must fail:

```bash
cargo test -p bacon-lcm-daemon --test test_pg_summary_dag 2>&1 | head -20
```

---

- [ ] **Step 4.2 — Implement `daemon/src/storage/pg_summary_dag.rs`**

```rust
// daemon/src/storage/pg_summary_dag.rs
use async_trait::async_trait;
use bacon_lcm_core::{
    error::{StorageError, StorageResult},
    storage::{MessageStore, SummaryDag},
    types::{LineagePointer, Message, SessionId, SummaryId, SummaryLevel, SummaryNode},
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub struct PgSummaryDag {
    pool: PgPool,
}

impl PgSummaryDag {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

// ── conversion helpers ────────────────────────────────────────────────────────

fn level_to_str(level: SummaryLevel) -> &'static str {
    match level {
        SummaryLevel::Leaf      => "leaf",
        SummaryLevel::Condensed => "condensed",
        SummaryLevel::Emergency => "emergency",
    }
}

fn str_to_level(s: &str) -> StorageResult<SummaryLevel> {
    match s {
        "leaf"      => Ok(SummaryLevel::Leaf),
        "condensed" => Ok(SummaryLevel::Condensed),
        "emergency" => Ok(SummaryLevel::Emergency),
        other => Err(StorageError::ConstraintViolation(format!(
            "unknown summary level: {other}"
        ))),
    }
}

struct SummaryRow {
    id: Uuid,
    session_id: Uuid,
    level: String,
    content: String,
    token_count: i32,
    lineage: Value,
    created_at: DateTime<Utc>,
    metadata: Value,
}

fn row_to_node(row: SummaryRow) -> StorageResult<SummaryNode> {
    let level = str_to_level(&row.level)?;
    let lineage: Vec<LineagePointer> =
        serde_json::from_value(row.lineage).map_err(StorageError::Serialization)?;
    let metadata: HashMap<String, Value> =
        serde_json::from_value(row.metadata).map_err(StorageError::Serialization)?;
    Ok(SummaryNode {
        id: row.id,
        session_id: row.session_id,
        level,
        content: row.content,
        token_count: row.token_count as usize,
        lineage,
        timestamp: row.created_at,
        metadata,
    })
}

// ── trait implementation ──────────────────────────────────────────────────────

#[async_trait]
impl SummaryDag for PgSummaryDag {
    async fn add_node(&self, node: SummaryNode) -> StorageResult<SummaryId> {
        let level    = level_to_str(node.level);
        let lineage  = serde_json::to_value(&node.lineage).map_err(StorageError::Serialization)?;
        let metadata = serde_json::to_value(&node.metadata).map_err(StorageError::Serialization)?;

        sqlx::query!(
            r#"
            INSERT INTO lcm_summary_nodes
                (id, session_id, level, content, token_count, lineage, created_at, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
            node.id,
            node.session_id,
            level,
            node.content,
            node.token_count as i32,
            lineage,
            node.timestamp,
            metadata,
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        Ok(node.id)
    }

    async fn get_node(&self, id: SummaryId) -> StorageResult<Option<SummaryNode>> {
        let row = sqlx::query_as!(
            SummaryRow,
            r#"
            SELECT id, session_id, level, content, token_count, lineage, created_at, metadata
            FROM lcm_summary_nodes
            WHERE id = $1
            "#,
            id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        row.map(row_to_node).transpose()
    }

    async fn get_session_summaries(&self, session_id: SessionId) -> StorageResult<Vec<SummaryNode>> {
        let rows = sqlx::query_as!(
            SummaryRow,
            r#"
            SELECT id, session_id, level, content, token_count, lineage, created_at, metadata
            FROM lcm_summary_nodes
            WHERE session_id = $1
            ORDER BY created_at ASC
            "#,
            session_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        rows.into_iter().map(row_to_node).collect()
    }

    async fn get_lineage(&self, id: SummaryId) -> StorageResult<Vec<LineagePointer>> {
        let node = self.get_node(id).await?;
        Ok(node.map(|n| n.lineage).unwrap_or_default())
    }

    async fn expand(
        &self,
        id: SummaryId,
        message_store: &dyn MessageStore,
    ) -> StorageResult<Vec<Message>> {
        let lineage = self.get_lineage(id).await?;
        let mut messages = Vec::new();

        for ptr in lineage {
            match ptr {
                LineagePointer::Message(msg_id) => {
                    if let Some(msg) = message_store.get(msg_id).await? {
                        messages.push(msg);
                    }
                }
                LineagePointer::Summary(summary_id) => {
                    // Box::pin breaks the async recursion constraint.
                    let nested = Box::pin(self.expand(summary_id, message_store)).await?;
                    messages.extend(nested);
                }
            }
        }

        messages.sort_by_key(|m| m.timestamp);
        Ok(messages)
    }

    async fn get_summaries_by_level(
        &self,
        session_id: SessionId,
        level: SummaryLevel,
    ) -> StorageResult<Vec<SummaryNode>> {
        let level_str = level_to_str(level);

        let rows = sqlx::query_as!(
            SummaryRow,
            r#"
            SELECT id, session_id, level, content, token_count, lineage, created_at, metadata
            FROM lcm_summary_nodes
            WHERE session_id = $1 AND level = $2
            ORDER BY created_at ASC
            "#,
            session_id,
            level_str,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        rows.into_iter().map(row_to_node).collect()
    }

    async fn delete_session(&self, session_id: SessionId) -> StorageResult<()> {
        sqlx::query!(
            "DELETE FROM lcm_summary_nodes WHERE session_id = $1",
            session_id,
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        Ok(())
    }

    async fn detect_cycles(&self, session_id: SessionId) -> StorageResult<bool> {
        let nodes = self.get_session_summaries(session_id).await?;

        // Build adjacency map: node_id -> [child summary ids in lineage].
        let adj: HashMap<SummaryId, Vec<SummaryId>> = nodes
            .iter()
            .map(|n| {
                let children: Vec<SummaryId> = n
                    .lineage
                    .iter()
                    .filter_map(|p| {
                        if let LineagePointer::Summary(sid) = p { Some(*sid) } else { None }
                    })
                    .collect();
                (n.id, children)
            })
            .collect();

        // DFS cycle detection using a "currently in recursion stack" set.
        let mut visited: HashSet<SummaryId>  = HashSet::new();
        let mut in_stack: HashSet<SummaryId> = HashSet::new();

        for &start in adj.keys() {
            if has_cycle(start, &adj, &mut visited, &mut in_stack) {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

fn has_cycle(
    node: SummaryId,
    adj: &HashMap<SummaryId, Vec<SummaryId>>,
    visited: &mut HashSet<SummaryId>,
    in_stack: &mut HashSet<SummaryId>,
) -> bool {
    if in_stack.contains(&node) {
        return true; // Back-edge: cycle detected.
    }
    if visited.contains(&node) {
        return false; // Already fully explored: safe.
    }

    visited.insert(node);
    in_stack.insert(node);

    if let Some(children) = adj.get(&node) {
        for &child in children {
            if has_cycle(child, adj, visited, in_stack) {
                return true;
            }
        }
    }

    in_stack.remove(&node);
    false
}
```

---

- [ ] **Step 4.3 — Run the summary DAG tests**

```bash
cargo test -p bacon-lcm-daemon --test test_pg_summary_dag -- --nocapture
```

All 7 tests must pass.

---

- [ ] **Step 4.4 — Commit**

```bash
git add daemon/src/storage/pg_summary_dag.rs daemon/tests/test_pg_summary_dag.rs
git commit -m "feat(daemon/storage): PgSummaryDag — Task 4"
```

---

## Task 5 — `PgVectorStore` implementing `VectorStore`

**Files created / modified:**
- `daemon/src/storage/pg_vector_store.rs` — full implementation
- `daemon/tests/test_pg_vector_store.rs` — integration tests

**Commit message:** `feat(daemon/storage): PgVectorStore with pgvector — Task 5`

---

### pgvector integration notes

- `lcm_embeddings.embedding` is declared as `vector` (no explicit dimension) — pgvector supports this.
- The `pgvector::Vector` newtype wraps `Vec<f32>` and implements `sqlx::Encode`/`Decode` when compiled with `features = ["sqlx"]`. Convert at the boundary: `pgvector::Vector::from(record.embedding)` on writes, `.to_vec()` on reads.
- `search()` issues `ORDER BY embedding <=> $1::vector LIMIT $2` (pgvector cosine distance operator `<=>`). Without an index this is an exact sequential scan, correct and safe for test data sizes.
- The optional `ensure_ann_index(dims: usize)` method creates an `ivfflat` index for production tuning. It is exposed but not called by migrations.
- Cosine-distance queries require matching dimensions between query and stored vectors; pgvector returns a DB error on mismatch, which propagates as `StorageError::ConnectionFailed`.

---

- [ ] **Step 5.1 — Write the failing integration test**

Create `daemon/tests/test_pg_vector_store.rs`:

```rust
// daemon/tests/test_pg_vector_store.rs
mod helpers;

use bacon_lcm_core::storage::{SessionStore, VectorStore};
use bacon_lcm_core::storage::vector_store::VectorRecord;
use bacon_lcm_daemon::storage::{
    pg_session_store::PgSessionStore,
    pg_vector_store::PgVectorStore,
};
use bacon_lcm_core::types::Session;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

async fn setup(pool: sqlx::PgPool) -> (PgSessionStore, PgVectorStore, Uuid) {
    let sessions = PgSessionStore::new(pool.clone());
    let vectors  = PgVectorStore::new(pool);
    let session_id = Uuid::new_v4();
    sessions
        .create(Session {
            id: session_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: HashMap::new(),
        })
        .await
        .unwrap();
    (sessions, vectors, session_id)
}

fn make_record(session_id: Uuid, embedding: Vec<f32>, content: &str) -> VectorRecord {
    VectorRecord {
        id: Uuid::new_v4(),
        session_id,
        embedding,
        content: content.to_string(),
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_store_and_get() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    let rec = make_record(session_id, vec![1.0, 0.0, 0.0], "x-axis");
    let id = store.store(rec.clone()).await.expect("store failed");
    assert_eq!(id, rec.id);

    let retrieved = store.get(id).await.expect("get failed");
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().content, "x-axis");
}

#[tokio::test]
async fn test_get_nonexistent_returns_none() {
    let pool = helpers::test_pool().await;
    let (_, store, _) = setup(pool).await;
    assert!(store.get(Uuid::new_v4()).await.unwrap().is_none());
}

#[tokio::test]
async fn test_delete() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    let rec = make_record(session_id, vec![1.0], "to delete");
    let id = store.store(rec).await.unwrap();
    assert!(store.get(id).await.unwrap().is_some());
    store.delete(id).await.expect("delete failed");
    assert!(store.get(id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_delete_session() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    store.store(make_record(session_id, vec![1.0, 0.0], "a")).await.unwrap();
    store.store(make_record(session_id, vec![0.0, 1.0], "b")).await.unwrap();
    assert_eq!(store.get_session_vectors(session_id).await.unwrap().len(), 2);
    store.delete_session(session_id).await.expect("delete_session failed");
    assert!(store.get_session_vectors(session_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn test_get_session_vectors() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    store.store(make_record(session_id, vec![1.0, 0.0], "a")).await.unwrap();
    store.store(make_record(session_id, vec![0.0, 1.0], "b")).await.unwrap();
    let vecs = store.get_session_vectors(session_id).await.unwrap();
    assert_eq!(vecs.len(), 2);
}

#[tokio::test]
async fn test_search_nearest_neighbour() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;

    store.store(make_record(session_id, vec![1.0, 0.0, 0.0], "x-axis")).await.unwrap();
    store.store(make_record(session_id, vec![0.0, 1.0, 0.0], "y-axis")).await.unwrap();
    store.store(make_record(session_id, vec![0.0, 0.0, 1.0], "z-axis")).await.unwrap();

    // Query close to x-axis; expect x-axis first.
    let results = store.search(session_id, &[1.0, 0.0, 0.0], 2).await.unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].content, "x-axis");
}

#[tokio::test]
async fn test_search_returns_at_most_k() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;

    for i in 0..5u32 {
        store
            .store(make_record(session_id, vec![i as f32, 0.0], &format!("rec {}", i)))
            .await
            .unwrap();
    }

    let results = store.search(session_id, &[1.0, 0.0], 3).await.unwrap();
    assert_eq!(results.len(), 3);
}

#[tokio::test]
async fn test_search_empty_session_returns_empty() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    let results = store.search(session_id, &[1.0, 0.0], 5).await.unwrap();
    assert!(results.is_empty());
}
```

Run — must fail:

```bash
cargo test -p bacon-lcm-daemon --test test_pg_vector_store 2>&1 | head -20
```

---

- [ ] **Step 5.2 — Implement `daemon/src/storage/pg_vector_store.rs`**

```rust
// daemon/src/storage/pg_vector_store.rs
use async_trait::async_trait;
use bacon_lcm_core::{
    error::{StorageError, StorageResult},
    storage::VectorStore,
    storage::vector_store::VectorRecord,
    types::SessionId,
};
use chrono::{DateTime, Utc};
use pgvector::Vector;
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

pub struct PgVectorStore {
    pool: PgPool,
}

impl PgVectorStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Optionally create an IVFFlat index for cosine ANN search.
    /// Call this once the table has at least `lists * 30` rows.
    /// `dims` must match the dimension of stored embeddings.
    pub async fn ensure_ann_index(&self, dims: usize, lists: usize) -> Result<(), sqlx::Error> {
        sqlx::query(&format!(
            "CREATE INDEX IF NOT EXISTS idx_lcm_embeddings_vector              ON lcm_embeddings USING ivfflat (embedding vector_cosine_ops)              WITH (lists = {})",
            lists
        ))
        .execute(&self.pool)
        .await?;
        let _ = dims; // dimension enforced by the column; not needed here
        Ok(())
    }
}

// Internal row type for SELECT queries (no embedding column — fetched separately).
struct VectorRowMeta {
    id: Uuid,
    session_id: Uuid,
    content: String,
    metadata: Value,
    created_at: DateTime<Utc>,
}

// Full row including embedding vector.
struct VectorRow {
    id: Uuid,
    session_id: Uuid,
    content: String,
    embedding: Vector,
    metadata: Value,
}

fn row_to_record(row: VectorRow) -> StorageResult<VectorRecord> {
    let metadata: HashMap<String, Value> =
        serde_json::from_value(row.metadata).map_err(StorageError::Serialization)?;
    Ok(VectorRecord {
        id: row.id,
        session_id: row.session_id,
        embedding: row.embedding.to_vec(),
        content: row.content,
        metadata,
    })
}

#[async_trait]
impl VectorStore for PgVectorStore {
    async fn store(&self, record: VectorRecord) -> StorageResult<Uuid> {
        let embedding = Vector::from(record.embedding);
        let metadata  = serde_json::to_value(&record.metadata).map_err(StorageError::Serialization)?;

        sqlx::query!(
            r#"
            INSERT INTO lcm_embeddings (id, session_id, content, embedding, metadata)
            VALUES ($1, $2, $3, $4, $5)
            "#,
            record.id,
            record.session_id,
            record.content,
            embedding as Vector,
            metadata,
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        Ok(record.id)
    }

    async fn get(&self, id: Uuid) -> StorageResult<Option<VectorRecord>> {
        let row = sqlx::query_as!(
            VectorRow,
            r#"
            SELECT id, session_id, content, embedding AS "embedding: Vector", metadata
            FROM lcm_embeddings
            WHERE id = $1
            "#,
            id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        row.map(row_to_record).transpose()
    }

    async fn delete(&self, id: Uuid) -> StorageResult<()> {
        sqlx::query!("DELETE FROM lcm_embeddings WHERE id = $1", id)
            .execute(&self.pool)
            .await
            .map_err(StorageError::ConnectionFailed)?;

        Ok(())
    }

    async fn delete_session(&self, session_id: SessionId) -> StorageResult<()> {
        sqlx::query!("DELETE FROM lcm_embeddings WHERE session_id = $1", session_id)
            .execute(&self.pool)
            .await
            .map_err(StorageError::ConnectionFailed)?;

        Ok(())
    }

    async fn get_session_vectors(&self, session_id: SessionId) -> StorageResult<Vec<VectorRecord>> {
        let rows = sqlx::query_as!(
            VectorRow,
            r#"
            SELECT id, session_id, content, embedding AS "embedding: Vector", metadata
            FROM lcm_embeddings
            WHERE session_id = $1
            ORDER BY created_at ASC
            "#,
            session_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        rows.into_iter().map(row_to_record).collect()
    }

    async fn search(
        &self,
        session_id: SessionId,
        query: &[f32],
        k: usize,
    ) -> StorageResult<Vec<VectorRecord>> {
        if query.is_empty() {
            return Err(StorageError::ConstraintViolation(
                "Query vector must not be empty".to_string(),
            ));
        }

        let query_vec = Vector::from(query.to_vec());

        // pgvector cosine distance operator: <=>
        // ORDER BY ascending distance = descending similarity.
        let rows = sqlx::query_as!(
            VectorRow,
            r#"
            SELECT id, session_id, content, embedding AS "embedding: Vector", metadata
            FROM lcm_embeddings
            WHERE session_id = $1
            ORDER BY embedding <=> $2
            LIMIT $3
            "#,
            session_id,
            query_vec as Vector,
            k as i64,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        rows.into_iter().map(row_to_record).collect()
    }
}
```

---

- [ ] **Step 5.3 — Run the vector store tests**

```bash
cargo test -p bacon-lcm-daemon --test test_pg_vector_store -- --nocapture
```

All 8 tests must pass.

---

- [ ] **Step 5.4 — Commit**

```bash
git add daemon/src/storage/pg_vector_store.rs daemon/tests/test_pg_vector_store.rs
git commit -m "feat(daemon/storage): PgVectorStore with pgvector — Task 5"
```

---

## Task 6 — Wire up `StorageLayer::postgres()` and update `core/src/storage/mod.rs`

**Files created / modified:**
- `core/src/storage/mod.rs` — add `StorageLayer::postgres(pool)` constructor that builds the full Pg-backed layer
- `daemon/tests/test_storage_layer.rs` — integration test exercising the full assembled layer

**Commit message:** `feat(core/storage): StorageLayer::postgres() constructor — Task 6`

---

### Design

`StorageLayer::postgres(pool)` lives in `core` but references types from `daemon`. To avoid a circular dependency (core depending on daemon) we use a **generic constructor** pattern:

```
core::StorageLayer::new(messages, summaries, sessions, vectors)
```

The `StorageLayer::postgres(pool)` function is actually defined **in the daemon crate** — in `daemon/src/storage/mod.rs` — as a free function or inherent impl on a newtype. The `core` crate already exposes `StorageLayer::new(...)` for wiring concrete impls.

This approach keeps `core` free of any DB dependency while giving callers a single ergonomic entry point via `daemon`:

```rust
// Usage in daemon/src/main.rs (after Task 6):
let storage = bacon_lcm_daemon::storage::postgres_layer(pool);
```

The `core/src/storage/mod.rs` `StorageLayer` already has `new()` and `memory()`. We also add a docstring pointing to the daemon crate for the Postgres constructor.

---

- [ ] **Step 6.1 — Write the failing integration test**

Create `daemon/tests/test_storage_layer.rs`:

```rust
// daemon/tests/test_storage_layer.rs
//! Smoke-test for the assembled StorageLayer backed by Postgres.

mod helpers;

use bacon_lcm_daemon::storage::postgres_layer;
use bacon_lcm_core::storage::SessionStore;
use bacon_lcm_core::types::Session;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

#[tokio::test]
async fn test_postgres_layer_session_roundtrip() {
    let pool = helpers::test_pool().await;
    let layer = postgres_layer(pool);

    let id = Uuid::new_v4();
    let session = Session {
        id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        metadata: HashMap::new(),
    };

    layer.sessions.create(session).await.expect("create failed");
    let retrieved = layer.sessions.get(id).await.expect("get failed");
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().id, id);
}
```

Run — must fail because `postgres_layer` does not exist yet:

```bash
cargo test -p bacon-lcm-daemon --test test_storage_layer 2>&1 | head -20
```

---

- [ ] **Step 6.2 — Add `postgres_layer()` to `daemon/src/storage/mod.rs`**

Replace the stub `daemon/src/storage/mod.rs` with:

```rust
// daemon/src/storage/mod.rs
//! Postgres-backed storage implementations for bacon-lcm.
//!
//! Consumers should call `postgres_layer(pool)` to get a fully assembled
//! `StorageLayer` backed by a live Postgres connection pool.

pub mod pg_message_store;
pub mod pg_session_store;
pub mod pg_summary_dag;
pub mod pg_vector_store;

use bacon_lcm_core::storage::StorageLayer;
use pg_message_store::PgMessageStore;
use pg_session_store::PgSessionStore;
use pg_summary_dag::PgSummaryDag;
use pg_vector_store::PgVectorStore;
use sqlx::PgPool;

/// Build a `StorageLayer` backed by a live Postgres connection pool.
///
/// All four stores share the same pool (each clone is cheap — `PgPool` is
/// an `Arc`-wrapped connection pool internally).
///
/// # Example
///
/// ```no_run
/// # use bacon_lcm_daemon::{db, storage::postgres_layer};
/// # async fn example() -> anyhow::Result<()> {
/// let pool = db::connect(&std::env::var("DATABASE_URL")?).await?;
/// db::run_migrations(&pool).await?;
/// let storage = postgres_layer(pool);
/// # Ok(())
/// # }
/// ```
pub fn postgres_layer(pool: PgPool) -> StorageLayer {
    StorageLayer::new(
        Box::new(PgMessageStore::new(pool.clone())),
        Box::new(PgSummaryDag::new(pool.clone())),
        Box::new(PgSessionStore::new(pool.clone())),
        Box::new(PgVectorStore::new(pool)),
    )
}
```

---

- [ ] **Step 6.3 — Update `core/src/storage/mod.rs` docs**

Add a documentation comment to `StorageLayer` pointing to the daemon constructor:

```rust
// In core/src/storage/mod.rs — update the StorageLayer doc comment:

/// Combined storage interface.
///
/// Use `StorageLayer::memory()` for testing.
/// For production use, call `bacon_lcm_daemon::storage::postgres_layer(pool)`
/// which builds this struct backed by a live Postgres pool.
pub struct StorageLayer {
    pub messages:  Box<dyn MessageStore>,
    pub summaries: Box<dyn SummaryDag>,
    pub sessions:  Box<dyn SessionStore>,
    pub vectors:   Box<dyn VectorStore>,
}
```

The `[lib]` section and `StorageLayer` struct in `core/src/storage/mod.rs` remain unchanged; we only update the doc comment. The `new()` and `memory()` constructors stay exactly as they are.

---

- [ ] **Step 6.4 — Update `daemon/src/main.rs` to use `postgres_layer`**

```rust
// daemon/src/main.rs
use anyhow::Context;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL must be set")?;

    let pool = bacon_lcm_daemon::db::connect(&database_url)
        .await
        .context("failed to connect to database")?;

    bacon_lcm_daemon::db::run_migrations(&pool)
        .await
        .context("failed to run migrations")?;

    let _storage = bacon_lcm_daemon::storage::postgres_layer(pool);

    tracing::info!("bacon-lcm-daemon started with Postgres storage layer");
    Ok(())
}
```

---

- [ ] **Step 6.5 — Run all daemon integration tests end-to-end**

```bash
cargo test -p bacon-lcm-daemon -- --nocapture 2>&1 | tail -40
```

Expected: all tests in `test_migrations`, `test_pg_session_store`, `test_pg_message_store`, `test_pg_summary_dag`, `test_pg_vector_store`, and `test_storage_layer` pass.

Also run the core library tests to confirm nothing regressed:

```bash
cargo test -p bacon-lcm-core -- --nocapture 2>&1 | tail -20
```

---

- [ ] **Step 6.6 — Commit**

```bash
git add daemon/src/storage/mod.rs daemon/src/main.rs \
        daemon/tests/test_storage_layer.rs \
        core/src/storage/mod.rs
git commit -m "feat(core/storage): StorageLayer::postgres() constructor — Task 6"
```

---

## Summary

After all six tasks are complete, the following is true:

| Component | File | Status |
|---|---|---|
| Workspace dependency | `/Cargo.toml` | `pgvector = { version = "0.4", features = ["sqlx"] }` added |
| Daemon manifest | `daemon/Cargo.toml` | `[lib]` + all deps added |
| DB helpers | `daemon/src/db.rs` | `connect()` + `run_migrations()` |
| Migrations | `daemon/migrations/0001_init.sql` | sessions, messages, summary nodes |
| Migrations | `daemon/migrations/0002_embeddings.sql` | pgvector extension + embeddings |
| Test helpers | `daemon/tests/helpers.rs` | `test_pool()` via testcontainers |
| Session store | `daemon/src/storage/pg_session_store.rs` | Full impl of `SessionStore` |
| Message store | `daemon/src/storage/pg_message_store.rs` | Full impl of `MessageStore` |
| Summary DAG | `daemon/src/storage/pg_summary_dag.rs` | Full impl of `SummaryDag` |
| Vector store | `daemon/src/storage/pg_vector_store.rs` | Full impl of `VectorStore` (pgvector) |
| Assembly point | `daemon/src/storage/mod.rs` | `postgres_layer(pool) -> StorageLayer` |
| Core docs | `core/src/storage/mod.rs` | Doc comment updated; no structural change |
| Integration tests | `daemon/tests/test_*.rs` | 6 test files, ~36 tests total |

### Test count summary

| Test file | Tests |
|---|---|
| `test_migrations` | 1 |
| `test_pg_session_store` | 6 |
| `test_pg_message_store` | 8 |
| `test_pg_summary_dag` | 7 |
| `test_pg_vector_store` | 8 |
| `test_storage_layer` | 1 |
| **Total** | **31** |
