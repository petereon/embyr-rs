//! hourly-metrics-rollup (ADR-084) slice 3 — retention sweeper.
//!
//! AC-HMR-08: rows older than the retention window are deleted, newer rows
//! are kept; deleting a project removes its rows with it. Driven through
//! `run_cycle` (the sweeper's own testable port), like its sibling sweepers.

#[path = "../common/mod.rs"]
mod common;

use common::{hour_ago, insert_project, rollup_row_count, start_system_db};
use embyr_server::sweepers::hourly_metrics_retention_sweeper::run_cycle;

const RETENTION_DAYS: i64 = 14;

async fn seed_row(pool: &sqlx::PgPool, project_id: &str, hour: chrono::DateTime<chrono::Utc>) {
    sqlx::query(
        "INSERT INTO hourly_project_metrics (project_id, hour, op, requests, b00) \
         VALUES ($1, $2, 0, 1, 1)",
    )
    .bind(project_id)
    .bind(hour)
    .execute(pool)
    .await
    .expect("seed rollup row");
}

// @AC-HMR-08 @real-io
#[tokio::test]
async fn sweeper_deletes_only_rows_older_than_retention() {
    let (_container, _url, db) = start_system_db().await;
    insert_project(db.pool(), "hmr-retention").await;
    let day = 24;
    seed_row(db.pool(), "hmr-retention", hour_ago(15 * day)).await;
    seed_row(db.pool(), "hmr-retention", hour_ago(RETENTION_DAYS * day + 2)).await;
    seed_row(db.pool(), "hmr-retention", hour_ago(13 * day)).await;
    seed_row(db.pool(), "hmr-retention", hour_ago(1)).await;

    run_cycle(&db, RETENTION_DAYS).await;

    let remaining: Vec<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT hour FROM hourly_project_metrics WHERE project_id = 'hmr-retention' ORDER BY hour",
    )
    .fetch_all(db.pool())
    .await
    .expect("read remaining rows");
    assert_eq!(remaining, vec![hour_ago(13 * day), hour_ago(1)]);

    run_cycle(&db, RETENTION_DAYS).await; // idempotent
    assert_eq!(rollup_row_count(db.pool(), "hmr-retention").await, 2);
}

// @AC-HMR-08 @real-io
#[tokio::test]
async fn deleting_a_project_removes_its_rollup_rows() {
    let (_container, _url, db) = start_system_db().await;
    insert_project(db.pool(), "hmr-cascade").await;
    seed_row(db.pool(), "hmr-cascade", hour_ago(1)).await;
    assert_eq!(rollup_row_count(db.pool(), "hmr-cascade").await, 1);

    sqlx::query("DELETE FROM projects WHERE id = 'hmr-cascade'")
        .execute(db.pool())
        .await
        .expect("delete project");

    assert_eq!(rollup_row_count(db.pool(), "hmr-cascade").await, 0);
}
