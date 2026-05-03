// daemon/src/http.rs
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
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

    /// Create a minimal test AppState using a lazy pool that will fail SELECT 1.
    /// /health will return 503 (DB down) — that's expected and acceptable here.
    async fn make_test_state() -> AppState {
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
