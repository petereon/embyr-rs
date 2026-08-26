//! Slice 03 (US-03, ADR-033) — A Listen Subscription Against a
//! Rule-Protected Collection Is Admitted Only If the Initial Snapshot's
//! Filter Is Compliant.
//!
//! Extends `check_query_compliance()` (ADR-031, unchanged) to `Listen`'s own
//! subscribe-time admission decision — the SAME mechanism `RunQuery`
//! already uses (`handle_run_query`'s own non-group arm), now composed into
//! `handle_add_target`'s async streaming call shape instead of a
//! request/response one.
//!
//! Acceptance criteria verified here (slice-03-subscribe-time-compliance-check.md):
//!   AC-17-113: a Listen subscription whose filter satisfies a
//!              rule-protected collection's own rule is admitted — the
//!              stream opens and the initial snapshot runs successfully.
//!   AC-17-114: a Listen subscription whose filter does NOT satisfy the
//!              rule is rejected outright — the stream never delivers any
//!              snapshot event before the terminal error.
//!   AC-17-115: `check_query_compliance()`/`QueryComplianceOutcome`/
//!              `UnsatisfiedConjunct` (ADR-031) are reused completely
//!              unchanged — a structural/reuse property, verified via `git
//!              diff` confirming zero changes to
//!              `crates/embyr-core/src/access_control/mod.rs`, NOT via a
//!              runtime assertion here (mirrors `security_rules_query_path`'s
//!              own AC-17-112 precedent: asserting AST/source shape in a
//!              test would itself be a banned "AST-shape test").
//!   AC-17-116: the rejection is a distinguishable terminal stream error
//!              (`PermissionDenied` with a `[REASON_CODE]`-style message,
//!              the SAME convention `query_compliance_rejection` already
//!              produces for `RunQuery`) — distinguishable from
//!              `authenticate()`-level rejections (`Unauthenticated`).
//!
//! Driving port: gRPC :8080 `Listen` (via `SecurityRulesFullContext` + this
//! feature's own `open_listen_stream_filtered_as`/`collect_initial_snapshot`
//! helpers) — Pillar 3, real Postgres, real gRPC, no mocks. Mirrors
//! `security_rules_query_path`'s own US-01/US-02 admit/reject mechanism
//! (`ownership_equality_query_compliance.rs`/
//! `noncompliant_query_rejected_preexecution.rs`) verbatim, composed into a
//! new async call shape.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    collect_initial_snapshot, equality_where_filter, mint_client_identity_token, now_unix,
    open_listen_stream_filtered_as, SecurityRulesFullContext,
};

use std::time::Duration;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use tokio_stream::StreamExt;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-113: a Listen subscription whose filter satisfies a rule-protected
// collection's own rule is admitted — the stream opens and the initial
// snapshot runs successfully.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has a READ rule requiring
///          `request.auth.uid == resource.data.owner_id` (the context's own
///          default-seeded document, `owner_id: "maria-santos"`)
///   And:   Maria Santos holds a verified identity
///   When:  Maria opens a Listen subscription with filter
///          `owner_id == "maria-santos"` (her own uid)
///   Then:  the subscription is admitted — the initial snapshot runs and
///          returns her own document, no terminal error
///
/// AC-17-113
///
/// @driving_port @real-io @US-03 @AC-17-113
#[tokio::test]
async fn a_compliant_listen_subscription_is_admitted_and_the_initial_snapshot_runs() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srr03-allow").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let filter = equality_where_filter(&[("owner_id", "maria-santos")]);
    let mut stream = open_listen_stream_filtered_as(
        &ctx,
        "journal_entries",
        filter,
        Some(&marias_token),
    )
    .await;

    let names = collect_initial_snapshot(&mut stream).await;

    assert_eq!(
        names.len(),
        1,
        "AC-17-113: expected exactly Maria's own default-seeded document in the admitted \
         subscription's initial snapshot, got: {names:?}"
    );
    assert!(
        names[0].ends_with("-maria-doc"),
        "AC-17-113: expected Maria's own document, got: {names:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-114: a Listen subscription whose filter does NOT satisfy the rule is
// rejected outright — the stream never delivers any snapshot event before
// the terminal error.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has the SAME READ rule as AC-17-113
///   And:   Dana Kim holds a verified identity (uid "dana-kim")
///   When:  Dana opens a Listen subscription with filter
///          `owner_id == "maria-santos"` — Maria's uid, NOT Dana's own uid
///   Then:  the subscription is rejected before the initial snapshot ever
///          runs — the FIRST message the client receives is the terminal
///          error, never a `DocumentChange`
///
/// AC-17-114
///
/// @error @driving_port @real-io @US-03 @AC-17-114 @security-critical
#[tokio::test]
async fn a_noncompliant_listen_subscription_is_rejected_before_any_row_is_read() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srr03-deny").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let filter = equality_where_filter(&[("owner_id", "maria-santos")]);
    let mut stream = open_listen_stream_filtered_as(
        &ctx,
        "journal_entries",
        filter,
        Some(&danas_token),
    )
    .await;

    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("AC-17-114: timed out waiting for the terminal rejection")
        .expect("AC-17-114: stream ended with no message at all");

    let err = first.expect_err(
        "AC-17-114: a non-compliant subscription must be rejected before any row is read — the \
         FIRST message must be the terminal error, never a DocumentChange from the initial \
         snapshot",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-114: rejection must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[OWNERSHIP_FILTER_MISSING]"),
        "AC-17-114/AC-17-116: rejection message must carry the SAME stable reason-code token \
         query_compliance_rejection() already produces for RunQuery, got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-116: the rejection is a distinguishable terminal stream error,
// distinguishable from authenticate()-level rejections.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has the SAME READ rule as AC-17-113/114
///   When:  (a) Dana opens a non-compliant Listen subscription (a compliance
///          rejection) and (b) a caller presents a wrong `api_key` against
///          the SAME collection (an `authenticate()`-level rejection)
///   Then:  the two rejections are distinguishable both by gRPC status CODE
///          (`PermissionDenied` vs `Unauthenticated`) and by message content
///
/// AC-17-116
///
/// @error @driving_port @real-io @US-03 @AC-17-116
#[tokio::test]
async fn the_subscribe_time_rejection_is_distinguishable_from_an_authenticate_level_rejection() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srr03-distinct").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let filter = equality_where_filter(&[("owner_id", "maria-santos")]);
    let mut compliance_stream = open_listen_stream_filtered_as(
        &ctx,
        "journal_entries",
        filter,
        Some(&danas_token),
    )
    .await;
    let compliance_err = tokio::time::timeout(Duration::from_secs(5), compliance_stream.next())
        .await
        .expect("timed out waiting for the compliance rejection")
        .expect("stream ended with no message at all")
        .expect_err("expected a compliance rejection for a non-compliant filter");

    // `Status::unauthenticated` never reaches `handle_add_target` at all — a
    // wrong api_key is rejected synchronously by `authenticate()` inside
    // `handle_listen`, before the stream is even established. The call
    // itself therefore returns `Err`, not a first-message error.
    let bad_key_ctx_channel =
        tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
            .expect("valid endpoint")
            .connect()
            .await
            .expect("connect to gRPC server");
    let mut bad_key_client =
        embyr_proto::firestore::firestore_client::FirestoreClient::new(bad_key_ctx_channel);
    let req_stream = tokio_stream::once(common::add_target_request(
        &ctx.project_id,
        "journal_entries",
    ));
    let mut bad_key_request = tonic::Request::new(req_stream);
    bad_key_request.metadata_mut().insert(
        "authorization",
        "Bearer wrong-api-key-totally-invalid".parse().unwrap(),
    );
    let auth_err = bad_key_client
        .listen(bad_key_request)
        .await
        .expect_err("expected an authenticate()-level rejection for a wrong api_key");

    assert_eq!(
        compliance_err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-116: compliance rejection must be PermissionDenied, got {:?}",
        compliance_err.code()
    );
    assert_eq!(
        auth_err.code(),
        tonic::Code::Unauthenticated,
        "AC-17-116: authenticate()-level rejection must be Unauthenticated, got {:?}",
        auth_err.code()
    );
    assert_ne!(
        compliance_err.code(),
        auth_err.code(),
        "AC-17-116: the two rejection classes must use different gRPC status codes"
    );
    assert!(
        compliance_err.message().contains("[OWNERSHIP_FILTER_MISSING]"),
        "AC-17-116: the compliance rejection must carry the reason-code token, got: {}",
        compliance_err.message()
    );
    assert!(
        !auth_err.message().contains("[OWNERSHIP_FILTER_MISSING]"),
        "AC-17-116: the authenticate()-level rejection must NEVER carry the compliance \
         reason-code token, got: {}",
        auth_err.message()
    );
}
