// SCAFFOLD: true
//! Common test infrastructure — composite-index-real-creation acceptance
//! tests (feature-delta.md, ADR-072).
//!
//! Reused via `#[path]` import from `firestore_composite_indexes_admin_api`'s
//! own common module directly (same as `composite_index_requirement_rules`'s
//! own identical precedent) — this feature needs no access-rule-specific
//! scaffolding, just the full composition root (`SecurityRulesFullContext`)
//! for a real customer-DB connection (`cust_pool`, needed to query
//! `pg_indexes`/`pg_index`/`EXPLAIN` directly), real gRPC `RunQuery`/
//! `CreateDocument`, and real admin HTTP `CreateIndex`/`ListIndexes`/
//! `DeleteIndex`.
//!
//! Infrastructure policy: Strategy A (real, minimal, end-to-end) per
//! feature-delta.md § Walking Skeleton Strategy — every scenario in this
//! feature is a real HTTP/gRPC call against a real testcontainers Postgres
//! (both system DB and customer DB), never an in-memory double. Matches
//! this session's own established precedent for every production
//! -readiness-audit-derived fix.

#![allow(dead_code, unused_imports)]

use std::time::Duration;

#[path = "../../firestore_composite_indexes_admin_api/common/mod.rs"]
pub mod firestore_composite_indexes_admin_api_common;
pub use firestore_composite_indexes_admin_api_common::SecurityRulesFullContext;

use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    run_query_request::QueryType,
    structured_query::{
        field_filter::Operator as FieldOp, filter::FilterType, CollectionSelector, Direction,
        FieldFilter as ProtoFieldFilter, FieldReference, Filter, Order,
    },
    value::ValueType,
    CreateDocumentRequest, Document, RunQueryRequest, StructuredQuery, Value,
};

// ─── gRPC driving-port helpers (mirrors firestore_composite_indexes_admin_api's
//     own cix01/cix03 local helpers — reused here across all 3 cxr0N files
//     within this one feature, per Mandate 4 pragmatism) ─────────────────────

pub fn make_channel(addr: std::net::SocketAddr) -> tonic::transport::Channel {
    tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect_lazy()
}

pub fn make_authed_request<T>(payload: T, api_key: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(payload);
    req.metadata_mut()
        .insert("authorization", format!("bearer {api_key}").parse().unwrap());
    req
}

pub fn string_value(s: &str) -> Value {
    Value { value_type: Some(ValueType::StringValue(s.to_string())) }
}

pub fn integer_value(i: i64) -> Value {
    Value { value_type: Some(ValueType::IntegerValue(i)) }
}

pub fn field_ref(path: &str) -> FieldReference {
    FieldReference { field_path: path.to_string() }
}

/// Real `CreateDocument` gRPC call (Pillar 3: real driving port, not a
/// direct-DB bypass) — mirrors cix01's own `seed_document` exactly.
pub async fn seed_document_via_grpc(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    collection: &str,
    doc_id: &str,
    fields: std::collections::HashMap<String, Value>,
) {
    let parent = format!("projects/{project_id}/databases/(default)/documents");
    let req = make_authed_request(
        CreateDocumentRequest {
            parent,
            collection_id: collection.to_string(),
            document_id: doc_id.to_string(),
            document: Some(Document { name: String::new(), fields, ..Default::default() }),
            ..Default::default()
        },
        api_key,
    );
    client.create_document(req).await.expect("seed document should succeed");
}

/// The canonical DISCUSS US-01 Example 1 worked query shape: `category ==
/// "electronics"` [equality role] `ORDER BY score DESC` [sort role] — the
/// ONLY shape any of this feature's own ACs (AC-CXR-01/02/03) exercise or
/// assert against (ADR-072 Decision D).
pub fn category_equal_score_desc_structured_query(collection: &str) -> StructuredQuery {
    StructuredQuery {
        from: vec![CollectionSelector { collection_id: collection.to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(ProtoFieldFilter {
                field: Some(field_ref("category")),
                op: FieldOp::Equal as i32,
                value: Some(string_value("electronics")),
            })),
        }),
        order_by: vec![Order { field: Some(field_ref("score")), direction: Direction::Descending as i32 }],
        ..Default::default()
    }
}

pub async fn run_grpc_query(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    sq: StructuredQuery,
) -> Result<Vec<Document>, tonic::Status> {
    let parent = format!("projects/{project_id}/databases/(default)/documents");
    let req = make_authed_request(
        RunQueryRequest { parent, query_type: Some(QueryType::StructuredQuery(sq)), ..Default::default() },
        api_key,
    );
    let stream = client.run_query(req).await?.into_inner();
    use tokio_stream::StreamExt;
    let responses: Vec<_> = stream.collect().await;
    let mut docs = Vec::new();
    for r in responses {
        let r = r?;
        if let Some(doc) = r.document {
            docs.push(doc);
        }
    }
    Ok(docs)
}

// ─── Admin HTTP driving-port polling helpers ──────────────────────────────

/// One-shot `ListIndexes` lookup for a single index id's own `status` field
/// — the port-exposed observable this feature's status-lifecycle ACs
/// (AC-CXR-02/07/08/09) are about, read via the SAME admin route operators
/// actually poll, never an internal DB column read directly.
pub async fn current_index_status(
    ctx: &SecurityRulesFullContext,
    cookie: &str,
    project_id: &str,
    index_id: &str,
) -> Option<String> {
    let resp = reqwest::Client::new()
        .get(ctx.admin_url(&format!("/admin/v1/projects/{project_id}/indexes")))
        .header("Cookie", cookie)
        .send()
        .await
        .expect("list_composite_indexes request failed");
    let body: Vec<serde_json::Value> = resp.json().await.expect("JSON array");
    body.into_iter()
        .find(|row| row["id"] == index_id)
        .and_then(|row| row["status"].as_str().map(|s| s.to_string()))
}

/// Poll `ListIndexes` (bounded — never hangs CI) until the observed status
/// is one of `targets`, or return the last-observed status once
/// `max_attempts` is exhausted. Used both to catch a transient "building"
/// window (AC-CXR-06/07) and to wait for a terminal "ready"/"failed"
/// outcome (AC-CXR-02/08/09).
pub async fn wait_until_status_is_one_of(
    ctx: &SecurityRulesFullContext,
    cookie: &str,
    project_id: &str,
    index_id: &str,
    targets: &[&str],
    max_attempts: u32,
    interval: Duration,
) -> Option<String> {
    for _ in 0..max_attempts {
        if let Some(status) = current_index_status(ctx, cookie, project_id, index_id).await {
            if targets.contains(&status.as_str()) {
                return Some(status);
            }
        }
        tokio::time::sleep(interval).await;
    }
    current_index_status(ctx, cookie, project_id, index_id).await
}

// ─── Real Postgres index-catalog proof helpers (AC-CXR-01/03/10) ─────────

/// Every real index on `documents` whose name carries the server-generated
/// `cix_` prefix (ADR-072 Decision A: never derived from user input) —
/// queried directly against `pg_indexes`, never the `composite_indexes`
/// metadata row.
pub async fn cix_prefixed_index_names(cust_pool: &sqlx::PgPool) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT indexname FROM pg_indexes \
         WHERE schemaname = 'public' AND tablename = 'documents' AND indexname LIKE 'cix\\_%' ESCAPE '\\'",
    )
    .fetch_all(cust_pool)
    .await
    .expect("pg_indexes catalog query failed")
}

/// `pg_index.indisvalid` for a named index — the authoritative ready/failed
/// signal ADR-072 Decision B locks (catches a mid-build connection drop the
/// client-side `Ok`/`Err` alone would miss).
pub async fn index_is_valid(cust_pool: &sqlx::PgPool, index_name: &str) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT indisvalid FROM pg_index WHERE indexrelid = $1::regclass")
        .bind(index_name)
    .fetch_one(cust_pool)
    .await
    .expect("pg_index validity query failed")
}

/// Real `EXPLAIN` of the canonical `category == "electronics" ORDER BY
/// score DESC` shape (AC-CXR-03) — built via the SAME public
/// `append_filter`/`order_by_expr` functions `run_query` itself calls
/// (`embyr_pg_storage::encoding::query`), so the EXPLAIN'd SQL text is
/// byte-identical to what a real `RunQuery` executes, never a
/// hand-approximated re-implementation.
pub async fn explain_category_equal_electronics_order_by_score_desc(
    cust_pool: &sqlx::PgPool,
    project_id: &str,
    collection_path: &str,
) -> Vec<String> {
    use embyr_core::domain::field_value::FieldValue;
    use embyr_core::domain::query::{FieldFilter, FilterOp, OrderBy, OrderDirection, QueryFilter};
    use embyr_pg_storage::encoding::query::{append_filter, order_by_expr};
    use sqlx::{Postgres, QueryBuilder};

    // A fresh testcontainers Postgres has no autovacuum-driven ANALYZE run
    // yet within this test's short window (unlike a real production DB,
    // where autovacuum runs continuously) — without it, the planner has no
    // real row-count/selectivity statistics and defaults to a Seq Scan
    // regardless of which indexes exist. Explicit ANALYZE here gives the
    // planner the SAME informational basis a real deployment already has,
    // so this EXPLAIN reflects genuine cost-based index usage, not a
    // testing-environment artifact.
    sqlx::query("ANALYZE documents")
        .execute(cust_pool)
        .await
        .expect("ANALYZE documents failed");

    let filter = QueryFilter::Field(FieldFilter {
        field_path: "category".to_string(),
        op: FilterOp::Equal,
        value: FieldValue::String("electronics".to_string()),
    });
    let order_by = OrderBy { field_path: "score".to_string(), direction: OrderDirection::Descending };

    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
        "EXPLAIN SELECT project_id, collection_path, document_id, fields, version, create_time, update_time \
         FROM documents WHERE project_id = ",
    );
    qb.push_bind(project_id.to_string());
    qb.push(" AND collection_path = ");
    qb.push_bind(collection_path.to_string());
    qb.push(" AND NOT deleted AND ");
    append_filter(&mut qb, &filter);
    qb.push(format!(" ORDER BY {}", order_by_expr(&order_by)));

    let rows: Vec<(String,)> = qb
        .build_query_as()
        .fetch_all(cust_pool)
        .await
        .expect("EXPLAIN query failed");
    rows.into_iter().map(|(line,)| line).collect()
}

// ─── Bulk seeding (AC-CXR-06 — a large, actively-written collection) ──────

/// Seed `count` "products" documents directly into the customer DB (the
/// same bypass-the-endpoint allowance `SecurityRulesFullContext::
/// seed_document` already uses), batched inside one transaction for speed —
/// large enough that a plain, write-blocking `CREATE INDEX` would leave an
/// observable lock window, unlike `CREATE INDEX CONCURRENTLY`'s non
/// -blocking behavior (ADR-072 Decision B).
pub async fn seed_bulk_products(cust_pool: &sqlx::PgPool, project_id: &str, count: usize) {
    let mut tx = cust_pool.begin().await.expect("begin bulk-seed transaction");
    for n in 0..count {
        let category = if n % 2 == 0 { "electronics" } else { "garden" };
        let fields = serde_json::json!({
            "category": {"t": "S", "v": category},
            "score": {"t": "I", "v": n as i64},
        });
        sqlx::query(
            "INSERT INTO documents (project_id, collection_path, document_id, fields) \
             VALUES ($1, 'products', $2, $3)",
        )
        .bind(project_id)
        .bind(format!("bulk-{n}"))
        .bind(&fields)
        .execute(&mut *tx)
        .await
        .expect("bulk insert product document failed");
    }
    tx.commit().await.expect("commit bulk-seed transaction");
}

// ─── agent-mode project (AC-CXR-09 — a build with no reachable customer DB) ─

/// Insert a SECOND project under the SAME account, `backend_mode = 'agent'`
/// — `resolve_dsn_without_api_key`'s existing `other => None` match arm
/// (ADR-072 Decision C) already covers any backend_mode outside
/// `{aws_secret, gcp_secret, direct_pg}`, giving a deterministic,
/// non-flaky way to force a build failure without tampering with a live DB
/// connection mid-test.
pub async fn insert_agent_mode_project(ctx: &SecurityRulesFullContext, project_id: &str) {
    sqlx::query(
        "INSERT INTO projects (id, account_id, status, backend_mode, api_key_hash_current) \
         VALUES ($1, $2, 'active', 'agent', 'placeholder_hash_agent_mode')",
    )
    .bind(project_id)
    .bind(ctx.account_id)
    .execute(&ctx.sys_pool)
    .await
    .expect("insert agent-mode project");
}
