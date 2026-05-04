// daemon/tests/helpers.rs
use sqlx::PgPool;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

pub async fn test_pool() -> PgPool {
    let container = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .expect("failed to start Postgres container");

    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("failed to get container port");

    let url = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let pool = sqlx::PgPool::connect(&url)
        .await
        .expect("failed to connect to test database");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations failed");

    std::mem::forget(container);
    pool
}
