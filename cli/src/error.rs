// cli/src/error.rs
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("LCM error: {0}")]
    Lcm(#[from] bacon_lcm_core::LcmError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}
