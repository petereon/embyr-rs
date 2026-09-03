//! RW04 (Slice 04, US-04, Release 1) — An Odd-Prefix Import, a
//! Same-Specificity Tie, or an Unrelated-Shape Overlap Is Rejected, Naming
//! the Offending Pattern(s).
//!
//! Acceptance criteria verified here (feature-delta.md US-04, ADR-064):
//!   AC-17-250: two recursive-wildcard patterns at an identical fixed-prefix
//!              depth and skeleton (a genuine tie) are rejected, naming both.
//!   AC-17-251: two tied recursive-wildcard patterns within the SAME import
//!              are rejected together, before either is stored.
//!   AC-17-252: a rejected import (for a tie) leaves every existing pattern
//!              and rule completely unchanged.
//!   AC-17-253: a recursive-wildcard pattern and a fixed-depth (4b) pattern
//!              that are NOT in a structural containment relationship
//!              (different leaf collection names) import together without
//!              issue, mirroring 4b's own AC-17-221 precedent.
//!   AC-17-254: a corrected, re-submitted file with the tied pattern removed
//!              imports successfully.
//!
//! Driving port: Admin HTTP :9090 (`SecurityRulesAdminContext`, real
//! `build_admin_router` composition root — reused unchanged from Slice 01's
//! own rw01 fixture).
//!
//! AC-17-233 (odd-prefix taxonomy) is already exercised by rw01's own
//! `an_odd_prefix_recursive_wildcard_is_rejected` (Slice 01, ADR-064 §
//! Decision — Parser) — this slice's own job for that construct is
//! confirming the taxonomy is complete, not duplicating that test.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesAdminContext;

const EXPEDITIONS_CATCH_ALL_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId}/{path=**} {
      allow read: if request.auth.uid != "";
    }
  }
}
"#;

const EXPEDITIONS_CATCH_ALL_RENAMED_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expedition_id}/{path=**} {
      allow read: if request.auth.uid != "";
    }
  }
}
"#;

const INTRA_FILE_TIE_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId}/{path=**} {
      allow read: if request.auth.uid != "";
    }
    match /expeditions/{eid}/{path=**} {
      allow read: if request.auth.uid != "";
    }
  }
}
"#;

async fn recursive_pattern_row_exists(pool: &sqlx::PgPool, project_id: &str, fixed_prefix_pattern: &str) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM access_rule_patterns \
         WHERE project_id = $1 AND collection_path_pattern = $2 AND is_recursive",
    )
    .bind(project_id)
    .bind(fixed_prefix_pattern)
    .fetch_one(pool)
    .await
    .expect("count access_rule_patterns rows")
        > 0
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-250: two recursive-wildcard patterns at an identical fixed-prefix
// depth and skeleton (a differently-named tie) are rejected, naming both.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `expeditions/{expeditionId}/{path=**}` is already imported and
///          active
///   When:  Alex imports a file containing
///          `expeditions/{expedition_id}/{path=**}` (a differently-named
///          wildcard at the same structural position)
///   Then:  the import is rejected, naming both the new pattern and the
///          existing pattern it ties with
///
/// AC-17-250
///
/// @driving_port @real-io @US-04 @AC-17-250
#[tokio::test]
async fn a_same_specificity_tie_against_an_already_stored_recursive_pattern_is_rejected_naming_both() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let stored_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EXPEDITIONS_CATCH_ALL_RULES_FILE }))
        .send()
        .await
        .expect("first import request failed");
    assert_eq!(stored_resp.status().as_u16(), 200, "the first recursive pattern must import cleanly");

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EXPEDITIONS_CATCH_ALL_RENAMED_RULES_FILE }))
        .send()
        .await
        .expect("tied import request failed");

    assert_eq!(resp.status().as_u16(), 400, "AC-17-250: a same-specificity tie must be rejected");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    let offending = body["offending_blocks"].as_array().expect("offending_blocks must be an array");
    assert!(
        offending.iter().all(|b| b["construct"] == "PATTERN_OVERLAP"),
        "distinguishable from RECURSIVE_WILDCARD_ODD_PREFIX/etc.: {offending:?}"
    );
    let path_patterns: Vec<&str> = offending.iter().map(|b| b["path_pattern"].as_str().unwrap()).collect();
    assert!(
        path_patterns.contains(&"expeditions/{expedition_id}"),
        "AC-17-250: must name the NEW colliding pattern: {path_patterns:?}"
    );
    let details: Vec<&str> = offending.iter().map(|b| b["detail"].as_str().unwrap()).collect();
    assert!(
        details.iter().any(|d| d.contains("expeditions/{expeditionId}")),
        "AC-17-250: must name the EXISTING colliding pattern too: {details:?}"
    );

    assert!(
        !recursive_pattern_row_exists(&ctx.pool, "trailmark-prod", "expeditions/{expedition_id}").await,
        "AC-17-252: the rejected new pattern must never be stored"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-251: two tied recursive-wildcard patterns within the SAME file are
// rejected together, before either is stored.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: no recursive-wildcard pattern is yet stored for `expeditions/*`
///   When:  Alex imports a single file containing both
///          `expeditions/{expeditionId}/{path=**}` and
///          `expeditions/{eid}/{path=**}`
///   Then:  the entire import is rejected, naming both colliding blocks, and
///          neither is stored
///
/// AC-17-251
///
/// @driving_port @real-io @US-04 @AC-17-251
#[tokio::test]
async fn two_tied_recursive_patterns_within_the_same_import_are_rejected_together() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": INTRA_FILE_TIE_RULES_FILE }))
        .send()
        .await
        .expect("intra-file tie import request failed");

    assert_eq!(resp.status().as_u16(), 400, "AC-17-251: an intra-file tie must reject the whole import");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    let offending = body["offending_blocks"].as_array().expect("offending_blocks must be an array");
    let path_patterns: Vec<&str> = offending.iter().map(|b| b["path_pattern"].as_str().unwrap()).collect();
    assert!(path_patterns.contains(&"expeditions/{expeditionId}"));
    assert!(path_patterns.contains(&"expeditions/{eid}"));

    assert!(!recursive_pattern_row_exists(&ctx.pool, "trailmark-prod", "expeditions/{expeditionId}").await);
    assert!(!recursive_pattern_row_exists(&ctx.pool, "trailmark-prod", "expeditions/{eid}").await);
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-252: a rejected import leaves every existing pattern and rule
// completely unchanged.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `profiles` (4a) and `expeditions/{expeditionId}/{path=**}`
///          (this feature) both already have active rules
///   When:  Alex imports a new file that is entirely rejected for a
///          recursive-wildcard tie
///   Then:  both existing rules are unchanged, unaffected by the rejected
///          import attempt
///
/// AC-17-252
///
/// @driving_port @real-io @US-04 @AC-17-252
#[tokio::test]
async fn a_rejected_tie_import_leaves_every_existing_pattern_and_rule_unchanged() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let mixed_seed_file = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /expeditions/{expeditionId}/{path=**} {
              allow read: if request.auth.uid != "";
            }
            match /profiles/{userId} {
              allow read, write: if request.auth.uid == userId;
            }
          }
        }
    "#;
    let seed_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": mixed_seed_file }))
        .send()
        .await
        .expect("seed import request failed");
    assert_eq!(seed_resp.status().as_u16(), 200);

    let rejected_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EXPEDITIONS_CATCH_ALL_RENAMED_RULES_FILE }))
        .send()
        .await
        .expect("rejected import request failed");
    assert_eq!(rejected_resp.status().as_u16(), 400);

    let expeditions_condition: Option<String> = sqlx::query_scalar(
        "SELECT read_condition FROM access_rule_patterns WHERE project_id = $1 AND collection_path_pattern = $2",
    )
    .bind("trailmark-prod")
    .bind("expeditions/{expeditionId}")
    .fetch_optional(&ctx.pool)
    .await
    .expect("query access_rule_patterns");
    assert_eq!(
        expeditions_condition.as_deref(),
        Some("request.auth.uid != \"\""),
        "AC-17-252: the pre-existing expeditions catch-all must be unchanged by the rejected import"
    );

    let profiles_condition: Option<String> = sqlx::query_scalar(
        "SELECT condition_source FROM access_rules WHERE project_id = $1 AND collection_path = $2",
    )
    .bind("trailmark-prod")
    .bind("profiles")
    .fetch_optional(&ctx.pool)
    .await
    .expect("query access_rules");
    assert_eq!(
        profiles_condition.as_deref(),
        Some("request.auth.uid == request.path.userId"),
        "AC-17-252: the unrelated pre-existing profiles rule must be completely unaffected"
    );

    assert!(!recursive_pattern_row_exists(&ctx.pool, "trailmark-prod", "expeditions/{expedition_id}").await);
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-253: a recursive-wildcard pattern and a 4b fixed-depth pattern that
// are NOT in a containment relationship (different leaf collection names, at
// the SAME reach length) import together without issue.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: no pattern is yet stored for `expeditions/*/journal_entries` or
///          `expeditions/*/announcements/*`
///   When:  Alex imports a file containing both
///          `expeditions/{expeditionId}/journal_entries/{entryId}` (4b) and
///          a recursive-wildcard pattern whose own fixed prefix is
///          `expeditions/{expeditionId}/announcements/{announcementId}` — a
///          DIFFERENT leaf collection name at the identical reach length, so
///          its reach does not actually overlap `journal_entries` at all
///   Then:  both import successfully
///
/// AC-17-253
///
/// @driving_port @real-io @US-04 @AC-17-253
#[tokio::test]
async fn a_recursive_pattern_and_a_4b_pattern_with_no_containment_relationship_import_together() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let unrelated_shapes_file = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /expeditions/{expeditionId}/journal_entries/{entryId} {
              allow read, write: if request.auth.uid == resource.data.owner_id;
            }
            match /expeditions/{expeditionId}/announcements/{announcementId}/{path=**} {
              allow read: if request.auth.uid != "";
            }
          }
        }
    "#;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": unrelated_shapes_file }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-253: a recursive pattern and a 4b pattern with different leaf names must both import"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["imported"].as_array().expect("array").len(), 2);

    let pattern_row_exists: bool = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM access_rule_patterns WHERE project_id = $1 AND collection_path_pattern = $2 AND NOT is_recursive",
    )
    .bind("trailmark-prod")
    .bind("expeditions/{expeditionId}/journal_entries")
    .fetch_one(&ctx.pool)
    .await
    .expect("query access_rule_patterns")
        > 0;
    assert!(pattern_row_exists);

    assert!(
        recursive_pattern_row_exists(
            &ctx.pool,
            "trailmark-prod",
            "expeditions/{expeditionId}/announcements/{announcementId}"
        )
        .await
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-254: a corrected, re-submitted file with the tied pattern removed
// imports successfully.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: a file was previously rejected for a recursive-wildcard tie (the
///          original pattern is already active from before the rejection)
///   When:  Alex removes the colliding duplicate and re-submits a file
///          containing only the original, non-tied pattern
///   Then:  the corrected import succeeds
///
/// AC-17-254
///
/// @driving_port @real-io @US-04 @AC-17-254
#[tokio::test]
async fn a_corrected_resubmit_with_the_tied_pattern_removed_imports_successfully() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let stored_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EXPEDITIONS_CATCH_ALL_RULES_FILE }))
        .send()
        .await
        .expect("first import request failed");
    assert_eq!(stored_resp.status().as_u16(), 200);

    let rejected_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EXPEDITIONS_CATCH_ALL_RENAMED_RULES_FILE }))
        .send()
        .await
        .expect("rejected import request failed");
    assert_eq!(rejected_resp.status().as_u16(), 400, "the tied pattern must be rejected first");

    let corrected_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EXPEDITIONS_CATCH_ALL_RULES_FILE }))
        .send()
        .await
        .expect("corrected re-submit request failed");

    assert_eq!(
        corrected_resp.status().as_u16(),
        200,
        "AC-17-254: the corrected file, with the tied pattern removed, must import successfully"
    );

    assert!(!recursive_pattern_row_exists(&ctx.pool, "trailmark-prod", "expeditions/{expedition_id}").await);
}
