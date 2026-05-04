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
