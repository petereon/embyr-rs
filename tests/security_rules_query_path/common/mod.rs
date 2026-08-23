//! Common test infrastructure — security-rules-query-path acceptance tests
//! (Slice 01, ADR-031).
//!
//! Reuses `SecurityRulesFullContext`/`create_document`/`string_field`/
//! `mint_client_identity_token`/`now_unix` from
//! `security-rules-write-path`'s own fixture module via a path import (this
//! feature reads the SAME `access_rules` table `security-rules` already
//! established — no new driving-port context class needed, per ADR-031 §
//! Enforcement, "No new driven port"). `tests/security_rules/` and
//! `tests/security_rules_write_path/` files are never touched — only
//! imported, read-only, from here.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_write_path/common/mod.rs"]
mod security_rules_write_path_common;
pub use security_rules_write_path_common::{
    create_document, mint_client_identity_token, now_unix, seed_write_access_rule_full,
    string_field, SecurityRulesFullContext,
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

/// AC-17-55 (Slice 02): a filter using an operator OTHER than `==` on the
/// rule's referenced field — the Slice-01 `run_query` helper below only
/// ever builds `==` filters, so a dedicated builder is needed to prove a
/// superficially-"right field" `!=` filter does not satisfy an equality
/// rule.
pub fn not_equal_filter(field_path: &str, value: &str) -> Filter {
    Filter {
        filter_type: Some(FilterType::FieldFilter(FieldFilter {
            field: Some(FieldReference { field_path: field_path.to_string() }),
            op: FieldOp::NotEqual as i32,
            value: Some(string_value(value)),
        })),
    }
}

/// Real gRPC `RunQuery` call taking an already-built raw `Filter` (or
/// `None`) and an explicit `api_key` (Slice 02: AC-17-56's distinguishability
/// test needs a deliberately WRONG api key to observe `authenticate()`'s own
/// rejection shape). `run_query` below (Slice 01's existing equality-only,
/// ctx-api-key entry point) delegates here unchanged — additive only, no
/// existing caller's signature or behavior changes.
pub async fn run_query_raw(
    ctx: &SecurityRulesFullContext,
    collection_id: &str,
    filter: Option<Filter>,
    client_identity_token: Option<&str>,
    api_key: &str,
) -> Result<Vec<Document>, tonic::Status> {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let sq = StructuredQuery {
        from: vec![CollectionSelector {
            collection_id: collection_id.to_string(),
            all_descendants: false,
        }],
        r#where: filter,
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
        format!("Bearer {api_key}").parse().unwrap(),
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

/// Real gRPC `RunQuery` call — driving port entry (Pillar 3), mirroring
/// `create_document`/`update_document`'s own shape (real `FirestoreClient`
/// against the real composition root, real `authorization` + optional
/// `x-embyr-client-identity` metadata). `equality_filters` is a list of
/// `(field_path, string_value)` pairs, AND-composed via `CompositeFilter`
/// when more than one — the only filter shape Slice 01's own ACs exercise
/// (string equality on `owner_id`-style fields). Delegates to
/// `run_query_raw` using `ctx.api_key`.
pub async fn run_query(
    ctx: &SecurityRulesFullContext,
    collection_id: &str,
    equality_filters: &[(&str, &str)],
    client_identity_token: Option<&str>,
) -> Result<Vec<Document>, tonic::Status> {
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

    run_query_raw(ctx, collection_id, where_filter, client_identity_token, &ctx.api_key).await
}
