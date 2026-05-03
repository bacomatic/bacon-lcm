// core/src/compaction.rs
use crate::error::{CompactionError, CompactionOpResult};
use crate::types::{CompactionConfig, CompactionResult, SessionId};

/// Compaction engine for three-level escalation protocol
pub struct CompactionEngine {
    config: CompactionConfig,
}

impl CompactionEngine {
    pub fn new(config: CompactionConfig) -> Self {
        Self { config }
    }

    /// Perform standard (L1/L2) compaction
    pub async fn compact(
        &self,
        _session_id: SessionId,
    ) -> CompactionOpResult<CompactionResult> {
        // TODO: Implement full compaction logic in a future task
        Err(CompactionError::NoMessagesToCompact)
    }

    /// Perform emergency (L3) compaction
    pub async fn emergency_compaction(
        &self,
        _session_id: SessionId,
    ) -> CompactionOpResult<CompactionResult> {
        // TODO: Implement emergency compaction in a future task
        Err(CompactionError::NoMessagesToCompact)
    }

    /// Get current compaction configuration
    pub fn config(&self) -> &CompactionConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::new_session_id;
    use crate::types::CompactionConfig;

    #[test]
    fn test_engine_creation() {
        let config = CompactionConfig::default();
        let engine = CompactionEngine::new(config.clone());
        assert_eq!(engine.config().thresholds.soft_limit, config.thresholds.soft_limit);
    }

    #[tokio::test]
    async fn test_compact_returns_error_when_no_messages() {
        let engine = CompactionEngine::new(CompactionConfig::default());
        let session_id = new_session_id();

        let result = engine.compact(session_id).await;
        assert!(result.is_err());
        matches!(result.unwrap_err(), CompactionError::NoMessagesToCompact);
    }

    #[tokio::test]
    async fn test_emergency_compaction_returns_error_when_no_messages() {
        let engine = CompactionEngine::new(CompactionConfig::default());
        let session_id = new_session_id();

        let result = engine.emergency_compaction(session_id).await;
        assert!(result.is_err());
        matches!(result.unwrap_err(), CompactionError::NoMessagesToCompact);
    }
}
