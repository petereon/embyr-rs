//! Shared helpers — hourly-metrics-rollup (ADR-084) acceptance tests that need
//! only a system DB (no admin HTTP surface). Each test gets its own Postgres
//! testcontainer, migrated with the real `migrations/`.

#![allow(dead_code)]

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use embyr_core::metrics_rollup::truncate_to_hour;
use embyr_server::adapters::system_db::SystemDb;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

pub async fn start_system_db() -> (ContainerAsync<Postgres>, String, Arc<SystemDb>) {
    let container = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("start Postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("host port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let db = Arc::new(SystemDb::new(&url).await.expect("SystemDb::new"));
    db.migrate().await.expect("system DB migrations");
    (container, url, db)
}

/// Insert an `active` project (own account) and return its id.
pub async fn insert_project(pool: &sqlx::PgPool, project_id: &str) {
    let account_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (name) VALUES ('hmr-test-account') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("insert account");
    sqlx::query(
        "INSERT INTO projects (id, account_id, backend_mode, api_key_hash_current, status, name) \
         VALUES ($1, $2, 'direct_pg', 'unused-hash', 'active', $1)",
    )
    .bind(project_id)
    .bind(account_id)
    .execute(pool)
    .await
    .expect("insert project");
}

/// The UTC hour `hours_ago` hours before the current one.
pub fn hour_ago(hours_ago: i64) -> DateTime<Utc> {
    truncate_to_hour(Utc::now()) - Duration::hours(hours_ago)
}

/// `(requests, units, errors, Σ buckets)` of one rollup row, `None` if absent.
pub async fn rollup_row(
    pool: &sqlx::PgPool,
    project_id: &str,
    hour: DateTime<Utc>,
    op: i16,
) -> Option<(i64, i64, i64, i64)> {
    sqlx::query_as(
        "SELECT requests, units, errors, \
                (b00+b01+b02+b03+b04+b05+b06+b07+b08+b09+b10+b11+b12)::bigint \
         FROM hourly_project_metrics WHERE project_id = $1 AND hour = $2 AND op = $3",
    )
    .bind(project_id)
    .bind(hour)
    .bind(op)
    .fetch_optional(pool)
    .await
    .expect("query rollup row")
}

pub async fn rollup_row_count(pool: &sqlx::PgPool, project_id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM hourly_project_metrics WHERE project_id = $1")
        .bind(project_id)
        .fetch_one(pool)
        .await
        .expect("count rollup rows")
}
