//! PM02 (Slice 02, Walking Skeleton — Activity B, US-02, Release 1) — A
//! Concrete Document Path Is Deterministically Routed to Its Matching
//! Pattern on Reads.
//!
//! Acceptance criteria verified here (feature-delta.md US-02, ADR-063,
//! slice-02-route-and-evaluate-on-read.md):
//!   AC-17-207: a concrete document path structurally matching an imported
//!              multi-segment pattern is routed to that pattern, with every
//!              wildcard segment bound to its own concrete value from the
//!              request's own path.
//!   AC-17-208: a signed-in end user reading a routed document whose bound
//!              variables satisfy the pattern's condition succeeds.
//!   AC-17-209: a signed-in end user reading a routed document whose bound
//!              variables do NOT satisfy the condition is denied
//!              PermissionDenied.
//!   AC-17-210: two concrete paths matching the SAME pattern shape at
//!              different wildcard values (two different expeditions)
//!              resolve to two completely independent bindings — no
//!              cross-path leakage.
//!   AC-17-211: a concrete path matching NO stored pattern of any kind falls
//!              through to whatever pre-existing behavior already governs
//!              it, unaffected (US-05-style zero-regression guardrail,
//!              proven directly by this slice too).
//!   AC-17-212: a denied read's response is identical whether or not the
//!              target document actually exists, reusing the existing
//!              existence-non-leakage mechanism (AC-17-10) unchanged.
//!
//! Setup uses the REAL admin import endpoint from Slice 01
//! (`POST .../access_rules/import`) — not a hand-seeded `access_rule_patterns`
//! row — proving Slice 01's import and Slice 02's routing/evaluation compose
//! end-to-end, mirroring `security-rules-cel-parity`'s own cp02 discipline.
//!
//! Driving port: admin HTTP :9090 (import) + gRPC :8080 `GetDocument`, via
//! `SecurityRulesFullContext` — the same `embyr_server::start_test_server`
//! composition root cp02/sr02 both use (Pillar 3).
//!
//! Error ratio: 4 error/edge (AC-17-209/210/211/212) out of 5 = 80%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

const EXPEDITIONS_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId}/journal_entries/{entryId} {
      allow read, write: if request.auth.uid == resource.data.owner_id;
    }
  }
}
"#;

/// Shared setup: a fresh `SecurityRulesFullContext`, the multi-segment
/// `expeditions/{expeditionId}/journal_entries/{entryId}` pattern imported
/// via Slice 01's real admin endpoint, and a signing key registered for
/// client-identity token verification.
async fn setup(project_id: &str) -> (SecurityRulesFullContext, SigningKey) {
    let ctx = SecurityRulesFullContext::new(project_id).await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{project_id}/access_rules/import")))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EXPEDITIONS_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(resp.status().as_u16(), 200, "setup: expeditions pattern import must succeed");

    (ctx, signing_key)
}

fn journal_entry_resource_name(project_id: &str, expedition_id: &str, entry_id: &str) -> String {
    format!(
        "projects/{project_id}/databases/(default)/documents/expeditions/{expedition_id}/journal_entries/{entry_id}"
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-207/208 (WALKING SKELETON — Activity B): the routed pattern's
// captured `expeditionId` correctly gates the document's own owner.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `expeditions/{expeditionId}/journal_entries/{entryId}` has an
///          imported pattern requiring `request.auth.uid ==
///          resource.data.owner_id`
///   And:   Maria Santos owns `expeditions/trek-2026/journal_entries/entry-042`
///   When:  Maria calls `getDoc()` on that concrete, routed document
///   Then:  the read succeeds and returns the document
///
/// AC-17-207, AC-17-208
///
/// @walking_skeleton @driving_port @real-io @US-02 @AC-17-207 @AC-17-208
#[tokio::test]
async fn a_routed_documents_owner_reads_their_own_document_successfully() {
    let (ctx, signing_key) = setup("trailmark-prod-pm02-owner").await;
    ctx.seed_document(
        "expeditions/trek-2026/journal_entries",
        "entry-042",
        serde_json::json!({ "owner_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    let resp = ctx
        .get_document(
            &journal_entry_resource_name(&ctx.project_id, "trek-2026", "entry-042"),
            Some(&marias_token),
        )
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-207/208: Maria's read of her own routed document must succeed: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-209: a non-owner's read of the same routed document is denied
// (error/edge).
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-209
///
/// @error @driving_port @real-io @US-02 @AC-17-209
#[tokio::test]
async fn a_non_owners_read_of_the_same_routed_document_is_denied() {
    let (ctx, signing_key) = setup("trailmark-prod-pm02-nonowner").await;
    ctx.seed_document(
        "expeditions/trek-2026/journal_entries",
        "entry-042",
        serde_json::json!({ "owner_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let resp = ctx
        .get_document(
            &journal_entry_resource_name(&ctx.project_id, "trek-2026", "entry-042"),
            Some(&danas_token),
        )
        .await;

    let err = resp.expect_err("AC-17-209: Dana's read of Maria's routed document must be denied");
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-209: denial must be attributable to the routed pattern's rule, got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-210: two concrete paths matching the SAME pattern shape at
// different wildcard values resolve to fully independent bindings — no
// cross-expedition leakage in either direction.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: the SAME pattern also governs `expeditions/coastal-explorer-2026/
///          journal_entries/entry-777`, owned by Dana, not Maria
///   When:  Maria reads her OWN `trek-2026` entry, then Maria reads Dana's
///          `coastal-explorer-2026` entry
///   Then:  Maria's own read succeeds; Maria's read of Dana's independent
///          expedition is denied — the `expeditionId="trek-2026"` binding
///          never leaks into the `coastal-explorer-2026` evaluation
///
/// AC-17-210
///
/// @error @driving_port @real-io @US-02 @AC-17-210
#[tokio::test]
async fn two_concrete_paths_matching_the_same_pattern_shape_resolve_independently() {
    let (ctx, signing_key) = setup("trailmark-prod-pm02-nonleak").await;
    ctx.seed_document(
        "expeditions/trek-2026/journal_entries",
        "entry-042",
        serde_json::json!({ "owner_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;
    ctx.seed_document(
        "expeditions/coastal-explorer-2026/journal_entries",
        "entry-777",
        serde_json::json!({ "owner_id": {"t": "S", "v": "dana-kim"} }),
    )
    .await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    let own_resp = ctx
        .get_document(
            &journal_entry_resource_name(&ctx.project_id, "trek-2026", "entry-042"),
            Some(&marias_token),
        )
        .await;
    assert!(own_resp.is_ok(), "AC-17-210 control: Maria's own trek-2026 entry must still succeed");

    let other_expedition_resp = ctx
        .get_document(
            &journal_entry_resource_name(&ctx.project_id, "coastal-explorer-2026", "entry-777"),
            Some(&marias_token),
        )
        .await;
    let err = other_expedition_resp.expect_err(
        "AC-17-210: Maria's read of Dana's independent coastal-explorer-2026 entry must be denied — \
         the trek-2026 binding must never leak across expeditions",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-210: cross-expedition read must be denied, got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-211: a concrete path matching NO stored pattern of any kind falls
// through to the pre-existing unrestricted behavior, unaffected.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-211
///
/// @error @driving_port @real-io @US-02 @AC-17-211 @security-regression
#[tokio::test]
async fn a_path_matching_no_stored_pattern_falls_through_to_pre_existing_behavior() {
    let (ctx, _signing_key) = setup("trailmark-prod-pm02-fallthrough").await;
    // `trail_guides` has NEITHER an exact-match rule NOR any multi-segment
    // pattern defined anywhere in this project — the routing lookup must
    // return `None` and this read must proceed exactly as it would have
    // before this feature (US-05-style zero-regression guardrail).
    ctx.seed_document("trail_guides", "guide-1", serde_json::json!({})).await;

    let resp = ctx
        .get_document(
            &format!(
                "projects/{}/databases/(default)/documents/trail_guides/guide-1",
                ctx.project_id
            ),
            None,
        )
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-211: a collection with no rule/pattern of any kind must remain unrestricted: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-212: existence non-leakage — a denied routed read never reveals
// whether the target document exists, reusing AC-17-10's mechanism
// unchanged.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-212
///
/// @error @driving_port @real-io @US-02 @AC-17-212 @security-regression
#[tokio::test]
async fn a_denied_routed_read_never_reveals_whether_the_target_document_exists() {
    let (ctx, signing_key) = setup("trailmark-prod-pm02-existencenonleak").await;
    ctx.seed_document(
        "expeditions/trek-2026/journal_entries",
        "entry-042",
        serde_json::json!({ "owner_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;
    // expeditions/trek-2026/journal_entries/does-not-exist is deliberately
    // NEVER seeded.

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let resp_existing_wrong_owner = ctx
        .get_document(
            &journal_entry_resource_name(&ctx.project_id, "trek-2026", "entry-042"),
            Some(&danas_token),
        )
        .await;
    let resp_nonexistent = ctx
        .get_document(
            &journal_entry_resource_name(&ctx.project_id, "trek-2026", "does-not-exist"),
            Some(&danas_token),
        )
        .await;

    let err_existing = resp_existing_wrong_owner
        .expect_err("AC-17-212: wrong-owner read of an EXISTING routed document must be denied");
    let err_nonexistent = resp_nonexistent
        .expect_err("AC-17-212: read of a NON-EXISTENT routed document must ALSO be denied");

    assert_eq!(
        err_existing.code(),
        err_nonexistent.code(),
        "AC-17-212: existence non-leakage — both denials must carry the IDENTICAL gRPC status code"
    );
    assert_eq!(
        err_existing.code(),
        tonic::Code::PermissionDenied,
        "AC-17-212: must be PermissionDenied, never NotFound (that would leak non-existence)"
    );
    assert_eq!(
        err_existing.message(),
        err_nonexistent.message(),
        "AC-17-212: existence non-leakage — both denials must carry the IDENTICAL message"
    );
}
