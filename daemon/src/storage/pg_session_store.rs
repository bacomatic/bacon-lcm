// daemon/src/storage/pg_session_store.rs
use async_trait::async_trait;
use bacon_lcm_core::{
    error::{StorageError, StorageResult},
    storage::SessionStore,
    types::{Session, SessionId},
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

pub struct PgSessionStore {
    pool: PgPool,
}

impl PgSessionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Helper to convert a raw row into a `Session`.
fn row_to_session(
    id: Uuid,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    metadata: Value,
) -> Result<Session, StorageError> {
    let metadata: HashMap<String, Value> =
        serde_json::from_value(metadata).map_err(StorageError::Serialization)?;
    Ok(Session {
        id,
        created_at,
        updated_at,
        metadata,
    })
}

#[async_trait]
impl SessionStore for PgSessionStore {
    async fn create(&self, session: Session) -> StorageResult<SessionId> {
        let metadata =
            serde_json::to_value(&session.metadata).map_err(StorageError::Serialization)?;
        sqlx::query(
            r#"INSERT INTO lcm_sessions (id, created_at, updated_at, metadata)
               VALUES ($1, $2, $3, $4)"#,
        )
        .bind(session.id)
        .bind(session.created_at)
        .bind(session.updated_at)
        .bind(metadata)
        .execute(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;
        Ok(session.id)
    }

    async fn get(&self, id: SessionId) -> StorageResult<Option<Session>> {
        let row = sqlx::query(
            r#"SELECT id, created_at, updated_at, metadata
               FROM lcm_sessions WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        match row {
            None => Ok(None),
            Some(row) => {
                use sqlx::Row;
                let id: Uuid = row.try_get("id").map_err(StorageError::ConnectionFailed)?;
                let created_at: DateTime<Utc> =
                    row.try_get("created_at").map_err(StorageError::ConnectionFailed)?;
                let updated_at: DateTime<Utc> =
                    row.try_get("updated_at").map_err(StorageError::ConnectionFailed)?;
                let metadata: Value =
                    row.try_get("metadata").map_err(StorageError::ConnectionFailed)?;
                row_to_session(id, created_at, updated_at, metadata).map(Some)
            }
        }
    }

    async fn update(&self, session: Session) -> StorageResult<()> {
        let metadata =
            serde_json::to_value(&session.metadata).map_err(StorageError::Serialization)?;
        sqlx::query(
            r#"UPDATE lcm_sessions SET updated_at = $2, metadata = $3 WHERE id = $1"#,
        )
        .bind(session.id)
        .bind(session.updated_at)
        .bind(metadata)
        .execute(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;
        Ok(())
    }

    async fn delete(&self, id: SessionId) -> StorageResult<()> {
        sqlx::query("DELETE FROM lcm_sessions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(StorageError::ConnectionFailed)?;
        Ok(())
    }

    async fn list(&self) -> StorageResult<Vec<SessionId>> {
        let rows = sqlx::query(
            "SELECT id FROM lcm_sessions ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        rows.into_iter()
            .map(|row| {
                use sqlx::Row;
                row.try_get::<Uuid, _>("id")
                    .map_err(StorageError::ConnectionFailed)
            })
            .collect()
    }

    async fn exists(&self, id: SessionId) -> StorageResult<bool> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM lcm_sessions WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        use sqlx::Row;
        let count: i64 = row.try_get("cnt").map_err(StorageError::ConnectionFailed)?;
        Ok(count > 0)
    }
}
