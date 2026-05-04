// core/src/session/mod.rs
//! Session management module.
//!
//! Sub-modules:
//! - [`compaction`] – `CompactionEngine` for three-level escalation
//! - [`context`]    – `ContextAssembler` for building the active context window
//! - [`core`]       – `SessionCore` with low-level session state & storage ops
//! - [`manager`]    – `SessionManager` with the full public API + compaction lock

pub mod compaction;
pub mod context;
pub mod core;
pub mod manager;

use crate::types::{LineagePointer, Session, SummaryNode};

/// Result of describing a summary node and its lineage.
#[derive(Debug, Clone)]
pub struct DescribeResult {
    pub summary: SummaryNode,
    pub lineage: Vec<LineagePointer>,
    pub reachable_message_count: usize,
}

/// Snapshot of high-level session statistics.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub session: Session,
    pub message_count: usize,
    pub token_count: usize,
    pub summary_count: usize,
    pub is_compacting: bool,
}

// Re-export the public API type so `use bacon_lcm_core::session::LcmSession` works.
pub use manager::SessionManager as LcmSession;
