// daemon/tests/test_pg_message_store.rs
mod helpers;

use bacon_lcm_core::storage::{MessageStore, SessionStore};
use bacon_lcm_daemon::storage::{
    pg_message_store::PgMessageStore,
    pg_session_store::PgSessionStore,
};
use bacon_lcm_core::types::{Message, MessageRole, Session};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

async fn setup(pool: sqlx::PgPool) -> (PgSessionStore, PgMessageStore, Uuid) {
    let sessions = PgSessionStore::new(pool.clone());
    let messages = PgMessageStore::new(pool);
    let session_id = Uuid::new_v4();
    sessions.create(Session {
        id: session_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        metadata: HashMap::new(),
    }).await.unwrap();
    (sessions, messages, session_id)
}

fn make_message(session_id: Uuid, content: &str) -> Message {
    Message {
        id: Uuid::new_v4(),
        session_id,
        role: MessageRole::User,
        content: content.to_string(),
        timestamp: Utc::now(),
        token_count: content.split_whitespace().count(),
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_store_and_get() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    let msg = make_message(session_id, "hello world");
    let id = store.store(msg.clone()).await.expect("store failed");
    assert_eq!(id, msg.id);
    let retrieved = store.get(id).await.expect("get failed");
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().content, "hello world");
}

#[tokio::test]
async fn test_get_nonexistent_returns_none() {
    let pool = helpers::test_pool().await;
    let (_, store, _) = setup(pool).await;
    assert!(store.get(Uuid::new_v4()).await.unwrap().is_none());
}

#[tokio::test]
async fn test_get_session_messages_ordered() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    for i in 0..4u32 {
        store.store(make_message(session_id, &format!("msg {}", i))).await.unwrap();
    }
    let messages = store.get_session_messages(session_id).await.unwrap();
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].content, "msg 0");
    assert_eq!(messages[3].content, "msg 3");
}

#[tokio::test]
async fn test_get_range() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    for i in 0..5u32 {
        store.store(make_message(session_id, &format!("msg {}", i))).await.unwrap();
    }
    let range = store.get_range(session_id, 1..3).await.unwrap();
    assert_eq!(range.len(), 2);
    assert_eq!(range[0].content, "msg 1");
    assert_eq!(range[1].content, "msg 2");
}

#[tokio::test]
async fn test_get_message_count() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    store.store(make_message(session_id, "a")).await.unwrap();
    store.store(make_message(session_id, "b")).await.unwrap();
    assert_eq!(store.get_message_count(session_id).await.unwrap(), 2);
}

#[tokio::test]
async fn test_get_token_count() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    // "one" → 1 token, "two words" → 2 tokens
    store.store(make_message(session_id, "one")).await.unwrap();
    store.store(make_message(session_id, "two words")).await.unwrap();
    assert_eq!(store.get_token_count(session_id).await.unwrap(), 3);
}

#[tokio::test]
async fn test_store_batch() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    let messages: Vec<_> = (0..3)
        .map(|i| make_message(session_id, &format!("batch {}", i)))
        .collect();
    let ids = store.store_batch(messages).await.unwrap();
    assert_eq!(ids.len(), 3);
    assert_eq!(store.get_message_count(session_id).await.unwrap(), 3);
}

#[tokio::test]
async fn test_delete_session() {
    let pool = helpers::test_pool().await;
    let (_, store, session_id) = setup(pool).await;
    store.store(make_message(session_id, "x")).await.unwrap();
    assert_eq!(store.get_message_count(session_id).await.unwrap(), 1);
    store.delete_session(session_id).await.unwrap();
    assert_eq!(store.get_message_count(session_id).await.unwrap(), 0);
}
