// daemon/src/storage/pg_summary_dag.rs
use async_trait::async_trait;
use bacon_lcm_core::{
    error::{StorageError, StorageResult},
    storage::{MessageStore, SummaryDag},
    types::{LineagePointer, Message, SessionId, SummaryId, SummaryLevel, SummaryNode},
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub struct PgSummaryDag {
    pool: PgPool,
}

impl PgSummaryDag {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

// --- helpers -----------------------------------------------------------------

fn level_to_str(level: SummaryLevel) -> &'static str {
    match level {
        SummaryLevel::Leaf => "leaf",
        SummaryLevel::Condensed => "condensed",
        SummaryLevel::Emergency => "emergency",
    }
}

fn str_to_level(s: &str) -> Result<SummaryLevel, StorageError> {
    match s {
        "leaf" => Ok(SummaryLevel::Leaf),
        "condensed" => Ok(SummaryLevel::Condensed),
        "emergency" => Ok(SummaryLevel::Emergency),
        other => Err(StorageError::ConstraintViolation(format!(
            "unknown summary level: {other}"
        ))),
    }
}

fn row_to_summary_node(row: &sqlx::postgres::PgRow) -> Result<SummaryNode, StorageError> {
    let id: Uuid = row.try_get("id").map_err(StorageError::ConnectionFailed)?;
    let session_id: Uuid = row
        .try_get("session_id")
        .map_err(StorageError::ConnectionFailed)?;
    let level_str: String = row
        .try_get("level")
        .map_err(StorageError::ConnectionFailed)?;
    let level = str_to_level(&level_str)?;
    let content: String = row
        .try_get("content")
        .map_err(StorageError::ConnectionFailed)?;
    let token_count_i32: i32 = row
        .try_get("token_count")
        .map_err(StorageError::ConnectionFailed)?;
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(StorageError::ConnectionFailed)?;
    let lineage_val: Value = row
        .try_get("lineage")
        .map_err(StorageError::ConnectionFailed)?;
    let lineage: Vec<LineagePointer> =
        serde_json::from_value(lineage_val).map_err(StorageError::Serialization)?;
    let metadata_val: Value = row
        .try_get("metadata")
        .map_err(StorageError::ConnectionFailed)?;
    let metadata: HashMap<String, Value> =
        serde_json::from_value(metadata_val).map_err(StorageError::Serialization)?;

    Ok(SummaryNode {
        id,
        session_id,
        level,
        content,
        token_count: token_count_i32 as usize,
        lineage,
        timestamp: created_at,
        metadata,
    })
}

// --- trait impl --------------------------------------------------------------

#[async_trait]
impl SummaryDag for PgSummaryDag {
    async fn add_node(&self, node: SummaryNode) -> StorageResult<SummaryId> {
        let lineage =
            serde_json::to_value(&node.lineage).map_err(StorageError::Serialization)?;
        let metadata =
            serde_json::to_value(&node.metadata).map_err(StorageError::Serialization)?;

        sqlx::query(
            r#"INSERT INTO lcm_summary_nodes
               (id, session_id, level, content, token_count, lineage, created_at, metadata)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(node.id)
        .bind(node.session_id)
        .bind(level_to_str(node.level))
        .bind(&node.content)
        .bind(node.token_count as i32)
        .bind(lineage)
        .bind(node.timestamp)
        .bind(metadata)
        .execute(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        Ok(node.id)
    }

    async fn get_node(&self, id: SummaryId) -> StorageResult<Option<SummaryNode>> {
        let row = sqlx::query(
            r#"SELECT id, session_id, level, content, token_count, lineage, created_at, metadata
               FROM lcm_summary_nodes WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        match row {
            None => Ok(None),
            Some(r) => row_to_summary_node(&r).map(Some),
        }
    }

    async fn get_session_summaries(&self, session_id: SessionId) -> StorageResult<Vec<SummaryNode>> {
        let rows = sqlx::query(
            r#"SELECT id, session_id, level, content, token_count, lineage, created_at, metadata
               FROM lcm_summary_nodes
               WHERE session_id = $1
               ORDER BY created_at"#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        rows.iter().map(row_to_summary_node).collect()
    }

    async fn get_lineage(&self, id: SummaryId) -> StorageResult<Vec<LineagePointer>> {
        let row = sqlx::query(
            "SELECT lineage FROM lcm_summary_nodes WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        match row {
            None => Ok(vec![]),
            Some(r) => {
                let lineage_val: Value =
                    r.try_get("lineage").map_err(StorageError::ConnectionFailed)?;
                let lineage: Vec<LineagePointer> =
                    serde_json::from_value(lineage_val).map_err(StorageError::Serialization)?;
                Ok(lineage)
            }
        }
    }

    async fn expand(
        &self,
        id: SummaryId,
        message_store: &dyn MessageStore,
    ) -> StorageResult<Vec<Message>> {
        let lineage = self.get_lineage(id).await?;
        let mut messages = Vec::new();

        for pointer in lineage {
            if let LineagePointer::Message(msg_id) = pointer {
                if let Some(msg) = message_store.get(msg_id).await? {
                    messages.push(msg);
                }
            }
        }

        messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        Ok(messages)
    }

    async fn get_summaries_by_level(
        &self,
        session_id: SessionId,
        level: SummaryLevel,
    ) -> StorageResult<Vec<SummaryNode>> {
        let rows = sqlx::query(
            r#"SELECT id, session_id, level, content, token_count, lineage, created_at, metadata
               FROM lcm_summary_nodes
               WHERE session_id = $1 AND level = $2
               ORDER BY created_at"#,
        )
        .bind(session_id)
        .bind(level_to_str(level))
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        rows.iter().map(row_to_summary_node).collect()
    }

    async fn delete_session(&self, session_id: SessionId) -> StorageResult<()> {
        sqlx::query("DELETE FROM lcm_summary_nodes WHERE session_id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(StorageError::ConnectionFailed)?;
        Ok(())
    }

    async fn detect_cycles(&self, session_id: SessionId) -> StorageResult<bool> {
        let nodes = self.get_session_summaries(session_id).await?;

        // Build adjacency map: SummaryId → Vec<SummaryId> (only Summary→Summary edges)
        let mut adj: HashMap<SummaryId, Vec<SummaryId>> = HashMap::new();
        for node in &nodes {
            let summary_children: Vec<SummaryId> = node
                .lineage
                .iter()
                .filter_map(|p| {
                    if let LineagePointer::Summary(sid) = p {
                        Some(*sid)
                    } else {
                        None
                    }
                })
                .collect();
            adj.insert(node.id, summary_children);
        }

        // Iterative DFS with path tracking to detect back-edges
        let mut globally_visited: HashSet<SummaryId> = HashSet::new();

        for start_id in adj.keys().copied() {
            if globally_visited.contains(&start_id) {
                continue;
            }

            // Stack items: (node_id, on_current_path)
            // We use a manual stack: push (node, entering=true) and (node, entering=false)
            // to track when we leave a node's subtree.
            let mut stack: Vec<(SummaryId, bool)> = vec![(start_id, true)];
            let mut path: HashSet<SummaryId> = HashSet::new();

            while let Some((node_id, entering)) = stack.pop() {
                if entering {
                    if path.contains(&node_id) {
                        // Back-edge: cycle detected
                        return Ok(true);
                    }
                    if globally_visited.contains(&node_id) {
                        continue;
                    }
                    path.insert(node_id);
                    globally_visited.insert(node_id);

                    // Push the "leaving" marker first so it fires after all children
                    stack.push((node_id, false));

                    if let Some(children) = adj.get(&node_id) {
                        for &child in children {
                            stack.push((child, true));
                        }
                    }
                } else {
                    // Leaving this node — remove from current path
                    path.remove(&node_id);
                }
            }
        }

        Ok(false)
    }
}
