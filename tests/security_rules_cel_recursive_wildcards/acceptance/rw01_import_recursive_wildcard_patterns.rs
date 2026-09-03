//! RW01 (Slice 01, Walking Skeleton, US-01, Release 1) — Alex Imports a Real
//! File Containing an Even-Prefix Recursive-Wildcard Pattern.
//!
//! Acceptance criteria verified here (feature-delta.md US-01, ADR-064):
//!   AC-17-232: a terminal, even-prefix recursive-wildcard `match` block is
//!              parsed and decomposed into a storable fixed-prefix-plus-open
//!              -remainder shape.
//!   AC-17-233: a recursive-wildcard segment at an odd-prefix position is
//!              rejected with a specific, distinguishable reason.
//!   AC-17-234: a recursive-wildcard segment that is not the pattern's own
//!              final segment is rejected with a specific reason.
//!   AC-17-235: re-importing an unchanged recursive-wildcard file produces no
//!              observable state change beyond confirming the pattern remains
//!              active (idempotent).
//!   AC-17-236: a file mixing a recursive-wildcard pattern with 4a's own
//!              shapes imports all of them correctly in one pass.
//!   AC-17-237: the empty-fixed-prefix case (a project-wide `{document=**}`
//!              catch-all) is a valid, importable shape.
//!
//! Driving port: Admin HTTP :9090 (`SecurityRulesAdminContext`, real
//! `build_admin_router` composition root — reused unchanged from
//! `security-rules-cel-path-matching`'s own pm01 fixture).
//!
//! Scope: storage only (ADR-064 § Decision — Schema/Parser). Routing,
//! precedence, and overlap detection against recursive patterns are Slices
//! 02/04's own concern — not exercised here.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesAdminContext;

const CATCH_ALL_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /{document=**} {
      allow read, write: if false;
    }
  }
}
"#;

const EXPEDITIONS_CATCH_ALL_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId}/{path=**} {
      allow read: if expeditionId != "";
    }
  }
}
"#;

/// Query `access_rule_patterns` directly for a stored recursive-wildcard
/// pattern's own row — mirrors pm01's own direct-`ctx.pool` query discipline.
async fn recursive_pattern_row(
    pool: &sqlx::PgPool,
    project_id: &str,
    collection_path_pattern: &str,
) -> Option<(i16, String, bool, Option<String>, Option<String>)> {
    sqlx::query_as::<_, (i16, String, bool, Option<String>, Option<String>)>(
        "SELECT ancestor_segment_count, literal_skeleton, is_recursive, read_condition, write_condition \
         FROM access_rule_patterns WHERE project_id = $1 AND collection_path_pattern = $2 AND is_recursive",
    )
    .bind(project_id)
    .bind(collection_path_pattern)
    .fetch_optional(pool)
    .await
    .expect("query access_rule_patterns")
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-232 / AC-17-237 (WALKING SKELETON): a project-wide, empty-fixed
// -prefix recursive-wildcard catch-all is imported, decomposed, and stored
// active.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists, no recursive-wildcard pattern is
///          yet defined
///   When:  Alex imports a `.rules` file containing
///          `match /{document=**} { allow read, write: if false; }`
///   Then:  a recursive-wildcard pattern is stored and active, with an empty
///          fixed prefix
///
/// AC-17-232, AC-17-237
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-17-232 @AC-17-237
#[tokio::test]
async fn a_project_wide_recursive_wildcard_catch_all_is_imported_and_active() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": CATCH_ALL_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(resp.status().as_u16(), 200, "AC-17-237: a project-wide catch-all import must return 200");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    let imported = body["imported"].as_array().expect("imported must be an array");
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0]["collection_path"], "");
    assert_eq!(imported[0]["read_condition"], "false");
    assert_eq!(imported[0]["write_condition"], "false");

    let row = recursive_pattern_row(&ctx.pool, "trailmark-prod", "")
        .await
        .expect("AC-17-237: the empty-fixed-prefix pattern must be stored in access_rule_patterns");
    let (ancestor_segment_count, literal_skeleton, is_recursive, read_condition, write_condition) = row;
    assert_eq!(ancestor_segment_count, 0);
    assert_eq!(literal_skeleton, "");
    assert!(is_recursive);
    assert_eq!(read_condition.as_deref(), Some("false"));
    assert_eq!(write_condition.as_deref(), Some("false"));
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-232: a narrower, prefix-scoped recursive-wildcard pattern is
// imported, decomposed, and stored active, retaining the prefix's own named
// wildcard capture.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists
///   When:  Alex imports a file containing
///          `match /expeditions/{expeditionId}/{path=**} { allow read: if
///          request.auth.uid != ""; }`
///   Then:  a recursive-wildcard pattern is stored and active, with fixed
///          prefix `expeditions/{expeditionId}`
///
/// AC-17-232
///
/// @driving_port @real-io @US-01 @AC-17-232
#[tokio::test]
async fn a_narrower_prefix_scoped_recursive_wildcard_pattern_is_imported_and_active() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EXPEDITIONS_CATCH_ALL_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(resp.status().as_u16(), 200, "AC-17-232: an in-scope recursive-wildcard import must return 200");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["imported"][0]["collection_path"], "expeditions/{expeditionId}");

    let row = recursive_pattern_row(&ctx.pool, "trailmark-prod", "expeditions/{expeditionId}")
        .await
        .expect("AC-17-232: the prefix-scoped pattern must be stored in access_rule_patterns");
    let (ancestor_segment_count, literal_skeleton, is_recursive, read_condition, _write_condition) = row;
    assert_eq!(ancestor_segment_count, 2, "AC-17-232: the fixed prefix's own segment count must be retained, always even");
    assert_eq!(literal_skeleton, "expeditions");
    assert!(is_recursive);
    assert_eq!(
        read_condition.as_deref(),
        Some("request.path.expeditionId != \"\""),
        "AC-17-232: the prefix's own named wildcard capture must be rewritten via 4a's/4b's unchanged mechanism"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-233: an odd-prefix recursive wildcard is rejected with a specific,
// distinguishable reason.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists
///   When:  Alex imports a file containing
///          `match /expeditions/{path=**} { allow read: if true; }` — the
///          recursive wildcard sits directly after a bare collection name (an
///          ODD-length prefix)
///   Then:  the import is rejected, naming the block and a reason
///          distinguishable from every other rejection reason
///
/// AC-17-233
///
/// @error @driving_port @real-io @US-01 @AC-17-233
#[tokio::test]
async fn an_odd_prefix_recursive_wildcard_is_rejected() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let source = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /expeditions/{path=**} {
              allow read: if true;
            }
          }
        }
    "#;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": source }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(resp.status().as_u16(), 400, "AC-17-233: an odd-prefix recursive wildcard must be rejected");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["offending_blocks"][0]["construct"],
        "RECURSIVE_WILDCARD_ODD_PREFIX",
        "AC-17-233: the rejection reason must be distinguishable from every other rejection reason"
    );

    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM access_rule_patterns WHERE project_id = $1")
            .bind("trailmark-prod")
            .fetch_one(&ctx.pool)
            .await
            .expect("count access_rule_patterns rows");
    assert_eq!(row_count, 0, "AC-17-233: a rejected import must touch zero storage");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-234: a recursive wildcard that is not the pattern's own final
// segment is rejected.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists
///   When:  Alex imports a file containing
///          `match /expeditions/{path=**}/journal_entries { allow read: if
///          true; }`
///   Then:  the import is rejected, naming the block and the specific reason
///          (recursive wildcard must be the terminal segment)
///
/// AC-17-234
///
/// @error @driving_port @real-io @US-01 @AC-17-234
#[tokio::test]
async fn a_non_terminal_recursive_wildcard_is_rejected() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let source = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /expeditions/{path=**}/journal_entries {
              allow read: if true;
            }
          }
        }
    "#;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": source }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(resp.status().as_u16(), 400, "AC-17-234: a non-terminal recursive wildcard must be rejected");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["offending_blocks"][0]["construct"], "RECURSIVE_WILDCARD_NOT_TERMINAL");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-235: re-importing an unchanged recursive-wildcard file is a no-op in
// effect.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: the project-wide catch-all already has an active pattern from a
///          prior import
///   When:  Alex imports the identical file again
///   Then:  the response confirms the pattern is unchanged and active, with
///          no duplicate row
///
/// AC-17-235
///
/// @driving_port @real-io @US-01 @AC-17-235
#[tokio::test]
async fn reimporting_an_unchanged_recursive_wildcard_file_is_a_noop_in_effect() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    for _ in 0..2 {
        let resp = ctx
            .client
            .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
            .header("Cookie", &cookie)
            .json(&serde_json::json!({ "rules_file": CATCH_ALL_RULES_FILE }))
            .send()
            .await
            .expect("import request failed");
        assert_eq!(resp.status().as_u16(), 200, "AC-17-235: each import of the same file must succeed");
    }

    let row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM access_rule_patterns WHERE project_id = $1 AND collection_path_pattern = $2 AND is_recursive",
    )
    .bind("trailmark-prod")
    .bind("")
    .fetch_one(&ctx.pool)
    .await
    .expect("count access_rule_patterns rows");
    assert_eq!(row_count, 1, "AC-17-235: re-importing must not create a duplicate access_rule_patterns row");

    let row = recursive_pattern_row(&ctx.pool, "trailmark-prod", "")
        .await
        .expect("pattern must still be active");
    assert_eq!(row.3.as_deref(), Some("false"), "AC-17-235: the pattern must remain unchanged");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-236: a file mixing a recursive-wildcard pattern with a 4a-shaped
// block imports both correctly.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists
///   When:  Alex imports a file containing both
///          `match /{document=**} { allow read, write: if false; }` AND a
///          4a-shaped `match /profiles/{userId} { ... }` block
///   Then:  both are stored and active
///
/// AC-17-236
///
/// @driving_port @real-io @US-01 @AC-17-236
#[tokio::test]
async fn a_file_mixing_a_recursive_wildcard_pattern_with_a_4a_shaped_block_imports_both() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let mixed_rules_file = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /{document=**} {
              allow read, write: if false;
            }
            match /profiles/{userId} {
              allow read, write: if request.auth.uid == userId;
            }
          }
        }
    "#;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": mixed_rules_file }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(resp.status().as_u16(), 200, "AC-17-236: a file mixing both shapes must import cleanly");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["imported"].as_array().expect("array").len(), 2);

    let catch_all = recursive_pattern_row(&ctx.pool, "trailmark-prod", "")
        .await
        .expect("AC-17-236: the recursive-wildcard pattern must be stored in access_rule_patterns");
    assert_eq!(catch_all.3.as_deref(), Some("false"));

    let single_collection: Option<String> = sqlx::query_scalar(
        "SELECT condition_source FROM access_rules WHERE project_id = $1 AND collection_path = $2",
    )
    .bind("trailmark-prod")
    .bind("profiles")
    .fetch_optional(&ctx.pool)
    .await
    .expect("query access_rules");
    assert_eq!(
        single_collection.as_deref(),
        Some("request.auth.uid == request.path.userId"),
        "AC-17-236: the 4a-shaped single-wildcard block must still be stored in access_rules exactly as before"
    );
}
