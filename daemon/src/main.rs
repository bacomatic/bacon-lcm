// daemon/src/main.rs
//! bacon-lcm daemon entry point.
//!
//! ## Environment variables
//!
//! | Variable       | Default      | Description                             |
//! |----------------|--------------|-----------------------------------------|
//! | `DATABASE_URL` | *(required)* | PostgreSQL connection string            |
//! | `LCM_PORT`     | `3333`       | HTTP server port                        |
//! | `RUST_LOG`     | `"info"`     | tracing-subscriber log filter           |

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
