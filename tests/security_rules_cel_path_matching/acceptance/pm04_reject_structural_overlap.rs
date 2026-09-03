//! PM04 (Slice 04, US-04, Release 1) — An Import (or a New Pattern) That
//! Would Introduce Structural Overlap Is Rejected, Naming the Colliding
//! Patterns.
//!
//! Acceptance criteria verified here (feature-delta.md US-04, ADR-063):
//!   AC-17-218: a new pattern that would structurally overlap an
//!              already-stored pattern is rejected, naming both patterns,
//!              with a distinguishable `PATTERN_OVERLAP` reason (AC-17-223).
//!   AC-17-219: two structurally-overlapping patterns within the SAME
//!              import are rejected together, before either is stored.
//!   AC-17-220: a rejected import leaves every existing pattern/rule —
//!              named in the file or not — completely unchanged.
//!   AC-17-221: patterns with different leaf collection names never
//!              structurally overlap, regardless of shared wildcard
//!              positions earlier in the path.
//!   AC-17-222: a corrected, re-submitted file with the colliding pattern
//!              removed imports successfully.
//!
//! Driving port: Admin HTTP :9090 (`SecurityRulesAdminContext`, real
//! `build_admin_router` composition root — reused unchanged from Slice 01's
//! own pm01 fixture).
//!
//! Error ratio: N/A (this slice's own scope is entirely rejection/guardrail
//! behavior; a dedicated "missing credentials" scenario is already proven
//! by pm01/cp01 for the same route and is not duplicated here).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesAdminContext;

const OWNER_CONDITION: &str = "request.auth.uid == resource.data.owner_id";

const WILDCARD_JOURNAL_ENTRIES_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId}/journal_entries/{entryId} {
      allow read, write: if request.auth.uid == resource.data.owner_id;
    }
  }
}
"#;

const LITERAL_EXCEPTION_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/trek-2026/journal_entries/{entryId} {
      allow read, write: if request.auth.uid == resource.data.owner_id;
    }
  }
}
"#;

const INTRA_FILE_OVERLAP_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId}/journal_entries/{entryId} {
      allow read, write: if request.auth.uid == resource.data.owner_id;
    }
    match /expeditions/trek-2026/journal_entries/{entryId} {
      allow read, write: if request.auth.uid == resource.data.owner_id;
    }
  }
}
"#;

async fn pattern_row_exists(
    pool: &sqlx::PgPool,
    project_id: &str,
    collection_path_pattern: &str,
) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM access_rule_patterns WHERE project_id = $1 AND collection_path_pattern = $2",
    )
    .bind(project_id)
    .bind(collection_path_pattern)
    .fetch_one(pool)
    .await
    .expect("count access_rule_patterns rows")
        > 0
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-218/223: a new pattern overlapping an already-stored pattern is
// rejected, naming both, with a distinguishable PATTERN_OVERLAP reason.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `expeditions/{expeditionId}/journal_entries/{entryId}` (wildcard)
///          is already imported and active
///   When:  Alex imports a file containing
///          `expeditions/trek-2026/journal_entries/{entryId}` (a literal
///          segment at the same position as the existing pattern's
///          wildcard)
///   Then:  the import is rejected, naming both the new and the existing
///          pattern, reason `PATTERN_OVERLAP`
///
/// AC-17-218, AC-17-223
///
/// @driving_port @real-io @US-04 @AC-17-218 @AC-17-223
#[tokio::test]
async fn a_new_pattern_overlapping_an_already_stored_pattern_is_rejected_naming_both() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let stored_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": WILDCARD_JOURNAL_ENTRIES_RULES_FILE }))
        .send()
        .await
        .expect("wildcard import request failed");
    assert_eq!(
        stored_resp.status().as_u16(),
        200,
        "the wildcard pattern must import cleanly first"
    );

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": LITERAL_EXCEPTION_RULES_FILE }))
        .send()
        .await
        .expect("literal-exception import request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "AC-17-218: a structurally-overlapping new pattern must be rejected"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["reason"], "IMPORT_REJECTED");
    let offending = body["offending_blocks"]
        .as_array()
        .expect("offending_blocks must be an array");
    assert!(!offending.is_empty());
    assert!(
        offending
            .iter()
            .all(|b| b["construct"] == "PATTERN_OVERLAP"),
        "AC-17-223: distinguishable from RECURSIVE_WILDCARD/NESTED_PATH/etc.: {offending:?}"
    );
    let path_patterns: Vec<&str> = offending
        .iter()
        .map(|b| b["path_pattern"].as_str().unwrap())
        .collect();
    assert!(
        path_patterns.contains(&"expeditions/trek-2026/journal_entries"),
        "AC-17-218: must name the NEW colliding pattern: {path_patterns:?}"
    );
    let details: Vec<&str> = offending
        .iter()
        .map(|b| b["detail"].as_str().unwrap())
        .collect();
    assert!(
        details
            .iter()
            .any(|d| d.contains("expeditions/{expeditionId}/journal_entries")),
        "AC-17-218: must name the EXISTING colliding pattern too: {details:?}"
    );

    assert!(
        !pattern_row_exists(
            &ctx.pool,
            "trailmark-prod",
            "expeditions/trek-2026/journal_entries"
        )
        .await,
        "AC-17-220: the rejected new pattern must never be stored"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-219: two overlapping patterns within the SAME file are rejected
// together, before either is stored.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: no pattern is yet stored for `expeditions/*/journal_entries`
///   When:  Alex imports a single file containing both
///          `expeditions/{expeditionId}/journal_entries/{entryId}` and
///          `expeditions/trek-2026/journal_entries/{entryId}`
///   Then:  the entire import is rejected, naming both colliding blocks,
///          and neither is stored
///
/// AC-17-219
///
/// @driving_port @real-io @US-04 @AC-17-219
#[tokio::test]
async fn two_overlapping_patterns_within_the_same_import_are_rejected_together() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": INTRA_FILE_OVERLAP_RULES_FILE }))
        .send()
        .await
        .expect("intra-file overlap import request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "AC-17-219: an intra-file overlap must reject the whole import"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    let offending = body["offending_blocks"]
        .as_array()
        .expect("offending_blocks must be an array");
    let path_patterns: Vec<&str> = offending
        .iter()
        .map(|b| b["path_pattern"].as_str().unwrap())
        .collect();
    assert!(path_patterns.contains(&"expeditions/{expeditionId}/journal_entries"));
    assert!(path_patterns.contains(&"expeditions/trek-2026/journal_entries"));

    assert!(
        !pattern_row_exists(
            &ctx.pool,
            "trailmark-prod",
            "expeditions/{expeditionId}/journal_entries"
        )
        .await,
        "AC-17-219: neither colliding pattern may be stored"
    );
    assert!(
        !pattern_row_exists(
            &ctx.pool,
            "trailmark-prod",
            "expeditions/trek-2026/journal_entries"
        )
        .await,
        "AC-17-219: neither colliding pattern may be stored"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-220: a rejected import leaves every existing pattern/rule
// completely unchanged, including collections unrelated to the overlap.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `profiles` (4a-shaped, single-collection) and
///          `expeditions/*/journal_entries` (this feature) both already have
///          active rules
///   When:  Alex imports a new file that is entirely rejected for structural
///          overlap on the SAME collection
///   Then:  both pre-existing rules are unchanged, unaffected by the
///          rejected import attempt
///
/// AC-17-220
///
/// @driving_port @real-io @US-04 @AC-17-220
#[tokio::test]
async fn a_rejected_import_leaves_every_existing_pattern_and_rule_unchanged() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let mixed_seed_file = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /expeditions/{expeditionId}/journal_entries/{entryId} {
              allow read, write: if request.auth.uid == resource.data.owner_id;
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
        .json(&serde_json::json!({ "rules_file": LITERAL_EXCEPTION_RULES_FILE }))
        .send()
        .await
        .expect("rejected import request failed");
    assert_eq!(rejected_resp.status().as_u16(), 400);

    let expeditions_condition: Option<String> = sqlx::query_scalar(
        "SELECT read_condition FROM access_rule_patterns WHERE project_id = $1 AND collection_path_pattern = $2",
    )
    .bind("trailmark-prod")
    .bind("expeditions/{expeditionId}/journal_entries")
    .fetch_optional(&ctx.pool)
    .await
    .expect("query access_rule_patterns");
    assert_eq!(
        expeditions_condition.as_deref(),
        Some(OWNER_CONDITION),
        "AC-17-220: the pre-existing expeditions pattern must be unchanged by the rejected import"
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
        "AC-17-220: the unrelated pre-existing profiles rule must be completely unaffected"
    );

    assert!(
        !pattern_row_exists(
            &ctx.pool,
            "trailmark-prod",
            "expeditions/trek-2026/journal_entries"
        )
        .await,
        "AC-17-220: the rejected new pattern must never be stored"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-221: different leaf collection names never structurally overlap,
// regardless of shared wildcard positions earlier in the path.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: no pattern is yet stored for `expeditions/*/announcements`
///   When:  Alex imports a file containing
///          `expeditions/{expeditionId}/announcements/{announcementId}`
///          alongside the unrelated
///          `expeditions/{expeditionId}/journal_entries/{entryId}` pattern
///   Then:  both import successfully — different leaf collection names
///          never structurally overlap
///
/// AC-17-221
///
/// @driving_port @real-io @US-04 @AC-17-221
#[tokio::test]
async fn different_leaf_collection_names_never_structurally_overlap() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let same_depth_different_leaf_file = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /expeditions/{expeditionId}/journal_entries/{entryId} {
              allow read, write: if request.auth.uid == resource.data.owner_id;
            }
            match /expeditions/{expeditionId}/announcements/{announcementId} {
              allow read, write: if request.auth.uid == resource.data.owner_id;
            }
          }
        }
    "#;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": same_depth_different_leaf_file }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-221: patterns sharing wildcard positions but differing in leaf collection name must both import"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["imported"].as_array().expect("array").len(), 2);

    assert!(
        pattern_row_exists(
            &ctx.pool,
            "trailmark-prod",
            "expeditions/{expeditionId}/journal_entries"
        )
        .await
    );
    assert!(
        pattern_row_exists(
            &ctx.pool,
            "trailmark-prod",
            "expeditions/{expeditionId}/announcements"
        )
        .await
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-222: a corrected, re-submitted file with the colliding pattern
// removed imports successfully.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: a file was previously rejected for structural overlap (the
///          wildcard pattern is already active from before the rejection)
///   When:  Alex removes the colliding literal-exception pattern and
///          re-submits a file containing only the non-overlapping,
///          already-active wildcard pattern
///   Then:  the corrected import succeeds
///
/// AC-17-222
///
/// @driving_port @real-io @US-04 @AC-17-222
#[tokio::test]
async fn a_corrected_resubmit_with_the_colliding_pattern_removed_imports_successfully() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let stored_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": WILDCARD_JOURNAL_ENTRIES_RULES_FILE }))
        .send()
        .await
        .expect("wildcard import request failed");
    assert_eq!(stored_resp.status().as_u16(), 200);

    let rejected_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": LITERAL_EXCEPTION_RULES_FILE }))
        .send()
        .await
        .expect("rejected import request failed");
    assert_eq!(
        rejected_resp.status().as_u16(),
        400,
        "the colliding pattern must be rejected first"
    );

    let corrected_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": WILDCARD_JOURNAL_ENTRIES_RULES_FILE }))
        .send()
        .await
        .expect("corrected re-submit request failed");

    assert_eq!(
        corrected_resp.status().as_u16(),
        200,
        "AC-17-222: the corrected file, with the colliding pattern removed, must import successfully"
    );

    assert!(
        !pattern_row_exists(
            &ctx.pool,
            "trailmark-prod",
            "expeditions/trek-2026/journal_entries"
        )
        .await,
        "the earlier rejection must have left the system in exactly its pre-import state"
    );
}
