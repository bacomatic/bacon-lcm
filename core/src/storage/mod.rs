// core/src/storage/mod.rs
//! Storage layer for LCM persistence
//!
//! Provides traits and implementations for:
//! - Message storage and retrieval
//! - Summary DAG management
//! - Session persistence
//! - Vector storage for embeddings

pub mod message_store;
pub mod summary_dag;

// Re-export main traits and implementations
pub use message_store::{InMemoryMessageStore, MessageStore};
pub use summary_dag::{InMemorySummaryDag, SummaryDag};

/// Combined storage interface for convenience
pub struct StorageLayer {
    pub messages: Box<dyn MessageStore>,
    pub summaries: Box<dyn SummaryDag>,
}

impl StorageLayer {
    pub fn new(
        messages: Box<dyn MessageStore>,
        summaries: Box<dyn SummaryDag>,
    ) -> Self {
        Self {
            messages,
            summaries,
        }
    }

    /// Create in-memory storage layer for testing
    pub fn memory() -> Self {
        Self {
            messages: Box::new(InMemoryMessageStore::new()),
            summaries: Box::new(InMemorySummaryDag::new()),
        }
    }
}
