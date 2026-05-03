// core/src/error.rs
use crate::types::{MessageId, SessionId, SummaryId};
use thiserror::Error;

/// Main LCM error type
#[derive(Error, Debug)]
pub enum LcmError {
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("Compaction error: {0}")]
    Compaction(#[from] CompactionError),

    #[error("Provider error: {0}")]
    Provider(#[from] ProviderError),

    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("Session not found: {0}")]
    SessionNotFound(SessionId),

    #[error("Message not found: {0}")]
    MessageNotFound(MessageId),

    #[error("Summary not found: {0}")]
    SummaryNotFound(SummaryId),

    #[error("Token limit exceeded: {current}/{max}")]
    TokenLimitExceeded { current: usize, max: usize },

    #[error("Invalid compaction level for operation")]
    InvalidCompactionLevel,

    #[error("Lineage cycle detected")]
    LineageCycle,

    #[error("Database connection failed: {0}")]
    DatabaseConnection(String),
}

/// Storage-related errors
#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Database connection failed: {0}")]
    ConnectionFailed(#[from] sqlx::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("UUID parsing error: {0}")]
    UuidParse(#[from] uuid::Error),

    #[error("Constraint violation: {0}")]
    ConstraintViolation(String),

    #[error("Migration failed: {0}")]
    MigrationFailed(String),
}

/// Compaction-related errors
#[derive(Error, Debug)]
pub enum CompactionError {
    #[error("No messages available for compaction")]
    NoMessagesToCompact,

    #[error("Compaction already in progress")]
    CompactionInProgress,

    #[error("Insufficient tokens for compaction: {0}")]
    InsufficientTokens(usize),

    #[error("Provider error during compaction: {0}")]
    ProviderError(#[from] ProviderError),

    #[error("Storage error during compaction: {0}")]
    StorageError(#[from] StorageError),

    #[error("Lineage validation failed: {0}")]
    LineageValidation(String),
}

/// Provider-related errors
#[derive(Error, Debug)]
pub enum ProviderError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Rate limited: retry after {0} seconds")]
    RateLimited(u64),

    #[error("Invalid API key or authentication failed")]
    AuthenticationFailed,

    #[error("Model not supported: {0}")]
    UnsupportedModel(String),

    #[error("Token limit exceeded for provider")]
    ProviderTokenLimitExceeded,

    #[error("Invalid response format: {0}")]
    InvalidResponse(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),
}

/// Configuration-related errors
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to load config file: {0}")]
    FileLoad(#[from] std::io::Error),

    #[error("Invalid configuration format: {0}")]
    InvalidFormat(#[from] serde_json::Error),

    #[error("Missing required configuration: {0}")]
    MissingRequired(String),

    #[error("Invalid configuration value: {0}")]
    InvalidValue(String),

    #[error("Environment variable error: {0}")]
    EnvironmentVar(#[from] std::env::VarError),
}

/// Result type alias for convenience
pub type LcmResult<T> = Result<T, LcmError>;
pub type StorageResult<T> = Result<T, StorageError>;
/// Note: CompactionOpResult is used instead of CompactionResult to avoid conflict
/// with the `CompactionResult` struct in `types.rs`.
pub type CompactionOpResult<T> = Result<T, CompactionError>;
pub type ProviderResult<T> = Result<T, ProviderError>;
pub type ConfigResult<T> = Result<T, ConfigError>;
