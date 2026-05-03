// mcp-server/src/main.rs
//! bacon-lcm MCP server entry point.
//!
//! ## Environment variables
//!
//! | Variable                  | Default   | Description                                    |
//! |---------------------------|-----------|------------------------------------------------|
//! | `LCM_SUMMARIZER_PROVIDER` | `"echo"`  | Summarizer provider: echo / openai / anthropic |
//! | `LCM_SUMMARIZER_MODEL`    | `"echo"`  | Model name for the summarizer                  |
//! | `LCM_SUMMARIZER_API_KEY`  | *(none)*  | API key (required for openai/anthropic)        |
//! | `LCM_SESSION_ID`          | *(none)*  | If set, attempt to restore an existing session |
//! | `RUST_LOG`                | `"info"`  | tracing-subscriber log filter                  |

use anyhow::Context;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

use bacon_lcm_mcp_server::server::LcmServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Logging ───────────────────────────────────────────────────────────────
    // Write tracing output to stderr so it does not interfere with the
    // JSON-RPC framing on stdout.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("bacon-lcm-mcp-server starting");

    // ── Optional session restore ──────────────────────────────────────────────
    let maybe_session_id = std::env::var("LCM_SESSION_ID").ok();

    let server = if let Some(ref id_str) = maybe_session_id {
        match uuid::Uuid::parse_str(id_str) {
            Ok(session_id) => {
                tracing::info!(%session_id, "attempting to restore session");
                let server = LcmServer::new();
                // Restoration from in-memory storage always fails with
                // `SessionNotFound` because in-memory storage does not survive
                // process restarts.  Log a warning and fall through to a fresh
                // session.  The daemon crate will later inject
                // `StorageLayer::postgres(pool)` to make restoration durable.
                match build_restored_session(session_id).await {
                    Ok(session) => {
                        server.set_session(session).await;
                        tracing::info!("session restored successfully");
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "could not restore session (in-memory storage does not \
                             persist across restarts); starting fresh"
                        );
                    }
                }
                server
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "LCM_SESSION_ID is not a valid UUID; ignoring and starting fresh"
                );
                LcmServer::new()
            }
        }
    } else {
        LcmServer::new()
    };

    // ── Start stdio transport ─────────────────────────────────────────────────
    tracing::info!("listening on stdio");
    let service = server
        .serve(stdio())
        .await
        .context("failed to start stdio transport")?;

    service.waiting().await.context("MCP service error")?;

    tracing::info!("bacon-lcm-mcp-server shutting down");
    Ok(())
}

// ── Session restore helper ────────────────────────────────────────────────────

/// Attempt to restore a session by ID using in-memory storage and env-configured
/// providers.
///
/// With in-memory storage this always returns `LcmError::SessionNotFound`.
/// The function exists as a clean extension point for when the daemon injects
/// a `StorageLayer::postgres(pool)`.
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

    LcmSession::restore(
        session_id,
        token_counter,
        summarizer,
        embedder,
        LcmConfig::defaults(),
        StorageLayer::memory(),
    )
    .await
}
