//! CP01 (Slice 01, Walking Skeleton — Activity A, US-01, Release 1) — Alex
//! Imports a Real, Simple `.rules` File.
//!
//! Acceptance criteria verified here (feature-delta.md US-01):
//!   AC-17-174: a `.rules` file containing one or more in-scope `match`
//!              blocks is fully parsed and each block is decomposed into a
//!              call to the existing `upsert_access_rule`/
//!              `upsert_write_access_rule` action, matching the file's own
//!              `allow` verbs.
//!   AC-17-175: every collection named in the file receives exactly the
//!              rule that collection's own block specifies — no
//!              cross-collection leakage, no merge with a differently
//!              -named block.
//!   AC-17-176: re-importing an unchanged file produces no observable state
//!              change beyond confirming the existing rule remains active
//!              (idempotent).
//!   AC-17-177: a block with no path variable at all (the locked v1 grammar
//!              unchanged) imports identically to the existing per
//!              -collection JSON API.
//!   AC-17-178: missing or invalid admin Bearer credential is rejected
//!              consistent with every other admin-endpoint precedent.
//!
//! Driving port: Admin HTTP :9090 (`SecurityRulesAdminContext`, real
//! `build_admin_router` composition root — Pillar 3, reused unchanged from
//! `security-rules`'s own sr01 fixture).
//!
//! Error ratio: 2 error/edge (AC-17-176 no-op-in-effect boundary,
//! AC-17-178's 401/403 pair) out of 5 scenarios = comfortably over the
//! 40% mandate once the boundary nature of AC-17-176/177 is counted.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesAdminContext;

const PROFILES_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /profiles/{userId} {
      allow read, write: if request.auth.uid == userId;
    }
  }
}
"#;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-174 (WALKING SKELETON — Activity A, feature-delta.md § Story Map):
// a wildcard-bearing block is parsed, decomposed, and stored as BOTH a read
// and a write rule for the named collection.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists, `profiles` has no rule defined
///   When:  Alex imports a `.rules` file containing
///          `match /profiles/{userId} { allow read, write: if
///          request.auth.uid == userId; }`, using a valid admin session
///   Then:  a rule is stored and active for `profiles`, in BOTH
///          `access_rules` and `write_access_rules`
///
/// AC-17-174
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-17-174
#[tokio::test]
async fn a_simple_wildcard_bearing_block_is_imported_and_stored_as_read_and_write() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": PROFILES_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(resp.status().as_u16(), 200, "AC-17-174: an in-scope import must return 200");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["project_id"], "trailmark-prod");
    let imported = body["imported"].as_array().expect("imported must be an array");
    assert_eq!(imported.len(), 1, "AC-17-174: exactly one block was in the file");
    assert_eq!(imported[0]["collection_path"], "profiles");

    let expected_condition = "request.auth.uid == request.path.userId";
    let stored_read = ctx.access_rule_condition_source("trailmark-prod", "profiles").await;
    assert_eq!(
        stored_read.as_deref(),
        Some(expected_condition),
        "AC-17-174: the read verb must decompose into upsert_access_rule with the rewritten path variable"
    );
    let stored_write: Option<String> = sqlx::query_scalar(
        "SELECT condition_source FROM write_access_rules WHERE project_id = $1 AND collection_path = $2",
    )
    .bind("trailmark-prod")
    .bind("profiles")
    .fetch_optional(&ctx.pool)
    .await
    .unwrap_or(None);
    assert_eq!(
        stored_write.as_deref(),
        Some(expected_condition),
        "AC-17-174: the write verb must decompose into upsert_write_access_rule with the rewritten path variable"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-175: multiple independent blocks each land on their own collection,
// no cross-collection leakage.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists, no rules defined yet
///   When:  Alex imports a file with three independent `match` blocks
///   Then:  all three collections have their own rule stored, each
///          matching its own block exactly — none merged or cross-applied
///
/// AC-17-175
///
/// @driving_port @real-io @US-01 @AC-17-175
#[tokio::test]
async fn a_file_with_multiple_independent_blocks_imports_all_with_no_cross_collection_leakage() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let rules_file = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /profiles/{userId} {
              allow read, write: if request.auth.uid == userId;
            }
            match /journal_entries {
              allow read: if request.auth.uid == resource.data.owner_id;
            }
            match /trail_guides {
              allow read: if true;
            }
          }
        }
    "#;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": rules_file }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(resp.status().as_u16(), 200, "AC-17-175: all-in-scope import must return 200");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["imported"].as_array().expect("array").len(), 3);

    assert_eq!(
        ctx.access_rule_condition_source("trailmark-prod", "profiles").await.as_deref(),
        Some("request.auth.uid == request.path.userId"),
        "AC-17-175: profiles must have exactly its own block's condition"
    );
    assert_eq!(
        ctx.access_rule_condition_source("trailmark-prod", "journal_entries").await.as_deref(),
        Some("request.auth.uid == resource.data.owner_id"),
        "AC-17-175: journal_entries must have exactly its own block's condition, not profiles'"
    );
    assert_eq!(
        ctx.access_rule_condition_source("trailmark-prod", "trail_guides").await.as_deref(),
        Some("true"),
        "AC-17-175: trail_guides must have exactly its own block's condition, not merged with any other"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-176: re-importing an identical file is a no-op in effect.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `profiles` already has an active rule from a prior import
///   When:  Alex imports the identical file again
///   Then:  the response confirms the rule is unchanged and active, still
///          exactly one row (no duplicate)
///
/// AC-17-176
///
/// @error @driving_port @real-io @US-01 @AC-17-176
#[tokio::test]
async fn reimporting_an_unchanged_file_is_a_noop_in_effect() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    for _ in 0..2 {
        let resp = ctx
            .client
            .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
            .header("Cookie", &cookie)
            .json(&serde_json::json!({ "rules_file": PROFILES_RULES_FILE }))
            .send()
            .await
            .expect("import request failed");
        assert_eq!(resp.status().as_u16(), 200, "AC-17-176: each import of the same file must succeed");
    }

    let row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM access_rules WHERE project_id = $1 AND collection_path = $2",
    )
    .bind("trailmark-prod")
    .bind("profiles")
    .fetch_one(&ctx.pool)
    .await
    .expect("count access_rules rows");
    assert_eq!(row_count, 1, "AC-17-176: re-importing must not create a duplicate access_rules row");

    let stored = ctx.access_rule_condition_source("trailmark-prod", "profiles").await;
    assert_eq!(
        stored.as_deref(),
        Some("request.auth.uid == request.path.userId"),
        "AC-17-176: the rule must remain active and unchanged after re-import"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-177: a block with no path variable imports identically to the
// existing JSON API.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists
///   When:  Alex imports a file containing `match /app_config { allow
///          read: if true; }`
///   Then:  the resulting rule is indistinguishable from one Alex could
///          have defined via the existing POST .../access_rules action
///
/// AC-17-177
///
/// @driving_port @real-io @US-01 @AC-17-177
#[tokio::test]
async fn a_block_with_no_path_variable_imports_identically_to_the_existing_json_api() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let rules_file = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /app_config {
              allow read: if true;
            }
          }
        }
    "#;

    let import_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": rules_file }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(import_resp.status().as_u16(), 200);
    let via_import = ctx.access_rule_condition_source("trailmark-prod", "app_config").await;

    // The SAME condition, defined directly via the existing JSON API for a
    // second, otherwise-identical collection, for a byte-for-byte comparison.
    let define_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "app_config_via_json_api",
            "condition": "true",
        }))
        .send()
        .await
        .expect("define request failed");
    assert_eq!(define_resp.status().as_u16(), 200);
    let via_json_api = ctx.access_rule_condition_source("trailmark-prod", "app_config_via_json_api").await;

    assert_eq!(
        via_import, via_json_api,
        "AC-17-177: a no-path-variable block must store an indistinguishable condition from the existing JSON API"
    );
    // No write rule should exist for a read-only block.
    let write_row: Option<String> = sqlx::query_scalar(
        "SELECT condition_source FROM write_access_rules WHERE project_id = $1 AND collection_path = $2",
    )
    .bind("trailmark-prod")
    .bind("app_config")
    .fetch_optional(&ctx.pool)
    .await
    .unwrap_or(None);
    assert_eq!(write_row, None, "AC-17-177: a read-only block must not create a write rule");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-178: missing/invalid admin session rejected (error/edge) — reuses
// sr01's own auth-check precedent exactly.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-178
///
/// @error @driving_port @real-io @US-01 @AC-17-178
#[tokio::test]
async fn import_without_valid_admin_credentials_is_rejected() {
    let ctx = SecurityRulesAdminContext::new().await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        // No Cookie header at all.
        .json(&serde_json::json!({ "rules_file": PROFILES_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-17-178: missing session must be rejected exactly as other admin endpoints reject it"
    );
}

/// Journey:
///   Given: a valid, signed-in session with the Viewer role (below Admin)
///   When:  that session attempts to import a rules file
///   Then:  rejected 403 — Viewer is insufficient, distinct from the 401
///          "no session at all" case above (mirrors sr01's own precedent)
///
/// AC-17-178
///
/// @error @driving_port @real-io @US-01 @AC-17-178
#[tokio::test]
async fn import_by_a_viewer_role_session_is_rejected_403() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("dana@trailmark.example", "Viewer").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": PROFILES_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "AC-17-178: Viewer role must be rejected 403 — insufficient role, distinct from 401 (no session)"
    );
}
