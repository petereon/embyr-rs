//! Slice 05 (US-05, ADR-030) — A Session With No Verified Identity Is
//! Evaluated as Anonymous on Writes.
//!
//! Acceptance criteria verified here (feature-delta.md US-05):
//!   AC-17-39: a never-signed-in session's create/update/delete is denied by
//!             a write rule requiring `request.auth != null`.
//!   AC-17-40: a never-signed-in session succeeds against a write rule that
//!             explicitly allows unauthenticated writes (`true`).
//!   AC-17-41: an invalid (expired) client-identity header on a write call
//!             is evaluated IDENTICALLY to no header at all — no new
//!             rejection class introduced, mirroring `security-rules`' own
//!             AC-17-13 precedent exactly.
//!
//! Production wiring note: `attach_client_identity_if_present()` is already
//! called by all three write handlers (`handle_create_document`,
//! `handle_update_document`, `handle_delete_document` — Slices 02/03/04's
//! own composition, per ADR-030's "three new call sites, one shared
//! pattern" decision). This slice adds NO new production wiring — it is
//! pure acceptance-test authorship proving the existing, unmodified
//! composition behaves correctly for the never-signed-in and
//! invalid-identity cases, mirroring `security-rules`' own US-03
//! (`sr03_anonymous_session_evaluated_as_null_auth.rs`) now applied to the
//! three write call sites instead of one.
//!
//! Driving port: gRPC :8080 `CreateDocument`/`UpdateDocument`/
//! `DeleteDocument` (via `SecurityRulesFullContext` + this feature's own
//! `create_document`/`update_document`/`delete_document`/
//! `seed_write_access_rule_full` helpers — Pillar 3, real Postgres, real
//! gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, delete_document, mint_client_identity_token, now_unix,
    seed_write_access_rule_full, string_field, update_document, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-39: a never-signed-in session's create/update/delete is denied by a
// write rule requiring `request.auth != null` (all three write operations —
// the identity-attach + AuthContext-threading mechanism is identical across
// all three handlers, proven together as one behavior).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has a WRITE rule requiring
///          `request.auth != null`
///   When:  a session that never presented a client-identity token calls
///          `createDoc()`, `updateDoc()`, and `deleteDoc()` against it
///   Then:  all three are denied, attributable to the rule — the pre-existing
///          seed document `maria-doc` (used as the update/delete target) is
///          left untouched, since none of the three denials mutate any
///          document
///
/// AC-17-39
///
/// @driving_port @real-io @US-05 @AC-17-39
#[tokio::test]
async fn a_never_signed_in_session_is_denied_on_create_update_and_delete_by_an_auth_required_write_rule(
) {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp05-denyanon").await;
    seed_write_access_rule_full(&ctx, "journal_entries", "request.auth != null").await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("nobody"));

    let create_resp =
        create_document(&ctx, "journal_entries", "swp05-denyanon-doc", fields.clone(), None)
            .await;
    let update_resp = update_document(
        &ctx,
        &ctx.document_resource_name("journal_entries", "maria-doc"),
        fields,
        None,
    )
    .await;
    let delete_resp = delete_document(
        &ctx,
        &ctx.document_resource_name("journal_entries", "maria-doc"),
        None,
    )
    .await;

    let create_err = create_resp.expect_err(
        "AC-17-39: a never-signed-in create must be denied by an auth-required write rule",
    );
    let update_err = update_resp.expect_err(
        "AC-17-39: a never-signed-in update must be denied by an auth-required write rule",
    );
    let delete_err = delete_resp.expect_err(
        "AC-17-39: a never-signed-in delete must be denied by an auth-required write rule",
    );

    for (op, err) in [("create", &create_err), ("update", &update_err), ("delete", &delete_err)] {
        assert_eq!(
            err.code(),
            tonic::Code::PermissionDenied,
            "AC-17-39: {op} denial must be attributable to the write rule (PermissionDenied), got {:?}",
            err.code()
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-40: a never-signed-in session succeeds against a write rule that
// explicitly allows unauthenticated writes.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trail_guides` has a WRITE rule of `true` (explicitly allows
///          unauthenticated writes)
///   When:  a session that never presented a client-identity token calls
///          `createDoc()` against it
///   Then:  the create succeeds
///
/// AC-17-40
///
/// @driving_port @real-io @US-05 @AC-17-40
#[tokio::test]
async fn a_never_signed_in_session_succeeds_against_a_write_rule_allowing_unauthenticated_writes()
{
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp05-publicwrite").await;
    seed_write_access_rule_full(&ctx, "trail_guides", "true").await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("title".to_string(), string_field("Anonymous Contribution"));

    let resp = create_document(&ctx, "trail_guides", "swp05-publicwrite-doc", fields, None).await;

    assert!(
        resp.is_ok(),
        "AC-17-40: anonymous writes are not blanket-denied — a write rule allowing \
         unauthenticated writes must permit the create: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-41: an invalid (expired) client-identity header on a write call is
// evaluated identically to no header at all — no new rejection class
// introduced (mirrors `security-rules`' own AC-17-13 precedent exactly).
// ─────────────────────────────────────────────────────────────────────────────

/// Reuses this file's own auth-required-rule Given from AC-17-39
/// (`journal_entries` requires `request.auth != null`) — an EXPIRED token
/// (one of the three DISCUSS-named equivalent invalid shapes:
/// malformed/expired/wrong-project) must be evaluated identically to
/// presenting no token at all on a `createDoc()` call, per `client-auth`'s
/// existing ADR-026 DDD-CA-5 semantics, unchanged.
///
/// AC-17-41
///
/// @error @driving_port @real-io @US-05 @AC-17-41
#[tokio::test]
async fn an_invalid_client_identity_header_on_a_write_call_is_evaluated_identically_to_no_header_at_all(
) {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp05-invalidheader").await;
    seed_write_access_rule_full(&ctx, "journal_entries", "request.auth != null").await;

    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    // Expired 1h ago — `attach_client_identity_if_present` attaches nothing
    // for this case (client-auth ADR-026 DDD-CA-5, unchanged).
    let expired_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() - 3600);

    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("nobody"));

    let resp_no_header = create_document(
        &ctx,
        "journal_entries",
        "swp05-invalidheader-doc-a",
        fields.clone(),
        None,
    )
    .await;
    let resp_expired_header = create_document(
        &ctx,
        "journal_entries",
        "swp05-invalidheader-doc-b",
        fields,
        Some(&expired_token),
    )
    .await;

    let err_no_header = resp_no_header
        .expect_err("AC-17-41: a no-header create must be denied by the auth-required write rule");
    let err_expired_header = resp_expired_header.expect_err(
        "AC-17-41: an expired-header create must be denied IDENTICALLY to a no-header create",
    );

    // Anchor assertion (checked BEFORE the cross-comparison below, mirroring
    // sr03's AC-17-13 discipline): without this, two RED-scaffold panics
    // would ALSO compare equal to each other, making the scenario pass
    // vacuously for the wrong reason instead of for AC-17-41's actual claim.
    assert_eq!(
        err_no_header.code(),
        tonic::Code::PermissionDenied,
        "AC-17-41: the no-header create must be denied specifically as PermissionDenied \
         (attributable to the rule), got {:?}",
        err_no_header.code()
    );

    assert_eq!(
        err_no_header.code(),
        err_expired_header.code(),
        "AC-17-41: no new rejection class — an invalid client-identity header on a write call \
         must be denied with the EXACT same gRPC status as no header at all"
    );
    assert_eq!(
        err_no_header.message(),
        err_expired_header.message(),
        "AC-17-41: no new rejection class — identical message too, per ADR-026 DDD-CA-5"
    );
}
