//! Common test infrastructure — security-rules-collection-group-rules
//! acceptance tests (Slice 01, US-01, ADR-032).
//!
//! Reuses `SecurityRulesAdminContext`/`assert_state_delta`/`set_to`/
//! `unchanged` from `security-rules-write-path`'s own fixture module via a
//! path import (this feature's Slice 01 is admin-API-only, the same
//! "Activity A" shape write-path's own Slice 01 was — no new driving-port
//! context class needed, mirroring ADR-032 § Enforcement, "No new driven
//! port"). `tests/security_rules/`, `tests/security_rules_write_path/`, and
//! `tests/security_rules_query_path/` files are never touched — only
//! imported, read-only, from here.
//!
//! `group_access_rules` is a NEW, disjoint table (ADR-032 § Decision —
//! Schema). Seed/read helpers below write/read it directly via raw SQL
//! (bypassing the define endpoint and the adapter), mirroring
//! `seed_write_access_rule`/`write_access_rule_condition_source`'s identical
//! shape for `write_access_rules`.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_write_path/common/mod.rs"]
mod security_rules_write_path_common;
pub use security_rules_write_path_common::{
    assert_state_delta, mint_client_identity_token, now_unix, seed_write_access_rule, set_to,
    unchanged, write_access_rule_condition_source, SecurityRulesAdminContext,
    SecurityRulesFullContext,
};

use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    run_query_request::QueryType,
    structured_query::{
        composite_filter::Operator as CompositeOp, field_filter::Operator as FieldOp,
        filter::FilterType, CollectionSelector, CompositeFilter, FieldFilter, FieldReference,
        Filter,
    },
    value::ValueType,
    Document, RunQueryRequest, StructuredQuery, Value,
};

fn string_value(s: &str) -> Value {
    Value { value_type: Some(ValueType::StringValue(s.to_string())) }
}

fn equality_filter(field_path: &str, value: &str) -> Filter {
    Filter {
        filter_type: Some(FilterType::FieldFilter(FieldFilter {
            field: Some(FieldReference { field_path: field_path.to_string() }),
            op: FieldOp::Equal as i32,
            value: Some(string_value(value)),
        })),
    }
}

/// Real gRPC `RunQuery` call, explicit `all_descendants` — driving port
/// entry (Pillar 3). Slice 04 needs BOTH `true` (the collection-group case
/// under test, AC-17-89/91/92) and `false` (AC-17-90's regression proof that
/// non-group querying on the SAME ungoverned collection id remains
/// unrestricted) — a dedicated helper here rather than reusing
/// `security_rules_query_path`'s own `run_query` (a sibling feature's
/// fixture, never imported across; this feature's own hierarchy only
/// imports from an ancestor wave — `security_rules_write_path`).
pub async fn run_query(
    ctx: &SecurityRulesFullContext,
    collection_id: &str,
    all_descendants: bool,
    equality_filters: &[(&str, &str)],
    client_identity_token: Option<&str>,
) -> Result<Vec<Document>, tonic::Status> {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let where_filter = match equality_filters {
        [] => None,
        [(field, value)] => Some(equality_filter(field, value)),
        many => Some(Filter {
            filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
                op: CompositeOp::And as i32,
                filters: many.iter().map(|(f, v)| equality_filter(f, v)).collect(),
            })),
        }),
    };

    let sq = StructuredQuery {
        from: vec![CollectionSelector {
            collection_id: collection_id.to_string(),
            all_descendants,
        }],
        r#where: where_filter,
        ..Default::default()
    };

    let mut request = tonic::Request::new(RunQueryRequest {
        parent: format!(
            "projects/{}/databases/(default)/documents",
            ctx.project_id
        ),
        query_type: Some(QueryType::StructuredQuery(sq)),
        ..Default::default()
    });
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", ctx.api_key).parse().unwrap(),
    );
    if let Some(token) = client_identity_token {
        request.metadata_mut().insert(
            "x-embyr-client-identity",
            format!("Bearer {token}").parse().unwrap(),
        );
    }

    let mut stream = client.run_query(request).await?.into_inner();
    use tokio_stream::StreamExt;
    let mut docs = Vec::new();
    while let Some(item) = stream.next().await {
        let response = item?;
        if let Some(document) = response.document {
            docs.push(document);
        }
    }
    Ok(docs)
}

/// Directly seed a `group_access_rules` row (bypassing the define endpoint
/// and the adapter) — used by AC-17-78's redefine-precondition setup and
/// AC-17-79's independence setup, mirroring
/// `seed_write_access_rule`'s identical bypass-the-endpoint allowance.
pub async fn seed_group_access_rule(
    ctx: &SecurityRulesAdminContext,
    project_id: &str,
    collection_id: &str,
    condition_source: &str,
) {
    sqlx::query(
        "INSERT INTO group_access_rules (project_id, collection_id, condition_source) \
         VALUES ($1, $2, $3)",
    )
    .bind(project_id)
    .bind(collection_id)
    .bind(condition_source)
    .execute(&ctx.pool)
    .await
    .expect("insert group_access_rules row");
}

/// Read the currently-stored `condition_source` for `(project_id,
/// collection_id)` in `group_access_rules` — the state-delta observable for
/// AC-17-77/78/79, structurally disjoint from
/// `SecurityRulesAdminContext::access_rule_condition_source`/
/// `write_access_rule_condition_source` (a DIFFERENT table, `FROM
/// group_access_rules` only).
pub async fn group_access_rule_condition_source(
    ctx: &SecurityRulesAdminContext,
    project_id: &str,
    collection_id: &str,
) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT condition_source FROM group_access_rules WHERE project_id = $1 AND collection_id = $2",
    )
    .bind(project_id)
    .bind(collection_id)
    .fetch_optional(&ctx.pool)
    .await
    .unwrap_or(None)
}
