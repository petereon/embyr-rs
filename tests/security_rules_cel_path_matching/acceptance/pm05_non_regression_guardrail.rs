//! PM05 (Slice 05, US-05, Release 1) — Untouched Patterns, 4a's Own
//! Imports, and the Full Regression Baseline Are Unaffected.
//!
//! Acceptance criteria verified here (feature-delta.md US-05,
//! slice-05-non-regression-guardrail.md):
//!   AC-17-224: the full pre-existing regression baseline passes unmodified
//!              — proven by re-running every `security_rules_*` cargo test
//!              target, not by anything in this file (this file proves the
//!              remaining 3 targeted, in-process guarantees).
//!   AC-17-225: 4a's own single-wildcard imported pattern
//!              (`profiles/{userId}`-shaped) is unaffected by this feature's
//!              multi-segment routing mechanism coexisting alongside it in
//!              the SAME project.
//!   AC-17-226: the original zero-wildcard exact-path rule shape (a rule for
//!              a literal collection with no wildcard at all) is unaffected
//!              by this feature's routing mechanism — including the
//!              adversarial case where the exact-path rule's own collection
//!              name (`journal_entries`) is textually IDENTICAL to a
//!              multi-segment pattern's own leaf collection name, proving
//!              the two storage/routing mechanisms never cross-talk on name
//!              alone.
//!   AC-17-227: a collection with no rule or pattern of any kind (never
//!              touched by 4a or 4b) retains fully unrestricted behavior,
//!              even with both mechanisms active in the same project.
//!
//! Single shared setup imports all 3 shapes (4a single-wildcard, the
//! original zero-wildcard exact-path shape, and this feature's own
//! multi-segment pattern) into ONE project in ONE import call — proving
//! real coexistence, not 3 isolated single-mechanism projects. Reuses
//! `SecurityRulesFullContext`'s own pre-seeded `journal_entries`/
//! `trail_guides` documents (mirrors `security_rules`'s own sr04 guardrail
//! discipline) rather than seeding new ones.
//!
//! Driving port: admin HTTP :9090 (import) + gRPC :8080 `GetDocument`, via
//! `SecurityRulesFullContext` — the same composition root every prior slice
//! in this feature uses.
//!
//! Error ratio: N/A — this slice's own scope is entirely a non-regression
//! proof obligation (guardrail-only, mirrors `security_rules`'s own sr04 and
//! `security-rules-cel-parity`'s own cp05 precedent).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

/// A single rules file importing all 3 pre-existing/new shapes at once:
///   - `profiles/{userId}` — 4a's own single-wildcard shape (AC-17-225)
///   - `journal_entries` — the original zero-wildcard exact-path shape
///     (AC-17-226), sharing its literal collection name with the pattern
///     below's own leaf, on purpose
///   - `expeditions/{expeditionId}/journal_entries/{entryId}` — this
///     feature's own multi-segment pattern
/// `trail_guides` and `app_config` (both pre-seeded by
/// `SecurityRulesFullContext::new`) are deliberately named in NO block here
/// — proving AC-17-227 for a genuinely untouched collection.
const COEXISTENCE_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /profiles/{userId} {
      allow read, write: if request.auth.uid == userId;
    }
    match /journal_entries {
      allow read: if request.auth.uid == resource.data.owner_id;
    }
    match /expeditions/{expeditionId}/journal_entries/{entryId} {
      allow read, write: if request.auth.uid == resource.data.owner_id;
    }
  }
}
"#;

async fn setup(project_id: &str) -> (SecurityRulesFullContext, SigningKey) {
    let ctx = SecurityRulesFullContext::new(project_id).await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{project_id}/access_rules/import")))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": COEXISTENCE_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "setup: a file mixing all 3 shapes must import cleanly"
    );

    (ctx, signing_key)
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-225: 4a's own single-wildcard `profiles/{userId}` pattern is
// unaffected by this feature's multi-segment pattern coexisting alongside
// it in the same project.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `profiles/{userId}` (4a) and
///          `expeditions/{expeditionId}/journal_entries/{entryId}` (4b) are
///          BOTH imported and active in the same project
///   When:  Maria reads her own `profiles/maria-santos` document
///   And:   Dana (a different signed-in user) reads that SAME document
///   Then:  Maria's read succeeds and Dana's is denied — exactly 4a's own
///          single-wildcard behavior, unperturbed by 4b's own mechanism
///          coexisting
///
/// AC-17-225
///
/// @driving_port @real-io @US-05 @AC-17-225 @security-regression
#[tokio::test]
async fn a_4a_single_wildcard_pattern_is_unaffected_by_coexisting_multi_segment_routing() {
    let (ctx, signing_key) = setup("trailmark-prod-pm05-4a-coexist").await;
    ctx.seed_document("profiles", "maria-santos", serde_json::json!({})).await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let profile_resource = format!(
        "projects/{}/databases/(default)/documents/profiles/maria-santos",
        ctx.project_id
    );

    let owner_resp = ctx.get_document(&profile_resource, Some(&marias_token)).await;
    assert!(
        owner_resp.is_ok(),
        "AC-17-225: Maria's read of her own profiles/{{userId}} document must still succeed: {:?}",
        owner_resp.err()
    );

    let nonowner_resp = ctx.get_document(&profile_resource, Some(&danas_token)).await;
    let err = nonowner_resp.expect_err("AC-17-225: Dana's read of Maria's profile must still be denied");
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-225: denial must still be attributable to 4a's own rule, got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-226: the original zero-wildcard exact-path rule shape is unaffected
// by this feature's routing mechanism, including when it shares a literal
// collection name with a multi-segment pattern's own leaf.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` (zero-wildcard, exact-path, 4a's own
///          predecessor shape) has an active rule, AND
///          `expeditions/{expeditionId}/journal_entries/{entryId}` (this
///          feature) is ALSO active, sharing the literal name
///          `journal_entries` at a structurally different depth
///   When:  Maria reads her own TOP-LEVEL `journal_entries/{doc}` document
///          (pre-seeded by `SecurityRulesFullContext::new`, never nested
///          under `expeditions`)
///   Then:  the exact-path rule governs it correctly (Maria's own document
///          succeeds, a document with no matching owner is denied) — the
///          multi-segment pattern engine never intercepts a top-level
///          `journal_entries` request merely because the names match
///
/// AC-17-226
///
/// @driving_port @real-io @US-05 @AC-17-226 @security-regression
#[tokio::test]
async fn a_zero_wildcard_exact_path_rule_is_unaffected_despite_sharing_a_literal_name_with_a_pattern_leaf() {
    let (ctx, signing_key) = setup("trailmark-prod-pm05-zero-wildcard").await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    // Pre-seeded by SecurityRulesFullContext::new: journal_entries/{project_id}-maria-doc
    // (owner_id = "maria-santos") and journal_entries/{project_id}-no-owner-doc.
    let owned_resource = format!(
        "projects/{}/databases/(default)/documents/journal_entries/{}-maria-doc",
        ctx.project_id, ctx.project_id
    );
    let unowned_resource = format!(
        "projects/{}/databases/(default)/documents/journal_entries/{}-no-owner-doc",
        ctx.project_id, ctx.project_id
    );

    let owner_resp = ctx.get_document(&owned_resource, Some(&marias_token)).await;
    assert!(
        owner_resp.is_ok(),
        "AC-17-226: Maria's read of her own top-level journal_entries document must succeed: {:?}",
        owner_resp.err()
    );

    let nonowner_resp = ctx.get_document(&unowned_resource, Some(&marias_token)).await;
    let err = nonowner_resp
        .expect_err("AC-17-226: Maria's read of a journal_entries document she does not own must be denied");
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-226: denial must still be attributable to the zero-wildcard exact-path rule, got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-227: a collection with no rule or pattern of any kind remains fully
// unrestricted, even with both 4a's and 4b's mechanisms active in the same
// project.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trail_guides` has NEITHER an exact-match rule NOR any
///          multi-segment pattern defined anywhere in this project — while
///          `profiles/{userId}`, `journal_entries`, and
///          `expeditions/{expeditionId}/journal_entries/{entryId}` are ALL
///          active for OTHER collections
///   When:  an anonymous caller reads the pre-seeded
///          `trail_guides/{project_id}-guide-doc` document
///   Then:  the read succeeds unrestricted — neither routing mechanism
///          reaches for a collection neither one governs
///
/// AC-17-227
///
/// @driving_port @real-io @US-05 @AC-17-227 @security-regression
#[tokio::test]
async fn a_collection_with_no_rule_or_pattern_of_any_kind_remains_fully_unrestricted() {
    let (ctx, _signing_key) = setup("trailmark-prod-pm05-no-rule").await;

    let guide_resource = format!(
        "projects/{}/databases/(default)/documents/trail_guides/{}-guide-doc",
        ctx.project_id, ctx.project_id
    );

    let resp = ctx.get_document(&guide_resource, None).await;

    assert!(
        resp.is_ok(),
        "AC-17-227: a collection with no rule/pattern of any kind must remain unrestricted \
         even with both 4a's and 4b's routing mechanisms active in the same project: {:?}",
        resp.err()
    );
}
