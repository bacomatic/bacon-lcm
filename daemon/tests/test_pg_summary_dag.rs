// daemon/tests/test_pg_summary_dag.rs
mod helpers;

use bacon_lcm_core::storage::{MessageStore, SessionStore, SummaryDag};
use bacon_lcm_daemon::storage::{
    pg_message_store::PgMessageStore,
    pg_session_store::PgSessionStore,
    pg_summary_dag::PgSummaryDag,
};
use bacon_lcm_core::types::{
    LineagePointer, Message, MessageRole, Session, SummaryLevel, SummaryNode,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

async fn setup(pool: sqlx::PgPool) -> (PgSessionStore, PgMessageStore, PgSummaryDag, Uuid) {
    let sessions = PgSessionStore::new(pool.clone());
    let messages = PgMessageStore::new(pool.clone());
    let summaries = PgSummaryDag::new(pool);
    let session_id = Uuid::new_v4();
    sessions.create(Session {
        id: session_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        metadata: HashMap::new(),
    }).await.unwrap();
    (sessions, messages, summaries, session_id)
}

fn make_summary(session_id: Uuid, level: SummaryLevel, lineage: Vec<LineagePointer>) -> SummaryNode {
    SummaryNode {
        id: Uuid::new_v4(),
        session_id,
        level,
        content: "summary text".to_string(),
        token_count: 10,
        lineage,
        timestamp: Utc::now(),
        metadata: HashMap::new(),
    }
}

fn make_message(session_id: Uuid) -> Message {
    Message {
        id: Uuid::new_v4(),
        session_id,
        role: MessageRole::User,
        content: "hello".to_string(),
        timestamp: Utc::now(),
        token_count: 1,
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_add_and_get_node() {
    let pool = helpers::test_pool().await;
    let (_, _, dag, session_id) = setup(pool).await;
    let node = make_summary(session_id, SummaryLevel::Leaf, vec![]);
    let id = dag.add_node(node.clone()).await.expect("add_node failed");
    assert_eq!(id, node.id);
    let retrieved = dag.get_node(id).await.expect("get_node failed");
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().content, "summary text");
}

#[tokio::test]
async fn test_get_nonexistent_returns_none() {
    let pool = helpers::test_pool().await;
    let (_, _, dag, _) = setup(pool).await;
    assert!(dag.get_node(Uuid::new_v4()).await.unwrap().is_none());
}

#[tokio::test]
async fn test_get_session_summaries() {
    let pool = helpers::test_pool().await;
    let (_, _, dag, session_id) = setup(pool).await;
    dag.add_node(make_summary(session_id, SummaryLevel::Leaf, vec![])).await.unwrap();
    dag.add_node(make_summary(session_id, SummaryLevel::Condensed, vec![])).await.unwrap();
    let summaries = dag.get_session_summaries(session_id).await.unwrap();
    assert_eq!(summaries.len(), 2);
}

#[tokio::test]
async fn test_get_lineage_message_pointers() {
    let pool = helpers::test_pool().await;
    let (_, _, dag, session_id) = setup(pool).await;
    let msg_id = Uuid::new_v4();
    let lineage = vec![LineagePointer::Message(msg_id)];
    let node = make_summary(session_id, SummaryLevel::Leaf, lineage);
    let id = dag.add_node(node).await.unwrap();
    let got = dag.get_lineage(id).await.unwrap();
    assert_eq!(got.len(), 1);
    assert!(matches!(got[0], LineagePointer::Message(m) if m == msg_id));
}

#[tokio::test]
async fn test_get_summaries_by_level() {
    let pool = helpers::test_pool().await;
    let (_, _, dag, session_id) = setup(pool).await;
    dag.add_node(make_summary(session_id, SummaryLevel::Leaf, vec![])).await.unwrap();
    dag.add_node(make_summary(session_id, SummaryLevel::Condensed, vec![])).await.unwrap();
    dag.add_node(make_summary(session_id, SummaryLevel::Emergency, vec![])).await.unwrap();
    let leaves = dag.get_summaries_by_level(session_id, SummaryLevel::Leaf).await.unwrap();
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].level, SummaryLevel::Leaf);
}

#[tokio::test]
async fn test_expand_resolves_messages() {
    let pool = helpers::test_pool().await;
    let (_, messages, dag, session_id) = setup(pool).await;
    let msg = make_message(session_id);
    let msg_id = messages.store(msg.clone()).await.unwrap();
    let node = make_summary(session_id, SummaryLevel::Leaf, vec![LineagePointer::Message(msg_id)]);
    let node_id = dag.add_node(node).await.unwrap();
    let expanded = dag.expand(node_id, &messages).await.unwrap();
    assert_eq!(expanded.len(), 1);
    assert_eq!(expanded[0].id, msg_id);
}

#[tokio::test]
async fn test_delete_session() {
    let pool = helpers::test_pool().await;
    let (_, _, dag, session_id) = setup(pool).await;
    dag.add_node(make_summary(session_id, SummaryLevel::Leaf, vec![])).await.unwrap();
    dag.delete_session(session_id).await.unwrap();
    let summaries = dag.get_session_summaries(session_id).await.unwrap();
    assert!(summaries.is_empty());
}
