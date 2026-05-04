// daemon/src/storage/mod.rs
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

/// Build a `StorageLayer` backed by PostgreSQL.
/// The pool must already be connected and migrations must have been run.
pub fn postgres_layer(pool: PgPool) -> StorageLayer {
    StorageLayer::new(
        Box::new(PgMessageStore::new(pool.clone())),
        Box::new(PgSummaryDag::new(pool.clone())),
        Box::new(PgSessionStore::new(pool.clone())),
        Box::new(PgVectorStore::new(pool)),
    )
}
