// core/src/compaction/mod.rs
//! Three-level compaction engine for context memory management.
//!
//! This module implements the LCM three-level escalation protocol:
//!
//! - **L1 (Leaf)**: Groups of recent raw messages are summarized into leaf
//!   summary nodes, preserving full lineage back to the originals.
//! - **L2 (Condensed)**: Multiple leaf summaries are merged into higher-level
//!   condensed nodes when L1 alone is insufficient.
//! - **L3 (Emergency)**: Aggressive, deterministic compaction that archives
//!   the oldest summaries and creates a terse stub — no LLM call required.
//!
//! Sub-modules:
//! - [`engine`]   – `CompactionEngine`, the main entry point
//! - [`levels`]   – Per-level compaction implementations (L1, L2, L3)
//! - [`strategy`] – Strategy selection logic based on token thresholds

pub mod engine;
pub mod levels;
pub mod strategy;

pub use engine::CompactionEngine;
