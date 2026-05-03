// daemon/src/storage/pg_message_store.rs
use async_trait::async_trait;
use bacon_lcm_core::{
    error::{StorageError, StorageResult},
    storage::MessageStore,
    types::{Message, MessageId, MessageRole, SessionId},
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

pub struct PgMessageStore {
    pool: PgPool,
}

impl PgMessageStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

// --- helpers -----------------------------------------------------------------

fn role_to_str(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
        MessageRole::Tool => "tool",
    }
}

fn str_to_role(s: &str) -> Result<MessageRole, StorageError> {
    match s {
        "user" => Ok(MessageRole::User),
        "assistant" => Ok(MessageRole::Assistant),
        "system" => Ok(MessageRole::System),
        "tool" => Ok(MessageRole::Tool),
        other => Err(StorageError::ConstraintViolation(format!(
            "unknown role: {other}"
        ))),
    }
}

fn row_to_message(row: &sqlx::postgres::PgRow) -> Result<Message, StorageError> {
    let id: Uuid = row.try_get("id").map_err(StorageError::ConnectionFailed)?;
    let session_id: Uuid = row
        .try_get("session_id")
        .map_err(StorageError::ConnectionFailed)?;
    let role_str: String = row.try_get("role").map_err(StorageError::ConnectionFailed)?;
    let role = str_to_role(&role_str)?;
    let content: String = row
        .try_get("content")
        .map_err(StorageError::ConnectionFailed)?;
    let token_count_i32: i32 = row
        .try_get("token_count")
        .map_err(StorageError::ConnectionFailed)?;
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(StorageError::ConnectionFailed)?;
    let metadata_val: Value = row
        .try_get("metadata")
        .map_err(StorageError::ConnectionFailed)?;
    let metadata: HashMap<String, Value> =
        serde_json::from_value(metadata_val).map_err(StorageError::Serialization)?;

    Ok(Message {
        id,
        session_id,
        role,
        content,
        timestamp: created_at,
        token_count: token_count_i32 as usize,
        metadata,
    })
}

// --- trait impl --------------------------------------------------------------

#[async_trait]
impl MessageStore for PgMessageStore {
    async fn store(&self, message: Message) -> StorageResult<MessageId> {
        let metadata =
            serde_json::to_value(&message.metadata).map_err(StorageError::Serialization)?;

        // Use a transaction so the sequence_number assignment is atomic.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(StorageError::ConnectionFailed)?;

        // Lock the parent session row to serialize concurrent inserts for this
        // session, then count existing messages to derive the next sequence number.
        sqlx::query("SELECT id FROM lcm_sessions WHERE id = $1 FOR UPDATE")
            .bind(message.session_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(StorageError::ConnectionFailed)?;

        let count_row = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM lcm_messages WHERE session_id = $1",
        )
        .bind(message.session_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        let count: i64 = count_row
            .try_get("cnt")
            .map_err(StorageError::ConnectionFailed)?;
        let sequence_number = count as i32;

        sqlx::query(
            r#"INSERT INTO lcm_messages
               (id, session_id, role, content, sequence_number, token_count, created_at, metadata)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(message.id)
        .bind(message.session_id)
        .bind(role_to_str(message.role))
        .bind(&message.content)
        .bind(sequence_number)
        .bind(message.token_count as i32)
        .bind(message.timestamp)
        .bind(metadata)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        tx.commit().await.map_err(StorageError::ConnectionFailed)?;

        Ok(message.id)
    }

    async fn get(&self, id: MessageId) -> StorageResult<Option<Message>> {
        let row = sqlx::query(
            r#"SELECT id, session_id, role, content, token_count, created_at, metadata
               FROM lcm_messages WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        match row {
            None => Ok(None),
            Some(r) => row_to_message(&r).map(Some),
        }
    }

    async fn get_range(
        &self,
        session_id: SessionId,
        range: std::ops::Range<usize>,
    ) -> StorageResult<Vec<Message>> {
        let limit = (range.end.saturating_sub(range.start)) as i64;
        let offset = range.start as i64;

        let rows = sqlx::query(
            r#"SELECT id, session_id, role, content, token_count, created_at, metadata
               FROM lcm_messages
               WHERE session_id = $1
               ORDER BY sequence_number
               LIMIT $2 OFFSET $3"#,
        )
        .bind(session_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        rows.iter().map(row_to_message).collect()
    }

    async fn get_session_messages(&self, session_id: SessionId) -> StorageResult<Vec<Message>> {
        let rows = sqlx::query(
            r#"SELECT id, session_id, role, content, token_count, created_at, metadata
               FROM lcm_messages
               WHERE session_id = $1
               ORDER BY sequence_number"#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        rows.iter().map(row_to_message).collect()
    }

    async fn get_message_count(&self, session_id: SessionId) -> StorageResult<usize> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM lcm_messages WHERE session_id = $1",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        let count: i64 = row.try_get("cnt").map_err(StorageError::ConnectionFailed)?;
        Ok(count as usize)
    }

    async fn get_token_count(&self, session_id: SessionId) -> StorageResult<usize> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(token_count), 0) AS total FROM lcm_messages WHERE session_id = $1",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        // SUM returns BIGINT (i64) from Postgres.
        let total: i64 = row
            .try_get("total")
            .map_err(StorageError::ConnectionFailed)?;
        Ok(total as usize)
    }

    async fn delete_session(&self, session_id: SessionId) -> StorageResult<()> {
        sqlx::query("DELETE FROM lcm_messages WHERE session_id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(StorageError::ConnectionFailed)?;
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
