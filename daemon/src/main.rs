// daemon/src/main.rs
use anyhow::Context;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL must be set")?;

    let pool = bacon_lcm_daemon::db::connect(&database_url)
        .await
        .context("failed to connect to database")?;

    bacon_lcm_daemon::db::run_migrations(&pool)
        .await
        .context("failed to run migrations")?;

    tracing::info!("bacon-lcm-daemon started");
    Ok(())
}
