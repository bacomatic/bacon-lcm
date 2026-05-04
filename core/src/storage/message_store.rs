// core/src/storage/message_store.rs
use crate::error::StorageResult;
use crate::types::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Trait for message storage operations
#[async_trait]
pub trait MessageStore: Send + Sync {
    /// Store a new message
    async fn store(&self, message: Message) -> StorageResult<MessageId>;

    /// Retrieve a message by ID
    async fn get(&self, id: MessageId) -> StorageResult<Option<Message>>;

    /// Get messages for a session in a range
    async fn get_range(&self, session_id: SessionId, range: std::ops::Range<usize>) -> StorageResult<Vec<Message>>;

    /// Get all messages for a session
    async fn get_session_messages(&self, session_id: SessionId) -> StorageResult<Vec<Message>>;

    /// Get message count for a session
    async fn get_message_count(&self, session_id: SessionId) -> StorageResult<usize>;

    /// Get total token count for a session
    async fn get_token_count(&self, session_id: SessionId) -> StorageResult<usize>;

    /// Delete messages for a session (for cleanup)
    async fn delete_session(&self, session_id: SessionId) -> StorageResult<()>;

    /// Store multiple messages in a batch
    async fn store_batch(&self, messages: Vec<Message>) -> StorageResult<Vec<MessageId>>;
}

/// In-memory message store implementation for testing
#[derive(Debug)]
pub struct InMemoryMessageStore {
    messages: Arc<RwLock<HashMap<MessageId, Message>>>,
    session_messages: Arc<RwLock<HashMap<SessionId, Vec<MessageId>>>>,
}

impl InMemoryMessageStore {
    pub fn new() -> Self {
        Self {
            messages: Arc::new(RwLock::new(HashMap::new())),
            session_messages: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryMessageStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MessageStore for InMemoryMessageStore {
    async fn store(&self, message: Message) -> StorageResult<MessageId> {
        let id = message.id;

        // Store the message
        {
            let mut messages = self.messages.write().await;
            messages.insert(id, message.clone());
        }

        // Update session index
        {
            let mut session_messages = self.session_messages.write().await;
            session_messages
                .entry(message.session_id)
                .or_insert_with(Vec::new)
                .push(id);
        }

        Ok(id)
    }

    async fn get(&self, id: MessageId) -> StorageResult<Option<Message>> {
        let messages = self.messages.read().await;
        Ok(messages.get(&id).cloned())
    }

    async fn get_range(&self, session_id: SessionId, range: std::ops::Range<usize>) -> StorageResult<Vec<Message>> {
        let session_messages = self.session_messages.read().await;
        let messages = self.messages.read().await;

        if let Some(message_ids) = session_messages.get(&session_id) {
            let start = range.start.min(message_ids.len());
            let end = range.end.min(message_ids.len());

            if start >= end {
                return Ok(Vec::new());
            }

            let range_ids = &message_ids[start..end];
            let mut result = Vec::with_capacity(range_ids.len());

            for &id in range_ids {
                if let Some(message) = messages.get(&id) {
                    result.push(message.clone());
                }
            }

            Ok(result)
        } else {
            Ok(Vec::new())
        }
    }

    async fn get_session_messages(&self, session_id: SessionId) -> StorageResult<Vec<Message>> {
        let session_messages = self.session_messages.read().await;
        let messages = self.messages.read().await;

        if let Some(message_ids) = session_messages.get(&session_id) {
            let mut result = Vec::with_capacity(message_ids.len());

            for &id in message_ids {
                if let Some(message) = messages.get(&id) {
                    result.push(message.clone());
                }
            }

            Ok(result)
        } else {
            Ok(Vec::new())
        }
    }

    async fn get_message_count(&self, session_id: SessionId) -> StorageResult<usize> {
        let session_messages = self.session_messages.read().await;
        Ok(session_messages.get(&session_id).map(|ids| ids.len()).unwrap_or(0))
    }

    async fn get_token_count(&self, session_id: SessionId) -> StorageResult<usize> {
        let messages = self.get_session_messages(session_id).await?;
        Ok(messages.iter().map(|m| m.token_count).sum())
    }

    async fn delete_session(&self, session_id: SessionId) -> StorageResult<()> {
        let mut session_messages = self.session_messages.write().await;
        let mut messages = self.messages.write().await;

        if let Some(message_ids) = session_messages.remove(&session_id) {
            for id in message_ids {
                messages.remove(&id);
            }
        }

        Ok(())
    }

    async fn store_batch(&self, messages: Vec<Message>) -> StorageResult<Vec<MessageId>> {
        let mut ids = Vec::with_capacity(messages.len());

        for message in messages {
            ids.push(self.store(message).await?);
        }

        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{new_message_id, new_session_id};
    use chrono::Utc;

    fn create_test_message(session_id: SessionId, content: &str) -> Message {
        Message {
            id: new_message_id(),
            session_id,
            role: MessageRole::User,
            content: content.to_string(),
            timestamp: Utc::now(),
            token_count: content.len() / 4, // Rough estimate
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_store_and_get_message() {
        let store = InMemoryMessageStore::new();
        let session_id = new_session_id();
        let message = create_test_message(session_id, "Hello world");

        let stored_id = store.store(message.clone()).await.unwrap();
        assert_eq!(stored_id, message.id);

        let retrieved = store.get(message.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().content, "Hello world");
    }

    #[tokio::test]
    async fn test_session_messages() {
        let store = InMemoryMessageStore::new();
        let session_id = new_session_id();

        let msg1 = create_test_message(session_id, "First message");
        let msg2 = create_test_message(session_id, "Second message");

        store.store(msg1).await.unwrap();
        store.store(msg2).await.unwrap();

        let messages = store.get_session_messages(session_id).await.unwrap();
        assert_eq!(messages.len(), 2);

        let count = store.get_message_count(session_id).await.unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_get_range() {
        let store = InMemoryMessageStore::new();
        let session_id = new_session_id();

        for i in 0..5 {
            let msg = create_test_message(session_id, &format!("Message {}", i));
            store.store(msg).await.unwrap();
        }

        let range = store.get_range(session_id, 1..3).await.unwrap();
        assert_eq!(range.len(), 2);
        assert_eq!(range[0].content, "Message 1");
        assert_eq!(range[1].content, "Message 2");
    }
}
