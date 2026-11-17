//! US-A10 — SUM/AVG/capped COUNT through the agent's RunAggregationQuery.
//!
//! Driving port: StorageAgent gRPC service. Semantics must match direct-PG
//! mode (same SQL builder): only integer/double values are aggregated,
//! SUM over nothing is 0, AVG over nothing is unset (null).

use embyr_proto::agent::{
    aggregation_op::Op, aggregation_result::Value, run_aggregation_query_request::QueryType,
    structured_query::CollectionSelector, AggregationCount, AggregationField, AggregationOp,
    RunAggregationQueryRequest, StructuredQuery,
};

use super::agent_common::start_test_agent;

fn request(ops: Vec<Op>) -> RunAggregationQueryRequest {
    RunAggregationQueryRequest {
        parent: "projects/finops-prod/databases/(default)/documents".to_string(),
        query_type: Some(QueryType::StructuredQuery(StructuredQuery {
            from: vec![CollectionSelector { collection_id: "orders".to_string(), all_descendants: false }],
            ..Default::default()
        })),
        aggregations: ops.into_iter().map(|op| AggregationOp { op: Some(op) }).collect(),
    }
}

fn field(f: &str) -> AggregationField {
    AggregationField { field_path: f.to_string() }
}

async fn seed(pool: &sqlx::PgPool, id: &str, fields: &str) {
    sqlx::query(
        "INSERT INTO documents (project_id, collection_path, document_id, fields, version, create_time, update_time) \
         VALUES ($1,$2,$3,$4::jsonb,1,NOW(),NOW())",
    )
    .bind("finops-prod")
    .bind("orders")
    .bind(id)
    .bind(fields)
    .execute(pool)
    .await
    .unwrap();
}

/// Mixed int/double/null/string/absent values: numeric ones are aggregated,
/// the rest ignored; capped COUNT and plain COUNT see every row.
#[tokio::test]
async fn sum_avg_and_counts_over_mixed_values_ignore_non_numeric_fields() {
    let (handle, mut client) = start_test_agent("finops-prod").await;
    seed(&handle.pool, "a", r#"{"n":{"t":"I","v":2}}"#).await;
    seed(&handle.pool, "b", r#"{"n":{"t":"D","v":4.5}}"#).await;
    seed(&handle.pool, "c", r#"{"n":{"t":"S","v":"x"}}"#).await;
    seed(&handle.pool, "d", r#"{"other":{"t":"I","v":9}}"#).await;
    let resp = client
        .run_aggregation_query(tonic::Request::new(request(vec![
            Op::Sum(field("n")),
            Op::Avg(field("n")),
            Op::Count(AggregationCount { up_to: 0 }),
            Op::Count(AggregationCount { up_to: 3 }),
        ])))
        .await
        .expect("run_aggregation_query")
        .into_inner();
    let got: Vec<_> = resp.results.into_iter().map(|r| r.value).collect();
    assert_eq!(
        got,
        [Some(Value::Sum(6.5)), Some(Value::Avg(3.25)), Some(Value::Count(4)), Some(Value::Count(3))]
    );
    drop(handle);
}

/// Empty set: SUM = 0, AVG = unset.
#[tokio::test]
async fn sum_and_avg_over_an_empty_set_are_zero_and_null() {
    let (handle, mut client) = start_test_agent("finops-prod").await;
    let resp = client
        .run_aggregation_query(tonic::Request::new(request(vec![Op::Sum(field("n")), Op::Avg(field("n"))])))
        .await
        .expect("run_aggregation_query")
        .into_inner();
    let got: Vec<_> = resp.results.into_iter().map(|r| r.value).collect();
    assert_eq!(got, [Some(Value::Sum(0.0)), None]);
    drop(handle);
}

/// A malicious field path is rejected before reaching the SQL builder.
#[tokio::test]
async fn aggregation_field_path_is_validated() {
    let (handle, mut client) = start_test_agent("finops-prod").await;
    let status = client
        .run_aggregation_query(tonic::Request::new(request(vec![Op::Sum(field("n'); DROP TABLE documents; --"))])))
        .await
        .expect_err("invalid field path must be rejected");
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    drop(handle);
}
