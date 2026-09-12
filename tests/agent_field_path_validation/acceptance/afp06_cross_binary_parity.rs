// SCAFFOLD: true
//! AFP06 (agent-field-path-validation, AC-AFP-06) — the identical malformed
//! `field_path` submitted to a `backend_mode=direct_pg` project (via
//! `embyr-server`) and a `backend_mode=agent` project (via `embyr-agent`)
//! both return `INVALID_ARGUMENT`, closing the confirmed asymmetry finding
//! #11 named.
//!
//! Driving ports: gRPC :8080 `RunQuery` on `embyr-server`
//! (`SecurityRulesFullContext`, direct_pg) AND StorageAgent gRPC :9191
//! `RunQuery` on `embyr-agent` (`start_test_agent`) — the SAME malformed
//! `field_path` sent to each of the two real, independently-running
//! backends in one test.
//!
//! Per feature-delta.md § Wave: DESIGN "Error Message Format Consistency":
//! `embyr-server` converts the shared `CoreError` via `e.to_string()`
//! (`Display`-prefixed: `"invalid argument: field path must match..."`),
//! while `embyr-agent`'s reused `core_error_to_status` forwards the bare
//! inner message (`"field path must match..."`, no prefix) — a conscious,
//! accepted DESIGN trade-off. This test asserts the shared SUBSTRING, never
//! exact cross-binary string equality.
//!
//! Red classification (pre-fix): `embyr-agent`'s own side of this
//! comparison currently returns `Status::internal` (or worse, a
//! successful-but-wrong response) instead of `INVALID_ARGUMENT` — see
//! `us_a09_field_path_validation.rs` for the empirically-confirmed exact
//! behavior. `embyr-server`'s own side already passes today (it already
//! calls the real charset guard).

#![allow(unused_imports)]

#[path = "../../security_rules/common/mod.rs"]
mod security_rules_common;
use security_rules_common::SecurityRulesFullContext;

#[path = "../../acceptance/embyr_agent/mod.rs"]
mod agent_common;
use agent_common::start_test_agent;

use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    run_query_request::QueryType as FsQueryType,
    structured_query::{
        field_filter::Operator as FsFieldOp, filter::FilterType as FsFilterType,
        CollectionSelector as FsCollectionSelector, FieldFilter as FsFieldFilter,
        FieldReference as FsFieldReference, Filter as FsFilter,
    },
    value::ValueType as FsValueType,
    RunQueryRequest as FsRunQueryRequest, StructuredQuery as FsStructuredQuery, Value as FsValue,
};

use embyr_proto::agent::{
    filter::FilterType as AgentFilterType, run_query_request::QueryType as AgentQueryType,
    structured_query::CollectionSelector as AgentCollectionSelector, value::ValueType as AgentValueType,
    FieldFilterOp as AgentFieldFilterOp, FieldFilterProto as AgentFieldFilterProto,
    Filter as AgentFilter, RunQueryRequest as AgentRunQueryRequest,
    StructuredQuery as AgentStructuredQuery, Value as AgentValue,
};

const MALFORMED_FIELD_PATH: &str = "name;DROP TABLE documents;";

fn assert_invalid_argument_with_shared_message(status: &tonic::Status, who: &str) {
    assert_eq!(
        status.code(),
        tonic::Code::InvalidArgument,
        "{who}: expected INVALID_ARGUMENT for field_path {MALFORMED_FIELD_PATH:?}, got {:?}: {}",
        status.code(),
        status.message()
    );
    assert!(
        status.message().contains("field path must match"),
        "{who}: expected the shared spec-mandated charset-violation substring, got: {}",
        status.message()
    );
}

/// @driving_port @real_io @us_a09 @error @AC-AFP-06
///
/// Feature: The agent's own rejection now matches embyr-server's own rejection for the identical malformed field path
///   Given a "backend_mode=direct_pg" project and a "backend_mode=agent" project both receive
///     a RunQuery filter with field_path "name;DROP TABLE documents;"
///   When each request is evaluated by its own backend
///   Then both RPCs return INVALID_ARGUMENT
///   And both rejection messages report the same spec-mandated charset violation (shared substring)
#[tokio::test]
async fn identical_malformed_field_path_rejected_identically_by_both_backend_modes() {
    // --- embyr-server side (backend_mode=direct_pg) ---
    let server_ctx = SecurityRulesFullContext::new("meridian-direct-pg-afp06").await;
    let server_channel =
        tonic::transport::Channel::from_shared(format!("http://{}", server_ctx.server.grpc_addr))
            .unwrap()
            .connect_lazy();
    let mut server_client = FirestoreClient::new(server_channel);
    let mut server_req = tonic::Request::new(FsRunQueryRequest {
        parent: format!(
            "projects/{}/databases/(default)/documents",
            server_ctx.project_id
        ),
        query_type: Some(FsQueryType::StructuredQuery(FsStructuredQuery {
            from: vec![FsCollectionSelector { collection_id: "patients".to_string(), all_descendants: false }],
            r#where: Some(FsFilter {
                filter_type: Some(FsFilterType::FieldFilter(FsFieldFilter {
                    field: Some(FsFieldReference { field_path: MALFORMED_FIELD_PATH.to_string() }),
                    op: FsFieldOp::Equal as i32,
                    value: Some(FsValue { value_type: Some(FsValueType::StringValue("active".to_string())) }),
                })),
            }),
            ..Default::default()
        })),
        ..Default::default()
    });
    server_req
        .metadata_mut()
        .insert("authorization", format!("bearer {}", server_ctx.api_key).parse().unwrap());
    let server_status = match server_client.run_query(server_req).await {
        Err(status) => status,
        Ok(response) => {
            use tokio_stream::StreamExt;
            let mut stream = response.into_inner();
            stream
                .next()
                .await
                .expect("stream must yield at least one message")
                .expect_err("embyr-server (direct_pg) must reject the malformed field path")
        }
    };
    assert_invalid_argument_with_shared_message(&server_status, "embyr-server (backend_mode=direct_pg)");

    // --- embyr-agent side (backend_mode=agent) ---
    let (_agent_handle, mut agent_client) = start_test_agent("meridian-agent-afp06").await;
    let agent_req = AgentRunQueryRequest {
        parent: "projects/meridian-agent-afp06/databases/(default)/documents".to_string(),
        query_type: Some(AgentQueryType::StructuredQuery(AgentStructuredQuery {
            from: vec![AgentCollectionSelector { collection_id: "patients".to_string(), all_descendants: false }],
            filter: Some(AgentFilter {
                filter_type: Some(AgentFilterType::FieldFilter(AgentFieldFilterProto {
                    field_path: MALFORMED_FIELD_PATH.to_string(),
                    op: AgentFieldFilterOp::Equal as i32,
                    value: Some(AgentValue { value_type: Some(AgentValueType::StringValue("active".to_string())) }),
                })),
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    let agent_status = match agent_client.run_query(tonic::Request::new(agent_req)).await {
        Err(status) => status,
        Ok(response) => {
            let mut stream = response.into_inner();
            stream
                .message()
                .await
                .expect_err("embyr-agent (backend_mode=agent) must reject the malformed field path")
        }
    };
    assert_invalid_argument_with_shared_message(&agent_status, "embyr-agent (backend_mode=agent)");
}
