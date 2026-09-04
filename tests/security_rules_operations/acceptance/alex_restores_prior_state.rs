//! Slice 03 — Alex Restores a Rule to a Prior State, and the Restore Is
//! Itself Remembered (US-03, ADR-035 § Restore).
//!
//! Pure proof-obligation slice (feature-delta.md § Elephant Carpaccio
//! Slices, Slice 03: "zero new components, pure proof obligation"; ADR-035
//! § Restore: "zero new endpoint, zero new mechanism — confirmed, not
//! assumed"). "Restore" is calling the EXISTING `POST .../access_rules`
//! (`define_access_rule`, unmodified since `security-rules`' own Slice 01)
//! again with a condition value read from Slice 02's own history-retrieval
//! mechanism. No new production code is expected by this file.
//!
//! Acceptance criteria verified here (feature-delta.md US-03):
//!   AC-17-164: redefining with a prior history entry's exact text restores
//!              identical evaluated behavior — proven with a REAL
//!              `GetDocument` call (Pillar 3), not an inference from stored
//!              text.
//!   AC-17-165: the restore call itself produces a new history entry via
//!              Slice 01's own unmodified capture mechanism — correctly
//!              attributed, newest-first ordering preserved, 3 entries
//!              total (original A, B, restored-A).
//!   AC-17-166: restoring the ALREADY-CURRENT condition still produces a
//!              new history entry — no special-cased no-op, no
//!              deduplication.
//!   AC-17-167: an invalid restore condition is rejected via the EXISTING
//!              `SYNTAX_ERROR`/`UNSUPPORTED_CONSTRUCT` taxonomy
//!              `define_access_rule` already uses — no new error class.
//!
//! Driving ports: gRPC :8080 `GetDocument` (AC-17-164, via
//! `SecurityRulesFullContext` — the same real composition root
//! `security_rules_sr02_*` uses) and Admin HTTP :9090 `POST .../access_rules`
//! + `GET .../access_rules/:collection_path/history` (AC-17-165/166/167, via
//! `SecurityRulesAdminContext` — Slice 01/02's own fixture).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesAdminContext, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

/// Calls the EXISTING `define_access_rule` admin action against
/// `journal_entries` on `ctx.project_id` — used for the initial definition,
/// the redefinition, AND the restore (all three ARE the identical action,
/// per ADR-035 § Restore).
async fn define_journal_entries_rule(ctx: &SecurityRulesFullContext, cookie: &str, condition: &str) {
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules",
            ctx.project_id
        )))
        .header("Cookie", cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": condition,
        }))
        .send()
        .await
        .expect("define request failed");
    assert_eq!(resp.status().as_u16(), 200, "define/redefine/restore must succeed");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-164: restoring a prior condition's exact text restores identical
// evaluated behavior — real domain proof via GetDocument
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (mirrors the feature's own Journey — Change-and-recover flow
/// delta):
///   Given: `journal_entries` on `trailmark-prod` is defined with condition
///          A (ownership-only), then redefined to condition B
///          (auth-required, no ownership check) — Dana (non-owner) is
///          denied under A, allowed under B
///   When:  Alex restores the rule using condition A's exact text (the
///          SAME `define_access_rule` action, called again)
///   Then:  a real `GetDocument` call by Dana is denied again — identical
///          evaluated behavior to condition A restored
///
/// AC-17-164
///
/// @walking_skeleton @driving_port @real-io @US-03 @AC-17-164
#[tokio::test]
async fn restoring_a_prior_conditions_exact_text_restores_identical_evaluated_behavior() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sro03-restore-behavior").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let condition_a = "request.auth.uid == resource.data.owner_id";
    let condition_b = "request.auth != null";

    // Condition A active: Dana (non-owner) is denied.
    define_journal_entries_rule(&ctx, &cookie, condition_a).await;
    let under_a = ctx
        .get_document(
            &ctx.document_resource_name("journal_entries", "maria-doc"),
            Some(&danas_token),
        )
        .await;
    assert!(
        under_a.is_err(),
        "precondition: under condition A, Dana (non-owner) must be denied"
    );

    // Redefine to condition B: Dana is now allowed.
    define_journal_entries_rule(&ctx, &cookie, condition_b).await;
    let under_b = ctx
        .get_document(
            &ctx.document_resource_name("journal_entries", "maria-doc"),
            Some(&danas_token),
        )
        .await;
    assert!(
        under_b.is_ok(),
        "precondition: under redefined condition B, Dana must be allowed: {:?}",
        under_b.err()
    );

    // Restore: redefine with condition A's EXACT text.
    define_journal_entries_rule(&ctx, &cookie, condition_a).await;
    let restored = ctx
        .get_document(
            &ctx.document_resource_name("journal_entries", "maria-doc"),
            Some(&danas_token),
        )
        .await;
    let err = restored.expect_err(
        "AC-17-164: after restoring condition A's exact text, Dana must be denied again",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-164: restored behavior must be identical to condition A's original evaluated \
         behavior (PermissionDenied), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-165: the restore itself produces a new, correctly-attributed
// history entry, newest-first, 3 entries total
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` is defined with condition A, then redefined
///          to condition B
///   When:  Alex restores the rule using condition A's exact text
///   Then:  retrieving history (Slice 02's own mechanism, unmodified)
///          returns 3 entries, newest first: the restore (condition A,
///          attributed to the acting admin), then B, then the original A
///
/// AC-17-165
///
/// @driving_port @real-io @US-03 @AC-17-165
#[tokio::test]
async fn the_restore_itself_produces_a_new_correctly_attributed_history_entry() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let condition_a = "request.auth.uid == resource.data.owner_id";
    let condition_b = "request.auth != null";

    for condition in [condition_a, condition_b, condition_a] {
        let resp = ctx
            .client
            .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
            .header("Cookie", &cookie)
            .json(&serde_json::json!({
                "collection_path": "journal_entries",
                "condition": condition,
            }))
            .send()
            .await
            .expect("define/redefine/restore request failed");
        assert_eq!(resp.status().as_u16(), 200);
    }

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/journal_entries/history"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("history request failed");
    assert_eq!(resp.status().as_u16(), 200);

    let body: serde_json::Value = resp.json().await.expect("parse history response");
    let history = body["history"].as_array().expect("history must be an array");
    assert_eq!(
        history.len(),
        3,
        "AC-17-165: restore must produce a THIRD history entry alongside the original A and B \
         entries, got {}",
        history.len()
    );
    assert_eq!(
        history[0]["condition"], condition_a,
        "AC-17-165: the restore's own new entry (condition A) must be newest, so first"
    );
    assert_eq!(
        history[0]["actor_account_id"],
        ctx.account_id.to_string(),
        "AC-17-165: the restore's history entry must be attributed to the acting admin"
    );
    assert_eq!(
        history[1]["condition"], condition_b,
        "AC-17-165: the intermediate redefinition (condition B) must be second"
    );
    assert_eq!(
        history[2]["condition"], condition_a,
        "AC-17-165: the original definition (condition A) must remain oldest, so last — \
         untouched by the restore"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-166: restoring the already-current condition still produces a new
// entry — no special-cased no-op
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-166
///
/// @driving_port @real-io @US-03 @AC-17-166
#[tokio::test]
async fn restoring_the_already_current_condition_still_produces_a_new_history_entry() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let condition = "request.auth != null";

    // Define, then "restore" the exact same, already-current condition.
    for _ in 0..2 {
        let resp = ctx
            .client
            .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
            .header("Cookie", &cookie)
            .json(&serde_json::json!({
                "collection_path": "journal_entries",
                "condition": condition,
            }))
            .send()
            .await
            .expect("define/restore request failed");
        assert_eq!(resp.status().as_u16(), 200);
    }

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/journal_entries/history"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("history request failed");
    let body: serde_json::Value = resp.json().await.expect("parse history response");
    let history = body["history"].as_array().expect("history must be an array");
    assert_eq!(
        history.len(),
        2,
        "AC-17-166: a no-op restore (identical condition text) must still capture a new \
         history entry — no deduplication, got {}",
        history.len()
    );
    assert!(
        history.iter().all(|entry| entry["condition"] == condition),
        "AC-17-166: both entries must carry the identical condition text"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-167: an invalid restore condition uses the existing
// SYNTAX_ERROR/UNSUPPORTED_CONSTRUCT taxonomy — no new error class
// ─────────────────────────────────────────────────────────────────────────────

/// Input variations of the same behavior (Mandate 5): both a plain syntax
/// error and an out-of-v1-scope construct, attempted as a "restore" against
/// a collection with a real, already-defined rule — each rejected via the
/// EXISTING taxonomy `define_access_rule` already uses for a first-time
/// definition (`sr01`'s own AC-17-03/AC-17-04).
///
/// AC-17-167
///
/// @error @driving_port @real-io @US-03 @AC-17-167
#[tokio::test]
async fn an_invalid_restore_condition_is_rejected_via_the_existing_taxonomy() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;
    ctx.seed_access_rule(
        "trailmark-prod",
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    for (invalid_condition, expected_reason) in [
        (
            // SUPERSEDED (found during `security-rules-cel-cross-document
            // -reads` Slice 01): a bare `get()` is now a supported
            // construct (ADR-066); this case uses chaining instead —
            // still genuinely unsupported (Resolution 2).
            "exists(/databases/$(database)/documents/orgs/$(get(/databases/$(database)/documents/users/$(request.auth.uid)).data.orgId))",
            "UNSUPPORTED_CONSTRUCT",
        ),
        ("(request.auth.uid == resource.data.owner_id", "SYNTAX_ERROR"),
    ] {
        let resp = ctx
            .client
            .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
            .header("Cookie", &cookie)
            .json(&serde_json::json!({
                "collection_path": "journal_entries",
                "condition": invalid_condition,
            }))
            .send()
            .await
            .expect("restore request failed");

        assert_eq!(
            resp.status().as_u16(),
            400,
            "AC-17-167: an invalid restore condition must be rejected 400, same as any \
             other invalid define"
        );
        let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
        assert_eq!(
            body["reason"], expected_reason,
            "AC-17-167: must use the EXISTING taxonomy, no new 'restore' error class"
        );
    }

    // The existing rule must remain untouched by the rejected restore attempts.
    let unchanged = ctx
        .access_rule_condition_source("trailmark-prod", "journal_entries")
        .await;
    assert_eq!(
        unchanged.as_deref(),
        Some("request.auth.uid == resource.data.owner_id"),
        "AC-17-167: a rejected restore attempt must not alter the currently-active condition"
    );
}
