// core/src/storage/session_store.rs
use crate::error::StorageResult;
use crate::types::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Trait for session storage operations
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Create a new session
    async fn create(&self, session: Session) -> StorageResult<SessionId>;

    /// Retrieve a session by ID
    async fn get(&self, id: SessionId) -> StorageResult<Option<Session>>;

    /// Update an existing session
    async fn update(&self, session: Session) -> StorageResult<()>;

    /// Delete a session by ID
    async fn delete(&self, id: SessionId) -> StorageResult<()>;

    /// List all session IDs
    async fn list(&self) -> StorageResult<Vec<SessionId>>;

    /// Check whether a session exists
    async fn exists(&self, id: SessionId) -> StorageResult<bool>;
}

/// In-memory session store implementation for testing
#[derive(Debug)]
pub struct InMemorySessionStore {
    sessions: Arc<RwLock<HashMap<SessionId, Session>>>,
}

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for InMemorySessionStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn create(&self, session: Session) -> StorageResult<SessionId> {
        let id = session.id;
        let mut sessions = self.sessions.write().await;
        sessions.insert(id, session);
        Ok(id)
    }

    async fn get(&self, id: SessionId) -> StorageResult<Option<Session>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(&id).cloned())
    }

    async fn update(&self, session: Session) -> StorageResult<()> {
        let mut sessions = self.sessions.write().await;
        sessions.insert(session.id, session);
        Ok(())
    }

    async fn delete(&self, id: SessionId) -> StorageResult<()> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(&id);
        Ok(())
    }

    async fn list(&self) -> StorageResult<Vec<SessionId>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.keys().copied().collect())
    }

    async fn exists(&self, id: SessionId) -> StorageResult<bool> {
        let sessions = self.sessions.read().await;
        Ok(sessions.contains_key(&id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::new_session_id;
    use chrono::Utc;

    fn create_test_session() -> Session {
        Session {
            id: new_session_id(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_create_and_get_session() {
        let store = InMemorySessionStore::new();
        let session = create_test_session();
        let id = session.id;

        let stored_id = store.create(session.clone()).await.unwrap();
        assert_eq!(stored_id, id);

        let retrieved = store.get(id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, id);
    }

    #[tokio::test]
    async fn test_get_nonexistent_session() {
        let store = InMemorySessionStore::new();
        let id = new_session_id();

        let retrieved = store.get(id).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_update_session() {
        let store = InMemorySessionStore::new();
        let session = create_test_session();
        let id = session.id;

        store.create(session.clone()).await.unwrap();

        let mut updated = session.clone();
        updated.metadata.insert(
            "key".to_string(),
            serde_json::Value::String("value".to_string()),
        );
        store.update(updated).await.unwrap();

        let retrieved = store.get(id).await.unwrap().unwrap();
        assert!(retrieved.metadata.contains_key("key"));
    }

    #[tokio::test]
    async fn test_delete_session() {
        let store = InMemorySessionStore::new();
        let session = create_test_session();
        let id = session.id;

        store.create(session).await.unwrap();
        assert!(store.exists(id).await.unwrap());

        store.delete(id).await.unwrap();
        assert!(!store.exists(id).await.unwrap());
    }

    #[tokio::test]
    async fn test_list_sessions() {
        let store = InMemorySessionStore::new();

        let s1 = create_test_session();
        let s2 = create_test_session();
        let id1 = s1.id;
        let id2 = s2.id;

        store.create(s1).await.unwrap();
        store.create(s2).await.unwrap();

        let ids = store.list().await.unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }

    #[tokio::test]
    async fn test_exists() {
        let store = InMemorySessionStore::new();
        let session = create_test_session();
        let id = session.id;

        assert!(!store.exists(id).await.unwrap());

        store.create(session).await.unwrap();
        assert!(store.exists(id).await.unwrap());
    }
}
