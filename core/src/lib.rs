// core/src/lib.rs
//! bacon-lcm-core - Lossless Context Memory core library
//!
//! This library provides the core LCM functionality including:
//! - Session management with three-level compaction
//! - PostgreSQL persistence layer
//! - Extensible provider system for LLMs and embeddings
//! - Type-safe ID management
//! - Comprehensive error handling

pub mod config;
pub mod error;
pub mod ids;
pub mod metrics;
pub mod providers;
pub mod session;
pub mod storage;
pub mod types;

// Re-export key types for convenience
// NOTE: These re-exports are stubs during initial scaffolding; allow unused for now.
#[allow(unused_imports)]
pub use config::*;
#[allow(unused_imports)]
pub use error::*;
#[allow(unused_imports)]
pub use ids::*;
#[allow(unused_imports)]
pub use providers::*;
pub use session::LcmSession;
// Also re-export the session helper types
pub use session::{DescribeResult, SessionInfo};
// CompactionEngine is now nested under session::compaction
pub use session::compaction::CompactionEngine;
#[allow(unused_imports)]
pub use storage::*;
#[allow(unused_imports)]
pub use types::*;

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
