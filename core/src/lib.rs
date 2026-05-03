// core/src/lib.rs
//! bacon-lcm-core - Lossless Context Memory core library
//! 
//! This library provides the core LCM functionality including:
//! - Session management with three-level compaction
//! - PostgreSQL persistence layer
//! - Extensible provider system for LLMs and embeddings
//! - Type-safe ID management
//! - Comprehensive error handling

pub mod types;
pub mod ids;
pub mod config;
pub mod error;
pub mod session;
pub mod compaction;
pub mod storage;
pub mod providers;
pub mod metrics;

// Re-export key types for convenience
pub use types::*;
pub use ids::*;
pub use config::*;
pub use error::*;
pub use session::LcmSession;
pub use compaction::CompactionEngine;
pub use storage::*;
pub use providers::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_library_imports() {
        // Basic smoke test to ensure library compiles
        assert!(true);
    }
}
