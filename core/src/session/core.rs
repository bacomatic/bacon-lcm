// core/src/session/core.rs
use crate::error::{LcmError, LcmResult};
use crate::ids::{new_message_id, new_session_id};
use crate::providers::{Embedder, ProviderRegistry, Summarizer, TokenCounter};
use crate::session::compaction::CompactionEngine;
use crate::session::context::ContextAssembler;
use crate::storage::{StorageLayer, VectorRecord};
use crate::types::{
    ContextItem, LineagePointer, Message, MessageId, MessageRole, Session, SessionId, SummaryId,
};
use crate::config::LcmConfig;
use super::DescribeResult;
use std::sync::Arc;
use uuid::Uuid;

/// Core session state: session data, storage access, and helper components.
///
/// `SessionCore` is responsible for low-level operations — storing messages,
/// generating embeddings, assembling context, and deciding when compaction is
/// required. Higher-level orchestration (e.g. the compaction lock) lives in
/// [`crate::session::manager::SessionManager`] / the public [`LcmSession`] wrapper.
pub struct SessionCore {
    pub session: Session,
    pub storage: StorageLayer,
    pub providers: ProviderRegistry,
    pub config: LcmConfig,
    pub context_assembler: ContextAssembler,
    pub compaction_engine: Arc<CompactionEngine>,
}

impl SessionCore {
    /// Initialise a brand-new session and persist it to storage.
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

        storage.sessions.create(session.clone()).await?;

        let providers = ProviderRegistry::new(token_counter, summarizer, embedder);
        let context_assembler = ContextAssembler::new(config.compaction.fresh_tail_count);
        let compaction_engine = Arc::new(CompactionEngine::new(config.compaction.clone()));

        Ok(Self {
            session,
            storage,
            providers,
            config,
            context_assembler,
            compaction_engine,
        })
    }

    /// Restore a `SessionCore` from an existing session in storage.
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
        let compaction_engine = Arc::new(CompactionEngine::new(config.compaction.clone()));

        Ok(Self {
            session,
            storage,
            providers,
            config,
            context_assembler,
            compaction_engine,
        })
    }

    // ------------------------------------------------------------------
    // Low-level operations used by SessionManager / LcmSession
    // ------------------------------------------------------------------

    /// Persist a message, optionally generate its embedding, update the session
    /// timestamp, and return the new message id.
    pub async fn store_message(
        &mut self,
        role: MessageRole,
        content: String,
    ) -> LcmResult<MessageId> {
        let token_count = self.providers.token_counter.count(&content).await?;

        let message = Message {
            id: new_message_id(),
            session_id: self.session.id,
            role,
            content,
            timestamp: chrono::Utc::now(),
            token_count,
            metadata: std::collections::HashMap::new(),
        };

        let message_id = self.storage.messages.store(message.clone()).await?;

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

        self.session.updated_at = chrono::Utc::now();
        let _ = self.storage.sessions.update(self.session.clone()).await;

        Ok(message_id)
    }

    /// Return the assembled context window for this session.
    pub async fn get_context(&self) -> LcmResult<Vec<ContextItem>> {
        self.context_assembler
            .assemble_context(
                self.session.id,
                &*self.storage.messages,
                &*self.storage.summaries,
            )
            .await
    }

    /// Return the total token count of the active context.
    pub async fn get_token_count(&self) -> LcmResult<usize> {
        self.context_assembler
            .get_context_token_count(
                self.session.id,
                &*self.storage.messages,
                &*self.storage.summaries,
            )
            .await
    }

    /// Return the number of messages stored for this session.
    pub async fn get_message_count(&self) -> LcmResult<usize> {
        self.storage
            .messages
            .get_message_count(self.session.id)
            .await
            .map_err(LcmError::Storage)
    }

    /// Return `true` if the soft compaction threshold has been exceeded.
    pub async fn needs_compaction(&self) -> LcmResult<bool> {
        self.context_assembler
            .needs_compaction(
                self.session.id,
                &*self.storage.messages,
                &*self.storage.summaries,
                self.config.compaction.thresholds.soft_limit,
                self.config.compaction.thresholds.hard_limit,
            )
            .await
    }

    /// Return `true` if the hard (emergency) compaction threshold has been exceeded.
    pub async fn needs_emergency_compaction(&self) -> LcmResult<bool> {
        self.context_assembler
            .needs_emergency_compaction(
                self.session.id,
                &*self.storage.messages,
                &*self.storage.summaries,
                self.config.compaction.thresholds.hard_limit,
            )
            .await
    }

    /// Describe a summary node, returning it together with its lineage.
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

    /// Expand a summary to its original messages.
    pub async fn expand(&self, summary_id: SummaryId) -> LcmResult<Vec<Message>> {
        self.storage
            .summaries
            .expand(summary_id, &*self.storage.messages)
            .await
            .map_err(LcmError::Storage)
    }

    /// Search for messages similar to `query` using the configured embedder.
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

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    fn is_null_embedder(&self) -> bool {
        self.providers.embedder.name() == "null"
    }

    async fn count_reachable_messages(&self, lineage: &[LineagePointer]) -> LcmResult<usize> {
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
}
