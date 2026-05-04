// cli/src/migrate.rs
use crate::error::CliError;
use clap::Args;
use sqlx::{PgPool, Row};

#[derive(Debug, Args)]
pub struct MigrateCommand {
    /// Source PostgreSQL URL (TypeScript / old Rust schema)
    #[arg(long)]
    pub from_url: String,

    /// Destination PostgreSQL URL (current Rust schema)
    #[arg(long)]
    pub to_url: String,

    /// Print what would be migrated without writing
    #[arg(long)]
    pub dry_run: bool,
}

impl MigrateCommand {
    pub async fn run(self) -> Result<(), CliError> {
        let src = PgPool::connect(&self.from_url).await?;
        let dst = PgPool::connect(&self.to_url).await?;

        let session_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
            .fetch_one(&src)
            .await
            .unwrap_or(0);
        let msg_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&src)
            .await
            .unwrap_or(0);

        println!("Migration plan:");
        println!("  sessions: {}", session_count);
        println!("  messages: {}", msg_count);

        if self.dry_run {
            println!("--dry-run: no data written.");
            return Ok(());
        }

        // Run sqlx migrations on destination first
        sqlx::migrate!("../daemon/migrations")
            .run(&dst)
            .await
            .map_err(|e| CliError::Other(format!("migration failed: {e}")))?;

        // Copy sessions using raw rows
        let sessions = sqlx::query("SELECT id, created_at, updated_at, metadata FROM sessions")
            .fetch_all(&src)
            .await?;

        for s in &sessions {
            let id: uuid::Uuid = s.get("id");
            let created_at: chrono::DateTime<chrono::Utc> = s.get("created_at");
            let updated_at: chrono::DateTime<chrono::Utc> = s.get("updated_at");
            let metadata: serde_json::Value = s.get("metadata");

            sqlx::query(
                "INSERT INTO sessions (id, created_at, updated_at, metadata)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(id)
            .bind(created_at)
            .bind(updated_at)
            .bind(metadata)
            .execute(&dst)
            .await?;
        }
        println!("Copied {} sessions.", sessions.len());

        // Copy messages using raw rows
        let messages = sqlx::query(
            "SELECT id, session_id, role, content, token_count, metadata, created_at FROM messages",
        )
        .fetch_all(&src)
        .await?;

        for m in &messages {
            let id: uuid::Uuid = m.get("id");
            let session_id: uuid::Uuid = m.get("session_id");
            let role: String = m.get("role");
            let content: String = m.get("content");
            let token_count: i64 = m.get("token_count");
            let metadata: serde_json::Value = m.get("metadata");
            let created_at: chrono::DateTime<chrono::Utc> = m.get("created_at");

            sqlx::query(
                "INSERT INTO messages (id, session_id, role, content, token_count, metadata, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(id)
            .bind(session_id)
            .bind(role)
            .bind(content)
            .bind(token_count)
            .bind(metadata)
            .bind(created_at)
            .execute(&dst)
            .await?;
        }
        println!("Copied {} messages.", messages.len());

        println!("Migration complete.");
        Ok(())
    }
}
