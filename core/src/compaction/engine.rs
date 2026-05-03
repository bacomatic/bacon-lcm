// core/src/compaction/engine.rs
//! Main `CompactionEngine` — the public entry point for compaction operations.
//!
//! The engine owns the compaction configuration and delegates to the level
//! implementations in [`super::levels`].  It uses the strategy selector from
//! [`super::strategy`] to decide which level to attempt first and whether to
//! escalate after each pass.

use crate::error::CompactionOpResult;
use crate::providers::Summarizer;
use crate::storage::{MessageStore, SummaryDag};
use crate::types::{CompactionConfig, CompactionResult, SessionId};

use super::levels::{l1_leaf_compaction, l2_condensed_compaction, l3_emergency_compaction};
use super::strategy::{select_post_l1_level, CompactionLevel};

/// Three-level compaction engine for LCM context memory management.
///
/// The engine implements the escalation protocol:
///
/// 1. **L1 (Leaf)** — summarize groups of raw messages into leaf nodes
/// 2. **L2 (Condensed)** — merge leaf summaries into condensed nodes
/// 3. **L3 (Emergency)** — deterministic archival with no LLM call
///
/// The engine does *not* own storage or provider references; callers pass
/// them in per-operation so the engine can remain cheaply clonable.
pub struct CompactionEngine {
    config: CompactionConfig,
}

impl CompactionEngine {
    /// Create a new `CompactionEngine` with the given configuration.
    pub fn new(config: CompactionConfig) -> Self {
        Self { config }
    }

    /// Get the current compaction configuration.
    pub fn config(&self) -> &CompactionConfig {
        &self.config
    }

    /// Perform standard (L1 / L2) compaction.
    ///
    /// Starts with L1 (leaf) compaction.  If the context is still over the
    /// soft limit after L1, escalates to L2 (condensed).
    ///
    /// The caller supplies references to storage and provider interfaces so
    /// the engine remains decoupled from concrete implementations.
    pub async fn compact(
        &self,
        session_id: SessionId,
        message_store: &dyn MessageStore,
        summary_dag: &dyn SummaryDag,
        summarizer: &dyn Summarizer,
    ) -> CompactionOpResult<CompactionResult> {
        // Attempt L1 first
        let l1_result = l1_leaf_compaction(
            session_id,
            &self.config,
            message_store,
            summary_dag,
            summarizer,
        )
        .await?;

        // Estimate remaining tokens to decide whether to escalate.
        // After L1 the remaining token count is approximately:
        //   original_tokens - (tokens_before_compaction - tokens_after_compaction)
        let tokens_saved = l1_result.tokens_before.saturating_sub(l1_result.tokens_after);
        let estimated_tokens = self
            .config
            .thresholds
            .soft_limit
            .saturating_add(1) // we know we were over the soft limit
            .saturating_sub(tokens_saved);

        let next_level = select_post_l1_level(estimated_tokens, &self.config);

        match next_level {
            CompactionLevel::None | CompactionLevel::Leaf => Ok(l1_result),
            CompactionLevel::Condensed => {
                // Escalate to L2
                let l2_result = l2_condensed_compaction(
                    session_id,
                    &self.config,
                    summary_dag,
                    summarizer,
                )
                .await;

                match l2_result {
                    Ok(l2) => Ok(merge_results(l1_result, l2)),
                    Err(_) => Ok(l1_result), // L2 failure is non-fatal
                }
            }
            CompactionLevel::Emergency => {
                // L1 wasn't enough and we're over the hard limit
                let l3_result =
                    l3_emergency_compaction(session_id, &self.config, summary_dag).await;

                match l3_result {
                    Ok(l3) => Ok(merge_results(l1_result, l3)),
                    Err(_) => Ok(l1_result),
                }
            }
        }
    }

    /// Perform emergency (L3) compaction directly.
    ///
    /// This bypasses L1/L2 and immediately creates a deterministic emergency
    /// stub, archiving all existing summaries.  Use this when the hard limit
    /// has already been exceeded.
    pub async fn emergency_compaction(
        &self,
        session_id: SessionId,
        summary_dag: &dyn SummaryDag,
    ) -> CompactionOpResult<CompactionResult> {
        l3_emergency_compaction(session_id, &self.config, summary_dag).await
    }
}

/// Merge two compaction results into one, combining their statistics.
fn merge_results(a: CompactionResult, b: CompactionResult) -> CompactionResult {
    CompactionResult {
        level: b.level, // report the highest level reached
        summaries_created: {
            let mut combined = a.summaries_created;
            combined.extend(b.summaries_created);
            combined
        },
        messages_compacted: a.messages_compacted + b.messages_compacted,
        tokens_before: a.tokens_before + b.tokens_before,
        tokens_after: a.tokens_after + b.tokens_after,
        duration_ms: a.duration_ms + b.duration_ms,
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CompactionError;
    use crate::ids::{new_message_id, new_session_id, new_summary_id};
    use crate::providers::EchoSummarizer;
    use crate::storage::{InMemoryMessageStore, InMemorySummaryDag};
    use crate::types::{
        CompactionConfig, Message, MessageRole, SummaryLevel, SummaryNode, ThresholdConfig,
    };
    use chrono::Utc;
    use std::collections::HashMap;

    fn test_config() -> CompactionConfig {
        CompactionConfig {
            thresholds: ThresholdConfig {
                model_max_tokens: 128000,
                soft_limit: 80000,
                hard_limit: 110000,
            },
            fresh_tail_count: 2,
            leaf_group_size: 3,
            condensed_group_size: 2,
            parallel_compaction: false,
            max_concurrent_compactions: 1,
        }
    }

    fn make_message(session_id: SessionId, content: &str, token_count: usize) -> Message {
        Message {
            id: new_message_id(),
            session_id,
            role: MessageRole::User,
            content: content.to_string(),
            timestamp: Utc::now(),
            token_count,
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn test_engine_creation() {
        let config = test_config();
        let engine = CompactionEngine::new(config.clone());
        assert_eq!(
            engine.config().thresholds.soft_limit,
            config.thresholds.soft_limit
        );
        assert_eq!(
            engine.config().thresholds.hard_limit,
            config.thresholds.hard_limit
        );
    }

    #[tokio::test]
    async fn test_compact_with_messages() {
        let config = test_config();
        let engine = CompactionEngine::new(config);
        let message_store = InMemoryMessageStore::new();
        let summary_dag = InMemorySummaryDag::new();
        let summarizer = EchoSummarizer::default();
        let session_id = new_session_id();

        // Store enough messages for compaction (> fresh_tail_count)
        for i in 0..8 {
            let msg = make_message(session_id, &format!("Test message {}", i), 100);
            message_store.store(msg).await.unwrap();
        }

        let result = engine
            .compact(session_id, &message_store, &summary_dag, &summarizer)
            .await
            .unwrap();

        assert_eq!(result.level, SummaryLevel::Leaf);
        assert!(!result.summaries_created.is_empty());
        assert!(result.messages_compacted > 0);
    }

    #[tokio::test]
    async fn test_compact_no_messages() {
        let config = test_config();
        let engine = CompactionEngine::new(config);
        let message_store = InMemoryMessageStore::new();
        let summary_dag = InMemorySummaryDag::new();
        let summarizer = EchoSummarizer::default();
        let session_id = new_session_id();

        let result = engine
            .compact(session_id, &message_store, &summary_dag, &summarizer)
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CompactionError::NoMessagesToCompact
        ));
    }

    #[tokio::test]
    async fn test_emergency_compaction_with_summaries() {
        let config = test_config();
        let engine = CompactionEngine::new(config);
        let summary_dag = InMemorySummaryDag::new();
        let session_id = new_session_id();

        // Add some summaries to archive
        for i in 0..3 {
            let node = SummaryNode {
                id: new_summary_id(),
                session_id,
                level: SummaryLevel::Leaf,
                content: format!("Summary {}", i),
                token_count: 200,
                lineage: vec![],
                timestamp: Utc::now(),
                metadata: HashMap::new(),
            };
            summary_dag.add_node(node).await.unwrap();
        }

        let result = engine
            .emergency_compaction(session_id, &summary_dag)
            .await
            .unwrap();

        assert_eq!(result.level, SummaryLevel::Emergency);
        assert_eq!(result.summaries_created.len(), 1);
        assert_eq!(result.messages_compacted, 3);
    }

    #[tokio::test]
    async fn test_emergency_compaction_no_summaries() {
        let config = test_config();
        let engine = CompactionEngine::new(config);
        let summary_dag = InMemorySummaryDag::new();
        let session_id = new_session_id();

        let result = engine
            .emergency_compaction(session_id, &summary_dag)
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CompactionError::NoMessagesToCompact
        ));
    }

    #[test]
    fn test_merge_results() {
        let a = CompactionResult {
            level: SummaryLevel::Leaf,
            summaries_created: vec![new_summary_id()],
            messages_compacted: 5,
            tokens_before: 1000,
            tokens_after: 200,
            duration_ms: 50,
        };
        let b = CompactionResult {
            level: SummaryLevel::Condensed,
            summaries_created: vec![new_summary_id()],
            messages_compacted: 3,
            tokens_before: 400,
            tokens_after: 100,
            duration_ms: 30,
        };

        let merged = merge_results(a, b);
        assert_eq!(merged.level, SummaryLevel::Condensed);
        assert_eq!(merged.summaries_created.len(), 2);
        assert_eq!(merged.messages_compacted, 8);
        assert_eq!(merged.tokens_before, 1400);
        assert_eq!(merged.tokens_after, 300);
        assert_eq!(merged.duration_ms, 80);
    }
}
