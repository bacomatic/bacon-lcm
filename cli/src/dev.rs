// cli/src/dev.rs
use clap::Args;
use tracing::info;

use bacon_lcm_core::{
    providers::{create_embedder, create_summarizer, create_token_counter},
    storage::StorageLayer,
    LcmConfig, LcmError, LcmSession,
};

use crate::error::CliError;

#[derive(Debug, Args)]
pub struct DevCommand {
    /// Enable hot-reload watch mode (stub: logs intent only)
    #[arg(short, long)]
    pub watch: bool,

    /// PostgreSQL DATABASE_URL (overrides env var)
    #[arg(short, long)]
    pub database_url: Option<String>,
}

impl DevCommand {
    pub async fn run(self) -> Result<(), CliError> {
        if self.watch {
            info!("--watch requested; hot-reload is not yet implemented");
        }

        let db_url = self
            .database_url
            .or_else(|| std::env::var("DATABASE_URL").ok());

        let storage = match db_url {
            Some(url) => {
                info!("Connecting to Postgres at {url}");
                let pool = sqlx::PgPool::connect(&url).await?;
                bacon_lcm_daemon::storage::postgres_layer(pool)
            }
            None => {
                info!("No DATABASE_URL — using in-memory storage");
                StorageLayer::memory()
            }
        };

        let token_counter = create_token_counter("naive", None).map_err(LcmError::from)?;
        let summarizer = create_summarizer("echo", "echo".to_string(), None, None, None, None)
            .map_err(LcmError::from)?;
        let embedder = create_embedder("null", None, None, None, None).map_err(LcmError::from)?;

        let session = LcmSession::new(
            token_counter,
            summarizer,
            embedder,
            LcmConfig::defaults(),
            storage,
        )
        .await?;

        let info = session.get_session_info().await?;
        println!("LCM dev session started: {}", info.session.id);
        println!("  messages : {}", info.message_count);
        println!("  tokens   : {}", info.token_count);
        println!("  summaries: {}", info.summary_count);

        Ok(())
    }
}
