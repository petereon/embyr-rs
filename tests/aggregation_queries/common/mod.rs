//! Common test infrastructure — aggregation-queries acceptance tests
//! (Slice 01, ADR-038/039/040).
//!
//! Reuses `SecurityRulesFullContext` (real gRPC :8080 + admin :9090
//! production composition root) and its `seed_access_rule`/
//! `seed_client_identity_credential`/`create_document` fixture helpers via a
//! path import — this feature reuses the SAME `access_rules`/
//! `group_access_rules` tables and `check_query_compliance()` mechanism
//! `security-rules-query-path`/`security-rules-collection-group-rules`
//! already established (ADR-039 § Decision 1). No new driven port, no new
//! fixture context class — only a new driving-port call
//! (`run_count_aggregation` below), since `RunAggregationQuery` is a
//! brand-new RPC with no existing helper to reuse.

#![allow(dead_code, unused_imports)]

// Both `create_document`/`mint_client_identity_token`/etc. AND
// `seed_group_access_rule_full`/`seed_document_at_path` must come through the
// SAME single path-inclusion chain — `#[path]` module inclusion instantiates
// a fresh module each time it appears, so importing `SecurityRulesFullContext`
// via two independent `#[path]` chains (even of the identical source file)
// produces two structurally-identical but nominally-DISTINCT Rust types.
// `security_rules_collection_group_rules::common` already re-exports
// everything this module needs from its own single chain — import only that.
#[path = "../../security_rules_collection_group_rules/common/mod.rs"]
mod security_rules_collection_group_rules_common;
pub use security_rules_collection_group_rules_common::{
    create_document, mint_client_identity_token, now_unix, seed_document_at_path,
    seed_group_access_rule_full, string_field, SecurityRulesFullContext,
};

use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    run_aggregation_query_request::QueryType as AggregationQueryType,
    structured_aggregation_query::{
        aggregation::{Count, Operator as AggregationOperator},
        Aggregation, QueryType as StructuredAggQueryType,
    },
    structured_query::{
        composite_filter::Operator as CompositeOp, field_filter::Operator as FieldOp,
        filter::FilterType, CollectionSelector, CompositeFilter, FieldFilter, FieldReference,
        Filter,
    },
    value::ValueType,
    RunAggregationQueryRequest, StructuredAggregationQuery, StructuredQuery, Value,
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

/// Real gRPC `RunAggregationQuery` call with a COUNT aggregation — driving
/// port entry (Pillar 3), mirroring `security_rules_query_path::run_query`'s
/// own shape (real `FirestoreClient`, real `authorization` + optional
/// `x-embyr-client-identity` metadata). Returns the parsed COUNT value
/// (the default `"field_0"` alias — AC-01-... zero-alias synthesis is
/// exercised implicitly by every caller since none of this slice's own
/// scenarios set an explicit alias).
pub async fn run_count_aggregation(
    ctx: &SecurityRulesFullContext,
    collection_id: &str,
    all_descendants: bool,
    equality_filters: &[(&str, &str)],
    client_identity_token: Option<&str>,
) -> Result<i64, tonic::Status> {
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

    let saq = StructuredAggregationQuery {
        query_type: Some(StructuredAggQueryType::StructuredQuery(sq)),
        aggregations: vec![Aggregation {
            operator: Some(AggregationOperator::Count(Count { up_to: None })),
            alias: String::new(),
        }],
    };

    let mut request = tonic::Request::new(RunAggregationQueryRequest {
        parent: format!("projects/{}/databases/(default)/documents", ctx.project_id),
        query_type: Some(AggregationQueryType::StructuredAggregationQuery(saq)),
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

    let mut stream = client.run_aggregation_query(request).await?.into_inner();
    use tokio_stream::StreamExt;
    let response = stream
        .next()
        .await
        .expect("expected exactly one RunAggregationQueryResponse message")?;
    let result = response
        .result
        .expect("response must carry an AggregationResult");
    let value = result
        .aggregate_fields
        .get("field_0")
        .expect("expected default-synthesized alias field_0");
    match &value.value_type {
        Some(ValueType::IntegerValue(n)) => Ok(*n),
        other => panic!("expected IntegerValue for a COUNT result, got {other:?}"),
    }
}
