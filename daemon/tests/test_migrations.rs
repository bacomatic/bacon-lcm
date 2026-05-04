// daemon/tests/test_migrations.rs
mod helpers;

#[tokio::test]
async fn migrations_apply_cleanly() {
    let pool = helpers::test_pool().await;

    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM information_schema.tables \
         WHERE table_schema = 'public' \
         AND table_name IN ('lcm_sessions','lcm_messages','lcm_summary_nodes','lcm_embeddings')"
    )
    .fetch_one(&pool)
    .await
    .expect("query failed");

    assert_eq!(count, 4, "all four tables must exist after migrations");
}
