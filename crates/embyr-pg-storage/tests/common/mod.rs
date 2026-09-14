//! Common test infrastructure — collection-group-query-index acceptance
//! tests (feature-delta.md, ADR-080).
//!
//! Infrastructure policy (Strategy A, per feature-delta.md § Walking
//! Skeleton Strategy — real, minimal, end-to-end): every scenario in this
//! feature runs against a real testcontainers Postgres and the real
//! `PostgresBackendAdapter`, never a mock/in-memory double. Placement:
//! crate-local `tests/` (Rust auto-discovery) rather than the workspace-root
//! `tests/<feature>/` convention used by cross-crate features (cxr0N,
//! cdo0N) — this feature's US-01/US-02/US-03 adapter-level correctness is
//! fully exercisable through `embyr-pg-storage`'s own public surface
//! (`PostgresBackendAdapter` implementing `BackendAdapter`), so pulling in
//! `embyr-server`/tonic/gRPC would cost real compile time and RAM on the
//! 8GB target machine for zero additional coverage. The one genuine CLI
//! driving-port proof (`embyr-db-prep` walking skeleton) lives separately,
//! registered under `embyr-db-prep`'s own Cargo.toml, mirroring
//! `customer_db_onboarding`'s own cdo0N placement.
//!
//! Keep seeded document counts SMALL (dozens, not thousands) and batch
//! sizes small (5-10) — resource-conscious per this feature's own explicit
//! constraint, while still large enough to make a "backfill in progress"
//! state observable/interruptible.

#![allow(dead_code, unused_imports)]

use std::collections::BTreeMap;
use std::time::Duration;

use embyr_core::domain::document::{CollectionPath, DocumentPath};
use embyr_core::domain::field_value::FieldValue;
use embyr_core::domain::project::ProjectId;
use embyr_core::domain::query::{
    AggregationKind, AggregationQuery, FieldFilter, FilterOp, OrderBy, OrderDirection,
    QueryFilter, StructuredQuery,
};
use embyr_core::storage::backend_adapter::{BackendAdapter, WritePrecondition};
use embyr_pg_storage::backend_adapter::PostgresBackendAdapter;
use sqlx::PgPool;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

// ─── Postgres container + adapter bootstrap ────────────────────────────────

/// Start a fresh Postgres 15-alpine testcontainer, migrated with EVERY
/// migration currently present in `migrations/customer/` at compile time
/// (whatever this feature's own DELIVER wave has landed by the time this
/// runs — today, that's every migration EXCEPT this feature's own, since
/// DISTILL does not author migration files, per this session's own
/// established convention: `composite-index-real-creation`'s DISTILL commit
/// touched zero `.sql` files).
///
/// Caller must keep the returned `ContainerAsync<Postgres>` alive for the
/// duration of any pool/adapter that connects to it.
pub async fn migrated_customer_db() -> (ContainerAsync<Postgres>, PgPool, PostgresBackendAdapter) {
    let container = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("failed to start Postgres testcontainer");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("failed to get host port for Postgres");
    let db_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pool = PgPool::connect(&db_url).await.expect("connect to testcontainer");
    let adapter = PostgresBackendAdapter::new_from_pool(pool.clone());
    adapter.migrate().await.expect("migrate() should apply every current migration");
    (container, pool, adapter)
}

/// Same as `migrated_customer_db`, but applies every migration EXCEPT the
/// LAST one currently embedded — simulates a customer database that has not
/// yet had its own DBA re-run `embyr-db-prep` after this feature's own
/// migration lands (US-03). Mirrors `customer_db_onboarding`'s cdo03 own
/// "apply only the first migration" fixture technique, generalized to "all
/// but the last".
///
/// Until this feature's own migration file exists, this is indistinguishable
/// from `migrated_customer_db` minus one migration that has nothing to do
/// with this feature — that is fine and expected: it still proves the
/// technique works, and becomes meaningful the moment DELIVER adds
/// `migrations/customer/0006_collection_group_index.sql` (or whatever
/// number is next when DELIVER lands it — `migrations/customer/` already
/// has 0001-0005; ADR-080's own "0002" citation has drifted, DELIVER should
/// use the real next number).
pub async fn customer_db_missing_latest_migration()
-> (ContainerAsync<Postgres>, PgPool, PostgresBackendAdapter) {
    let container = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("failed to start Postgres testcontainer");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("failed to get host port for Postgres");
    let db_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    apply_all_but_last_migration(&db_url).await;

    let pool = PgPool::connect(&db_url).await.expect("connect to testcontainer");
    let adapter = PostgresBackendAdapter::new_from_pool(pool.clone());
    (container, pool, adapter)
}

async fn apply_all_but_last_migration(db_url: &str) {
    use sqlx::migrate::Migrate;
    use sqlx::Connection;

    let migrator = sqlx::migrate!("../../migrations/customer");
    let migrations: Vec<_> = migrator.iter().collect();
    assert!(
        migrations.len() >= 2,
        "need at least 2 migrations to meaningfully simulate 'missing the latest one'"
    );
    let mut conn =
        sqlx::PgConnection::connect(db_url).await.expect("connect for fixture migration setup");
    conn.ensure_migrations_table().await.expect("ensure_migrations_table");
    for m in &migrations[..migrations.len() - 1] {
        conn.apply(m).await.expect("apply migration");
    }
}

/// Apply whatever migration(s) are missing relative to `migrated_customer_db`
/// — simulates a customer's DBA re-running `embyr-db-prep` mid-session
/// against the SAME database a long-lived adapter instance is already
/// serving (AC-CGI-11). Reuses `PostgresBackendAdapter::migrate()` itself
/// (real production entry point, not a hand-rolled re-implementation).
pub async fn apply_remaining_migrations_live(pool: &PgPool) {
    PostgresBackendAdapter::new_from_pool(pool.clone())
        .migrate()
        .await
        .expect("migrate() should apply any remaining migrations");
}

// ─── Document seeding via the real BackendAdapter port ─────────────────────

pub fn project(id: &str) -> ProjectId {
    ProjectId::new(id).expect("valid project id")
}

pub fn string_field(s: &str) -> FieldValue {
    FieldValue::String(s.to_string())
}

pub fn double_field(v: f64) -> FieldValue {
    FieldValue::Double(v)
}

/// Create one document via the real `create_document` driving-port method
/// (production write path — call site #1 of the 5 `INSERT INTO documents`
/// sites, ADR-080 § Reading Confirmation).
pub async fn seed_document(
    adapter: &PostgresBackendAdapter,
    project_id: &str,
    collection_path: &str,
    document_id: &str,
    fields: BTreeMap<String, FieldValue>,
) {
    let path = DocumentPath {
        project_id: project(project_id),
        collection_path: collection_path.to_string(),
        document_id: document_id.to_string(),
    };
    adapter
        .create_document(&path, fields)
        .await
        .unwrap_or_else(|e| panic!("seed_document({collection_path}/{document_id}) failed: {e}"));
}

/// Insert a document directly via raw SQL, bypassing the trigger entirely
/// (`ALTER TABLE ... DISABLE TRIGGER` around the insert) — models a document
/// that already existed BEFORE this feature's migration/trigger landed,
/// exactly the precondition US-02's backfill scenarios need. Requires the
/// `collection_id` column and `documents_collection_id_biu` trigger to
/// already exist (post-DELIVER); against today's pre-fix schema this simply
/// fails with a clear "trigger does not exist" error, which is the correct
/// RED signal (schema not yet migrated) for every US-02 scenario.
pub async fn seed_pre_existing_document_without_collection_id(
    pool: &PgPool,
    project_id: &str,
    collection_path: &str,
    document_id: &str,
) {
    sqlx::query("ALTER TABLE documents DISABLE TRIGGER documents_collection_id_biu")
        .execute(pool)
        .await
        .expect(
            "disable trigger for pre-existing-document fixture -- fails today because the \
             trigger does not exist yet (RED: schema not yet migrated, ADR-080 Decision A)",
        );

    sqlx::query(
        "INSERT INTO documents (project_id, collection_path, document_id, fields, version, \
         create_time, update_time) VALUES ($1, $2, $3, '{}'::jsonb, 1, NOW(), NOW())",
    )
    .bind(project_id)
    .bind(collection_path)
    .bind(document_id)
    .execute(pool)
    .await
    .expect("insert pre-existing document with trigger disabled");

    sqlx::query("ALTER TABLE documents ENABLE TRIGGER documents_collection_id_biu")
        .execute(pool)
        .await
        .expect("re-enable trigger after pre-existing-document fixture");
}

// ─── Raw-SQL verification reads (never the driving port — read-only proof) ─

pub async fn collection_id_of(
    pool: &PgPool,
    project_id: &str,
    collection_path: &str,
    document_id: &str,
) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT collection_id FROM documents \
         WHERE project_id = $1 AND collection_path = $2 AND document_id = $3",
    )
    .bind(project_id)
    .bind(collection_path)
    .bind(document_id)
    .fetch_one(pool)
    .await
    .expect("collection_id_of query failed -- RED if the column does not exist yet")
}

pub async fn version_and_update_time_of(
    pool: &PgPool,
    project_id: &str,
    collection_path: &str,
    document_id: &str,
) -> (i64, chrono::DateTime<chrono::Utc>) {
    sqlx::query_as::<_, (i64, chrono::DateTime<chrono::Utc>)>(
        "SELECT version, update_time FROM documents \
         WHERE project_id = $1 AND collection_path = $2 AND document_id = $3",
    )
    .bind(project_id)
    .bind(collection_path)
    .bind(document_id)
    .fetch_one(pool)
    .await
    .expect("version_and_update_time_of query failed")
}

pub async fn count_null_collection_id(pool: &PgPool, project_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM documents WHERE project_id = $1 AND collection_id IS NULL",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await
    .expect("count_null_collection_id query failed -- RED if the column does not exist yet")
}

pub async fn count_non_null_collection_id(pool: &PgPool, project_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM documents WHERE project_id = $1 AND collection_id IS NOT NULL",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await
    .expect("count_non_null_collection_id query failed -- RED if the column does not exist yet")
}

pub async fn documents_collection_group_idx_exists(pool: &PgPool) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = 'public' \
         AND tablename = 'documents' AND indexname = 'documents_collection_group_idx')",
    )
    .fetch_one(pool)
    .await
    .expect("pg_indexes catalog query failed")
}

pub async fn documents_collection_group_pending_idx_exists(pool: &PgPool) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = 'public' \
         AND tablename = 'documents' AND indexname = 'documents_collection_group_pending_idx')",
    )
    .fetch_one(pool)
    .await
    .expect("pg_indexes catalog query failed")
}

/// `EXPLAIN` the exact `all_descendants=true` predicate production code
/// executes, via the shared, single-source-of-truth
/// `push_all_descendants_predicate` builder (encoding/query.rs) — zero drift
/// risk once DELIVER wires all 4 mirrored call sites to it, mirroring
/// `composite_index_real_creation`'s own `explain_category_equal_...`
/// helper technique for a different predicate.
pub async fn explain_all_descendants(
    pool: &PgPool,
    project_id: &str,
    collection_id: &str,
    schema_available: bool,
) -> Vec<String> {
    use embyr_pg_storage::encoding::query::push_all_descendants_predicate;
    use sqlx::{Postgres as Pg, QueryBuilder};

    // A fresh testcontainers Postgres has no autovacuum-driven ANALYZE yet
    // within this test's short window — without it the planner defaults to
    // a Seq Scan regardless of which indexes exist (same reasoning
    // `composite_index_real_creation`'s own EXPLAIN helper documents).
    sqlx::query("ANALYZE documents").execute(pool).await.expect("ANALYZE documents failed");

    let mut qb: QueryBuilder<Pg> = QueryBuilder::new(
        "EXPLAIN SELECT project_id, collection_path, document_id, fields, version, \
         create_time, update_time FROM documents WHERE project_id = ",
    );
    qb.push_bind(project_id.to_string());
    qb.push(" AND ");
    push_all_descendants_predicate(&mut qb, collection_id, schema_available);
    qb.push(" AND NOT deleted");

    let rows: Vec<(String,)> = qb
        .build_query_as()
        .fetch_all(pool)
        .await
        .expect("EXPLAIN query failed");
    rows.into_iter().map(|(l,)| l).collect()
}

pub fn plan_is_index_assisted(explain_lines: &[String]) -> bool {
    let text = explain_lines.join("\n");
    (text.contains("Index Scan") || text.contains("Index Only Scan") || text.contains("Bitmap"))
        && !text.contains("Seq Scan on documents")
}

// ─── Query builders (domain types, business-language collection-group shape) ─

pub fn collection_group_selector(project_id: &str, collection_id: &str) -> CollectionPath {
    CollectionPath { project_id: project(project_id), collection_path: collection_id.to_string() }
}

pub fn collection_group_query(collection_id: &str, filter: Option<QueryFilter>) -> StructuredQuery {
    StructuredQuery {
        collection_id: collection_id.to_string(),
        all_descendants: true,
        filter,
        order_by: vec![],
        limit: None,
        offset: None,
        start_at: None,
        end_at: None,
        since_update_time: None,
    }
}

pub fn ordinary_collection_query(collection_id: &str) -> StructuredQuery {
    StructuredQuery {
        collection_id: collection_id.to_string(),
        all_descendants: false,
        filter: None,
        order_by: vec![],
        limit: None,
        offset: None,
        start_at: None,
        end_at: None,
        since_update_time: None,
    }
}

pub fn rating_gte_filter(min_rating: f64) -> QueryFilter {
    QueryFilter::Field(FieldFilter {
        field_path: "rating".to_string(),
        op: FilterOp::GreaterThanOrEqual,
        value: FieldValue::Double(min_rating),
    })
}

pub fn verified_equal_true_filter() -> QueryFilter {
    QueryFilter::Field(FieldFilter {
        field_path: "verified".to_string(),
        op: FilterOp::Equal,
        value: FieldValue::Boolean(true),
    })
}

pub fn count_aggregation(collection_id: &str, filter: Option<QueryFilter>) -> AggregationQuery {
    AggregationQuery {
        query: collection_group_query(collection_id, filter),
        aggregation: AggregationKind::Count,
        alias: "count".to_string(),
    }
}

pub fn sum_aggregation(collection_id: &str, field_path: &str) -> AggregationQuery {
    AggregationQuery {
        query: collection_group_query(collection_id, None),
        aggregation: AggregationKind::Sum(field_path.to_string()),
        alias: "sum".to_string(),
    }
}

pub fn avg_aggregation(collection_id: &str, field_path: &str) -> AggregationQuery {
    AggregationQuery {
        query: collection_group_query(collection_id, None),
        aggregation: AggregationKind::Avg(field_path.to_string()),
        alias: "avg".to_string(),
    }
}
