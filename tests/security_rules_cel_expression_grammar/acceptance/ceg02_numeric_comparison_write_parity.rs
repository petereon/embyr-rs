//! CEG02 (Slice 02, US-02, Release 1) — The Same Numeric-Comparison Grammar
//! Gates Writes.
//!
//! Acceptance criteria verified here (feature-delta.md US-02, ADR-065):
//!   AC-CEG-06: `request.resource.data.<field>` participates in the new
//!              relational-comparison grammar identically to
//!              `resource.data.<field>` (US-01).
//!   AC-CEG-07: a real `CreateDocument` call proposing a document violating
//!              the bound is denied; one satisfying it succeeds.
//!
//! Driving ports: Admin HTTP :9090 (`define_write_access_rule`) + gRPC
//! :8080 `CreateDocument` (`SecurityRulesFullContext`).
//!
//! `create_document` is inlined here rather than reused from
//! `security_rules_write_path`'s own common module — that module's free
//! function takes `&SecurityRulesFullContext` from ITS OWN `#[path]` chain,
//! a structurally-identical but Rust-distinct type from this feature's own
//! chain (reused from `security_rules_cel_parity` directly, per this
//! feature's own common/mod.rs) — mirrors `SecurityRulesFullContext::
//! get_document`'s own shape exactly.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesFullContext;

fn int_field(value: i64) -> embyr_proto::firestore::Value {
    embyr_proto::firestore::Value {
        value_type: Some(embyr_proto::firestore::value::ValueType::IntegerValue(value)),
    }
}

async fn create_document(
    ctx: &SecurityRulesFullContext,
    collection_id: &str,
    document_id: &str,
    fields: std::collections::HashMap<String, embyr_proto::firestore::Value>,
) -> Result<tonic::Response<embyr_proto::firestore::Document>, tonic::Status> {
    use embyr_proto::firestore::{firestore_client::FirestoreClient, CreateDocumentRequest, Document};

    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let mut request = tonic::Request::new(CreateDocumentRequest {
        parent: format!("projects/{}/databases/(default)/documents", ctx.project_id),
        collection_id: collection_id.to_string(),
        document_id: document_id.to_string(),
        document: Some(Document { name: String::new(), fields, ..Default::default() }),
        ..Default::default()
    });
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {}", ctx.api_key).parse().unwrap());

    client.create_document(request).await
}

/// Journey:
///   Given: Alex imports `allow write: if request.resource.data.cost_usd >=
///          0;` on `expeditions`
///   When:  a real `CreateDocument` call proposes `cost_usd: -5` (violates
///          the bound), and separately one proposes `cost_usd: 500`
///          (satisfies it)
///   Then:  the first is denied, the second succeeds
///
/// AC-CEG-06, AC-CEG-07
///
/// @driving_port @real-io @US-02 @AC-CEG-06 @AC-CEG-07
#[tokio::test]
async fn a_numeric_bound_clause_gates_a_real_create_in_both_directions() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ceg02-writebound").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let define_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/write_access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "expeditions",
            "condition": "request.resource.data.cost_usd >= 0",
        }))
        .send()
        .await
        .expect("define_write_access_rule request failed");
    assert_eq!(
        define_resp.status().as_u16(),
        200,
        "AC-CEG-06: a request.resource.data numeric-bound condition must be accepted"
    );

    let negative = create_document(
        &ctx,
        "expeditions",
        "over-budget",
        std::collections::HashMap::from([("cost_usd".to_string(), int_field(-5))]),
    )
    .await;
    let err = negative.expect_err("AC-CEG-07: cost_usd=-5 violates '>= 0' and must be denied");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());

    let positive = create_document(
        &ctx,
        "expeditions",
        "within-budget",
        std::collections::HashMap::from([("cost_usd".to_string(), int_field(500))]),
    )
    .await;
    assert!(
        positive.is_ok(),
        "AC-CEG-07: cost_usd=500 satisfies '>= 0' and must succeed: {:?}",
        positive.err()
    );
}
