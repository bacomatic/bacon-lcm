// daemon/tests/test_pg_vector_store.rs
mod helpers;

use bacon_lcm_core::storage::{SessionStore, VectorStore};
use bacon_lcm_core::storage::vector_store::VectorRecord;
use bacon_lcm_daemon::storage::{
    pg_session_store::PgSessionStore,
    pg_vector_store::PgVectorStore,
};
use bacon_lcm_core::types::Session;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

async fn setup(pool: sqlx::PgPool) -> (PgSessionStore, PgVectorStore, Uuid) {
    let sessions = PgSessionStore::new(pool.clone());
    let vectors = PgVectorStore::new(pool);
    let session_id = Uuid::new_v4();
    sessions.create(Session {
        id: session_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        metadata: HashMap::new(),
    }).await.unwrap();
    (sessions, vectors, session_id)
}

fn make_record(session_id: Uuid, embedding: Vec<f32>, content: &str) -> VectorRecord {
    VectorRecord {
        id: Uuid::new_v4(),
        session_id,
        embedding,
        content: content.to_string(),
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_store_and_get() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    let rec = make_record(session_id, vec![1.0, 0.0, 0.0], "hello");
    let id = store.store(rec.clone()).await.expect("store failed");
    assert_eq!(id, rec.id);
    let retrieved = store.get(id).await.expect("get failed");
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().content, "hello");
}

#[tokio::test]
async fn test_get_nonexistent_returns_none() {
    let pool = helpers::test_pool().await;
    let (_, store, _) = setup(pool).await;
    assert!(store.get(Uuid::new_v4()).await.unwrap().is_none());
}

#[tokio::test]
async fn test_delete() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    let rec = make_record(session_id, vec![1.0, 0.0], "to delete");
    let id = store.store(rec).await.unwrap();
    assert!(store.get(id).await.unwrap().is_some());
    store.delete(id).await.unwrap();
    assert!(store.get(id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_delete_session() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    store.store(make_record(session_id, vec![1.0, 0.0], "a")).await.unwrap();
    store.store(make_record(session_id, vec![0.0, 1.0], "b")).await.unwrap();
    store.delete_session(session_id).await.unwrap();
    assert!(store.get_session_vectors(session_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn test_get_session_vectors() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    store.store(make_record(session_id, vec![1.0, 0.0], "x")).await.unwrap();
    store.store(make_record(session_id, vec![0.0, 1.0], "y")).await.unwrap();
    let vecs = store.get_session_vectors(session_id).await.unwrap();
    assert_eq!(vecs.len(), 2);
}

#[tokio::test]
async fn test_search_nearest_neighbour() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    store.store(make_record(session_id, vec![1.0, 0.0, 0.0], "x-axis")).await.unwrap();
    store.store(make_record(session_id, vec![0.0, 1.0, 0.0], "y-axis")).await.unwrap();
    store.store(make_record(session_id, vec![0.0, 0.0, 1.0], "z-axis")).await.unwrap();
    // Query close to x-axis → x-axis should be first
    let results = store.search(session_id, &[1.0, 0.0, 0.0], 2).await.unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].content, "x-axis");
}

#[tokio::test]
async fn test_search_returns_at_most_k() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    for i in 0..5u32 {
        store.store(make_record(session_id, vec![i as f32, 0.0], &format!("rec {}", i))).await.unwrap();
    }
    let results = store.search(session_id, &[1.0, 0.0], 3).await.unwrap();
    assert_eq!(results.len(), 3);
}

#[tokio::test]
async fn test_search_empty_session_returns_empty() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    let results = store.search(session_id, &[1.0, 0.0], 5).await.unwrap();
    assert!(results.is_empty());
}
