//! RW02 (Slice 02, Walking Skeleton, US-02, Release 1) — A Concrete Path Is
//! Routed to the Single Most-Specific Applicable Pattern (Precedence Across
//! Exact-Match, 4b Fixed-Depth, and This Feature's Own Recursive Wildcard).
//!
//! Acceptance criteria verified here (feature-delta.md US-02, ADR-064,
//! slice-02-route-with-precedence-on-read.md):
//!   AC-17-238: a concrete path with no exact-match/4b match, but whose
//!              leading segments structurally match a stored recursive
//!              -wildcard pattern's own fixed prefix, is routed to it.
//!   AC-17-239: an exact-match rule OR a 4b fixed-depth pattern is ALWAYS
//!              preferred over a co-existing, structurally-overlapping
//!              recursive-wildcard pattern — the recursive pattern is never
//!              even evaluated when a more specific rule governs.
//!   AC-17-240: when two recursive-wildcard patterns structurally reach the
//!              same concrete path, the one with the longer/deeper fixed
//!              prefix governs.
//!   AC-17-241: a signed-in end user whose identity satisfies a
//!              recursive-wildcard-governed condition succeeds (folded into
//!              the AC-17-238/240 scenarios below).
//!   AC-17-242: a user whose identity does NOT satisfy a recursive-wildcard
//!              -governed condition is denied PermissionDenied.
//!   AC-17-243: a concrete path matching NO pattern of any kind falls
//!              through to pre-existing unrestricted behavior, unaffected.
//!   AC-17-244: a denied read's response is identical whether or not the
//!              target document exists (AC-17-10 existence non-leakage,
//!              reused unchanged).
//!
//! Setup uses the REAL admin import endpoint (Slice 01) — not a hand-seeded
//! `access_rule_patterns` row — proving import and routing/precedence
//! compose end-to-end, mirroring `security-rules-cel-path-matching`'s own
//! pm02 discipline exactly.
//!
//! Driving port: admin HTTP :9090 (import) + gRPC :8080 `GetDocument`, via
//! `SecurityRulesFullContext` — the same `embyr_server::start_test_server`
//! composition root pm02 uses (Pillar 3).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

/// AC-17-239: a 4b fixed-depth pattern (owner-only on `journal_entries`)
/// co-exists with a broader recursive-wildcard catch-all
/// (`expeditions/{expeditionId}/{path=**}`, any signed-in user) that
/// structurally reaches the SAME `journal_entries` documents too — proving
/// the 4b pattern always wins where both structurally apply.
const PRECEDENCE_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId}/journal_entries/{entryId} {
      allow read, write: if request.auth.uid == resource.data.owner_id;
    }
    match /expeditions/{expeditionId}/{path=**} {
      allow read: if request.auth.uid != "";
    }
  }
}
"#;

/// AC-17-240/242: two recursive-wildcard patterns at different depths — a
/// project-wide catch-all (deny everything) and a narrower
/// `expeditions/{expeditionId}/{path=**}` catch-all (allow any signed-in
/// user) — proving the longer/deeper prefix wins wherever both reach.
const DEPTH_PRECEDENCE_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /{document=**} {
      allow read, write: if false;
    }
    match /expeditions/{expeditionId}/{path=**} {
      allow read: if request.auth.uid != "";
    }
  }
}
"#;

async fn import(ctx: &SecurityRulesFullContext, cookie: &str, rules_file: &str) {
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/import",
            ctx.project_id
        )))
        .header("Cookie", cookie)
        .json(&serde_json::json!({ "rules_file": rules_file }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(resp.status().as_u16(), 200, "setup: rules-file import must succeed");
}

fn resource_name(project_id: &str, path: &str) -> String {
    format!("projects/{project_id}/databases/(default)/documents/{path}")
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-238/241 (WALKING SKELETON): a path with no exact-match/4b pattern,
// governed exclusively by a recursive-wildcard pattern, routes to it — a
// signed-in user satisfying the condition succeeds.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `expeditions/{expeditionId}/{path=**}` allows any signed-in user
///          to read
///   And:   `expeditions/trek-2026/photos/img-042` has NO exact-match rule
///          and NO 4b pattern (only `journal_entries` has a 4b pattern)
///   When:  Maria (signed in) calls `getDoc()` on that photo, and separately
///          an anonymous caller calls `getDoc()` on the SAME photo
///   Then:  Maria's read succeeds (routed to the recursive-wildcard pattern,
///          condition satisfied) and the anonymous read is denied (the SAME
///          pattern's own condition requires a signed-in caller) — the
///          denial half is what proves this is real routed evaluation, not
///          an accidental "no rule found" fallthrough allow (which would
///          have let the anonymous caller through too)
///
/// AC-17-238, AC-17-241
///
/// @walking_skeleton @driving_port @real-io @US-02 @AC-17-238 @AC-17-241
#[tokio::test]
async fn a_path_governed_only_by_a_recursive_wildcard_routes_to_it_and_a_satisfying_user_succeeds() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rw02-catchall").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    import(&ctx, &cookie, PRECEDENCE_RULES_FILE).await;

    ctx.seed_document("expeditions/trek-2026/photos", "img-042", serde_json::json!({}))
        .await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    let resp = ctx
        .get_document(
            &resource_name(&ctx.project_id, "expeditions/trek-2026/photos/img-042"),
            Some(&marias_token),
        )
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-238/241: a path governed only by the recursive-wildcard pattern must route to it \
         and succeed for a satisfying signed-in user: {:?}",
        resp.err()
    );

    let anon_resp = ctx
        .get_document(
            &resource_name(&ctx.project_id, "expeditions/trek-2026/photos/img-042"),
            None,
        )
        .await;
    let err = anon_resp.expect_err(
        "AC-17-238: an anonymous caller must be denied by the SAME recursive-wildcard pattern's \
         own condition — proving real routed evaluation, not an accidental unrestricted fallthrough",
    );
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-239: a 4b fixed-depth pattern always wins over a structurally
// -overlapping recursive-wildcard pattern — the recursive pattern's own more
// permissive condition is never even consulted.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has BOTH a 4b owner-only pattern AND a broader
///          recursive-wildcard catch-all that would allow any signed-in user
///   When:  Dana (signed in, NOT the owner) reads Maria's journal entry
///   Then:  the read is denied — the 4b pattern's owner-only condition
///          governs, proving the recursive pattern was never consulted (it
///          would have allowed Dana, since she IS signed in)
///
/// AC-17-239
///
/// @error @driving_port @real-io @US-02 @AC-17-239
#[tokio::test]
async fn a_4b_fixed_depth_pattern_always_wins_over_a_structurally_overlapping_recursive_wildcard() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rw02-4bwins").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    import(&ctx, &cookie, PRECEDENCE_RULES_FILE).await;

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
            &resource_name(&ctx.project_id, "expeditions/trek-2026/journal_entries/entry-042"),
            Some(&danas_token),
        )
        .await;

    let err = resp.expect_err(
        "AC-17-239: the 4b owner-only pattern must govern — Dana is signed in (which the \
         overlapping recursive catch-all alone would allow) but is NOT the owner",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-239: denial must come from the 4b pattern's own owner-only condition, got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-240/242: among two structurally-overlapping recursive-wildcard
// patterns, the longer/deeper fixed prefix wins; a non-satisfying user is
// denied under the winning (deeper) pattern's own condition.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: a project-wide `{document=**}` catch-all denies everything, AND a
///          narrower `expeditions/{expeditionId}/{path=**}` catch-all allows
///          any signed-in user — both structurally reach the SAME photo path
///   When:  Maria (signed in) reads `expeditions/trek-2026/photos/img-1`
///   Then:  the read succeeds — the DEEPER `expeditions/{expeditionId}/
///          {path=**}` prefix wins over the shallower project-wide one
///
/// AC-17-240, AC-17-241
///
/// @driving_port @real-io @US-02 @AC-17-240 @AC-17-241
#[tokio::test]
async fn among_two_recursive_wildcards_the_longer_deeper_prefix_wins() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rw02-depth").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    import(&ctx, &cookie, DEPTH_PRECEDENCE_RULES_FILE).await;

    ctx.seed_document("expeditions/trek-2026/photos", "img-1", serde_json::json!({}))
        .await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    let resp = ctx
        .get_document(
            &resource_name(&ctx.project_id, "expeditions/trek-2026/photos/img-1"),
            Some(&marias_token),
        )
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-240: the deeper expeditions-scoped recursive wildcard must win over the \
         shallower project-wide catch-all: {:?}",
        resp.err()
    );
}

/// Journey:
///   Given: the SAME depth-precedence setup as above
///   When:  an ANONYMOUS caller (no client-identity token) reads the same
///          photo, governed by the winning `expeditions/{expeditionId}/
///          {path=**}` pattern's own `request.auth.uid != ""` condition
///   Then:  the read is denied — anonymous does not satisfy the winning
///          (deeper) pattern's own condition
///
/// AC-17-242
///
/// @error @driving_port @real-io @US-02 @AC-17-242
#[tokio::test]
async fn a_user_not_satisfying_the_winning_recursive_wildcards_condition_is_denied() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rw02-denied").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    import(&ctx, &cookie, DEPTH_PRECEDENCE_RULES_FILE).await;

    ctx.seed_document("expeditions/trek-2026/photos", "img-1", serde_json::json!({}))
        .await;

    let resp = ctx
        .get_document(
            &resource_name(&ctx.project_id, "expeditions/trek-2026/photos/img-1"),
            None,
        )
        .await;

    let err = resp.expect_err(
        "AC-17-242: an anonymous caller must be denied by the winning recursive-wildcard \
         pattern's own condition",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-242: must be PermissionDenied, got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-243: a concrete path matching NO pattern of any kind (exact-match,
// 4b, or recursive-wildcard) falls through to pre-existing unrestricted
// behavior, unaffected.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-243
///
/// @error @driving_port @real-io @US-02 @AC-17-243 @security-regression
#[tokio::test]
async fn a_path_matching_no_pattern_of_any_kind_falls_through_to_pre_existing_behavior() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rw02-fallthrough").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    // Only the expeditions-scoped patterns are imported — `trail_guides` has
    // NEITHER an exact-match rule NOR any 4b/recursive pattern anywhere.
    import(&ctx, &cookie, PRECEDENCE_RULES_FILE).await;

    ctx.seed_document("trail_guides", "guide-1", serde_json::json!({})).await;

    let resp = ctx
        .get_document(&resource_name(&ctx.project_id, "trail_guides/guide-1"), None)
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-243: a collection matching no pattern of any kind must remain unrestricted: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-244: existence non-leakage — a denied recursively-routed read never
// reveals whether the target document exists (AC-17-10, reused unchanged).
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-244
///
/// @error @driving_port @real-io @US-02 @AC-17-244 @security-regression
#[tokio::test]
async fn a_denied_recursively_routed_read_never_reveals_whether_the_target_document_exists() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rw02-existencenonleak").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    import(&ctx, &cookie, DEPTH_PRECEDENCE_RULES_FILE).await;

    ctx.seed_document("expeditions/trek-2026/photos", "img-1", serde_json::json!({}))
        .await;
    // expeditions/trek-2026/photos/does-not-exist is deliberately NEVER seeded.

    let resp_existing = ctx
        .get_document(
            &resource_name(&ctx.project_id, "expeditions/trek-2026/photos/img-1"),
            None,
        )
        .await;
    let resp_nonexistent = ctx
        .get_document(
            &resource_name(&ctx.project_id, "expeditions/trek-2026/photos/does-not-exist"),
            None,
        )
        .await;

    let err_existing =
        resp_existing.expect_err("AC-17-244: an anonymous read of an EXISTING routed document must be denied");
    let err_nonexistent = resp_nonexistent
        .expect_err("AC-17-244: an anonymous read of a NON-EXISTENT routed document must ALSO be denied");

    assert_eq!(
        err_existing.code(),
        err_nonexistent.code(),
        "AC-17-244: existence non-leakage — both denials must carry the IDENTICAL gRPC status code"
    );
    assert_eq!(
        err_existing.code(),
        tonic::Code::PermissionDenied,
        "AC-17-244: must be PermissionDenied, never NotFound (that would leak non-existence)"
    );
    assert_eq!(
        err_existing.message(),
        err_nonexistent.message(),
        "AC-17-244: existence non-leakage — both denials must carry the IDENTICAL message"
    );
}
