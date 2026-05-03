// daemon/tests/test_pg_session_store.rs
mod helpers;

use bacon_lcm_core::storage::SessionStore;
use bacon_lcm_daemon::storage::pg_session_store::PgSessionStore;
use bacon_lcm_core::types::Session;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

fn make_session(id: Uuid) -> Session {
    Session {
        id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_create_and_get() {
    let pool = helpers::test_pool().await;
    let store = PgSessionStore::new(pool);
    let id = Uuid::new_v4();
    let stored_id = store.create(make_session(id)).await.expect("create failed");
    assert_eq!(stored_id, id);
    let retrieved = store.get(id).await.expect("get failed");
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().id, id);
}

#[tokio::test]
async fn test_get_nonexistent_returns_none() {
    let pool = helpers::test_pool().await;
    let store = PgSessionStore::new(pool);
    assert!(store.get(Uuid::new_v4()).await.expect("get failed").is_none());
}

#[tokio::test]
async fn test_update_metadata() {
    let pool = helpers::test_pool().await;
    let store = PgSessionStore::new(pool);
    let id = Uuid::new_v4();
    store.create(make_session(id)).await.unwrap();
    let mut updated = make_session(id);
    updated.metadata.insert("env".to_string(), serde_json::json!("prod"));
    store.update(updated).await.expect("update failed");
    let s = store.get(id).await.unwrap().unwrap();
    assert_eq!(s.metadata["env"], serde_json::json!("prod"));
}

#[tokio::test]
async fn test_delete() {
    let pool = helpers::test_pool().await;
    let store = PgSessionStore::new(pool);
    let id = Uuid::new_v4();
    store.create(make_session(id)).await.unwrap();
    assert!(store.exists(id).await.unwrap());
    store.delete(id).await.expect("delete failed");
    assert!(!store.exists(id).await.unwrap());
}

#[tokio::test]
async fn test_list() {
    let pool = helpers::test_pool().await;
    let store = PgSessionStore::new(pool);
    let id1 = Uuid::new_v4();
    let id2 = Uuid::new_v4();
    store.create(make_session(id1)).await.unwrap();
    store.create(make_session(id2)).await.unwrap();
    let ids = store.list().await.expect("list failed");
    assert!(ids.contains(&id1));
    assert!(ids.contains(&id2));
}

#[tokio::test]
async fn test_exists() {
    let pool = helpers::test_pool().await;
    let store = PgSessionStore::new(pool);
    let id = Uuid::new_v4();
    assert!(!store.exists(id).await.unwrap());
    store.create(make_session(id)).await.unwrap();
    assert!(store.exists(id).await.unwrap());
}
