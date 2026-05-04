// core/src/session/compaction.rs
//! Re-exports the [`CompactionEngine`] from [`crate::compaction`] so that
//! existing code that imports via `session::compaction::CompactionEngine`
//! continues to work without modification.

pub use crate::compaction::CompactionEngine;
