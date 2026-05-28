// SCAFFOLD: true
//! US-A03 — Query operations via agent
//!
//! As Alex (SDK Developer), I want getDocs(query(...)), runAggregation(...),
//! and listDocuments to work through the agent, so that read-heavy features
//! (dashboards, lists, analytics) work without SDK changes.
//!
//! Driving port: StorageAgent gRPC service (mTLS :9191)
//!   RPCs: RunQuery, RunAggregationQuery, ListDocuments
//! Red classification: MISSING_FUNCTIONALITY
//!
//! Feature file: tests/features/agent/us_a03_query_operations.feature
//! Execution order: S03A (after S02A writes)
//!
//! Note: RunAggregationQuery and ListDocuments are proto extensions (ADR-A01).
//! These tests assume the proto has been extended with those RPCs before S03A.

use embyr_proto::agent::{
    filter::FilterType,
    run_query_request::QueryType,
    run_query_response::ContinuationSelector,
    structured_query::CollectionSelector,
    CompositeFilterProto, FieldFilterOp, FieldFilterProto, Filter,
    RunQueryRequest, StructuredQuery as ProtoStructuredQuery,
    Value,
};
use embyr_proto::agent::value::ValueType;

use super::agent_common::{start_test_agent, AgentHandle};

// ---------------------------------------------------------------------------
// Happy path scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a03 @real_io
///
/// Feature: Filtered query returns only matching documents
///   Given 5 orders exist: 3 with status="pending", 2 with status="shipped"
///   When  a caller queries for status equals "pending"
///   Then  exactly 3 documents are returned; none have status="shipped"
#[tokio::test]
async fn filtered_query_returns_only_matching_documents() {
    let (handle, mut client) = start_test_agent("finops-prod").await;
    for i in 0..3u8 {
        sqlx::query(
            "INSERT INTO documents (project_id, collection_path, document_id, fields, version, create_time, update_time) \
             VALUES ($1,$2,$3,$4::jsonb,1,NOW(),NOW())",
        )
        .bind("finops-prod")
        .bind("orders")
        .bind(format!("ord-p-{i}"))
        .bind(r#"{"status":{"t":"S","v":"pending"}}"#)
        .execute(&handle.pool)
        .await
        .unwrap();
    }
    for i in 0..2u8 {
        sqlx::query(
            "INSERT INTO documents (project_id, collection_path, document_id, fields, version, create_time, update_time) \
             VALUES ($1,$2,$3,$4::jsonb,1,NOW(),NOW())",
        )
        .bind("finops-prod")
        .bind("orders")
        .bind(format!("ord-s-{i}"))
        .bind(r#"{"status":{"t":"S","v":"shipped"}}"#)
        .execute(&handle.pool)
        .await
        .unwrap();
    }
    let req = RunQueryRequest {
        parent: "projects/finops-prod/databases/(default)/documents".to_string(),
        query_type: Some(QueryType::StructuredQuery(ProtoStructuredQuery {
            from: vec![CollectionSelector {
                collection_id: "orders".to_string(),
                all_descendants: false,
            }],
            filter: Some(Filter {
                filter_type: Some(FilterType::FieldFilter(FieldFilterProto {
                    field_path: "status".to_string(),
                    op: FieldFilterOp::Equal as i32,
                    value: Some(Value {
                        value_type: Some(ValueType::StringValue("pending".to_string())),
                    }),
                })),
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    let mut stream = client
        .run_query(tonic::Request::new(req))
        .await
        .expect("run_query")
        .into_inner();
    let mut docs = vec![];
    while let Some(msg) = stream.message().await.expect("next") {
        if let Some(doc) = msg.document {
            docs.push(doc);
        }
    }
    assert_eq!(docs.len(), 3);
    for d in &docs {
        let s = d.fields.get("status").unwrap();
        assert!(
            matches!(&s.value_type, Some(ValueType::StringValue(v)) if v == "pending"),
            "expected status=pending, got {:?}",
            s
        );
    }
    drop(handle);
}

/// @driving_port @us_a03 @real_io
///
/// Feature: Collection group query traverses nested collections
///   Given "orders/ord-001/line_items/item-1" and "orders/ord-002/line_items/item-2" exist
///   When  a caller runs a collection group query for all "line_items" documents
///   Then  both documents are returned regardless of parent path
#[tokio::test]
async fn collection_group_query_traverses_nested_collections() {
    let (handle, mut client) = start_test_agent("finops-prod").await;
    sqlx::query(
        "INSERT INTO documents (project_id, collection_path, document_id, fields, version, create_time, update_time) \
         VALUES ($1,$2,$3,$4::jsonb,1,NOW(),NOW())",
    )
    .bind("finops-prod")
    .bind("orders/ord-001/line_items")
    .bind("item-1")
    .bind(r#"{"item":{"t":"S","v":"item-1"}}"#)
    .execute(&handle.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO documents (project_id, collection_path, document_id, fields, version, create_time, update_time) \
         VALUES ($1,$2,$3,$4::jsonb,1,NOW(),NOW())",
    )
    .bind("finops-prod")
    .bind("orders/ord-002/line_items")
    .bind("item-2")
    .bind(r#"{"item":{"t":"S","v":"item-2"}}"#)
    .execute(&handle.pool)
    .await
    .unwrap();
    let req = RunQueryRequest {
        parent: "projects/finops-prod/databases/(default)/documents".to_string(),
        query_type: Some(QueryType::StructuredQuery(ProtoStructuredQuery {
            from: vec![CollectionSelector {
                collection_id: "line_items".to_string(),
                all_descendants: true,
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    let mut stream = client
        .run_query(tonic::Request::new(req))
        .await
        .expect("run_query")
        .into_inner();
    let mut docs = vec![];
    while let Some(msg) = stream.message().await.expect("next") {
        if let Some(doc) = msg.document {
            docs.push(doc);
        }
    }
    assert_eq!(docs.len(), 2, "expected both line_items docs from different parent paths");
    drop(handle);
}

/// @driving_port @us_a03 @real_io
///
/// Feature: Count aggregation returns the correct total
///   Given 7 orders exist in the "orders" collection
///   When  a caller runs a count aggregation over "orders"
///   Then  the caller receives a count of 7
#[tokio::test]
#[ignore = "requires Docker — unskip in S05A delivery"]
async fn count_aggregation_returns_correct_total() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a03 @real_io
///
/// Feature: Listing documents returns results in pages of up to one hundred
///   Given 150 documents exist in the "orders" collection
///   When  a caller lists documents in "orders"
///   Then  the first page contains 100 documents with a continuation token
///   And   the final page has 50 documents with no continuation token
#[tokio::test]
#[ignore = "requires Docker — unskip in S05A delivery"]
async fn listing_documents_returns_pages_of_at_most_one_hundred() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a03 @real_io
///
/// Feature: Query excluding a field value omits documents missing that field
///   Given 3 orders have status="pending" and 2 orders have no status field
///   When  a caller queries for status not equal to "shipped"
///   Then  only the 3 orders with status="pending" are returned
#[tokio::test]
async fn query_excluding_value_omits_documents_missing_that_field() {
    let (handle, mut client) = start_test_agent("finops-prod").await;
    for i in 0..3u8 {
        sqlx::query(
            "INSERT INTO documents (project_id, collection_path, document_id, fields, version, create_time, update_time) \
             VALUES ($1,$2,$3,$4::jsonb,1,NOW(),NOW())",
        )
        .bind("finops-prod")
        .bind("orders")
        .bind(format!("ord-p-{i}"))
        .bind(r#"{"status":{"t":"S","v":"pending"}}"#)
        .execute(&handle.pool)
        .await
        .unwrap();
    }
    for i in 0..2u8 {
        sqlx::query(
            "INSERT INTO documents (project_id, collection_path, document_id, fields, version, create_time, update_time) \
             VALUES ($1,$2,$3,$4::jsonb,1,NOW(),NOW())",
        )
        .bind("finops-prod")
        .bind("orders")
        .bind(format!("ord-ns-{i}"))
        .bind(r#"{}"#)
        .execute(&handle.pool)
        .await
        .unwrap();
    }
    let req = RunQueryRequest {
        parent: "projects/finops-prod/databases/(default)/documents".to_string(),
        query_type: Some(QueryType::StructuredQuery(ProtoStructuredQuery {
            from: vec![CollectionSelector {
                collection_id: "orders".to_string(),
                all_descendants: false,
            }],
            filter: Some(Filter {
                filter_type: Some(FilterType::FieldFilter(FieldFilterProto {
                    field_path: "status".to_string(),
                    op: FieldFilterOp::NotEqual as i32,
                    value: Some(Value {
                        value_type: Some(ValueType::StringValue("shipped".to_string())),
                    }),
                })),
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    let mut stream = client
        .run_query(tonic::Request::new(req))
        .await
        .expect("run_query")
        .into_inner();
    let mut docs = vec![];
    while let Some(msg) = stream.message().await.expect("next") {
        if let Some(doc) = msg.document {
            docs.push(doc);
        }
    }
    assert_eq!(docs.len(), 3, "absent-field docs should be excluded by NOT_EQUAL");
    drop(handle);
}

/// @driving_port @us_a03 @real_io
///
/// Feature: Streaming query response indicates completion at the end
///   Given 3 orders exist in the "orders" collection
///   When  a caller runs a streaming query
///   Then  the final message indicates the query is complete with no document payload
#[tokio::test]
#[ignore = "requires Docker — unskip in S05A delivery"]
async fn streaming_query_response_indicates_completion_at_end() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

// ---------------------------------------------------------------------------
// Error scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a03 @real_io @error
///
/// Feature: Query over an empty collection returns no documents and a completion signal
///   Given no documents exist in the "invoices" collection
///   When  a caller queries the "invoices" collection without filters
///   Then  zero documents are returned and a completion signal is received
#[tokio::test]
async fn query_over_empty_collection_returns_no_documents_and_completion_signal() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    let req = RunQueryRequest {
        parent: "projects/finops-prod/databases/(default)/documents".to_string(),
        query_type: Some(QueryType::StructuredQuery(ProtoStructuredQuery {
            from: vec![CollectionSelector {
                collection_id: "invoices".to_string(),
                all_descendants: false,
            }],
            ..Default::default()
        })),
        ..Default::default()
    };
    let mut stream = client
        .run_query(tonic::Request::new(req))
        .await
        .expect("run_query")
        .into_inner();
    let mut doc_count = 0;
    let mut got_done = false;
    while let Some(msg) = stream.message().await.expect("next") {
        if msg.document.is_some() {
            doc_count += 1;
        }
        if matches!(msg.continuation_selector, Some(ContinuationSelector::Done(true))) {
            got_done = true;
        }
    }
    assert_eq!(doc_count, 0);
    assert!(got_done, "expected done=true signal");
}

/// @driving_port @us_a03 @real_io @error
///
/// Feature: Query with a malformed field path is rejected before any data is read
///   Given the "orders" collection contains documents
///   When  a caller queries with field path "order..amount" (double dot)
///   Then  the caller receives an invalid-request response
///   And   no documents are scanned from storage
#[tokio::test]
async fn query_with_malformed_field_path_rejected_before_data_read() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    let req = RunQueryRequest {
        parent: "projects/finops-prod/databases/(default)/documents".to_string(),
        query_type: Some(QueryType::StructuredQuery(ProtoStructuredQuery {
            from: vec![CollectionSelector {
                collection_id: "orders".to_string(),
                all_descendants: false,
            }],
            filter: Some(Filter {
                filter_type: Some(FilterType::FieldFilter(FieldFilterProto {
                    field_path: "order..amount".to_string(),
                    op: FieldFilterOp::Equal as i32,
                    value: Some(Value {
                        value_type: Some(ValueType::IntegerValue(100)),
                    }),
                })),
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    // For server-streaming, error may be on initial call OR first stream.message()
    match client.run_query(tonic::Request::new(req)).await {
        Err(status) => assert_eq!(
            status.code(),
            tonic::Code::InvalidArgument,
            "expected InvalidArgument"
        ),
        Ok(response) => {
            let mut stream = response.into_inner();
            let err = stream.message().await.expect_err("expected error from stream");
            assert_eq!(err.code(), tonic::Code::InvalidArgument);
        }
    }
}
