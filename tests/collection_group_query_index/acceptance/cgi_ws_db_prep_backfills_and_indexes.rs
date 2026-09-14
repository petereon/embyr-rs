// @walking_skeleton @driving_port @real-io @US-02
//! US-02 walking skeleton — the CLI driving port (`embyr-db-prep`), not just
//! the adapter methods it wraps, actually backfills pre-existing documents
//! and builds the collection-group indexes after `migrate()` succeeds
//! (ADR-080 § Component Boundaries: "after its existing migrate() call,
//! adds calls to ensure_collection_group_indexes() then
//! backfill_collection_id()").
//!
//! Per nw-distill's Driving Adapter Verification mandate: a pipeline/
//! adapter-level test (`crates/embyr-pg-storage/tests/cgi_backfill.rs`)
//! proves the BACKFILL MECHANISM works; it does NOT prove the CLI entry
//! point actually invokes it. This walking skeleton is that proof — real
//! subprocess, real exit code, real stdout.
//!
//! Journey: a DBA with pre-existing, un-backfilled documents in their
//! database re-runs embyr-db-prep (the same tool that already applies
//! migrations) and sees confirmation that collection_id backfill and index
//! preparation both ran, with the tool still exiting 0.
//!
//! Driving port: embyr-db-prep subprocess (CARGO_BIN_EXE_embyr-db-prep).
//! Reuses customer_db_onboarding's own common module (start_postgres_
//! container/create_ddl_role/role_connection_url/run_db_prep) — same
//! cross-feature reuse convention `composite_index_real_creation` already
//! established for `firestore_composite_indexes_admin_api`'s own common.
//!
//! Resource-conscious: dozens of pre-existing documents, not thousands.
//!
//! Interface this test specifies for DELIVER (test-driven, not
//! pre-implemented by DISTILL):
//!   - after migrate() succeeds, embyr-db-prep calls
//!     ensure_collection_group_indexes() then backfill_collection_id()
//!     (batch_size/throttle overridable via EMBYR_DB_PREP_BACKFILL_BATCH_SIZE
//!     / EMBYR_DB_PREP_BACKFILL_THROTTLE_MS env vars, so this test can keep
//!     batches small on an 8GB machine)
//!   - stdout contains a line naming how many documents were backfilled,
//!     e.g. "collection_id backfill complete: {n} documents backfilled"
//!
//! NOT #[ignore] — walking skeleton. RED today: the column/trigger this
//! fixture relies on do not exist yet (DELIVER's own migration file), so
//! the pre-existing-document fixture itself fails first — still a correct
//! RED (schema not yet migrated), consistent with every other scenario in
//! this feature.

#[path = "../../customer_db_onboarding/common/mod.rs"]
mod common;
use common::{create_ddl_role, role_connection_url, run_db_prep, start_postgres_container};

#[tokio::test]
async fn embyr_db_prep_backfills_pre_existing_documents_and_builds_the_collection_group_indexes() {
    let (_pg, db_url) = start_postgres_container().await;

    let sys_pool = sqlx::PgPool::connect(&db_url).await.expect("connect as superuser");
    create_ddl_role(&sys_pool, "elena_dba", "postgres").await;
    let elena_dsn = role_connection_url(&db_url, "elena_dba");

    // Given: pre-existing documents that predate the collection_id column
    // (simulated by disabling the trigger, mirroring
    // embyr-pg-storage/tests/common/mod.rs's own
    // seed_pre_existing_document_without_collection_id technique, applied
    // directly here since this test's own driving port is the CLI, not the
    // adapter).
    let elena_pool = sqlx::PgPool::connect(&elena_dsn).await.expect("connect as elena_dba");
    // First run establishes the schema (column + trigger + indexes are all
    // additive per ADR-080, so this first run is itself a normal, expected
    // "fresh database" prep — real behavior a first-time customer sees too).
    let first_run = run_db_prep(&[("EMBYR_DB_PREP_DSN", &elena_dsn)]);
    assert_eq!(first_run.exit_code, Some(0), "first-time prep must exit 0; stderr: {}", first_run.stderr);

    for n in 0..25 {
        sqlx::query("ALTER TABLE documents DISABLE TRIGGER documents_collection_id_biu")
            .execute(&elena_pool)
            .await
            .expect("disable trigger for fixture");
        sqlx::query(
            "INSERT INTO documents (project_id, collection_path, document_id, fields, version, \
             create_time, update_time) VALUES ($1, 'products/p1/reviews', $2, '{}'::jsonb, 1, NOW(), NOW())",
        )
        .bind("trailmark-prod-cgi-ws")
        .bind(format!("pre-{n}"))
        .execute(&elena_pool)
        .await
        .expect("insert pre-existing document");
        sqlx::query("ALTER TABLE documents ENABLE TRIGGER documents_collection_id_biu")
            .execute(&elena_pool)
            .await
            .expect("re-enable trigger");
    }

    // When: the DBA re-runs embyr-db-prep — migrate() is now a no-op
    // (already applied), but the backfill/index steps should still run
    // against the pre-existing NULL rows.
    let run = run_db_prep(&[
        ("EMBYR_DB_PREP_DSN", &elena_dsn),
        ("EMBYR_DB_PREP_BACKFILL_BATCH_SIZE", "5"),
        ("EMBYR_DB_PREP_BACKFILL_THROTTLE_MS", "5"),
    ]);

    assert_eq!(run.exit_code, Some(0), "backfill/index run must exit 0; stderr: {}", run.stderr);
    assert!(
        run.stdout.contains("collection_id backfill complete: 25 documents backfilled"),
        "expected the CLI to confirm exactly 25 documents backfilled, got stdout: {}",
        run.stdout
    );

    let null_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM documents WHERE collection_id IS NULL",
    )
    .fetch_one(&elena_pool)
    .await
    .expect("count null collection_id");
    assert_eq!(null_count, 0, "every pre-existing document must be backfilled by the CLI run");

    let idx_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'documents_collection_group_idx')",
    )
    .fetch_one(&elena_pool)
    .await
    .expect("check index existence");
    assert!(idx_exists, "the CLI run must also build documents_collection_group_idx");
}
