// core/benches/compaction.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;

use bacon_lcm_core::{
    LcmConfig,
    providers::{create_token_counter, create_summarizer, create_embedder},
    storage::StorageLayer,
    session::LcmSession,
    types::{MessageRole, CompactionConfig, ThresholdConfig},
};

/// Build a session with the given compaction config.
async fn make_session(config: LcmConfig) -> LcmSession {
    let token_counter = create_token_counter("naive", None).unwrap();
    let summarizer    = create_summarizer("echo", "echo".to_string(), None, None, None, None).unwrap();
    let embedder      = create_embedder("null", None, None, None, None).unwrap();
    LcmSession::new(token_counter, summarizer, embedder, config, StorageLayer::memory())
        .await
        .unwrap()
}

/// Config with a very low token threshold so compaction triggers quickly.
fn tight_config() -> LcmConfig {
    let mut config = LcmConfig::defaults();
    config.compaction = CompactionConfig {
        thresholds: ThresholdConfig {
            model_max_tokens: 200,
            soft_limit: 100,
            hard_limit: 150,
        },
        fresh_tail_count: 2,
        leaf_group_size: 5,
        condensed_group_size: 3,
        parallel_compaction: false,
        max_concurrent_compactions: 1,
    };
    config
}

/// Config with a very high threshold (no compaction during the benchmark).
fn loose_config() -> LcmConfig {
    let mut config = LcmConfig::defaults();
    config.compaction = CompactionConfig {
        thresholds: ThresholdConfig {
            model_max_tokens: 10_000_000,
            soft_limit:        8_000_000,
            hard_limit:        9_000_000,
        },
        fresh_tail_count: 10,
        leaf_group_size: 20,
        condensed_group_size: 10,
        parallel_compaction: false,
        max_concurrent_compactions: 1,
    };
    config
}

fn bench_compaction(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    c.bench_function("leaf_compaction_20_messages", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut session = make_session(tight_config()).await;
                for i in 0..20_u32 {
                    session
                        .add_message(MessageRole::User, format!("message number {i} with some padding content here"))
                        .await
                        .unwrap();
                }
                black_box(session.get_session_info().await.unwrap());
            });
        });
    });

    c.bench_function("get_context_100_messages", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut session = make_session(loose_config()).await;
                for i in 0..100_u32 {
                    session
                        .add_message(MessageRole::User, format!("context message {i}"))
                        .await
                        .unwrap();
                }
                black_box(session.get_context().await.unwrap())
            });
        });
    });

    c.bench_function("add_message_throughput_500", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut session = make_session(loose_config()).await;
                for i in 0..500_u32 {
                    session
                        .add_message(MessageRole::Assistant, format!("bench msg {i}"))
                        .await
                        .unwrap();
                }
                black_box(session.get_token_count().await.unwrap())
            });
        });
    });
}

criterion_group!(benches, bench_compaction);
criterion_main!(benches);
