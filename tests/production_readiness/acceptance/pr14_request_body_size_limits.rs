// @boundary @US-PR-24
//! request-body-size-limits (finding #24, Medium/Reliability) — every listener
//! surface has an explicit, deliberately-chosen byte ceiling instead of an
//! implicit tonic/axum library default (ADR-081).
//!
//! Acceptance criteria verified here (feature-delta.md DESIGN § Handoff
//! Package "Regression guards"):
//!   AC-RBL-01: a gRPC request just under the 10 MiB ceiling succeeds on the
//!              native :8080 listener; just over is rejected cleanly
//!              (tonic's `OUT_OF_RANGE`, not a panic/connection reset).
//!   AC-RBL-02: a REST (`accounts:*`, :8081) request just under the 2 MiB
//!              ceiling is NOT rejected on size grounds; just over gets a
//!              clean 413.
//!   AC-RBL-03: an admin (:9090) JSON request just under the 1 MiB ceiling
//!              succeeds; just over gets a clean 413.
//!   AC-RBL-04 (regression guard): the Stripe webhook route's own,
//!              independent 5 MiB ceiling (ADR-070) is unaffected by the new
//!              admin-level 1 MiB layer — a body between the two ceilings
//!              still reaches HMAC verification (401), not 413.
//!
//! Consolidation: ONE test function, ONE Postgres-backed server process,
//! covering all four surfaces — per `nw-test-optimization` skill's
//! cross-tier dedup pattern. Each surface's ceiling is an independent library
//! knob (tonic `max_decoding_message_size` / axum `DefaultBodyLimit`) with no
//! shared code path, so this is one acceptance scenario ("all four listener
//! ceilings are enforced correctly") exercised through four assertions, not
//! four scenarios — spinning a fresh subprocess + testcontainer per surface
//! would be Testing Theater via ceremony, not additional coverage (ponytail:
//! avoid extra container spin-up on an 8GB test box).
//!
//! Byte-precision: boundary payloads are computed to land within ~1 KiB of
//! each ceiling (gRPC: via `prost::Message::encoded_len` probe: JSON/raw
//! bodies: via exact string length) rather than using full round-number
//! buffers — proves the CHOSEN ceiling value, not just "some limit exists".
//!
//! Real-I/O strategy: real `embyr-server` subprocess + real Postgres
//! testcontainers (system DB + tenant DB), mirroring pr07/pr11's own
//! established harness. Layer: WS/`@wiring_e2e`.

use std::collections::HashMap;
use std::time::Duration;

use prost::Message as _;

use crate::common::{
    self, provision_project, start_postgres_container, ServerProcess, TEST_ENCRYPTION_KEY,
};

use embyr_proto::firestore::{firestore_client::FirestoreClient, value::ValueType, CreateDocumentRequest, Document, Value};

/// Test-local duplicate of DESIGN's locked gRPC ceiling (ADR-081) — matches
/// pr07's own precedent of duplicating the ceiling as a plain `usize` so the
/// test expresses "just under/over" relative to the SAME number DESIGN
/// locked, not a guess.
const GRPC_CEILING_BYTES: usize = 10 * 1024 * 1024;
const REST_CEILING_BYTES: usize = 2 * 1024 * 1024;
const ADMIN_CEILING_BYTES: usize = 1 * 1024 * 1024;
/// Stripe webhook's own, independent, already-shipped ceiling (ADR-070) —
/// untouched by this feature. Used only to pick a body size that sits
/// between the NEW admin ceiling and the OLD stripe ceiling for AC-RBL-04.
const STRIPE_CEILING_BYTES: usize = 5 * 1024 * 1024;

const STRIPE_SECRET_KEY: &str = "sk_live_request_body_size_limits_test";
const STRIPE_WEBHOOK_SECRET: &str = "whsec_request_body_size_limits_test";

fn create_document_request(
    project_id: &str,
    api_key: &str,
    document_id: &str,
    field_len: usize,
) -> tonic::Request<CreateDocumentRequest> {
    let mut fields = HashMap::new();
    fields.insert(
        "padding".to_string(),
        Value {
            value_type: Some(ValueType::StringValue("a".repeat(field_len))),
        },
    );
    let mut request = tonic::Request::new(CreateDocumentRequest {
        parent: format!("projects/{project_id}/databases/(default)/documents"),
        collection_id: "boundary".to_string(),
        document_id: document_id.to_string(),
        document: Some(Document {
            name: String::new(),
            fields,
            ..Default::default()
        }),
        ..Default::default()
    });
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {api_key}").parse().expect("valid header"));
    request
}

/// Build a JSON provision body of exactly `total_len` bytes by padding an
/// otherwise-valid `ProvisionRequest` with an ignored extra field (no
/// `deny_unknown_fields` on the struct — confirmed by reading
/// `admin/handlers/provision.rs`).
fn provision_body_of_len(project_id: &str, dsn: &str, total_len: usize) -> String {
    let prefix = format!(
        r#"{{"project_id":"{project_id}","dsn":"{dsn}","backend_mode":"direct_pg","padding":""#
    );
    let suffix = "\"}";
    let pad_len = total_len.saturating_sub(prefix.len() + suffix.len());
    format!("{prefix}{}{suffix}", "a".repeat(pad_len))
}

/// AC-RBL-01 through AC-RBL-04: all four listener-surface ceilings from
/// ADR-081, proven against one real running server instance.
#[tokio::test]
async fn request_body_ceilings_enforced_on_every_listener_surface() {
    let (_pg_system, system_db_url) = start_postgres_container().await;
    let (_pg_tenant, tenant_db_url) = start_postgres_container().await;

    let server = ServerProcess::start(
        &system_db_url,
        &[
            ("EMBYR_ADMIN_KEY", common::ADMIN_KEY),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
            ("STRIPE_SECRET_KEY", STRIPE_SECRET_KEY),
            ("STRIPE_WEBHOOK_SIGNING_SECRET", STRIPE_WEBHOOK_SECRET),
        ],
    );
    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(healthy, "server must start normally");

    let http = reqwest::Client::new();

    // ── AC-RBL-01: gRPC native (:8080) — 10 MiB ceiling ─────────────────────
    let project_id = "rbl-grpc-project";
    let api_key = provision_project(server.admin_port, project_id, &tenant_db_url).await;
    let channel = common::grpc_channel(server.grpc_port);
    // `create_document` echoes the full created document back (including our
    // padding field), so the client's OWN decode ceiling — separate from the
    // server's, and tonic defaults it to 4 MiB too — must be raised to
    // receive that response. Not a production concern: only this test's
    // client needs it, since a real SDK never round-trips a padding field.
    let mut grpc_client =
        FirestoreClient::new(channel).max_decoding_message_size(GRPC_CEILING_BYTES + 4096);

    // Probe request (tiny field) establishes the fixed protobuf overhead for
    // THIS exact message shape, so the two real requests below can target
    // the ceiling within ~1 KiB rather than guessing.
    let probe = create_document_request(project_id, &api_key, "probe", 0);
    let overhead = probe.get_ref().encoded_len();

    let under_field_len = GRPC_CEILING_BYTES.saturating_sub(overhead + 1024);
    let under_req = create_document_request(project_id, &api_key, "under-ceiling", under_field_len);
    let under_result = grpc_client.create_document(under_req).await;
    assert!(
        under_result.is_ok(),
        "a document just under the 10 MiB gRPC ceiling must be accepted, got: {:?}",
        under_result.err()
    );

    let over_field_len = GRPC_CEILING_BYTES.saturating_sub(overhead) + 1024;
    let over_req = create_document_request(project_id, &api_key, "over-ceiling", over_field_len);
    let over_result = grpc_client.create_document(over_req).await;
    let over_status = over_result.expect_err("a document over the 10 MiB gRPC ceiling must be rejected, not accepted");
    // tonic's own codec rejects an over-ceiling decode with `OutOfRange`
    // (`codec/decode.rs`), not `ResourceExhausted` — confirmed empirically
    // against tonic 0.12.3's real behavior, not assumed.
    assert_eq!(
        over_status.code(),
        tonic::Code::OutOfRange,
        "oversized gRPC request must be rejected with OUT_OF_RANGE, not a panic/connection reset, got: {over_status:?}"
    );

    // ── AC-RBL-02: REST/:8081 accounts bridge — 2 MiB ceiling ───────────────
    // `signInWithCustomToken` decodes the raw body as `Bytes` before any
    // JSON parsing (`accounts_bridge_dispatch`, `lib.rs`) — a non-JSON body
    // still proves the size gate; a malformed-JSON body just falls through
    // to `MISSING_TOKEN`, never 413, once past the gate.
    let rest_url = format!(
        "http://127.0.0.1:{}/v1/projects/rbl-rest-project/accounts:signInWithCustomToken",
        server.rest_port
    );
    let under_body = vec![b'x'; REST_CEILING_BYTES - 1024];
    let under_rest_resp = http
        .post(&rest_url)
        .body(under_body)
        .send()
        .await
        .expect("under-ceiling REST POST must complete");
    assert_ne!(
        under_rest_resp.status().as_u16(),
        413,
        "a body just under the 2 MiB REST ceiling must not be rejected on size grounds"
    );

    let over_body = vec![b'x'; REST_CEILING_BYTES + 1024];
    let over_rest_resp = http
        .post(&rest_url)
        .body(over_body)
        .send()
        .await
        .expect("over-ceiling REST POST must complete");
    assert_eq!(
        over_rest_resp.status().as_u16(),
        413,
        "a body over the 2 MiB REST ceiling must be rejected with 413"
    );

    // ── AC-RBL-03: admin (:9090) JSON routes — 1 MiB ceiling ────────────────
    let admin_url = format!("http://127.0.0.1:{}/admin/v1/projects", server.admin_port);

    let under_admin_body =
        provision_body_of_len("rbl-admin-under", &tenant_db_url, ADMIN_CEILING_BYTES - 1024);
    let under_admin_resp = http
        .post(&admin_url)
        .header("Authorization", format!("Bearer {}", common::ADMIN_KEY))
        .header("Content-Type", "application/json")
        .body(under_admin_body)
        .send()
        .await
        .expect("under-ceiling admin POST must complete");
    assert!(
        under_admin_resp.status().is_success(),
        "a provision request just under the 1 MiB admin ceiling must succeed, got: {}",
        under_admin_resp.status()
    );

    // Exactly 1 byte over (not the usual ~1 KiB margin): a `1 * 1024 * 1024`
    // ceiling has a degenerate first operand, so mutating `*` to `+` here
    // yields `1 + 1024 * 1024` = ceiling + 1 byte, not the huge swing a
    // `1024 * 1024` operand mutation produces. A KiB-scale margin can't see
    // a 1-byte drift; byte-exact can (cargo-mutants finding, QUALITY_GATE).
    let over_admin_body =
        provision_body_of_len("rbl-admin-over", &tenant_db_url, ADMIN_CEILING_BYTES + 1);
    let over_admin_resp = http
        .post(&admin_url)
        .header("Authorization", format!("Bearer {}", common::ADMIN_KEY))
        .header("Content-Type", "application/json")
        .body(over_admin_body)
        .send()
        .await
        .expect("over-ceiling admin POST must complete");
    assert_eq!(
        over_admin_resp.status().as_u16(),
        413,
        "a provision request over the 1 MiB admin ceiling must be rejected with 413"
    );

    // ── AC-RBL-04 (regression guard): Stripe webhook's own 5 MiB ceiling ────
    // unaffected by the new admin-level 1 MiB layer. A body strictly between
    // the two ceilings must still reach HMAC verification (401 on garbage
    // signature), never 413 — proving the new layer did not start governing
    // this route (DESIGN confirmed the manual `to_bytes` call bypasses the
    // `DefaultBodyLimit` extension entirely).
    let stripe_url = format!("http://127.0.0.1:{}/admin/v1/webhooks/stripe", server.admin_port);
    let mid_ceiling_len = (ADMIN_CEILING_BYTES + STRIPE_CEILING_BYTES) / 2;
    let mid_body = vec![b'a'; mid_ceiling_len];
    let stripe_resp = http
        .post(&stripe_url)
        .header(
            "Stripe-Signature",
            "t=0,v1=deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdead",
        )
        .body(mid_body)
        .send()
        .await
        .expect("mid-ceiling Stripe webhook POST must complete");
    assert_eq!(
        stripe_resp.status().as_u16(),
        401,
        "a body between the admin (1 MiB) and Stripe (5 MiB) ceilings must still reach HMAC \
         verification (401), proving the new admin-level DefaultBodyLimit does not govern the \
         Stripe webhook route"
    );
}
