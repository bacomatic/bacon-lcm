// core/src/compaction/strategy.rs
//! Compaction strategy selection logic.
//!
//! Determines which compaction level to apply based on the current token count
//! relative to the configured soft and hard thresholds.

use crate::types::CompactionConfig;

/// The compaction level selected by the strategy engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionLevel {
    /// No compaction needed — token count is within the soft limit.
    None,
    /// L1: Leaf compaction — group recent messages into leaf summaries.
    Leaf,
    /// L2: Condensed compaction — merge leaf summaries into higher-level nodes.
    Condensed,
    /// L3: Emergency compaction — aggressively archive oldest summaries.
    Emergency,
}

/// Decide the appropriate compaction level based on current token usage.
///
/// The escalation ladder is:
/// 1. `None` when `current_tokens <= soft_limit`
/// 2. `Leaf` when `current_tokens > soft_limit`  (default first attempt)
/// 3. `Condensed` when L1 was insufficient and we are still over soft_limit
/// 4. `Emergency` when `current_tokens > hard_limit`
pub fn select_compaction_level(
    current_tokens: usize,
    config: &CompactionConfig,
) -> CompactionLevel {
    if current_tokens <= config.thresholds.soft_limit {
        CompactionLevel::None
    } else if current_tokens > config.thresholds.hard_limit {
        CompactionLevel::Emergency
    } else {
        CompactionLevel::Leaf
    }
}

/// After an L1 pass, decide whether further compaction is needed.
pub fn select_post_l1_level(
    current_tokens: usize,
    config: &CompactionConfig,
) -> CompactionLevel {
    if current_tokens <= config.thresholds.soft_limit {
        CompactionLevel::None
    } else if current_tokens > config.thresholds.hard_limit {
        CompactionLevel::Emergency
    } else {
        CompactionLevel::Condensed
    }
}

/// After an L2 pass, decide whether emergency compaction is needed.
pub fn select_post_l2_level(
    current_tokens: usize,
    config: &CompactionConfig,
) -> CompactionLevel {
    if current_tokens <= config.thresholds.hard_limit {
        CompactionLevel::None
    } else {
        CompactionLevel::Emergency
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CompactionConfig;

    fn test_config() -> CompactionConfig {
        CompactionConfig {
            thresholds: crate::types::ThresholdConfig {
                model_max_tokens: 128000,
                soft_limit: 80000,
                hard_limit: 110000,
            },
            fresh_tail_count: 10,
            leaf_group_size: 20,
            condensed_group_size: 10,
            parallel_compaction: true,
            max_concurrent_compactions: 4,
        }
    }

    #[test]
    fn test_no_compaction_when_under_soft_limit() {
        let config = test_config();
        let level = select_compaction_level(50000, &config);
        assert_eq!(level, CompactionLevel::None);
    }

    #[test]
    fn test_no_compaction_at_soft_limit_boundary() {
        let config = test_config();
        let level = select_compaction_level(80000, &config);
        assert_eq!(level, CompactionLevel::None);
    }

    #[test]
    fn test_leaf_compaction_above_soft_limit() {
        let config = test_config();
        let level = select_compaction_level(90000, &config);
        assert_eq!(level, CompactionLevel::Leaf);
    }

    #[test]
    fn test_emergency_compaction_above_hard_limit() {
        let config = test_config();
        let level = select_compaction_level(120000, &config);
        assert_eq!(level, CompactionLevel::Emergency);
    }

    #[test]
    fn test_emergency_at_hard_limit_boundary() {
        // At hard_limit exactly we are NOT over, so Leaf
        let config = test_config();
        let level = select_compaction_level(110000, &config);
        assert_eq!(level, CompactionLevel::Leaf);
    }

    #[test]
    fn test_post_l1_escalation_to_condensed() {
        let config = test_config();
        // Still between soft and hard after L1
        let level = select_post_l1_level(95000, &config);
        assert_eq!(level, CompactionLevel::Condensed);
    }

    #[test]
    fn test_post_l1_no_further_compaction() {
        let config = test_config();
        let level = select_post_l1_level(70000, &config);
        assert_eq!(level, CompactionLevel::None);
    }

    #[test]
    fn test_post_l1_emergency_needed() {
        let config = test_config();
        let level = select_post_l1_level(115000, &config);
        assert_eq!(level, CompactionLevel::Emergency);
    }

    #[test]
    fn test_post_l2_emergency_needed() {
        let config = test_config();
        let level = select_post_l2_level(115000, &config);
        assert_eq!(level, CompactionLevel::Emergency);
    }

    #[test]
    fn test_post_l2_no_emergency_needed() {
        let config = test_config();
        let level = select_post_l2_level(100000, &config);
        assert_eq!(level, CompactionLevel::None);
    }
}
