//! CEG04 (Slice 04, US-04, Release 2) — Alex's Whitelist Clause Parses and
//! Enforces on Writes.
//!
//! Acceptance criteria verified here (feature-delta.md US-04, ADR-065):
//!   AC-CEG-09: a bracketed, comma-separated list of literals parses into
//!              `Operand::ListLiteral`.
//!   AC-CEG-10: `<operand> in <list literal>` parses into a new `Condition`
//!              shape distinct from `Compare`.
//!   AC-CEG-11: a real write whose proposed field value is a member of the
//!              list succeeds; one whose value is not a member is denied.
//!   AC-CEG-12: `in` against a non-list-literal RHS is rejected as a NAMED
//!              unsupported construct, never silently misevaluated.
//!
//! Driving ports: Admin HTTP :9090 (`define_write_access_rule`) + gRPC
//! :8080 `CreateDocument` (`SecurityRulesFullContext`).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesFullContext;

fn string_field(value: &str) -> embyr_proto::firestore::Value {
    embyr_proto::firestore::Value {
        value_type: Some(embyr_proto::firestore::value::ValueType::StringValue(
            value.to_string(),
        )),
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

// ─────────────────────────────────────────────────────────────────────────────
// AC-CEG-09/10/11: a whitelist clause imports, and gates a real write in
// both directions.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex imports `allow write: if request.resource.data.status in
///          ["draft", "published", "archived"];` on `journal_entries`
///   When:  a real `CreateDocument` proposes `status: "published"` (in the
///          list), and separately one proposes `status: "deleted"` (not in
///          the list)
///   Then:  the first succeeds, the second is denied
///
/// AC-CEG-09, AC-CEG-10, AC-CEG-11
///
/// @driving_port @real-io @US-04 @AC-CEG-09 @AC-CEG-10 @AC-CEG-11
#[tokio::test]
async fn a_whitelist_clause_imports_and_gates_a_real_create_in_both_directions() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ceg04-whitelist").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let define_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/write_access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "request.resource.data.status in [\"draft\", \"published\", \"archived\"]",
        }))
        .send()
        .await
        .expect("define_write_access_rule request failed");
    assert_eq!(
        define_resp.status().as_u16(),
        200,
        "AC-CEG-09/10: a whitelist 'in' condition must be accepted, not rejected"
    );

    let member = create_document(
        &ctx,
        "journal_entries",
        "published-entry",
        std::collections::HashMap::from([("status".to_string(), string_field("published"))]),
    )
    .await;
    assert!(
        member.is_ok(),
        "AC-CEG-11: status='published' is a list member and must succeed: {:?}",
        member.err()
    );

    let non_member = create_document(
        &ctx,
        "journal_entries",
        "deleted-entry",
        std::collections::HashMap::from([("status".to_string(), string_field("deleted"))]),
    )
    .await;
    let err = non_member.expect_err("AC-CEG-11: status='deleted' is not a list member and must be denied");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-CEG-12: `in` against a non-list-literal RHS is a named rejection.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-CEG-12
///
/// @error @driving_port @real-io @US-04 @AC-CEG-12
#[tokio::test]
async fn in_against_a_non_list_literal_rhs_is_rejected_as_a_named_construct() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ceg04-innonlist").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let define_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/write_access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "request.resource.data.status in request.resource.data.allowed_statuses",
        }))
        .send()
        .await
        .expect("define_write_access_rule request failed");

    assert_eq!(
        define_resp.status().as_u16(),
        400,
        "AC-CEG-12: 'in' against a non-list-literal RHS must be rejected"
    );
    let body: serde_json::Value = define_resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["reason"], "UNSUPPORTED_CONSTRUCT",
        "AC-CEG-12: must be a NAMED unsupported construct, distinguishable from SYNTAX_ERROR"
    );
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|s| s.contains("list literal")),
        "AC-CEG-12: the detail text must name what's unsupported, got {:?}",
        body["error"]
    );
}
