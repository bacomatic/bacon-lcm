// core/tests/property_tests.rs
//! Property-based tests for core compaction invariants.
//!
//! These run synchronously with proptest; tokio is entered via
//! `tokio::runtime::Runtime::new().unwrap().block_on(...)`.

use proptest::prelude::*;
use tokio::runtime::Runtime;

use bacon_lcm_core::{
    LcmConfig,
    providers::{create_token_counter, create_summarizer, create_embedder},
    storage::StorageLayer,
    session::LcmSession,
    types::{MessageRole, CompactionConfig, ThresholdConfig},
};

fn rt() -> Runtime {
    Runtime::new().unwrap()
}

async fn make_session_with_config(config: LcmConfig) -> LcmSession {
    let token_counter = create_token_counter("naive", None).unwrap();
    let summarizer    = create_summarizer("echo", "echo".to_string(), None, None, None, None).unwrap();
    let embedder      = create_embedder("null", None, None, None, None).unwrap();
    LcmSession::new(token_counter, summarizer, embedder, config, StorageLayer::memory())
        .await
        .unwrap()
}

/// Config with a very low threshold so compaction triggers during the test.
/// Note: validation requires soft_limit < hard_limit <= model_max_tokens.
fn tight_config() -> LcmConfig {
    let mut config = LcmConfig::defaults();
    config.compaction = CompactionConfig {
        thresholds: ThresholdConfig {
            model_max_tokens: 300,
            soft_limit: 150,
            hard_limit: 250,
        },
        fresh_tail_count: 2,
        leaf_group_size: 5,
        condensed_group_size: 3,
        parallel_compaction: false,
        max_concurrent_compactions: 1,
    };
    config
}

proptest! {
    /// Property: after adding N messages, message_count + summary_count > 0.
    #[test]
    fn session_counts_consistent(
        messages in prop::collection::vec("[a-z ]{1,40}", 1usize..=30)
    ) {
        let rt = rt();
        let result: Result<(), TestCaseError> = rt.block_on(async {
            let mut session = make_session_with_config(tight_config()).await;
            for msg in &messages {
                session.add_message(MessageRole::User, msg.clone()).await.unwrap();
            }
            let info = session.get_session_info().await.unwrap();
            // At least one message or summary must exist
            prop_assert!(info.message_count + info.summary_count > 0);
            // Token count must be non-negative (usize is always >= 0, but verify it's accessible)
            prop_assert!(info.token_count < usize::MAX);
            Ok(())
        });
        result?;
    }

    /// Property: context items are returned in non-decreasing timestamp order.
    #[test]
    fn context_items_ordered_by_timestamp(
        messages in prop::collection::vec("[a-z ]{1,30}", 1usize..=20)
    ) {
        let rt = rt();
        let result: Result<(), TestCaseError> = rt.block_on(async {
            let mut session = make_session_with_config(tight_config()).await;
            for msg in &messages {
                session.add_message(MessageRole::Assistant, msg.clone()).await.unwrap();
            }
            let context = session.get_context().await.unwrap();
            // Timestamps must be non-decreasing
            for window in context.windows(2) {
                prop_assert!(
                    window[0].timestamp() <= window[1].timestamp(),
                    "context out of order: {:?} > {:?}",
                    window[0].timestamp(),
                    window[1].timestamp()
                );
            }
            Ok(())
        });
        result?;
    }

    /// Property: token count is positive after adding non-empty messages.
    #[test]
    fn token_count_positive_after_messages(
        messages in prop::collection::vec("[a-z]{4,80}", 1usize..=10)
    ) {
        let rt = rt();
        let result: Result<(), TestCaseError> = rt.block_on(async {
            // Use a loose config so no compaction occurs (thresholds must satisfy
            // soft_limit < hard_limit <= model_max_tokens per LcmConfig::validate).
            let mut config = LcmConfig::defaults();
            config.compaction.thresholds.model_max_tokens = 10_000_000;
            config.compaction.thresholds.hard_limit       = 9_000_000;
            config.compaction.thresholds.soft_limit       = 8_000_000;

            let mut session = make_session_with_config(config).await;
            for msg in &messages {
                session.add_message(MessageRole::User, msg.clone()).await.unwrap();
            }
            let token_count = session.get_token_count().await.unwrap();
            prop_assert!(token_count > 0, "expected token_count > 0, got {}", token_count);
            Ok(())
        });
        result?;
    }
}
