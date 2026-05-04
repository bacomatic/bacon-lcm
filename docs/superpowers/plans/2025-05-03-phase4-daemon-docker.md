# Daemon + Docker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make bacon-lcm fully deployable: a long-running daemon with a health/metrics/status HTTP API, MCP server wired to Postgres when `DATABASE_URL` is set, and a Docker Compose environment that brings up both.

**Architecture:** The daemon binary exposes an axum HTTP server on port `3333` (health, metrics, status) and stays alive until SIGTERM/Ctrl-C. The MCP server binary selects in-memory vs. Postgres `StorageLayer` based on whether `DATABASE_URL` is set at startup — no IPC, no coupling between the two binaries. Docker Compose runs postgres, daemon, and mcp-server as three services.

**Tech Stack:** `axum 0.7` (HTTP), `tokio::signal` (graceful shutdown), `prometheus 0.13` (metrics), `sqlx` (Postgres pool already wired), Docker multi-stage build.

---

## File Map

### New files
- `daemon/src/http.rs` — axum router: `/health`, `/metrics`, `/status`
- `daemon/src/metrics.rs` — Prometheus registry + counter/gauge/histogram definitions
- `daemon/src/state.rs` — `AppState` struct shared across HTTP handlers
- `docker/Dockerfile` — multi-stage Rust build
- `docker/docker-compose.yml` — postgres + daemon + mcp-server services
- `docker/entrypoint.sh` — signal handling wrapper (not needed — Rust handles signals directly; omit)
- `.dockerignore` — exclude target/, docs/, etc.

### Modified files
- `daemon/Cargo.toml` — add `axum`, `prometheus`, `tokio` (signal feature already covered by "full")
- `Cargo.toml` — add `axum` to workspace deps
- `daemon/src/lib.rs` — re-export `http`, `metrics`, `state` modules
- `daemon/src/main.rs` — full service loop with HTTP server + graceful shutdown
- `mcp-server/src/server.rs` — `build_default_session()` switches to Postgres StorageLayer when `DATABASE_URL` is set
- `mcp-server/Cargo.toml` — add `bacon-lcm-daemon` as dependency (for `postgres_layer`)

---

## Task 1 — Add `axum` to workspace and daemon

**Files:**
- Modify: `Cargo.toml`
- Modify: `daemon/Cargo.toml`

- [ ] **Step 1.1 — Add `axum` to workspace deps**

In `Cargo.toml`, add to `[workspace.dependencies]`:

```toml
axum = { version = "0.7", features = ["json"] }
```

- [ ] **Step 1.2 — Add `axum` and `prometheus` to daemon deps**

In `daemon/Cargo.toml`, add to `[dependencies]`:

```toml
axum        = { workspace = true }
prometheus  = { workspace = true }
```

- [ ] **Step 1.3 — Verify workspace compiles**

```
cargo build --workspace
```

Expected: no errors (axum + prometheus are already in the dep tree via workspace).

- [ ] **Step 1.4 — Commit**

```bash
git add Cargo.toml daemon/Cargo.toml
git commit -m "feat(daemon): add axum and prometheus to dependencies (Task 1)"
```

---

## Task 2 — Prometheus metrics registry

**Files:**
- Create: `daemon/src/metrics.rs`
- Modify: `daemon/src/lib.rs`

- [ ] **Step 2.1 — Write the failing test first**

At the bottom of `daemon/src/metrics.rs` (create the file), add:

```rust
// daemon/src/metrics.rs
use prometheus::{Counter, Histogram, HistogramOpts, IntGauge, Registry};

/// All Prometheus metrics for the daemon.
pub struct Metrics {
    pub registry: Registry,
    pub messages_stored_total: Counter,
    pub compactions_total: Counter,
    pub compaction_duration_seconds: Histogram,
    pub active_sessions: IntGauge,
}

impl Metrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let messages_stored_total = Counter::new(
            "lcm_messages_stored_total",
            "Total number of messages stored across all sessions",
        )?;

        let compactions_total = Counter::new(
            "lcm_compactions_total",
            "Total number of compaction operations triggered",
        )?;

        let compaction_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "lcm_compaction_duration_seconds",
                "Duration of compaction operations in seconds",
            )
            .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0]),
        )?;

        let active_sessions = IntGauge::new(
            "lcm_active_sessions",
            "Number of currently active LCM sessions",
        )?;

        registry.register(Box::new(messages_stored_total.clone()))?;
        registry.register(Box::new(compactions_total.clone()))?;
        registry.register(Box::new(compaction_duration_seconds.clone()))?;
        registry.register(Box::new(active_sessions.clone()))?;

        Ok(Self {
            registry,
            messages_stored_total,
            compactions_total,
            compaction_duration_seconds,
            active_sessions,
        })
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new().expect("failed to create Prometheus metrics registry")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_registry_constructs() {
        let m = Metrics::new();
        assert!(m.is_ok(), "metrics registry construction failed: {:?}", m.err());
    }

    #[test]
    fn test_metrics_increment_counter() {
        let m = Metrics::new().unwrap();
        m.messages_stored_total.inc();
        assert_eq!(m.messages_stored_total.get(), 1.0);
    }

    #[test]
    fn test_metrics_active_sessions_gauge() {
        let m = Metrics::new().unwrap();
        m.active_sessions.set(3);
        assert_eq!(m.active_sessions.get(), 3);
    }

    #[test]
    fn test_metrics_encode_returns_non_empty() {
        let m = Metrics::new().unwrap();
        m.messages_stored_total.inc_by(5.0);
        let mut buf = String::new();
        use prometheus::Encoder;
        let encoder = prometheus::TextEncoder::new();
        let mfs = m.registry.gather();
        encoder.encode_utf8(&mfs, &mut buf).unwrap();
        assert!(buf.contains("lcm_messages_stored_total"));
        assert!(buf.contains("5"));
    }
}
```

- [ ] **Step 2.2 — Run tests (should fail — file doesn't exist yet)**

```
cargo test -p bacon-lcm-daemon --lib metrics
```

Expected: compile error — `daemon/src/metrics.rs` not found.

- [ ] **Step 2.3 — Register module in `daemon/src/lib.rs`**

Replace `daemon/src/lib.rs` with:

```rust
// daemon/src/lib.rs
pub mod db;
pub mod metrics;
pub mod storage;
```

- [ ] **Step 2.4 — Run tests (should pass)**

```
cargo test -p bacon-lcm-daemon --lib metrics
```

Expected:
```
running 4 tests
test metrics::tests::test_metrics_registry_constructs ... ok
test metrics::tests::test_metrics_increment_counter ... ok
test metrics::tests::test_metrics_active_sessions_gauge ... ok
test metrics::tests::test_metrics_encode_returns_non_empty ... ok

test result: ok. 4 passed; 0 failed
```

- [ ] **Step 2.5 — Commit**

```bash
git add daemon/src/metrics.rs daemon/src/lib.rs
git commit -m "feat(daemon): Prometheus metrics registry (Task 2)"
```

---

## Task 3 — `AppState` and axum HTTP router

**Files:**
- Create: `daemon/src/state.rs`
- Create: `daemon/src/http.rs`
- Modify: `daemon/src/lib.rs`

- [ ] **Step 3.1 — Write the failing tests first**

Create `daemon/src/state.rs`:

```rust
// daemon/src/state.rs
use std::sync::Arc;
use std::time::Instant;

use sqlx::PgPool;

use crate::metrics::Metrics;

/// Shared application state injected into every axum handler.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub metrics: Arc<Metrics>,
    pub started_at: Instant,
    pub version: &'static str,
}

impl AppState {
    pub fn new(pool: PgPool, metrics: Arc<Metrics>) -> Self {
        Self {
            pool,
            metrics,
            started_at: Instant::now(),
            version: env!("CARGO_PKG_VERSION"),
        }
    }

    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}
```

Create `daemon/src/http.rs`:

```rust
// daemon/src/http.rs
use std::sync::Arc;

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use prometheus::Encoder;
use serde_json::json;

use crate::state::AppState;

/// Build the axum router with /health, /metrics, and /status routes.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health",  get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/status",  get(status_handler))
        .with_state(state)
}

// ── /health ───────────────────────────────────────────────────────────────────

async fn health_handler(State(state): State<AppState>) -> Response {
    // Cheap DB connectivity check: single SELECT 1.
    let db_ok = sqlx::query("SELECT 1")
        .execute(&state.pool)
        .await
        .is_ok();

    let status_code = if db_ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    let body = json!({
        "status":   if db_ok { "healthy" } else { "degraded" },
        "database": if db_ok { "up" } else { "down" },
    });

    (status_code, axum::Json(body)).into_response()
}

// ── /metrics ──────────────────────────────────────────────────────────────────

async fn metrics_handler(State(state): State<AppState>) -> Response {
    let encoder = prometheus::TextEncoder::new();
    let mfs = state.metrics.registry.gather();
    let mut buf = String::new();
    if encoder.encode_utf8(&mfs, &mut buf).is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "failed to encode metrics").into_response();
    }
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        buf,
    )
        .into_response()
}

// ── /status ───────────────────────────────────────────────────────────────────

async fn status_handler(State(state): State<AppState>) -> Response {
    let active_sessions = state.metrics.active_sessions.get();
    let messages_stored = state.metrics.messages_stored_total.get();

    let body = json!({
        "version":         state.version,
        "uptime_secs":     state.uptime_secs(),
        "active_sessions": active_sessions,
        "messages_stored": messages_stored,
        "database":        state.pool.is_closed().then_some("closed").unwrap_or("open"),
    });

    (StatusCode::OK, axum::Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    /// Create a minimal test AppState with an in-memory SQLite pool.
    /// We can't easily spin up Postgres in unit tests, so we test the
    /// router structure (routing, JSON shape) separately from DB behaviour.
    ///
    /// The /health handler will return 503 because the dummy pool always
    /// fails SELECT 1 — that's acceptable for these unit tests; the
    /// integration tests (daemon/tests/) cover the real DB path.
    async fn make_test_state() -> AppState {
        // Use a closed pool (connect_lazy with an unreachable URL) so
        // the handler gracefully reports the DB as down.
        let pool = sqlx::PgPool::connect_lazy("postgres://invalid:5432/test")
            .expect("connect_lazy should not fail");
        let metrics = Arc::new(crate::metrics::Metrics::new().unwrap());
        AppState::new(pool, metrics)
    }

    #[tokio::test]
    async fn test_health_route_exists() {
        let state = make_test_state().await;
        let app = router(state);
        let resp = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        // 200 or 503 — either is fine; we just care the route exists and returns JSON
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("status").is_some());
    }

    #[tokio::test]
    async fn test_metrics_route_returns_prometheus_text() {
        let state = make_test_state().await;
        let app = router(state);
        let resp = app
            .oneshot(Request::builder().uri("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        assert!(text.contains("lcm_") || text.is_empty(), "unexpected body: {text}");
    }

    #[tokio::test]
    async fn test_status_route_returns_json_fields() {
        let state = make_test_state().await;
        let app = router(state);
        let resp = app
            .oneshot(Request::builder().uri("/status").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("version").is_some(), "missing 'version'");
        assert!(json.get("uptime_secs").is_some(), "missing 'uptime_secs'");
        assert!(json.get("active_sessions").is_some(), "missing 'active_sessions'");
        assert!(json.get("messages_stored").is_some(), "missing 'messages_stored'");
        assert!(json.get("database").is_some(), "missing 'database'");
    }

    #[tokio::test]
    async fn test_unknown_route_returns_404() {
        let state = make_test_state().await;
        let app = router(state);
        let resp = app
            .oneshot(Request::builder().uri("/nonexistent").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
```

- [ ] **Step 3.2 — Run tests (should fail — modules not registered)**

```
cargo test -p bacon-lcm-daemon --lib http
```

Expected: compile error — `http` and `state` modules not found.

- [ ] **Step 3.3 — Register modules in `daemon/src/lib.rs`**

Replace `daemon/src/lib.rs` with:

```rust
// daemon/src/lib.rs
pub mod db;
pub mod http;
pub mod metrics;
pub mod state;
pub mod storage;
```

- [ ] **Step 3.4 — Add `tower` to daemon dev-dependencies for `ServiceExt`**

In `daemon/Cargo.toml`, add to `[dev-dependencies]`:

```toml
tower = { version = "0.4" }
```

- [ ] **Step 3.5 — Run tests (should pass)**

```
cargo test -p bacon-lcm-daemon --lib http
```

Expected:
```
running 4 tests
test http::tests::test_health_route_exists ... ok
test http::tests::test_metrics_route_returns_prometheus_text ... ok
test http::tests::test_status_route_returns_json_fields ... ok
test http::tests::test_unknown_route_returns_404 ... ok

test result: ok. 4 passed; 0 failed
```

- [ ] **Step 3.6 — Run all daemon lib tests**

```
cargo test -p bacon-lcm-daemon --lib
```

Expected: all previously-passing tests still pass plus 4 new ones (8 total lib tests).

- [ ] **Step 3.7 — Commit**

```bash
git add daemon/src/state.rs daemon/src/http.rs daemon/src/lib.rs daemon/Cargo.toml
git commit -m "feat(daemon): AppState + axum HTTP router with /health /metrics /status (Task 3)"
```

---

## Task 4 — Full daemon `main.rs` with service loop and graceful shutdown

**Files:**
- Modify: `daemon/src/main.rs`

- [ ] **Step 4.1 — Replace `daemon/src/main.rs`**

```rust
// daemon/src/main.rs
//! bacon-lcm daemon entry point.
//!
//! ## Environment variables
//!
//! | Variable       | Default    | Description                             |
//! |----------------|------------|-----------------------------------------|
//! | `DATABASE_URL` | *(required)* | PostgreSQL connection string          |
//! | `LCM_PORT`     | `3333`     | HTTP server port                        |
//! | `RUST_LOG`     | `"info"`   | tracing-subscriber log filter           |

use std::sync::Arc;

use anyhow::Context;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use bacon_lcm_daemon::{
    db,
    http::router,
    metrics::Metrics,
    state::AppState,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Logging ───────────────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("bacon-lcm-daemon starting");

    // ── Database ──────────────────────────────────────────────────────────────
    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL environment variable must be set")?;

    let pool = db::connect(&database_url)
        .await
        .context("failed to connect to database")?;

    db::run_migrations(&pool)
        .await
        .context("failed to run database migrations")?;

    tracing::info!("database connected and migrations applied");

    // ── Shared state ──────────────────────────────────────────────────────────
    let metrics = Arc::new(Metrics::new().context("failed to initialise Prometheus metrics")?);
    let state = AppState::new(pool, Arc::clone(&metrics));

    // ── HTTP server ───────────────────────────────────────────────────────────
    let port: u16 = std::env::var("LCM_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3333);

    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("failed to bind to {addr}"))?;

    tracing::info!(%addr, "HTTP server listening");

    let app = router(state);

    // ── Graceful shutdown ─────────────────────────────────────────────────────
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("HTTP server error")?;

    tracing::info!("bacon-lcm-daemon shut down gracefully");
    Ok(())
}

/// Resolves when SIGTERM or Ctrl-C is received.
async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c    => { tracing::info!("received Ctrl-C, shutting down"); },
        _ = terminate => { tracing::info!("received SIGTERM, shutting down"); },
    }
}
```

- [ ] **Step 4.2 — Build the binary (no tests for main, just compile check)**

```
cargo build -p bacon-lcm-daemon
```

Expected: `Finished dev profile` with no errors.

- [ ] **Step 4.3 — Run all daemon lib tests to confirm nothing broke**

```
cargo test -p bacon-lcm-daemon --lib
```

Expected: all tests pass.

- [ ] **Step 4.4 — Commit**

```bash
git add daemon/src/main.rs
git commit -m "feat(daemon): full service loop with HTTP server and graceful shutdown (Task 4)"
```

---

## Task 5 — Wire MCP server to Postgres when `DATABASE_URL` is set

**Files:**
- Modify: `mcp-server/Cargo.toml`
- Modify: `mcp-server/src/server.rs`

The goal: `build_default_session()` uses `StorageLayer::postgres(pool)` when `DATABASE_URL` is present, falling back to `StorageLayer::memory()` otherwise. The `postgres_layer` factory lives in `bacon_lcm_daemon::storage`.

- [ ] **Step 5.1 — Write the failing test first**

Add to the `tests` module at the bottom of `mcp-server/src/server.rs`:

```rust
    #[tokio::test]
    async fn test_build_default_session_memory_when_no_db_url() {
        // When DATABASE_URL is not set the session must still build successfully.
        // Remove the var if it happens to be set in the environment.
        std::env::remove_var("DATABASE_URL");
        let session = build_default_session().await;
        assert!(
            session.is_ok(),
            "build_default_session failed without DATABASE_URL: {:?}",
            session.err()
        );
    }
```

Run:

```
cargo test -p bacon-lcm-mcp-server --lib server::tests::test_build_default_session_memory_when_no_db_url
```

Expected: PASS (this should already pass since `build_default_session` already uses memory storage).

- [ ] **Step 5.2 — Add `bacon-lcm-daemon` dependency to mcp-server**

In `mcp-server/Cargo.toml`, add to `[dependencies]`:

```toml
bacon-lcm-daemon = { path = "../daemon" }
sqlx             = { workspace = true }
```

- [ ] **Step 5.3 — Update `build_default_session` in `mcp-server/src/server.rs`**

Replace the `build_default_session` function (currently starts at `pub async fn build_default_session()`):

```rust
/// Build the default LCM session from environment variables.
///
/// If `DATABASE_URL` is set, uses the Postgres `StorageLayer`; otherwise
/// falls back to the in-memory layer (useful for local dev/testing).
pub async fn build_default_session() -> bacon_lcm_core::LcmResult<LcmSession> {
    let provider = std::env::var("LCM_SUMMARIZER_PROVIDER")
        .unwrap_or_else(|_| "echo".to_string());
    let model = std::env::var("LCM_SUMMARIZER_MODEL")
        .unwrap_or_else(|_| "echo".to_string());
    let api_key = std::env::var("LCM_SUMMARIZER_API_KEY").ok();

    let token_counter = create_token_counter("naive", None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;
    let summarizer = create_summarizer(
        &provider,
        model,
        None,
        api_key,
        None,
        None,
    )
    .map_err(bacon_lcm_core::LcmError::Provider)?;
    let embedder = create_embedder("null", None, None, None, None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;

    let config = LcmConfig::defaults();

    let storage = if let Ok(db_url) = std::env::var("DATABASE_URL") {
        tracing::info!("DATABASE_URL set — using Postgres storage layer");
        let pool = sqlx::PgPool::connect(&db_url)
            .await
            .map_err(|e| bacon_lcm_core::LcmError::Storage(
                bacon_lcm_core::StorageError::ConnectionFailed(e),
            ))?;
        bacon_lcm_daemon::storage::postgres_layer(pool)
    } else {
        tracing::debug!("DATABASE_URL not set — using in-memory storage layer");
        StorageLayer::memory()
    };

    LcmSession::new(token_counter, summarizer, embedder, config, storage).await
}
```

Also add the `tracing` import to the `use` block at the top of `server.rs` if not already present:

```rust
use tracing;
```

- [ ] **Step 5.4 — Also update `build_restored_session` in `mcp-server/src/main.rs`** to use Postgres storage when available:

Replace the `build_restored_session` function in `mcp-server/src/main.rs`:

```rust
/// Attempt to restore a session by ID.
///
/// Uses Postgres storage when DATABASE_URL is set; otherwise in-memory
/// (which always fails with SessionNotFound — the call site handles this).
async fn build_restored_session(
    session_id: uuid::Uuid,
) -> bacon_lcm_core::LcmResult<bacon_lcm_core::LcmSession> {
    use bacon_lcm_core::{
        LcmConfig, LcmSession,
        providers::{create_embedder, create_summarizer, create_token_counter},
        storage::StorageLayer,
    };

    let provider = std::env::var("LCM_SUMMARIZER_PROVIDER")
        .unwrap_or_else(|_| "echo".to_string());
    let model = std::env::var("LCM_SUMMARIZER_MODEL")
        .unwrap_or_else(|_| "echo".to_string());
    let api_key = std::env::var("LCM_SUMMARIZER_API_KEY").ok();

    let token_counter = create_token_counter("naive", None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;
    let summarizer = create_summarizer(&provider, model, None, api_key, None, None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;
    let embedder = create_embedder("null", None, None, None, None)
        .map_err(bacon_lcm_core::LcmError::Provider)?;

    let storage = if let Ok(db_url) = std::env::var("DATABASE_URL") {
        let pool = sqlx::PgPool::connect(&db_url)
            .await
            .map_err(|e| bacon_lcm_core::LcmError::Storage(
                bacon_lcm_core::StorageError::ConnectionFailed(e),
            ))?;
        bacon_lcm_daemon::storage::postgres_layer(pool)
    } else {
        StorageLayer::memory()
    };

    LcmSession::restore(
        session_id,
        token_counter,
        summarizer,
        embedder,
        LcmConfig::defaults(),
        storage,
    )
    .await
}
```

Also add `use sqlx;` to `mcp-server/src/main.rs` after the existing `use` statements.

- [ ] **Step 5.5 — Verify `StorageError` is re-exported from `bacon_lcm_core`**

```
grep -n "pub use\|StorageError" core/src/lib.rs | head -10
```

Expected: `StorageError` is visible via `bacon_lcm_core::StorageError`. If it isn't, add it to the `pub use` block in `core/src/lib.rs`.

- [ ] **Step 5.6 — Build and run all mcp-server lib tests**

```
cargo test -p bacon-lcm-mcp-server --lib
```

Expected: all 14 tests pass (13 existing + the new memory fallback test).

- [ ] **Step 5.7 — Commit**

```bash
git add mcp-server/Cargo.toml mcp-server/src/server.rs mcp-server/src/main.rs
git commit -m "feat(mcp-server): use Postgres StorageLayer when DATABASE_URL is set (Task 5)"
```

---

## Task 6 — Docker Compose environment

**Files:**
- Create: `docker/Dockerfile`
- Create: `docker/docker-compose.yml`
- Create: `.dockerignore`

This task has no Rust tests — validation is a `docker compose up` smoke test at the end.

- [ ] **Step 6.1 — Create `.dockerignore`**

```
# .dockerignore
target/
docs/
*.md
.git/
.gitignore
docker/
raw/
```

- [ ] **Step 6.2 — Create `docker/Dockerfile`**

```dockerfile
# docker/Dockerfile
# ── Build stage ───────────────────────────────────────────────────────────────
FROM rust:1.87-slim AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependency compilation separately from source changes.
# Copy manifests first so the dep layer is only rebuilt when deps change.
COPY Cargo.toml Cargo.lock ./
COPY core/Cargo.toml       core/Cargo.toml
COPY daemon/Cargo.toml     daemon/Cargo.toml
COPY mcp-server/Cargo.toml mcp-server/Cargo.toml
COPY cli/Cargo.toml        cli/Cargo.toml

# Create stub lib/main files so `cargo build` can resolve the workspace.
RUN mkdir -p core/src daemon/src mcp-server/src cli/src \
    && echo 'fn main(){}' > daemon/src/main.rs \
    && echo 'fn main(){}' > mcp-server/src/main.rs \
    && echo 'fn main(){}' > cli/src/main.rs \
    && echo ''            > core/src/lib.rs \
    && echo ''            > daemon/src/lib.rs \
    && echo ''            > mcp-server/src/lib.rs

RUN cargo build --release 2>/dev/null; true

# Now copy real source and build for real.
COPY . .
RUN touch core/src/lib.rs daemon/src/lib.rs mcp-server/src/lib.rs \
         daemon/src/main.rs mcp-server/src/main.rs cli/src/main.rs
RUN cargo build --release

# ── Runtime stage ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/bacon-lcm-daemon     /usr/local/bin/bacon-lcm-daemon
COPY --from=builder /app/target/release/bacon-lcm-mcp-server /usr/local/bin/bacon-lcm-mcp-server

# Migrations are embedded via sqlx::migrate! — no external SQL files needed.

EXPOSE 3333

CMD ["bacon-lcm-daemon"]
```

- [ ] **Step 6.3 — Create `docker/docker-compose.yml`**

```yaml
# docker/docker-compose.yml
version: "3.9"

services:
  postgres:
    image: pgvector/pgvector:pg16
    environment:
      POSTGRES_DB:       bacon_lcm
      POSTGRES_USER:     bacon_lcm
      POSTGRES_PASSWORD: bacon_lcm
    ports:
      - "5432:5432"
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U bacon_lcm -d bacon_lcm"]
      interval: 5s
      timeout: 5s
      retries: 10

  daemon:
    build:
      context: ..
      dockerfile: docker/Dockerfile
    command: bacon-lcm-daemon
    environment:
      DATABASE_URL: "postgres://bacon_lcm:bacon_lcm@postgres:5432/bacon_lcm"
      LCM_PORT:     "3333"
      RUST_LOG:     "info"
    ports:
      - "3333:3333"
    depends_on:
      postgres:
        condition: service_healthy
    healthcheck:
      test: ["CMD-SHELL", "curl -sf http://localhost:3333/health || exit 1"]
      interval: 10s
      timeout: 5s
      retries: 5

  mcp-server:
    build:
      context: ..
      dockerfile: docker/Dockerfile
    command: bacon-lcm-mcp-server
    environment:
      DATABASE_URL:            "postgres://bacon_lcm:bacon_lcm@postgres:5432/bacon_lcm"
      LCM_SUMMARIZER_PROVIDER: "echo"
      LCM_SUMMARIZER_MODEL:    "echo"
      RUST_LOG:                "info"
    stdin_open: true
    tty: false
    depends_on:
      postgres:
        condition: service_healthy

volumes:
  pgdata:
```

- [ ] **Step 6.4 — Verify workspace build passes (Docker will also run this)**

```
cargo build --workspace
```

Expected: `Finished dev profile` with no errors.

- [ ] **Step 6.5 — Commit**

```bash
git add docker/Dockerfile docker/docker-compose.yml .dockerignore
git commit -m "feat(docker): multi-stage Dockerfile and docker-compose for daemon + mcp-server (Task 6)"
```

---

## Task 7 — Full workspace test + sanity check

- [ ] **Step 7.1 — Run all non-integration tests**

```
cargo test --workspace --exclude bacon-lcm-daemon 2>&1
cargo test -p bacon-lcm-daemon --lib 2>&1
```

The daemon integration tests (in `daemon/tests/`) require a live Postgres instance via testcontainers and can be skipped here.

Expected: all unit tests pass across all crates.

- [ ] **Step 7.2 — Run the MCP smoke test**

```
cargo test -p bacon-lcm-mcp-server --test smoke_test
```

Expected:
```
running 1 test
test smoke_tools_list_returns_six_tools ... ok
```

- [ ] **Step 7.3 — Commit final state**

```bash
git add -A
git status  # confirm nothing unexpected is staged
git commit -m "feat: Phase 4 complete — daemon HTTP API, Postgres storage wiring, Docker (Task 7)"
```

---

## Reference: confirmed API signatures

- `StorageError::ConnectionFailed(sqlx::Error)` — use this for DB pool connection failures (defined in `core/src/error.rs`)
- `postgres_layer(pool: sqlx::PgPool) -> StorageLayer` — defined in `daemon/src/storage/mod.rs`
