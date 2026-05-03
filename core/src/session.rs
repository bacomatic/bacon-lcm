// core/src/session.rs
use crate::compaction::CompactionEngine;
use crate::config::LcmConfig;
use crate::context::ContextAssembler;
use crate::error::{LcmError, LcmResult};
use crate::ids::{new_message_id, new_session_id};
use crate::providers::{Embedder, ProviderRegistry, Summarizer, TokenCounter};
use crate::storage::{StorageLayer, VectorRecord};
use crate::types::*;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Main LCM session orchestrator
pub struct LcmSession {
    pub session: Session,
    storage: StorageLayer,
    providers: ProviderRegistry,
    config: LcmConfig,
    context_assembler: ContextAssembler,
    compaction_engine: Arc<CompactionEngine>,
    is_compacting: Arc<RwLock<bool>>,
}

impl LcmSession {
    /// Create a new LCM session
    pub async fn new(
        token_counter: Box<dyn TokenCounter>,
        summarizer: Box<dyn Summarizer>,
        embedder: Box<dyn Embedder>,
        config: LcmConfig,
        storage: StorageLayer,
    ) -> LcmResult<Self> {
        let session = Session {
            id: new_session_id(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: std::collections::HashMap::new(),
        };

        // Store the session
        storage.sessions.create(session.clone()).await?;

        let providers = ProviderRegistry::new(token_counter, summarizer, embedder);
        let context_assembler = ContextAssembler::new(config.compaction.fresh_tail_count);
        let compaction_engine = Arc::new(CompactionEngine::new(
            config.compaction.clone(),
        ));

        Ok(Self {
            session,
            storage,
            providers,
            config,
            context_assembler,
            compaction_engine,
            is_compacting: Arc::new(RwLock::new(false)),
        })
    }

    /// Add a message to the session
    pub async fn add_message(
        &mut self,
        role: MessageRole,
        content: String,
    ) -> LcmResult<MessageId> {
        // Count tokens
        let token_count = self.providers.token_counter.count(&content).await?;

        // Create message
        let message = Message {
            id: new_message_id(),
            session_id: self.session.id,
            role,
            content,
            timestamp: chrono::Utc::now(),
            token_count,
            metadata: std::collections::HashMap::new(),
        };

        // Store message
        let message_id = self.storage.messages.store(message.clone()).await?;

        // Generate and store embedding if embedder is configured
        if !self.is_null_embedder() {
            if let Ok(embedding) = self.providers.embedder.embed(&message.content).await {
                let record = VectorRecord {
                    id: Uuid::new_v4(),
                    session_id: self.session.id,
                    embedding,
                    content: message.content.clone(),
                    metadata: std::collections::HashMap::new(),
                };
                let _ = self.storage.vectors.store(record).await;
            }
        }

        // Update session timestamp
        self.session.updated_at = chrono::Utc::now();
        let _ = self.storage.sessions.update(self.session.clone()).await;

        // Check if compaction is needed
        if self
            .context_assembler
            .needs_compaction(
                self.session.id,
                &*self.storage.messages,
                &*self.storage.summaries,
                self.config.compaction.thresholds.soft_limit,
                self.config.compaction.thresholds.hard_limit,
            )
            .await?
        {
            self.trigger_compaction().await?;
        }

        Ok(message_id)
    }

    /// Get the active context window
    pub async fn get_context(&self) -> LcmResult<Vec<ContextItem>> {
        self.context_assembler
            .assemble_context(
                self.session.id,
                &*self.storage.messages,
                &*self.storage.summaries,
            )
            .await
    }

    /// Get current token count
    pub async fn get_token_count(&self) -> LcmResult<usize> {
        self.context_assembler
            .get_context_token_count(
                self.session.id,
                &*self.storage.messages,
                &*self.storage.summaries,
            )
            .await
    }

    /// Get message count
    pub async fn get_message_count(&self) -> LcmResult<usize> {
        self.storage
            .messages
            .get_message_count(self.session.id)
            .await
            .map_err(LcmError::Storage)
    }

    /// Describe a summary node
    pub async fn describe(&self, summary_id: SummaryId) -> LcmResult<DescribeResult> {
        let summary = self
            .storage
            .summaries
            .get_node(summary_id)
            .await?
            .ok_or(LcmError::SummaryNotFound(summary_id))?;

        let lineage = self.storage.summaries.get_lineage(summary_id).await?;
        let reachable_message_count = self.count_reachable_messages(&lineage).await?;

        Ok(DescribeResult {
            summary,
            lineage,
            reachable_message_count,
        })
    }

    /// Expand a summary to original messages
    pub async fn expand(&self, summary_id: SummaryId) -> LcmResult<Vec<Message>> {
        self.storage
            .summaries
            .expand(summary_id, &*self.storage.messages)
            .await
            .map_err(LcmError::Storage)
    }

    /// Search for similar messages (if embeddings are enabled)
    pub async fn search(&self, query: &str, limit: usize) -> LcmResult<Vec<VectorRecord>> {
        if self.is_null_embedder() {
            return Ok(Vec::new());
        }

        let query_embedding = self.providers.embedder.embed(query).await?;
        self.storage
            .vectors
            .search(self.session.id, &query_embedding, limit)
            .await
            .map_err(LcmError::Storage)
    }

    /// Get session information
    pub async fn get_session_info(&self) -> LcmResult<SessionInfo> {
        let message_count = self.get_message_count().await?;
        let token_count = self.get_token_count().await?;
        let summary_count = self
            .storage
            .summaries
            .get_session_summaries(self.session.id)
            .await?
            .len();

        Ok(SessionInfo {
            session: self.session.clone(),
            message_count,
            token_count,
            summary_count,
            is_compacting: *self.is_compacting.read().await,
        })
    }

    /// Trigger compaction if needed
    async fn trigger_compaction(&self) -> LcmResult<()> {
        // Check if already compacting
        {
            let mut is_compacting = self.is_compacting.write().await;
            if *is_compacting {
                return Ok(());
            }
            *is_compacting = true;
        }

        let result = self.perform_compaction().await;

        // Reset compaction flag
        {
            let mut is_compacting = self.is_compacting.write().await;
            *is_compacting = false;
        }

        result
    }

    /// Perform the actual compaction
    async fn perform_compaction(&self) -> LcmResult<()> {
        // Check if emergency compaction is needed
        let needs_emergency = self
            .context_assembler
            .needs_emergency_compaction(
                self.session.id,
                &*self.storage.messages,
                &*self.storage.summaries,
                self.config.compaction.thresholds.hard_limit,
            )
            .await?;

        if needs_emergency {
            self.compaction_engine
                .emergency_compaction(self.session.id)
                .await?;
        } else {
            self.compaction_engine.compact(self.session.id).await?;
        }

        Ok(())
    }

    /// Count reachable messages from lineage pointers
    async fn count_reachable_messages(
        &self,
        lineage: &[LineagePointer],
    ) -> LcmResult<usize> {
        let mut count = 0;

        for pointer in lineage {
            match pointer {
                LineagePointer::Message(_) => count += 1,
                LineagePointer::Summary(summary_id) => {
                    let nested_lineage =
                        self.storage.summaries.get_lineage(*summary_id).await?;
                    count += Box::pin(self.count_reachable_messages(&nested_lineage)).await?;
                }
            }
        }

        Ok(count)
    }

    /// Check if embedder is null (no embeddings)
    fn is_null_embedder(&self) -> bool {
        self.providers.embedder.name() == "null"
    }

    /// Restore a session from storage
    pub async fn restore(
        session_id: SessionId,
        token_counter: Box<dyn TokenCounter>,
        summarizer: Box<dyn Summarizer>,
        embedder: Box<dyn Embedder>,
        config: LcmConfig,
        storage: StorageLayer,
    ) -> LcmResult<Self> {
        let session = storage
            .sessions
            .get(session_id)
            .await?
            .ok_or(LcmError::SessionNotFound(session_id))?;

        let providers = ProviderRegistry::new(token_counter, summarizer, embedder);
        let context_assembler = ContextAssembler::new(config.compaction.fresh_tail_count);
        let compaction_engine = Arc::new(CompactionEngine::new(
            config.compaction.clone(),
        ));

        Ok(Self {
            session,
            storage,
            providers,
            config,
            context_assembler,
            compaction_engine,
            is_compacting: Arc::new(RwLock::new(false)),
        })
    }
}

/// Result of describing a summary
#[derive(Debug, Clone)]
pub struct DescribeResult {
    pub summary: SummaryNode,
    pub lineage: Vec<LineagePointer>,
    pub reachable_message_count: usize,
}

/// Session information
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub session: Session,
    pub message_count: usize,
    pub token_count: usize,
    pub summary_count: usize,
    pub is_compacting: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LcmConfig;
    use crate::providers::{create_embedder, create_summarizer, create_token_counter};
    use crate::storage::StorageLayer;

    async fn create_test_session() -> LcmSession {
        let config = LcmConfig::defaults();
        let storage = StorageLayer::memory();
        let token_counter = create_token_counter("naive", None).unwrap();
        let summarizer =
            create_summarizer("echo", "echo".to_string(), None, None, None, None).unwrap();
        let embedder = create_embedder("null", None, None, None, None).unwrap();

        LcmSession::new(token_counter, summarizer, embedder, config, storage)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_session_creation() {
        let session = create_test_session().await;

        assert_eq!(session.get_message_count().await.unwrap(), 0);
        assert_eq!(session.get_token_count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_add_message() {
        let mut session = create_test_session().await;

        let message_id = session
            .add_message(MessageRole::User, "Hello world".to_string())
            .await
            .unwrap();

        assert_eq!(session.get_message_count().await.unwrap(), 1);
        assert!(session.get_token_count().await.unwrap() > 0);

        let context = session.get_context().await.unwrap();
        assert_eq!(context.len(), 1);

        if let ContextItem::Message(msg) = &context[0] {
            assert_eq!(msg.content, "Hello world");
            assert_eq!(msg.id, message_id);
        } else {
            panic!("Expected Message context item");
        }
    }

    #[tokio::test]
    async fn test_add_multiple_messages() {
        let mut session = create_test_session().await;

        session
            .add_message(MessageRole::User, "First message".to_string())
            .await
            .unwrap();
        session
            .add_message(MessageRole::Assistant, "Second message".to_string())
            .await
            .unwrap();
        session
            .add_message(MessageRole::User, "Third message".to_string())
            .await
            .unwrap();

        assert_eq!(session.get_message_count().await.unwrap(), 3);

        let context = session.get_context().await.unwrap();
        assert_eq!(context.len(), 3);
    }

    #[tokio::test]
    async fn test_session_info() {
        let mut session = create_test_session().await;

        session
            .add_message(MessageRole::User, "Hello".to_string())
            .await
            .unwrap();

        let info = session.get_session_info().await.unwrap();
        assert_eq!(info.message_count, 1);
        assert!(info.token_count > 0);
        assert_eq!(info.summary_count, 0);
        assert!(!info.is_compacting);
    }

    #[tokio::test]
    async fn test_session_restore_not_found() {
        // Attempting to restore a session that does not exist should return an error
        use crate::ids::new_session_id;

        let config = LcmConfig::defaults();
        let storage = StorageLayer::memory();
        let token_counter = create_token_counter("naive", None).unwrap();
        let summarizer =
            create_summarizer("echo", "echo".to_string(), None, None, None, None).unwrap();
        let embedder = create_embedder("null", None, None, None, None).unwrap();

        let nonexistent_id = new_session_id();

        let result = LcmSession::restore(
            nonexistent_id,
            token_counter,
            summarizer,
            embedder,
            config,
            storage,
        )
        .await;

        assert!(result.is_err());
        if let Err(LcmError::SessionNotFound(id)) = result {
            assert_eq!(id, nonexistent_id);
        } else {
            panic!("Expected SessionNotFound error");
        }
    }

    #[tokio::test]
    async fn test_search_with_null_embedder() {
        let session = create_test_session().await;
        let results = session.search("test query", 5).await.unwrap();
        assert!(results.is_empty());
    }
}
