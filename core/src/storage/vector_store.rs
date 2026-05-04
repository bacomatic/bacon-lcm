// core/src/storage/vector_store.rs
use crate::error::{StorageError, StorageResult};
use crate::types::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Unique identifier for a vector embedding record
pub type VectorId = Uuid;

/// A stored vector embedding with associated metadata
#[derive(Debug, Clone)]
pub struct VectorRecord {
    pub id: VectorId,
    pub session_id: SessionId,
    /// The embedding vector (list of f32 values)
    pub embedding: Vec<f32>,
    /// Original content that was embedded
    pub content: String,
    /// Arbitrary metadata (e.g. message_id, role, etc.)
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Trait for vector storage and nearest-neighbour search
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Store a vector embedding record
    async fn store(&self, record: VectorRecord) -> StorageResult<VectorId>;

    /// Retrieve a vector record by ID
    async fn get(&self, id: VectorId) -> StorageResult<Option<VectorRecord>>;

    /// Delete a vector record by ID
    async fn delete(&self, id: VectorId) -> StorageResult<()>;

    /// Delete all vector records for a session
    async fn delete_session(&self, session_id: SessionId) -> StorageResult<()>;

    /// Get all vector records for a session
    async fn get_session_vectors(&self, session_id: SessionId) -> StorageResult<Vec<VectorRecord>>;

    /// Search for the `k` nearest neighbours to `query` within a session.
    /// Returns records sorted by ascending cosine distance (most similar first).
    async fn search(
        &self,
        session_id: SessionId,
        query: &[f32],
        k: usize,
    ) -> StorageResult<Vec<VectorRecord>>;
}

/// In-memory vector store using brute-force cosine similarity search
#[derive(Debug)]
pub struct InMemoryVectorStore {
    records: Arc<RwLock<HashMap<VectorId, VectorRecord>>>,
    session_index: Arc<RwLock<HashMap<SessionId, Vec<VectorId>>>>,
}

impl InMemoryVectorStore {
    pub fn new() -> Self {
        Self {
            records: Arc::new(RwLock::new(HashMap::new())),
            session_index: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Compute cosine similarity between two vectors.
    /// Returns a value in [-1, 1]; higher means more similar.
    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }

        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }

        dot / (norm_a * norm_b)
    }
}

impl Default for InMemoryVectorStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VectorStore for InMemoryVectorStore {
    async fn store(&self, record: VectorRecord) -> StorageResult<VectorId> {
        let id = record.id;
        let session_id = record.session_id;

        {
            let mut records = self.records.write().await;
            records.insert(id, record);
        }

        {
            let mut session_index = self.session_index.write().await;
            session_index
                .entry(session_id)
                .or_insert_with(Vec::new)
                .push(id);
        }

        Ok(id)
    }

    async fn get(&self, id: VectorId) -> StorageResult<Option<VectorRecord>> {
        let records = self.records.read().await;
        Ok(records.get(&id).cloned())
    }

    async fn delete(&self, id: VectorId) -> StorageResult<()> {
        let session_id_opt = {
            let records = self.records.read().await;
            records.get(&id).map(|r| r.session_id)
        };

        {
            let mut records = self.records.write().await;
            records.remove(&id);
        }

        if let Some(session_id) = session_id_opt {
            let mut session_index = self.session_index.write().await;
            if let Some(ids) = session_index.get_mut(&session_id) {
                ids.retain(|&v| v != id);
            }
        }

        Ok(())
    }

    async fn delete_session(&self, session_id: SessionId) -> StorageResult<()> {
        let ids_to_remove = {
            let mut session_index = self.session_index.write().await;
            session_index.remove(&session_id).unwrap_or_default()
        };

        let mut records = self.records.write().await;
        for id in ids_to_remove {
            records.remove(&id);
        }

        Ok(())
    }

    async fn get_session_vectors(&self, session_id: SessionId) -> StorageResult<Vec<VectorRecord>> {
        let session_index = self.session_index.read().await;
        let records = self.records.read().await;

        if let Some(ids) = session_index.get(&session_id) {
            let result = ids
                .iter()
                .filter_map(|id| records.get(id).cloned())
                .collect();
            Ok(result)
        } else {
            Ok(Vec::new())
        }
    }

    async fn search(
        &self,
        session_id: SessionId,
        query: &[f32],
        k: usize,
    ) -> StorageResult<Vec<VectorRecord>> {
        if query.is_empty() {
            return Err(StorageError::ConstraintViolation(
                "Query vector must not be empty".to_string(),
            ));
        }

        let session_vectors = self.get_session_vectors(session_id).await?;

        // Validate that stored vectors match query dimension
        for rec in &session_vectors {
            if rec.embedding.len() != query.len() {
                return Err(StorageError::ConstraintViolation(format!(
                    "Embedding dimension mismatch: expected {}, found {}",
                    query.len(),
                    rec.embedding.len()
                )));
            }
        }

        // Compute similarities and sort descending (highest similarity first)
        let mut scored: Vec<(f32, VectorRecord)> = session_vectors
            .into_iter()
            .map(|rec| {
                let sim = Self::cosine_similarity(query, &rec.embedding);
                (sim, rec)
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let results = scored
            .into_iter()
            .take(k)
            .map(|(_, rec)| rec)
            .collect();

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::new_session_id;

    fn new_vector_id() -> VectorId {
        Uuid::new_v4()
    }

    fn create_test_record(session_id: SessionId, embedding: Vec<f32>, content: &str) -> VectorRecord {
        VectorRecord {
            id: new_vector_id(),
            session_id,
            embedding,
            content: content.to_string(),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_store_and_get_vector() {
        let store = InMemoryVectorStore::new();
        let session_id = new_session_id();
        let record = create_test_record(session_id, vec![1.0, 0.0, 0.0], "test content");
        let id = record.id;

        let stored_id = store.store(record.clone()).await.unwrap();
        assert_eq!(stored_id, id);

        let retrieved = store.get(id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().content, "test content");
    }

    #[tokio::test]
    async fn test_delete_vector() {
        let store = InMemoryVectorStore::new();
        let session_id = new_session_id();
        let record = create_test_record(session_id, vec![1.0, 0.0], "to delete");
        let id = record.id;

        store.store(record).await.unwrap();
        assert!(store.get(id).await.unwrap().is_some());

        store.delete(id).await.unwrap();
        assert!(store.get(id).await.unwrap().is_none());

        // Session index should also be cleaned up
        let vecs = store.get_session_vectors(session_id).await.unwrap();
        assert!(vecs.is_empty());
    }

    #[tokio::test]
    async fn test_delete_session() {
        let store = InMemoryVectorStore::new();
        let session_id = new_session_id();

        store.store(create_test_record(session_id, vec![1.0, 0.0], "a")).await.unwrap();
        store.store(create_test_record(session_id, vec![0.0, 1.0], "b")).await.unwrap();

        let before = store.get_session_vectors(session_id).await.unwrap();
        assert_eq!(before.len(), 2);

        store.delete_session(session_id).await.unwrap();

        let after = store.get_session_vectors(session_id).await.unwrap();
        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn test_search_nearest_neighbours() {
        let store = InMemoryVectorStore::new();
        let session_id = new_session_id();

        // Three unit vectors in 3-D space
        store.store(create_test_record(session_id, vec![1.0, 0.0, 0.0], "x-axis")).await.unwrap();
        store.store(create_test_record(session_id, vec![0.0, 1.0, 0.0], "y-axis")).await.unwrap();
        store.store(create_test_record(session_id, vec![0.0, 0.0, 1.0], "z-axis")).await.unwrap();

        // Query close to x-axis
        let results = store.search(session_id, &[1.0, 0.0, 0.0], 2).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].content, "x-axis");
    }

    #[tokio::test]
    async fn test_search_returns_at_most_k() {
        let store = InMemoryVectorStore::new();
        let session_id = new_session_id();

        for i in 0..5 {
            let embedding = vec![i as f32, 0.0];
            store.store(create_test_record(session_id, embedding, &format!("record {}", i))).await.unwrap();
        }

        let results = store.search(session_id, &[1.0, 0.0], 3).await.unwrap();
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_search_empty_session() {
        let store = InMemoryVectorStore::new();
        let session_id = new_session_id();

        let results = store.search(session_id, &[1.0, 0.0], 5).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_search_empty_query_returns_error() {
        let store = InMemoryVectorStore::new();
        let session_id = new_session_id();

        let result = store.search(session_id, &[], 5).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_cosine_similarity() {
        // Identical vectors → similarity of 1
        let sim = InMemoryVectorStore::cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]);
        assert!((sim - 1.0).abs() < 1e-6);

        // Orthogonal vectors → similarity of 0
        let sim = InMemoryVectorStore::cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]);
        assert!(sim.abs() < 1e-6);

        // Opposite vectors → similarity of -1
        let sim = InMemoryVectorStore::cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]);
        assert!((sim + 1.0).abs() < 1e-6);
    }
}
