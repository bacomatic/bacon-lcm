// daemon/src/storage/pg_vector_store.rs
use async_trait::async_trait;
use bacon_lcm_core::{
    error::{StorageError, StorageResult},
    storage::VectorStore,
    storage::vector_store::VectorRecord,
    types::SessionId,
};
use pgvector::Vector;
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

pub struct PgVectorStore {
    pool: PgPool,
}

impl PgVectorStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

// --- helpers -----------------------------------------------------------------

fn row_to_vector_record(row: &sqlx::postgres::PgRow) -> Result<VectorRecord, StorageError> {
    let id: Uuid = row.try_get("id").map_err(StorageError::ConnectionFailed)?;
    let session_id: Uuid = row
        .try_get("session_id")
        .map_err(StorageError::ConnectionFailed)?;
    let content: String = row
        .try_get("content")
        .map_err(StorageError::ConnectionFailed)?;
    let embedding: Vector = row
        .try_get("embedding")
        .map_err(StorageError::ConnectionFailed)?;
    let metadata_val: Value = row
        .try_get("metadata")
        .map_err(StorageError::ConnectionFailed)?;
    let metadata: HashMap<String, Value> =
        serde_json::from_value(metadata_val).map_err(StorageError::Serialization)?;

    Ok(VectorRecord {
        id,
        session_id,
        embedding: embedding.to_vec(),
        content,
        metadata,
    })
}

// --- trait impl --------------------------------------------------------------

#[async_trait]
impl VectorStore for PgVectorStore {
    async fn store(&self, record: VectorRecord) -> StorageResult<Uuid> {
        let metadata =
            serde_json::to_value(&record.metadata).map_err(StorageError::Serialization)?;
        let embedding = Vector::from(record.embedding);

        sqlx::query(
            r#"INSERT INTO lcm_embeddings (id, session_id, content, embedding, metadata)
               VALUES ($1, $2, $3, $4, $5)"#,
        )
        .bind(record.id)
        .bind(record.session_id)
        .bind(&record.content)
        .bind(embedding)
        .bind(metadata)
        .execute(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        Ok(record.id)
    }

    async fn get(&self, id: Uuid) -> StorageResult<Option<VectorRecord>> {
        let row = sqlx::query(
            r#"SELECT id, session_id, content, embedding, metadata
               FROM lcm_embeddings WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        match row {
            None => Ok(None),
            Some(r) => row_to_vector_record(&r).map(Some),
        }
    }

    async fn delete(&self, id: Uuid) -> StorageResult<()> {
        sqlx::query("DELETE FROM lcm_embeddings WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(StorageError::ConnectionFailed)?;
        Ok(())
    }

    async fn delete_session(&self, session_id: SessionId) -> StorageResult<()> {
        sqlx::query("DELETE FROM lcm_embeddings WHERE session_id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(StorageError::ConnectionFailed)?;
        Ok(())
    }

    async fn get_session_vectors(&self, session_id: SessionId) -> StorageResult<Vec<VectorRecord>> {
        let rows = sqlx::query(
            r#"SELECT id, session_id, content, embedding, metadata
               FROM lcm_embeddings
               WHERE session_id = $1
               ORDER BY created_at"#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        rows.iter().map(row_to_vector_record).collect()
    }

    async fn search(
        &self,
        session_id: SessionId,
        query: &[f32],
        k: usize,
    ) -> StorageResult<Vec<VectorRecord>> {
        let query_vec = Vector::from(query.to_vec());
        let limit = k as i64;

        let rows = sqlx::query(
            r#"SELECT id, session_id, content, embedding, metadata
               FROM lcm_embeddings
               WHERE session_id = $1
               ORDER BY embedding <=> $2
               LIMIT $3"#,
        )
        .bind(session_id)
        .bind(query_vec)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::ConnectionFailed)?;

        rows.iter().map(row_to_vector_record).collect()
    }
}
