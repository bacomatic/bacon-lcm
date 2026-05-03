// core/src/storage/mod.rs
//! Storage layer for LCM persistence
//!
//! Provides traits and implementations for:
//! - Message storage and retrieval
//! - Summary DAG management
//! - Session persistence
//! - Vector storage for embeddings

pub mod message_store;
pub mod session_store;
pub mod summary_dag;
pub mod vector_store;

// Re-export main traits and implementations
pub use message_store::{InMemoryMessageStore, MessageStore};
pub use session_store::{InMemorySessionStore, SessionStore};
pub use summary_dag::{InMemorySummaryDag, SummaryDag};
pub use vector_store::{InMemoryVectorStore, VectorStore};

// Postgres implementations are provided by the daemon crate (require a live DB).
// Placeholder type aliases keep the public API stable.
// pub use pg_session_store::PgSessionStore;
// pub use pg_vector_store::PgVectorStore;

/// Combined storage interface for convenience
pub struct StorageLayer {
    pub messages: Box<dyn MessageStore>,
    pub summaries: Box<dyn SummaryDag>,
    pub sessions: Box<dyn SessionStore>,
    pub vectors: Box<dyn VectorStore>,
}

impl StorageLayer {
    pub fn new(
        messages: Box<dyn MessageStore>,
        summaries: Box<dyn SummaryDag>,
        sessions: Box<dyn SessionStore>,
        vectors: Box<dyn VectorStore>,
    ) -> Self {
        Self {
            messages,
            summaries,
            sessions,
            vectors,
        }
    }

    /// Create in-memory storage layer for testing
    pub fn memory() -> Self {
        Self {
            messages: Box::new(InMemoryMessageStore::new()),
            summaries: Box::new(InMemorySummaryDag::new()),
            sessions: Box::new(InMemorySessionStore::new()),
            vectors: Box::new(InMemoryVectorStore::new()),
        }
    }
}
