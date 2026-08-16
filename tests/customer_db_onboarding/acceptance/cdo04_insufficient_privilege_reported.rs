// @error @driving_port @real-io @US-01 @AC-01-04
//! Insufficient privilege is reported with actionable detail, not a raw
//! driver error.
//!
//! Journey (DISCUSS domain example 3):
//!   Given: Elena's login lacks the schema-modification (CREATE) privilege
//!          required on the target database.
//!   When:  Elena runs embyr-db-prep against it.
//!   Then:  Elena sees a message naming the missing privilege and the
//!          target database, and the tool exits non-zero.
//!
//! AC-01-04.

#[path = "../common/mod.rs"]
mod common;
use common::{create_postgres_role, role_connection_url, run_db_prep, start_postgres_container};

#[tokio::test]
#[ignore]
async fn insufficient_privilege_is_reported_with_actionable_detail() {
    let (_pg, db_url) = start_postgres_container().await;

    // Given: elena_dba is scoped to a role with CONNECT only — no CREATE.
    let sys_pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    create_postgres_role(&sys_pool, "elena_dba", &[]).await;
    drop(sys_pool);
    let elena_dsn = role_connection_url(&db_url, "elena_dba");

    // When: Elena runs the prep step.
    let run = run_db_prep(&[("EMBYR_DB_PREP_DSN", &elena_dsn)]);

    // Then: non-zero exit, message names the privilege and the database —
    // not a raw Postgres driver error stack trace.
    assert_ne!(
        run.exit_code,
        Some(0),
        "AC-01-04: insufficient-privilege run must exit non-zero"
    );
    let combined = format!("{}{}", run.stdout, run.stderr).to_lowercase();
    assert!(
        combined.contains("privilege") || combined.contains("permission"),
        "AC-01-04: message must name the missing privilege; got stdout={:?} stderr={:?}",
        run.stdout,
        run.stderr
    );
    assert!(
        combined.contains("postgres") || combined.contains("database") || combined.contains("elena_dba"),
        "AC-01-04: message must name the target database/role; got stdout={:?} stderr={:?}",
        run.stdout,
        run.stderr
    );
    assert!(
        !combined.contains("panicked at") && !combined.contains("unwrap"),
        "AC-01-04: message must not be a raw driver/panic stack trace; got stdout={:?} stderr={:?}",
        run.stdout,
        run.stderr
    );
}
