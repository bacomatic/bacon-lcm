// core/src/session/manager.rs
use crate::config::LcmConfig;
use crate::error::LcmResult;
use crate::providers::{Embedder, Summarizer, TokenCounter};
use crate::session::core::SessionCore;
use crate::storage::{StorageLayer, VectorRecord};
use crate::types::{ContextItem, Message, MessageId, MessageRole, Session, SessionId, SummaryId};
use super::{DescribeResult, SessionInfo};

/// High-level session manager: wraps [`SessionCore`] and adds the full public
/// API surface.
///
/// This is the type exposed to callers as [`crate::session::LcmSession`].
///
/// # Concurrency note
///
/// `add_message` takes `&mut self`, which means the borrow checker already
/// prevents concurrent calls on the same [`SessionManager`] instance.  There
/// is therefore no need for an extra runtime lock around the compaction flag —
/// a plain `bool` field is sufficient.
pub struct SessionManager {
    core: SessionCore,
    /// Tracks whether a compaction pass is currently running.
    /// Protected by `&mut self` — no extra `Arc<RwLock<…>>` required.
    is_compacting: bool,
}

impl SessionManager {
    // ------------------------------------------------------------------
    // Construction
    // ------------------------------------------------------------------

    /// Create a new session, persisting it to `storage`.
    pub async fn new(
        token_counter: Box<dyn TokenCounter>,
        summarizer: Box<dyn Summarizer>,
        embedder: Box<dyn Embedder>,
        config: LcmConfig,
        storage: StorageLayer,
    ) -> LcmResult<Self> {
        let core = SessionCore::new(token_counter, summarizer, embedder, config, storage).await?;
        Ok(Self {
            core,
            is_compacting: false,
        })
    }

    /// Restore an existing session from storage by its id.
    pub async fn restore(
        session_id: SessionId,
        token_counter: Box<dyn TokenCounter>,
        summarizer: Box<dyn Summarizer>,
        embedder: Box<dyn Embedder>,
        config: LcmConfig,
        storage: StorageLayer,
    ) -> LcmResult<Self> {
        let core = SessionCore::restore(
            session_id,
            token_counter,
            summarizer,
            embedder,
            config,
            storage,
        )
        .await?;
        Ok(Self {
            core,
            is_compacting: false,
        })
    }

    // ------------------------------------------------------------------
    // Public API (mirrors LcmSession)
    // ------------------------------------------------------------------

    /// Expose the underlying `Session` record.
    pub fn session(&self) -> &Session {
        self.core.session()
    }

    /// Add a message to the session, triggering compaction if necessary.
    ///
    /// Message persistence is the primary contract of this method.  Compaction
    /// is a background concern: if it fails for any reason the message has
    /// already been stored successfully, so we deliberately swallow the
    /// compaction error rather than surfacing it to the caller.
    pub async fn add_message(
        &mut self,
        role: MessageRole,
        content: String,
    ) -> LcmResult<MessageId> {
        let message_id = self.core.store_message(role, content).await?;

        // `needs_compaction` is best-effort; ignore errors so they don't
        // bubble up and mask the successful message store.
        if let Ok(true) = self.core.needs_compaction().await {
            // Compaction errors are intentionally swallowed: the message has
            // already been persisted and the compaction can be retried on the
            // next call.
            let _ = self.trigger_compaction().await;
        }

        Ok(message_id)
    }

    /// Get the active context window.
    pub async fn get_context(&self) -> LcmResult<Vec<ContextItem>> {
        self.core.get_context().await
    }

    /// Get the current total token count.
    pub async fn get_token_count(&self) -> LcmResult<usize> {
        self.core.get_token_count().await
    }

    /// Get the number of messages in the session.
    pub async fn get_message_count(&self) -> LcmResult<usize> {
        self.core.get_message_count().await
    }

    /// Describe a summary node and its lineage.
    pub async fn describe(&self, summary_id: SummaryId) -> LcmResult<DescribeResult> {
        self.core.describe(summary_id).await
    }

    /// Expand a summary to its original messages.
    pub async fn expand(&self, summary_id: SummaryId) -> LcmResult<Vec<Message>> {
        self.core.expand(summary_id).await
    }

    /// Semantic search (returns empty if embeddings are disabled).
    pub async fn search(&self, query: &str, limit: usize) -> LcmResult<Vec<VectorRecord>> {
        self.core.search(query, limit).await
    }

    /// Aggregate session information snapshot.
    pub async fn get_session_info(&self) -> LcmResult<SessionInfo> {
        let message_count = self.get_message_count().await?;
        let token_count = self.get_token_count().await?;
        let summary_count = self
            .core
            .storage()
            .summaries
            .get_session_summaries(self.core.session().id)
            .await?
            .len();

        Ok(SessionInfo {
            session: self.core.session().clone(),
            message_count,
            token_count,
            summary_count,
            is_compacting: self.is_compacting,
        })
    }

    // ------------------------------------------------------------------
    // Compaction
    // ------------------------------------------------------------------

    /// Set the compaction flag, run compaction, then clear the flag.
    ///
    /// Because `add_message` takes `&mut self`, only one compaction pass can
    /// ever run at a time for a given [`SessionManager`].  The `is_compacting`
    /// flag is therefore a plain `bool` — no `Arc<RwLock<…>>` needed.
    async fn trigger_compaction(&mut self) -> LcmResult<()> {
        if self.is_compacting {
            return Ok(());
        }
        self.is_compacting = true;
        let result = self.perform_compaction().await;
        self.is_compacting = false;
        result
    }

    async fn perform_compaction(&self) -> LcmResult<()> {
        let session_id = self.core.session().id;
        if self.core.needs_emergency_compaction().await? {
            self.core
                .compaction_engine()
                .emergency_compaction(session_id)
                .await?;
        } else {
            self.core
                .compaction_engine()
                .compact(session_id)
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LcmConfig;
    use crate::error::LcmError;
    use crate::ids::new_session_id;
    use crate::providers::{create_embedder, create_summarizer, create_token_counter};
    use crate::storage::StorageLayer;

    async fn create_test_session() -> SessionManager {
        let config = LcmConfig::defaults();
        let storage = StorageLayer::memory();
        let token_counter = create_token_counter("naive", None).unwrap();
        let summarizer =
            create_summarizer("echo", "echo".to_string(), None, None, None, None).unwrap();
        let embedder = create_embedder("null", None, None, None, None).unwrap();

        SessionManager::new(token_counter, summarizer, embedder, config, storage)
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
        let config = LcmConfig::defaults();
        let storage = StorageLayer::memory();
        let token_counter = create_token_counter("naive", None).unwrap();
        let summarizer =
            create_summarizer("echo", "echo".to_string(), None, None, None, None).unwrap();
        let embedder = create_embedder("null", None, None, None, None).unwrap();

        let nonexistent_id = new_session_id();

        let result = SessionManager::restore(
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
