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
// NOTE: These re-exports are stubs during initial scaffolding; allow unused for now.
#[allow(unused_imports)]
pub use types::*;
#[allow(unused_imports)]
pub use ids::*;
#[allow(unused_imports)]
pub use config::*;
#[allow(unused_imports)]
pub use error::*;
pub use session::LcmSession;
pub use compaction::CompactionEngine;
#[allow(unused_imports)]
pub use storage::*;
#[allow(unused_imports)]
pub use providers::*;

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn test_library_imports() {
        // Basic smoke test to ensure library compiles
        assert!(true);
    }
}
