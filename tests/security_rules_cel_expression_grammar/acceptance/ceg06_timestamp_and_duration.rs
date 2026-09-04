//! CEG06 (Slice 06, US-06, Release 3) — Alex's Time-Window Clause Parses
//! and Enforces on Writes.
//!
//! Acceptance criteria verified here (feature-delta.md US-06, ADR-065):
//!   AC-CEG-15: `request.time` parses into `Operand::RequestTime`,
//!              resolving to a caller-supplied "now" `FieldValue::Timestamp`.
//!   AC-CEG-16: `duration.value(<int>, '<unit>')` parses ONLY as the
//!              right-hand side of `+`/`-` against a timestamp-typed left
//!              operand.
//!   AC-CEG-17: `<timestamp> +/- duration.value(...)` evaluates to a
//!              correctly-offset `Timestamp`, then participates in
//!              relational comparison.
//!   AC-CEG-18: an unrecognized duration unit is a NAMED rejection.
//!   AC-CEG-19: nested arithmetic or an unsupported operator (`*`/`/`/`%`)
//!              is a NAMED rejection, never a bare SyntaxError.
//!
//! Driving ports: Admin HTTP :9090 (`define_write_access_rule`) + gRPC
//! :8080 `CreateDocument`/`UpdateDocument` (`SecurityRulesFullContext`).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesFullContext;

fn timestamp_field(secs: i64) -> embyr_proto::firestore::Value {
    embyr_proto::firestore::Value {
        value_type: Some(embyr_proto::firestore::value::ValueType::TimestampValue(
            prost_types::Timestamp { seconds: secs, nanos: 0 },
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

async fn define_write_rule(ctx: &SecurityRulesFullContext, cookie: &str, collection_path: &str, condition: &str) {
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/write_access_rules",
            ctx.project_id
        )))
        .header("Cookie", cookie)
        .json(&serde_json::json!({
            "collection_path": collection_path,
            "condition": condition,
        }))
        .send()
        .await
        .expect("define_write_access_rule request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "setup: write-rule definition must succeed for '{condition}'"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-CEG-15/16/17: a time-window clause gates a real CreateDocument call
// correctly in both directions.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex imports `allow create: if request.resource.data.
///          submitted_at + duration.value(24, 'h') > request.time;` on
///          `journal_entries` (the submission is fresh iff it's still
///          within 24 hours)
///   When:  a real `CreateDocument` proposes a `submitted_at` timestamp
///          from 1 hour ago (still fresh), and separately one from 48
///          hours ago (stale)
///   Then:  the first succeeds, the second is denied
///
/// AC-CEG-15, AC-CEG-16, AC-CEG-17
///
/// @driving_port @real-io @US-06 @AC-CEG-15 @AC-CEG-16 @AC-CEG-17
#[tokio::test]
async fn a_time_window_clause_gates_a_real_create_in_both_directions() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ceg06-timewindow").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    define_write_rule(
        &ctx,
        &cookie,
        "journal_entries",
        "request.resource.data.submitted_at + duration.value(24, 'h') > request.time",
    )
    .await;

    let now = chrono::Utc::now().timestamp();
    let one_hour_ago = now - 3600;
    let forty_eight_hours_ago = now - (48 * 3600);

    let fresh = create_document(
        &ctx,
        "journal_entries",
        "fresh-submission",
        std::collections::HashMap::from([(
            "submitted_at".to_string(),
            timestamp_field(one_hour_ago),
        )]),
    )
    .await;
    assert!(
        fresh.is_ok(),
        "AC-CEG-17: submitted 1 hour ago is still within the 24h window and must succeed: {:?}",
        fresh.err()
    );

    let stale = create_document(
        &ctx,
        "journal_entries",
        "stale-submission",
        std::collections::HashMap::from([(
            "submitted_at".to_string(),
            timestamp_field(forty_eight_hours_ago),
        )]),
    )
    .await;
    let err = stale.expect_err("AC-CEG-17: submitted 48 hours ago is outside the 24h window and must be denied");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-CEG-18: an unrecognized duration unit is a named rejection.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-CEG-18
///
/// @error @driving_port @real-io @US-06 @AC-CEG-18
#[tokio::test]
async fn an_unrecognized_duration_unit_is_rejected_as_a_named_construct() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ceg06-badunit").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/write_access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "request.resource.data.submitted_at + duration.value(24, 'weeks') > request.time",
        }))
        .send()
        .await
        .expect("define_write_access_rule request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "AC-CEG-18: an unrecognized duration unit must be rejected"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["reason"], "UNSUPPORTED_CONSTRUCT");
    assert!(
        body["error"].as_str().is_some_and(|s| s.contains("duration unit")),
        "AC-CEG-18: the detail text must name the unrecognized unit, got {:?}",
        body["error"]
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-CEG-19: nested arithmetic or an unsupported operator is a named
// rejection.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-CEG-19
///
/// @error @driving_port @real-io @US-06 @AC-CEG-19
#[tokio::test]
async fn an_unsupported_arithmetic_operator_is_rejected_as_a_named_construct() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ceg06-badop").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/write_access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "resource.data.photo_count * 2 <= 20",
        }))
        .send()
        .await
        .expect("define_write_access_rule request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "AC-CEG-19: the '*' operator (not in this feature's own locked scope) must be rejected"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["reason"], "UNSUPPORTED_CONSTRUCT");
}

/// AC-CEG-19: chained/nested arithmetic (more than one `+`/`-` per operand)
/// is rejected the same way.
///
/// @error @driving_port @real-io @US-06 @AC-CEG-19
#[tokio::test]
async fn nested_arithmetic_is_rejected_as_a_named_construct() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ceg06-nested").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/write_access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "resource.data.a + 1 + 2 <= 20",
        }))
        .send()
        .await
        .expect("define_write_access_rule request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "AC-CEG-19: nested arithmetic (more than one '+'/'-' per operand) must be rejected"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["reason"], "UNSUPPORTED_CONSTRUCT");
}
