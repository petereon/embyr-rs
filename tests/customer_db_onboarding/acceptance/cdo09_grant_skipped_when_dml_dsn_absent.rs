// @real-io @US-01
//! OQ-5 sequencing scenario (feature-delta.md § Wave: DESIGN / Open
//! Questions, and § DESIGN Handoff Package flag 4c): the revised role-scoped
//! grant introduces an ordering dependency. If Elena runs embyr-db-prep
//! without `EMBYR_DB_PREP_DML_ROLE_DSN` (e.g. before the `embyr_app` role
//! exists yet), `verify_schema_readiness()` reports `NotPrepped` at
//! provisioning time even though the schema itself is fully applied, until
//! the tool is re-run (idempotently) with the DML-role DSN supplied.
//!
//! Journey (chained across three steps — each reuses the prior step's own
//! state, not copy-pasted fixtures):
//!   Given: a fresh database and an elevated role only (no DML role exists
//!          yet, mirroring "Elena preps the database before her ops team
//!          has created the embyr_app role").
//!   When (1): Elena runs embyr-db-prep WITHOUT the DML-role DSN.
//!   Then (1): migration success is still reported via the unchanged
//!             success message; an informational (non-error) note explains
//!             read-verification access was not established.
//!   When (2): `verify_schema_readiness()` is called using a DML-only role
//!             that now exists but was never granted.
//!   Then (2): it reports `NotPrepped`, not an unhandled error — even
//!             though the schema itself is fully applied.
//!   When (3): Elena re-runs embyr-db-prep, this time WITH the DML-role DSN.
//!   Then (3): the grant succeeds, and a subsequent
//!             `verify_schema_readiness()` call (as the DML role) now
//!             reports `Ready`.

#[path = "../common/mod.rs"]
mod common;
use common::{create_ddl_role, create_postgres_role, role_connection_url, run_db_prep, start_postgres_container};
use embyr_pg_storage::backend_adapter::PostgresBackendAdapter;

#[tokio::test]
async fn grant_step_is_optional_and_a_followup_run_closes_the_sequencing_gap() {
    let (_pg, db_url) = start_postgres_container().await;
    let sys_pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    create_ddl_role(&sys_pool, "elena_dba", "postgres").await;
    drop(sys_pool);
    let elena_dsn = role_connection_url(&db_url, "elena_dba");

    // Step 1: prep run without the DML-role DSN.
    let first_run = run_db_prep(&[("EMBYR_DB_PREP_DSN", &elena_dsn)]);
    assert_eq!(
        first_run.exit_code,
        Some(0),
        "step 1: migration success must still be reported unchanged; stderr: {}",
        first_run.stderr
    );
    assert!(
        first_run.stdout.to_lowercase().contains("ready"),
        "step 1: unchanged success message expected; got: {}",
        first_run.stdout
    );
    let combined = format!("{}{}", first_run.stdout, first_run.stderr).to_lowercase();
    assert!(
        combined.contains("not established")
            || combined.contains("grant") && combined.contains("skip"),
        "step 1: an informational (non-error) note about the unestablished \
         read-verification access is expected; got stdout={:?} stderr={:?}",
        first_run.stdout,
        first_run.stderr
    );

    // embyr_app now created by the ops team, but never granted (the tool
    // was never re-run with its DSN yet).
    let sys_pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    create_postgres_role(&sys_pool, "embyr_app", &[]).await;
    drop(sys_pool);
    let embyr_app_dsn = role_connection_url(&db_url, "embyr_app");

    // Step 2: verify_schema_readiness() as the ungranted DML role.
    let dml_adapter = PostgresBackendAdapter::new(&embyr_app_dsn)
        .await
        .expect("connect as embyr_app to call verify_schema_readiness");
    let readiness_before_grant = dml_adapter.verify_schema_readiness().await;
    assert!(
        readiness_before_grant.is_ok(),
        "step 2: verify_schema_readiness() must return a handled result \
         (NotPrepped), not an unhandled Err, even though the schema itself \
         is fully applied; got: {:?}",
        readiness_before_grant.err()
    );
    assert!(
        matches!(
            readiness_before_grant.unwrap(),
            embyr_core::domain::schema_readiness::SchemaReadiness::NotPrepped { .. }
        ),
        "step 2: an ungranted DML role must see NotPrepped, not Ready, even \
         though the schema is fully applied (OQ-5's documented trade-off)"
    );

    // Step 3: Elena re-runs the tool, this time with the DML-role DSN.
    let second_run = run_db_prep(&[
        ("EMBYR_DB_PREP_DSN", &elena_dsn),
        ("EMBYR_DB_PREP_DML_ROLE_DSN", &embyr_app_dsn),
    ]);
    assert_eq!(
        second_run.exit_code,
        Some(0),
        "step 3: follow-up run with the DML-role DSN must succeed; stderr: {}",
        second_run.stderr
    );

    let readiness_after_grant = dml_adapter
        .verify_schema_readiness()
        .await
        .expect("step 3: verify_schema_readiness() must succeed after the grant");
    assert!(
        matches!(
            readiness_after_grant,
            embyr_core::domain::schema_readiness::SchemaReadiness::Ready { .. }
        ),
        "step 3: after the follow-up grant, the DML role must see Ready"
    );
}
