//! RW05 (Slice 05, US-05, Release 1) — Untouched Patterns, 4a's and 4b's Own
//! Imports, and the Full Regression Baseline Are Unaffected.
//!
//! Acceptance criteria verified here (feature-delta.md US-05,
//! slice-05-non-regression-guardrail.md):
//!   AC-17-255: the full pre-existing regression baseline (7 prior JOB-17
//!              sibling epics + 4a's own 6 slices + 4b's own 6 slices + this
//!              feature's own Slices 01-04) passes unmodified — proven by
//!              re-running every `security_rules_*` cargo test target, not
//!              by anything in this file (this file proves the remaining 3
//!              targeted, in-process guarantees). NOT represented by any
//!              function in this file (not even an `#[ignore]`d marker) —
//!              satisfied by the out-of-band full regression sweep reported
//!              alongside this slice's commit, mirroring
//!              `security_rules_collection_group_rules`'s own
//!              `regression_baseline_unaffected.rs` precedent exactly.
//!   AC-17-256: 4a's own exact-match rule (`profiles/{userId}`) and 4b's own
//!              fixed-depth pattern (`expeditions/{expeditionId}/
//!              journal_entries/{entryId}`) are unaffected by this feature's
//!              own precedence mechanism, even once a co-existing,
//!              structurally-overlapping recursive-wildcard catch-all
//!              becomes active (Resolution 2: exact-match and fixed-depth
//!              patterns always outrank a recursive wildcard). Write-path
//!              parity for this SAME fact is already proven by rw03's own
//!              AC-17-249 — not duplicated here.
//!   AC-17-257: `app_config` — a collection with no rule of any kind and no
//!              reaching recursive-wildcard pattern (the catch-all here is
//!              deliberately SCOPED to `expeditions/*`, not project-wide, so
//!              it genuinely does not reach `app_config`) — remains fully
//!              unrestricted.
//!   AC-17-258: importing a recursive-wildcard pattern has zero effect, at
//!              the storage layer, on a pre-existing 4a rule it does not
//!              structurally overlap (`profiles`, a different top-level
//!              collection than the catch-all's own `expeditions` prefix).
//!
//! Single shared setup per test imports all co-existing shapes in ONE import
//! call — proving real coexistence, not isolated single-mechanism projects,
//! mirroring `security-rules-cel-path-matching`'s own pm05 discipline
//! exactly. Reuses `SecurityRulesFullContext`'s own pre-seeded `app_config`
//! document (mirrors pm05's own reuse of pre-seeded `trail_guides`) rather
//! than seeding a new one.
//!
//! Driving port: admin HTTP :9090 (import) + gRPC :8080 `GetDocument`, via
//! `SecurityRulesFullContext` — the same composition root every prior slice
//! in this feature uses.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

/// AC-17-256: a project-wide recursive-wildcard catch-all (deny everything)
/// co-exists with 4a's own `profiles/{userId}` rule and 4b's own
/// `journal_entries` pattern — both specific rules structurally overlapped
/// by the catch-all, both proven unaffected by it.
const CATCH_ALL_COEXISTENCE_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /profiles/{userId} {
      allow read, write: if request.auth.uid == userId;
    }
    match /expeditions/{expeditionId}/journal_entries/{entryId} {
      allow read, write: if request.auth.uid == resource.data.owner_id;
    }
    match /{document=**} {
      allow read, write: if false;
    }
  }
}
"#;

/// AC-17-257/258: a recursive-wildcard catch-all SCOPED to `expeditions/*`
/// only — deliberately does not reach `app_config` or `profiles`, proving
/// both that an untouched collection stays unrestricted and that an
/// unrelated 4a rule's own storage row is unaffected by the import.
const SCOPED_CATCH_ALL_RULES_FILE: &str = r#"
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
// AC-17-256: 4a's own exact-match rule and 4b's own fixed-depth pattern are
// unaffected by a co-existing, structurally-overlapping recursive-wildcard
// catch-all.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `profiles/{userId}` (4a) and `expeditions/{expeditionId}/
///          journal_entries/{entryId}` (4b) are both active, AND a
///          project-wide `{document=**}` catch-all (deny everything) is
///          imported afterward, structurally reaching both
///   When:  Maria reads her own `profiles` document and her own journal
///          entry; Dana (a different signed-in user) reads both too
///   Then:  the outcomes are identical to what 4a/4b already deliver —
///          Maria succeeds on both, Dana is denied on both — completely
///          unaffected by the catch-all's own existence
///
/// AC-17-256
///
/// @driving_port @real-io @US-05 @AC-17-256 @security-regression
#[tokio::test]
async fn a4a_and_4b_specific_rules_are_unaffected_by_a_coexisting_project_wide_catch_all() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rw05-4a4b-coexist").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    import(&ctx, &cookie, CATCH_ALL_COEXISTENCE_RULES_FILE).await;

    ctx.seed_document("profiles", "maria-santos", serde_json::json!({})).await;
    ctx.seed_document(
        "expeditions/trek-2026/journal_entries",
        "entry-042",
        serde_json::json!({ "owner_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let profile_resource = resource_name(&ctx.project_id, "profiles/maria-santos");
    let entry_resource = resource_name(&ctx.project_id, "expeditions/trek-2026/journal_entries/entry-042");

    assert!(
        ctx.get_document(&profile_resource, Some(&marias_token)).await.is_ok(),
        "AC-17-256: Maria's read of her own 4a profiles document must still succeed"
    );
    let err = ctx
        .get_document(&profile_resource, Some(&danas_token))
        .await
        .expect_err("AC-17-256: Dana's read of Maria's profile must still be denied by 4a's own rule");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());

    assert!(
        ctx.get_document(&entry_resource, Some(&marias_token)).await.is_ok(),
        "AC-17-256: Maria's read of her own 4b journal entry must still succeed"
    );
    let err = ctx
        .get_document(&entry_resource, Some(&danas_token))
        .await
        .expect_err("AC-17-256: Dana's read of Maria's journal entry must still be denied by 4b's own pattern");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-257: a collection with no rule and no reaching recursive-wildcard
// pattern remains fully unrestricted.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `app_config` has never had any rule, pattern, or reaching
///          catch-all defined — the only catch-all active is SCOPED to
///          `expeditions/*`, which structurally does not reach `app_config`
///   When:  an anonymous caller reads the pre-seeded `app_config` document
///   Then:  the read succeeds unrestricted
///
/// AC-17-257
///
/// @driving_port @real-io @US-05 @AC-17-257 @security-regression
#[tokio::test]
async fn a_collection_with_no_rule_and_no_reaching_recursive_wildcard_pattern_remains_unrestricted() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rw05-no-rule").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    import(&ctx, &cookie, SCOPED_CATCH_ALL_RULES_FILE).await;

    let config_resource = resource_name(&ctx.project_id, &format!("app_config/{}-config-doc", ctx.project_id));

    let resp = ctx.get_document(&config_resource, None).await;

    assert!(
        resp.is_ok(),
        "AC-17-257: a collection with no rule and no reaching recursive-wildcard pattern must \
         remain fully unrestricted: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-258: importing a recursive-wildcard pattern has zero effect, at the
// storage layer, on a pre-existing 4a row it does not structurally overlap.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `profiles` (4a) already has an active rule from a prior import
///   When:  Alex separately imports a recursive-wildcard catch-all scoped to
///          `expeditions/*` — a different top-level collection, no
///          structural overlap with `profiles` at all
///   Then:  `profiles`'s own stored condition is byte-identical to before
///          the recursive-wildcard import — zero effect on an unrelated row
///
/// AC-17-258
///
/// @driving_port @real-io @US-05 @AC-17-258 @security-regression
#[tokio::test]
async fn importing_a_recursive_wildcard_pattern_has_zero_effect_on_an_unrelated_4a_row() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rw05-noninterference").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let profiles_only_file = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /profiles/{userId} {
              allow read, write: if request.auth.uid == userId;
            }
          }
        }
    "#;
    import(&ctx, &cookie, profiles_only_file).await;

    let before: Option<String> = sqlx::query_scalar(
        "SELECT condition_source FROM access_rules WHERE project_id = $1 AND collection_path = $2",
    )
    .bind(&ctx.project_id)
    .bind("profiles")
    .fetch_optional(&ctx.sys_pool)
    .await
    .expect("query access_rules before");
    assert_eq!(before.as_deref(), Some("request.auth.uid == request.path.userId"));

    import(&ctx, &cookie, SCOPED_CATCH_ALL_RULES_FILE).await;

    let after: Option<String> = sqlx::query_scalar(
        "SELECT condition_source FROM access_rules WHERE project_id = $1 AND collection_path = $2",
    )
    .bind(&ctx.project_id)
    .bind("profiles")
    .fetch_optional(&ctx.sys_pool)
    .await
    .expect("query access_rules after");

    assert_eq!(
        after, before,
        "AC-17-258: importing a non-overlapping recursive-wildcard pattern must leave the \
         unrelated 4a `profiles` row byte-identical"
    );
}
