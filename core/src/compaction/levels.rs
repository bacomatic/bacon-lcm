// core/src/compaction/levels.rs
//! Per-level compaction implementations.
//!
//! Each function takes references to the storage layer and provider interfaces
//! and performs a single compaction pass at the designated level.

use crate::error::{CompactionError, CompactionOpResult};
use crate::ids::new_summary_id;
use crate::providers::Summarizer;
use crate::storage::{MessageStore, SummaryDag};
use crate::types::{
    CompactionConfig, CompactionResult, LineagePointer, Message, SessionId,
    SummaryLevel, SummaryNode,
};
use std::collections::HashMap;
use std::time::Instant;

// -------------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------------

/// Group messages into chunks of approximately `group_size` messages.
fn chunk_messages(messages: Vec<Message>, group_size: usize) -> Vec<Vec<Message>> {
    if messages.is_empty() || group_size == 0 {
        return Vec::new();
    }
    messages.chunks(group_size).map(|c| c.to_vec()).collect()
}

/// Group summary nodes into chunks of approximately `group_size` nodes.
fn chunk_summaries(summaries: Vec<SummaryNode>, group_size: usize) -> Vec<Vec<SummaryNode>> {
    if summaries.is_empty() || group_size == 0 {
        return Vec::new();
    }
    summaries.chunks(group_size).map(|c| c.to_vec()).collect()
}

// -------------------------------------------------------------------------
// L1 — Leaf compaction
// -------------------------------------------------------------------------

/// Perform L1 (leaf) compaction: summarize groups of raw messages into leaf
/// summary nodes.
///
/// The `fresh_tail_count` most recent messages are excluded from compaction so
/// that the model always has immediate conversational context.
///
/// Returns a [`CompactionResult`] describing what was created.
pub async fn l1_leaf_compaction(
    session_id: SessionId,
    config: &CompactionConfig,
    message_store: &dyn MessageStore,
    summary_dag: &dyn SummaryDag,
    summarizer: &dyn Summarizer,
) -> CompactionOpResult<CompactionResult> {
    let start = Instant::now();

    // Fetch all messages for the session, sorted by timestamp
    let mut messages = message_store.get_session_messages(session_id).await?;
    messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    if messages.is_empty() {
        return Err(CompactionError::NoMessagesToCompact);
    }

    // Exclude the fresh tail
    let compactable_count = messages.len().saturating_sub(config.fresh_tail_count);
    if compactable_count == 0 {
        return Err(CompactionError::NoMessagesToCompact);
    }

    let compactable: Vec<Message> = messages.into_iter().take(compactable_count).collect();
    let tokens_before: usize = compactable.iter().map(|m| m.token_count).sum();

    // Group into chunks
    let chunks = chunk_messages(compactable, config.leaf_group_size);
    if chunks.is_empty() {
        return Err(CompactionError::NoMessagesToCompact);
    }

    let mut summaries_created = Vec::new();
    let mut tokens_after: usize = 0;
    let mut total_messages_compacted: usize = 0;

    for chunk in chunks {
        let lineage: Vec<LineagePointer> = chunk
            .iter()
            .map(|m| LineagePointer::Message(m.id))
            .collect();
        let messages_in_chunk = chunk.len();

        // Ask the summarizer to produce a summary
        let summary_text = summarizer.summarize(&chunk).await?;
        let summary_token_count = summary_text.len() / 4; // Rough estimate

        let summary_node = SummaryNode {
            id: new_summary_id(),
            session_id,
            level: SummaryLevel::Leaf,
            content: summary_text,
            token_count: summary_token_count,
            lineage,
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
        };

        summary_dag.add_node(summary_node.clone()).await?;
        summaries_created.push(summary_node.id);
        tokens_after += summary_token_count;
        total_messages_compacted += messages_in_chunk;
    }

    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(CompactionResult {
        level: SummaryLevel::Leaf,
        summaries_created,
        messages_compacted: total_messages_compacted,
        tokens_before,
        tokens_after,
        duration_ms,
    })
}

// -------------------------------------------------------------------------
// L2 — Condensed compaction
// -------------------------------------------------------------------------

/// Perform L2 (condensed) compaction: merge existing leaf summaries into
/// higher-level condensed summary nodes.
///
/// This is invoked when L1 compaction alone was not sufficient to bring the
/// context window below the soft limit.
pub async fn l2_condensed_compaction(
    session_id: SessionId,
    config: &CompactionConfig,
    summary_dag: &dyn SummaryDag,
    summarizer: &dyn Summarizer,
) -> CompactionOpResult<CompactionResult> {
    let start = Instant::now();

    // Gather all Leaf-level summaries for the session
    let leaf_summaries = summary_dag
        .get_summaries_by_level(session_id, SummaryLevel::Leaf)
        .await?;

    if leaf_summaries.is_empty() {
        return Err(CompactionError::NoMessagesToCompact);
    }

    let tokens_before: usize = leaf_summaries.iter().map(|s| s.token_count).sum();

    // Group leaf summaries into chunks
    let chunks = chunk_summaries(leaf_summaries, config.condensed_group_size);
    if chunks.is_empty() {
        return Err(CompactionError::NoMessagesToCompact);
    }

    let mut summaries_created = Vec::new();
    let mut tokens_after: usize = 0;
    let mut total_messages_compacted: usize = 0;

    for chunk in chunks {
        // Build pseudo-messages from summary content for the summarizer
        let pseudo_messages: Vec<Message> = chunk
            .iter()
            .map(|s| Message {
                id: s.id, // reuse id for lineage tracking
                session_id,
                role: crate::types::MessageRole::System,
                content: s.content.clone(),
                timestamp: s.timestamp,
                token_count: s.token_count,
                metadata: HashMap::new(),
            })
            .collect();

        let lineage: Vec<LineagePointer> = chunk
            .iter()
            .map(|s| LineagePointer::Summary(s.id))
            .collect();
        let messages_in_chunk = chunk.len();

        let summary_text = summarizer.summarize(&pseudo_messages).await?;
        let summary_token_count = summary_text.len() / 4;

        let summary_node = SummaryNode {
            id: new_summary_id(),
            session_id,
            level: SummaryLevel::Condensed,
            content: summary_text,
            token_count: summary_token_count,
            lineage,
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
        };

        summary_dag.add_node(summary_node.clone()).await?;
        summaries_created.push(summary_node.id);
        tokens_after += summary_token_count;
        total_messages_compacted += messages_in_chunk;
    }

    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(CompactionResult {
        level: SummaryLevel::Condensed,
        summaries_created,
        messages_compacted: total_messages_compacted,
        tokens_before,
        tokens_after,
        duration_ms,
    })
}

// -------------------------------------------------------------------------
// L3 — Emergency compaction
// -------------------------------------------------------------------------

/// Perform L3 (emergency) compaction: aggressively reduce context size by
/// creating a terse deterministic stub from all existing summaries.
///
/// This level does **not** call the LLM summarizer — it simply concatenates
/// a fixed-format stub that references the archived summaries, ensuring the
/// context stays within the hard limit regardless of summarizer availability.
pub async fn l3_emergency_compaction(
    session_id: SessionId,
    _config: &CompactionConfig,
    summary_dag: &dyn SummaryDag,
) -> CompactionOpResult<CompactionResult> {
    let start = Instant::now();

    // Gather ALL summaries (leaf + condensed) for the session
    let all_summaries = summary_dag.get_session_summaries(session_id).await?;

    if all_summaries.is_empty() {
        return Err(CompactionError::NoMessagesToCompact);
    }

    let tokens_before: usize = all_summaries.iter().map(|s| s.token_count).sum();

    // Build lineage pointing to every existing summary
    let lineage: Vec<LineagePointer> = all_summaries
        .iter()
        .map(|s| LineagePointer::Summary(s.id))
        .collect();

    let archived_count = all_summaries.len();

    // Create a terse deterministic stub — no LLM call required
    let stub = format!(
        "[Emergency compaction: {} summary nodes archived. \
         Use lcm_describe / lcm_expand to retrieve original content.]",
        archived_count
    );
    let stub_token_count = stub.len() / 4;

    let emergency_node = SummaryNode {
        id: new_summary_id(),
        session_id,
        level: SummaryLevel::Emergency,
        content: stub,
        token_count: stub_token_count,
        lineage,
        timestamp: chrono::Utc::now(),
        metadata: HashMap::new(),
    };

    summary_dag.add_node(emergency_node.clone()).await?;

    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(CompactionResult {
        level: SummaryLevel::Emergency,
        summaries_created: vec![emergency_node.id],
        messages_compacted: archived_count,
        tokens_before,
        tokens_after: stub_token_count,
        duration_ms,
    })
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{new_message_id, new_session_id};
    use crate::providers::EchoSummarizer;
    use crate::storage::{InMemoryMessageStore, InMemorySummaryDag};
    use crate::types::{MessageRole, SummaryLevel, ThresholdConfig};
    use chrono::Utc;

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

    // -- chunk helpers ----------------------------------------------------

    #[test]
    fn test_chunk_messages_basic() {
        let sid = new_session_id();
        let msgs: Vec<Message> = (0..7)
            .map(|i| make_message(sid, &format!("msg {}", i), 10))
            .collect();
        let chunks = chunk_messages(msgs, 3);
        assert_eq!(chunks.len(), 3); // 3+3+1
        assert_eq!(chunks[0].len(), 3);
        assert_eq!(chunks[1].len(), 3);
        assert_eq!(chunks[2].len(), 1);
    }

    #[test]
    fn test_chunk_messages_empty() {
        let chunks = chunk_messages(Vec::new(), 5);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_summaries_basic() {
        let sid = new_session_id();
        let summaries: Vec<SummaryNode> = (0..5)
            .map(|i| SummaryNode {
                id: new_summary_id(),
                session_id: sid,
                level: SummaryLevel::Leaf,
                content: format!("summary {}", i),
                token_count: 50,
                lineage: vec![],
                timestamp: Utc::now(),
                metadata: HashMap::new(),
            })
            .collect();
        let chunks = chunk_summaries(summaries, 2);
        assert_eq!(chunks.len(), 3); // 2+2+1
    }

    // -- L1 tests ---------------------------------------------------------

    #[tokio::test]
    async fn test_l1_leaf_compaction_basic() {
        let config = test_config();
        let message_store = InMemoryMessageStore::new();
        let summary_dag = InMemorySummaryDag::new();
        let summarizer = EchoSummarizer::default();
        let session_id = new_session_id();

        // Store 7 messages; with fresh_tail_count=2, 5 are compactable
        for i in 0..7 {
            let msg = make_message(session_id, &format!("Message {}", i), 100);
            message_store.store(msg).await.unwrap();
        }

        let result = l1_leaf_compaction(
            session_id,
            &config,
            &message_store,
            &summary_dag,
            &summarizer,
        )
        .await
        .unwrap();

        assert_eq!(result.level, SummaryLevel::Leaf);
        // 5 compactable messages / group_size 3 = 2 chunks (3+2)
        assert_eq!(result.summaries_created.len(), 2);
        assert_eq!(result.messages_compacted, 5);
        assert!(result.tokens_before > 0);
        assert!(result.tokens_after < result.tokens_before);

        // Verify summaries were stored
        let stored = summary_dag
            .get_session_summaries(session_id)
            .await
            .unwrap();
        assert_eq!(stored.len(), 2);
        assert!(stored.iter().all(|s| s.level == SummaryLevel::Leaf));
    }

    #[tokio::test]
    async fn test_l1_no_messages() {
        let config = test_config();
        let message_store = InMemoryMessageStore::new();
        let summary_dag = InMemorySummaryDag::new();
        let summarizer = EchoSummarizer::default();
        let session_id = new_session_id();

        let result = l1_leaf_compaction(
            session_id,
            &config,
            &message_store,
            &summary_dag,
            &summarizer,
        )
        .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CompactionError::NoMessagesToCompact
        ));
    }

    #[tokio::test]
    async fn test_l1_all_in_fresh_tail() {
        let config = test_config(); // fresh_tail_count = 2
        let message_store = InMemoryMessageStore::new();
        let summary_dag = InMemorySummaryDag::new();
        let summarizer = EchoSummarizer::default();
        let session_id = new_session_id();

        // Only 2 messages — all in fresh tail
        for i in 0..2 {
            let msg = make_message(session_id, &format!("msg {}", i), 100);
            message_store.store(msg).await.unwrap();
        }

        let result = l1_leaf_compaction(
            session_id,
            &config,
            &message_store,
            &summary_dag,
            &summarizer,
        )
        .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CompactionError::NoMessagesToCompact
        ));
    }

    #[tokio::test]
    async fn test_l1_preserves_lineage() {
        let config = test_config();
        let message_store = InMemoryMessageStore::new();
        let summary_dag = InMemorySummaryDag::new();
        let summarizer = EchoSummarizer::default();
        let session_id = new_session_id();

        // Store 5 messages (3 compactable with fresh_tail=2)
        let mut msg_ids = Vec::new();
        for i in 0..5 {
            let msg = make_message(session_id, &format!("Message {}", i), 100);
            msg_ids.push(msg.id);
            message_store.store(msg).await.unwrap();
        }

        let result = l1_leaf_compaction(
            session_id,
            &config,
            &message_store,
            &summary_dag,
            &summarizer,
        )
        .await
        .unwrap();

        // Verify lineage points back to original messages
        for summary_id in &result.summaries_created {
            let node = summary_dag.get_node(*summary_id).await.unwrap().unwrap();
            assert!(!node.lineage.is_empty());
            for pointer in &node.lineage {
                match pointer {
                    LineagePointer::Message(mid) => {
                        assert!(msg_ids.contains(mid));
                    }
                    _ => panic!("L1 lineage should only point to messages"),
                }
            }
        }
    }

    // -- L2 tests ---------------------------------------------------------

    #[tokio::test]
    async fn test_l2_condensed_compaction_basic() {
        let config = test_config(); // condensed_group_size = 2
        let summary_dag = InMemorySummaryDag::new();
        let summarizer = EchoSummarizer::default();
        let session_id = new_session_id();

        // Manually add 4 leaf summaries
        for i in 0..4 {
            let node = SummaryNode {
                id: new_summary_id(),
                session_id,
                level: SummaryLevel::Leaf,
                content: format!("Leaf summary {}", i),
                token_count: 200,
                lineage: vec![],
                timestamp: Utc::now(),
                metadata: HashMap::new(),
            };
            summary_dag.add_node(node).await.unwrap();
        }

        let result = l2_condensed_compaction(
            session_id,
            &config,
            &summary_dag,
            &summarizer,
        )
        .await
        .unwrap();

        assert_eq!(result.level, SummaryLevel::Condensed);
        // 4 leaf summaries / condensed_group_size 2 = 2 condensed nodes
        assert_eq!(result.summaries_created.len(), 2);
        assert_eq!(result.messages_compacted, 4);
        assert!(result.tokens_before > 0);
        assert!(result.tokens_after < result.tokens_before);

        // Verify condensed summaries were stored
        let condensed = summary_dag
            .get_summaries_by_level(session_id, SummaryLevel::Condensed)
            .await
            .unwrap();
        assert_eq!(condensed.len(), 2);
    }

    #[tokio::test]
    async fn test_l2_no_leaf_summaries() {
        let config = test_config();
        let summary_dag = InMemorySummaryDag::new();
        let summarizer = EchoSummarizer::default();
        let session_id = new_session_id();

        let result = l2_condensed_compaction(
            session_id,
            &config,
            &summary_dag,
            &summarizer,
        )
        .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CompactionError::NoMessagesToCompact
        ));
    }

    #[tokio::test]
    async fn test_l2_lineage_points_to_summaries() {
        let config = test_config();
        let summary_dag = InMemorySummaryDag::new();
        let summarizer = EchoSummarizer::default();
        let session_id = new_session_id();

        let mut leaf_ids = Vec::new();
        for i in 0..3 {
            let node = SummaryNode {
                id: new_summary_id(),
                session_id,
                level: SummaryLevel::Leaf,
                content: format!("Leaf {}", i),
                token_count: 100,
                lineage: vec![],
                timestamp: Utc::now(),
                metadata: HashMap::new(),
            };
            leaf_ids.push(node.id);
            summary_dag.add_node(node).await.unwrap();
        }

        let result = l2_condensed_compaction(
            session_id,
            &config,
            &summary_dag,
            &summarizer,
        )
        .await
        .unwrap();

        for summary_id in &result.summaries_created {
            let node = summary_dag.get_node(*summary_id).await.unwrap().unwrap();
            for pointer in &node.lineage {
                match pointer {
                    LineagePointer::Summary(sid) => {
                        assert!(leaf_ids.contains(sid));
                    }
                    _ => panic!("L2 lineage should only point to summaries"),
                }
            }
        }
    }

    // -- L3 tests ---------------------------------------------------------

    #[tokio::test]
    async fn test_l3_emergency_compaction_basic() {
        let config = test_config();
        let summary_dag = InMemorySummaryDag::new();
        let session_id = new_session_id();

        // Add some leaf and condensed summaries
        for i in 0..3 {
            let node = SummaryNode {
                id: new_summary_id(),
                session_id,
                level: SummaryLevel::Leaf,
                content: format!("Leaf summary {}", i),
                token_count: 500,
                lineage: vec![],
                timestamp: Utc::now(),
                metadata: HashMap::new(),
            };
            summary_dag.add_node(node).await.unwrap();
        }

        let result = l3_emergency_compaction(session_id, &config, &summary_dag)
            .await
            .unwrap();

        assert_eq!(result.level, SummaryLevel::Emergency);
        assert_eq!(result.summaries_created.len(), 1);
        assert_eq!(result.messages_compacted, 3); // 3 summaries archived
        assert!(result.tokens_after < result.tokens_before);

        // Verify the emergency stub was created
        let emergency_id = result.summaries_created[0];
        let node = summary_dag.get_node(emergency_id).await.unwrap().unwrap();
        assert_eq!(node.level, SummaryLevel::Emergency);
        assert!(node.content.contains("Emergency compaction"));
        assert!(node.content.contains("3 summary nodes archived"));
    }

    #[tokio::test]
    async fn test_l3_no_summaries() {
        let config = test_config();
        let summary_dag = InMemorySummaryDag::new();
        let session_id = new_session_id();

        let result = l3_emergency_compaction(session_id, &config, &summary_dag).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CompactionError::NoMessagesToCompact
        ));
    }

    #[tokio::test]
    async fn test_l3_creates_deterministic_stub() {
        let config = test_config();
        let summary_dag = InMemorySummaryDag::new();
        let session_id = new_session_id();

        // Add 5 summaries
        for i in 0..5 {
            let node = SummaryNode {
                id: new_summary_id(),
                session_id,
                level: SummaryLevel::Leaf,
                content: format!("Summary content {}", i),
                token_count: 100,
                lineage: vec![],
                timestamp: Utc::now(),
                metadata: HashMap::new(),
            };
            summary_dag.add_node(node).await.unwrap();
        }

        let result = l3_emergency_compaction(session_id, &config, &summary_dag)
            .await
            .unwrap();

        let node = summary_dag
            .get_node(result.summaries_created[0])
            .await
            .unwrap()
            .unwrap();

        // Stub should mention the exact number of archived nodes
        assert!(node.content.contains("5 summary nodes archived"));
        // Stub should mention how to retrieve content
        assert!(node.content.contains("lcm_describe"));
        assert!(node.content.contains("lcm_expand"));
    }

    #[tokio::test]
    async fn test_l3_lineage_covers_all_summaries() {
        let config = test_config();
        let summary_dag = InMemorySummaryDag::new();
        let session_id = new_session_id();

        let mut all_ids = Vec::new();
        for i in 0..4 {
            let node = SummaryNode {
                id: new_summary_id(),
                session_id,
                level: SummaryLevel::Leaf,
                content: format!("S{}", i),
                token_count: 50,
                lineage: vec![],
                timestamp: Utc::now(),
                metadata: HashMap::new(),
            };
            all_ids.push(node.id);
            summary_dag.add_node(node).await.unwrap();
        }

        let result = l3_emergency_compaction(session_id, &config, &summary_dag)
            .await
            .unwrap();

        let emergency_node = summary_dag
            .get_node(result.summaries_created[0])
            .await
            .unwrap()
            .unwrap();

        // Emergency node lineage should reference all existing summaries
        let lineage_summary_ids: Vec<_> = emergency_node
            .lineage
            .iter()
            .filter_map(|p| match p {
                LineagePointer::Summary(id) => Some(*id),
                _ => None,
            })
            .collect();

        for id in &all_ids {
            assert!(
                lineage_summary_ids.contains(id),
                "Emergency lineage missing summary {}",
                id
            );
        }
    }
}
