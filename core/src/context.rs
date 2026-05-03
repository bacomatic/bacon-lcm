// core/src/context.rs
use crate::error::LcmResult;
use crate::storage::{MessageStore, SummaryDag};
use crate::types::*;

/// Assembles the active context window from messages and summaries
pub struct ContextAssembler {
    fresh_tail_count: usize,
}

impl ContextAssembler {
    pub fn new(fresh_tail_count: usize) -> Self {
        Self { fresh_tail_count }
    }

    /// Assemble the active context for a session
    pub async fn assemble_context(
        &self,
        session_id: SessionId,
        message_store: &dyn MessageStore,
        summary_dag: &dyn SummaryDag,
    ) -> LcmResult<Vec<ContextItem>> {
        // Get all messages
        let mut messages = message_store.get_session_messages(session_id).await?;

        // Get all summaries
        let mut summaries = summary_dag.get_session_summaries(session_id).await?;

        // Sort by timestamp
        messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        summaries.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

        // Take fresh tail (most recent messages)
        let fresh_tail: Vec<ContextItem> = messages
            .split_off(messages.len().saturating_sub(self.fresh_tail_count))
            .into_iter()
            .map(ContextItem::Message)
            .collect();

        // Convert remaining messages to context items
        let historical_messages: Vec<ContextItem> = messages
            .into_iter()
            .map(ContextItem::Message)
            .collect();

        // Combine historical messages with summaries
        let mut context = Vec::new();
        context.extend(historical_messages);
        context.extend(summaries.into_iter().map(ContextItem::Summary));

        // Sort everything by timestamp
        context.sort_by(|a, b| a.timestamp().cmp(&b.timestamp()));

        // Add fresh tail at the end
        context.extend(fresh_tail);

        Ok(context)
    }

    /// Get token count for the active context
    pub async fn get_context_token_count(
        &self,
        session_id: SessionId,
        message_store: &dyn MessageStore,
        summary_dag: &dyn SummaryDag,
    ) -> LcmResult<usize> {
        let context = self.assemble_context(session_id, message_store, summary_dag).await?;
        Ok(context.iter().map(|item| item.token_count()).sum())
    }

    /// Check if compaction is needed
    pub async fn needs_compaction(
        &self,
        session_id: SessionId,
        message_store: &dyn MessageStore,
        summary_dag: &dyn SummaryDag,
        soft_limit: usize,
        _hard_limit: usize,
    ) -> LcmResult<bool> {
        let token_count = self
            .get_context_token_count(session_id, message_store, summary_dag)
            .await?;
        Ok(token_count > soft_limit)
    }

    /// Check if emergency compaction is needed
    pub async fn needs_emergency_compaction(
        &self,
        session_id: SessionId,
        message_store: &dyn MessageStore,
        summary_dag: &dyn SummaryDag,
        hard_limit: usize,
    ) -> LcmResult<bool> {
        let token_count = self
            .get_context_token_count(session_id, message_store, summary_dag)
            .await?;
        Ok(token_count > hard_limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{new_message_id, new_session_id, new_summary_id};
    use crate::storage::{InMemoryMessageStore, InMemorySummaryDag};
    use crate::types::{MessageRole, SummaryLevel};
    use chrono::{Duration, Utc};
    use std::collections::HashMap;

    fn create_test_message(session_id: SessionId, content: &str, hours_ago: i64) -> Message {
        Message {
            id: new_message_id(),
            session_id,
            role: MessageRole::User,
            content: content.to_string(),
            timestamp: Utc::now() - Duration::hours(hours_ago),
            token_count: content.len() / 4,
            metadata: HashMap::new(),
        }
    }

    fn create_test_summary(session_id: SessionId, hours_ago: i64) -> SummaryNode {
        SummaryNode {
            id: new_summary_id(),
            session_id,
            level: SummaryLevel::Leaf,
            content: "Test summary".to_string(),
            token_count: 50,
            lineage: vec![],
            timestamp: Utc::now() - Duration::hours(hours_ago),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_context_assembly() {
        let assembler = ContextAssembler::new(2);
        let message_store = InMemoryMessageStore::new();
        let summary_dag = InMemorySummaryDag::new();
        let session_id = new_session_id();

        // Create messages at different times
        let msg1 = create_test_message(session_id, "Message 1", 5);
        let msg2 = create_test_message(session_id, "Message 2", 4);
        let msg3 = create_test_message(session_id, "Message 3", 3);
        let msg4 = create_test_message(session_id, "Message 4", 2);
        let msg5 = create_test_message(session_id, "Message 5", 1);

        // Store messages
        message_store.store(msg1).await.unwrap();
        message_store.store(msg2).await.unwrap();
        message_store.store(msg3).await.unwrap();
        message_store.store(msg4).await.unwrap();
        message_store.store(msg5).await.unwrap();

        // Create and store summary
        let summary = create_test_summary(session_id, 2);
        summary_dag.add_node(summary).await.unwrap();

        // Assemble context
        let context = assembler
            .assemble_context(session_id, &message_store, &summary_dag)
            .await
            .unwrap();

        // Should have: 3 historical messages + 1 summary + 2 fresh tail = 6 items
        assert_eq!(context.len(), 6);

        // Fresh tail should be the last 2 messages
        if let Some(ContextItem::Message(msg)) = context.last() {
            assert_eq!(msg.content, "Message 5");
        }
        if let Some(ContextItem::Message(msg)) = context.get(context.len() - 2) {
            assert_eq!(msg.content, "Message 4");
        }
    }

    #[tokio::test]
    async fn test_compaction_needed() {
        let assembler = ContextAssembler::new(2);
        let message_store = InMemoryMessageStore::new();
        let summary_dag = InMemorySummaryDag::new();
        let session_id = new_session_id();

        // Create a message whose naive token count (len/4) is 25_000.
        // Use a soft_limit of 20_000 (< 25_000) to trigger compaction,
        // and a hard_limit of 30_000 (> 25_000) so emergency is not triggered.
        let long_message = create_test_message(session_id, &"x".repeat(100000), 0);
        message_store.store(long_message).await.unwrap();

        // Check if compaction is needed (soft limit 20000)
        let needs_compaction = assembler
            .needs_compaction(
                session_id,
                &message_store,
                &summary_dag,
                20000,
                30000,
            )
            .await
            .unwrap();

        assert!(needs_compaction);

        // Check emergency compaction (hard limit 30000 > 25000 token count)
        let needs_emergency = assembler
            .needs_emergency_compaction(
                session_id,
                &message_store,
                &summary_dag,
                30000,
            )
            .await
            .unwrap();

        assert!(!needs_emergency); // Should not exceed hard limit
    }

    #[tokio::test]
    async fn test_empty_context() {
        let assembler = ContextAssembler::new(10);
        let message_store = InMemoryMessageStore::new();
        let summary_dag = InMemorySummaryDag::new();
        let session_id = new_session_id();

        let context = assembler
            .assemble_context(session_id, &message_store, &summary_dag)
            .await
            .unwrap();

        assert!(context.is_empty());
    }

    #[tokio::test]
    async fn test_token_count() {
        let assembler = ContextAssembler::new(10);
        let message_store = InMemoryMessageStore::new();
        let summary_dag = InMemorySummaryDag::new();
        let session_id = new_session_id();

        // Each "x" = 1 char, token_count = content.len() / 4 = 100
        let msg = create_test_message(session_id, &"x".repeat(400), 0);
        message_store.store(msg).await.unwrap();

        let token_count = assembler
            .get_context_token_count(session_id, &message_store, &summary_dag)
            .await
            .unwrap();

        assert_eq!(token_count, 100);
    }
}
