// daemon/tests/test_storage_layer.rs
//! Smoke test: verifies that `postgres_layer()` returns a fully working
//! `StorageLayer` backed by Postgres.
mod helpers;

use bacon_lcm_core::types::{Message, MessageRole, Session};
use bacon_lcm_daemon::storage::postgres_layer;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

#[tokio::test]
async fn test_postgres_layer_end_to_end() {
    let pool = helpers::test_pool().await;
    let layer = postgres_layer(pool);

    // Create a session
    let session_id = Uuid::new_v4();
    layer.sessions.create(Session {
        id: session_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        metadata: HashMap::new(),
    }).await.expect("session create failed");

    // Store a message
    let msg = Message {
        id: Uuid::new_v4(),
        session_id,
        role: MessageRole::User,
        content: "hello postgres layer".to_string(),
        timestamp: Utc::now(),
        token_count: 3,
        metadata: HashMap::new(),
    };
    layer.messages.store(msg.clone()).await.expect("message store failed");

    // Verify round-trip
    let messages = layer.messages.get_session_messages(session_id).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "hello postgres layer");
}
