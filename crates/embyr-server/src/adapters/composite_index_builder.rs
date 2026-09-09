//! One-shot background task building a real composite index against a
//! customer database (composite-index-real-creation, ADR-072 Decision B/C).
//!
//! Fired via `tokio::spawn` from `create_composite_index`'s own handler
//! body — NOT a generic job queue (ADR-072: no other feature in this
//! codebase needs one). Reuses `customer_db_connect::resolve_dsn_without_api_key`
//! (extracted from `transaction_sweeper.rs`) + `PostgresBackendAdapter::new`.
//! Always writes a terminal `composite_indexes.status` (`ready`/`failed`) via
//! `SystemDb::update_composite_index_status` — never leaves the row silently
//! stuck if a connection/build attempt was actually made (a process crash
//! mid-build is the one case this does NOT self-heal, per ADR-072's own
//! explicit, locked "recover via delete-then-recreate" answer).

use std::sync::Arc;

use uuid::Uuid;

use crate::adapters::aws_secret_fetcher::AwsSecretFetcher;
use crate::adapters::composite_index_ddl::{build_create_index_sql, build_drop_index_sql, index_name_for};
use crate::adapters::customer_db_connect::{resolve_dsn_without_api_key, PgConnectInfo};
use crate::adapters::gcp_secret_fetcher::GcpSecretFetcher;
use crate::adapters::postgres_backend::PostgresBackendAdapter;
use crate::adapters::system_db::SystemDb;
use crate::admin::handlers::composite_indexes::IndexFieldSpec;

/// Spawn the one-shot build task. Returns the `JoinHandle` for callers that
/// want to await it directly (tests); the production handler fires and
/// forgets it (the row's own `status` column is the observable outcome).
#[allow(clippy::too_many_arguments)]
pub fn spawn_build(
    system_db: Arc<SystemDb>,
    connect_info: PgConnectInfo,
    aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
    gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
    encryption_key: [u8; 32],
    encryption_key_previous: Option<[u8; 32]>,
    index_id: Uuid,
    project_id: String,
    fields: Vec<IndexFieldSpec>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let status = run_build(
            &connect_info,
            aws_secret_fetcher.as_deref(),
            gcp_secret_fetcher.as_deref(),
            &encryption_key,
            encryption_key_previous.as_ref(),
            index_id,
            &project_id,
            &fields,
        )
        .await;

        if let Err(e) = system_db.update_composite_index_status(index_id, status).await {
            tracing::error!(
                project_id = %project_id,
                index_id = %index_id,
                error = %e,
                "composite_index_builder: failed to write terminal status"
            );
        }
    })
}

/// Run the real build: resolve DSN -> connect -> `CREATE INDEX CONCURRENTLY`
/// -> `pg_index.indisvalid` re-check (ADR-072 Decision B — the authoritative
/// ready/failed signal, catching a mid-build connection drop the statement's
/// own `Ok`/`Err` alone would miss) -> best-effort `DROP ... IF EXISTS` on
/// failure. Returns the terminal status string to write back.
#[allow(clippy::too_many_arguments)]
async fn run_build(
    connect_info: &PgConnectInfo,
    aws_secret_fetcher: Option<&AwsSecretFetcher>,
    gcp_secret_fetcher: Option<&GcpSecretFetcher>,
    encryption_key: &[u8; 32],
    encryption_key_previous: Option<&[u8; 32]>,
    index_id: Uuid,
    project_id: &str,
    fields: &[IndexFieldSpec],
) -> &'static str {
    let Some(dsn) = resolve_dsn_without_api_key(
        project_id,
        connect_info,
        aws_secret_fetcher,
        gcp_secret_fetcher,
        encryption_key,
        encryption_key_previous,
    )
    .await
    else {
        return "failed";
    };

    let adapter = match PostgresBackendAdapter::new(&dsn).await {
        Ok(adapter) => adapter,
        Err(e) => {
            tracing::warn!(
                project_id = %project_id,
                index_id = %index_id,
                error = %e,
                "composite_index_builder: failed to connect to customer database"
            );
            return "failed";
        }
    };

    let create_sql = match build_create_index_sql(index_id, fields) {
        Ok(sql) => sql,
        Err(e) => {
            // Fields are already validated (AC-CXR-04) before the row is
            // even inserted — reaching here would mean the validated row
            // itself is malformed. Defensive-only, never expected in
            // practice.
            tracing::error!(
                project_id = %project_id,
                index_id = %index_id,
                error = %e,
                "composite_index_builder: DDL build rejected post-validation"
            );
            return "failed";
        }
    };

    let create_result = sqlx::query(&create_sql).execute(adapter.pool()).await;
    if let Err(e) = &create_result {
        tracing::warn!(
            project_id = %project_id,
            index_id = %index_id,
            error = %e,
            "composite_index_builder: CREATE INDEX CONCURRENTLY failed"
        );
    }

    // ADR-072 Decision B: ALWAYS re-check validity, regardless of the
    // statement's own Ok/Err — this is the authoritative signal.
    let valid: bool = sqlx::query_scalar::<_, bool>(
        "SELECT indisvalid FROM pg_index WHERE indexrelid = $1::regclass",
    )
    .bind(index_name_for(index_id))
    .fetch_one(adapter.pool())
    .await
    .unwrap_or(false);

    if create_result.is_ok() && valid {
        return "ready";
    }

    // Best-effort cleanup — free the deterministic name for a future retry
    // (ADR-072 Decision B/C's idempotent-re-POST retry path).
    let drop_sql = build_drop_index_sql(index_id);
    if let Err(e) = sqlx::query(&drop_sql).execute(adapter.pool()).await {
        tracing::warn!(
            project_id = %project_id,
            index_id = %index_id,
            error = %e,
            "composite_index_builder: best-effort DROP after failed build also failed"
        );
    }
    "failed"
}
