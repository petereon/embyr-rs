//! PM01 (Slice 01, Walking Skeleton — Activities A, US-01, Release 1) —
//! Alex Imports a Real File Containing Multi-Segment and Nested-Match-Block
//! Patterns.
//!
//! Acceptance criteria verified here (feature-delta.md US-01, ADR-063):
//!   AC-17-202: a `match` block whose path pattern spans more than 2
//!              segments is parsed and decomposed into a storable pattern.
//!   AC-17-203: a nested `match { match { ... } } }` outer-syntax shell
//!              decomposes to the identical internal pattern representation
//!              the equivalent flat multi-segment syntax produces.
//!   AC-17-205: re-importing an unchanged multi-segment file produces no
//!              observable state change beyond confirming the existing
//!              pattern remains active (idempotent).
//!   AC-17-206: a file mixing this feature's multi-segment patterns with
//!              4a's own single-collection/single-wildcard shapes imports
//!              both correctly in one pass.
//!
//! Driving port: Admin HTTP :9090 (`SecurityRulesAdminContext`, real
//! `build_admin_router` composition root — reused unchanged from
//! `security-rules-cel-parity`'s own cp01 fixture).
//!
//! Error ratio: 1 error/edge (missing admin credentials) out of 5 scenarios.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesAdminContext;

const EXPEDITIONS_OWNER_CONDITION: &str = "request.auth.uid == resource.data.owner_id";

const EXPEDITIONS_NESTED_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId} {
      match /journal_entries/{entryId} {
        allow read, write: if request.auth.uid == resource.data.owner_id;
      }
    }
  }
}
"#;

const EXPEDITIONS_FLAT_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId}/journal_entries/{entryId} {
      allow read, write: if request.auth.uid == resource.data.owner_id;
    }
  }
}
"#;

/// Query `access_rule_patterns` directly for a stored pattern's own row —
/// mirrors cp01's own direct-`ctx.pool` query discipline for
/// `write_access_rules` (no reason to grow `SecurityRulesAdminContext` for a
/// query this test suite alone needs).
async fn pattern_row(
    pool: &sqlx::PgPool,
    project_id: &str,
    collection_path_pattern: &str,
) -> Option<(i16, String, Option<String>, Option<String>, Option<String>)> {
    sqlx::query_as::<_, (i16, String, Option<String>, Option<String>, Option<String>)>(
        "SELECT ancestor_segment_count, literal_skeleton, leaf_variable, read_condition, write_condition \
         FROM access_rule_patterns WHERE project_id = $1 AND collection_path_pattern = $2",
    )
    .bind(project_id)
    .bind(collection_path_pattern)
    .fetch_optional(pool)
    .await
    .expect("query access_rule_patterns")
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-202 (WALKING SKELETON — Activity A): a nested-match-block
// multi-segment pattern is imported, decomposed, and stored active.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists, no pattern is yet defined for
///          `expeditions/*/journal_entries`
///   When:  Alex imports a `.rules` file containing
///          `match /expeditions/{expeditionId} { match
///          /journal_entries/{entryId} { allow read, write: if
///          request.auth.uid == resource.data.owner_id; } } }`
///   Then:  a pattern is stored and active for
///          `expeditions/{expeditionId}/journal_entries`, with both
///          `expeditionId` and `entryId` retained distinctly
///
/// AC-17-202
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-17-202
#[tokio::test]
async fn a_nested_match_block_multi_segment_pattern_is_imported_and_active() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EXPEDITIONS_NESTED_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(resp.status().as_u16(), 200, "AC-17-202: an in-scope multi-segment import must return 200");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    let imported = body["imported"].as_array().expect("imported must be an array");
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0]["collection_path"], "expeditions/{expeditionId}/journal_entries");
    assert_eq!(imported[0]["read_condition"], EXPEDITIONS_OWNER_CONDITION);
    assert_eq!(imported[0]["write_condition"], EXPEDITIONS_OWNER_CONDITION);

    let row = pattern_row(&ctx.pool, "trailmark-prod", "expeditions/{expeditionId}/journal_entries")
        .await
        .expect("AC-17-202: the pattern must be stored in access_rule_patterns");
    let (ancestor_segment_count, literal_skeleton, leaf_variable, read_condition, write_condition) = row;
    assert_eq!(ancestor_segment_count, 3);
    assert_eq!(literal_skeleton, "expeditions/journal_entries");
    assert_eq!(
        leaf_variable.as_deref(),
        Some("entryId"),
        "AC-17-204: the leaf wildcard must be retained by its own name, distinguishable from expeditionId"
    );
    assert_eq!(read_condition.as_deref(), Some(EXPEDITIONS_OWNER_CONDITION));
    assert_eq!(write_condition.as_deref(), Some(EXPEDITIONS_OWNER_CONDITION));
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-203: the flat multi-segment syntax form decomposes identically to
// the nested-match-block form.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: the nested-match-block file above has already been imported
///   When:  Alex imports a DIFFERENT file expressing the identical rule via
///          flat multi-segment syntax
///   Then:  the resulting stored pattern is indistinguishable from the one
///          the nested-match-block form produced — still exactly one row
///
/// AC-17-203
///
/// @driving_port @real-io @US-01 @AC-17-203
#[tokio::test]
async fn the_flat_multi_segment_form_decomposes_identically_to_the_nested_form() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let nested_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EXPEDITIONS_NESTED_RULES_FILE }))
        .send()
        .await
        .expect("nested import request failed");
    assert_eq!(nested_resp.status().as_u16(), 200);

    let flat_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EXPEDITIONS_FLAT_RULES_FILE }))
        .send()
        .await
        .expect("flat import request failed");
    assert_eq!(flat_resp.status().as_u16(), 200, "AC-17-203: the flat-syntax equivalent must also import cleanly");

    let row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM access_rule_patterns WHERE project_id = $1 AND collection_path_pattern = $2",
    )
    .bind("trailmark-prod")
    .bind("expeditions/{expeditionId}/journal_entries")
    .fetch_one(&ctx.pool)
    .await
    .expect("count access_rule_patterns rows");
    assert_eq!(
        row_count, 1,
        "AC-17-203: both syntax forms must decompose to the SAME stored pattern, never two rows"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-205: re-importing an unchanged multi-segment file is a no-op in
// effect.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `expeditions/{expeditionId}/journal_entries` already has an
///          active pattern from a prior import
///   When:  Alex imports the identical file again
///   Then:  the response confirms the pattern is unchanged and active, with
///          no duplicate row
///
/// AC-17-205
///
/// @error @driving_port @real-io @US-01 @AC-17-205
#[tokio::test]
async fn reimporting_an_unchanged_multi_segment_file_is_a_noop_in_effect() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    for _ in 0..2 {
        let resp = ctx
            .client
            .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
            .header("Cookie", &cookie)
            .json(&serde_json::json!({ "rules_file": EXPEDITIONS_NESTED_RULES_FILE }))
            .send()
            .await
            .expect("import request failed");
        assert_eq!(resp.status().as_u16(), 200, "AC-17-205: each import of the same file must succeed");
    }

    let row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM access_rule_patterns WHERE project_id = $1 AND collection_path_pattern = $2",
    )
    .bind("trailmark-prod")
    .bind("expeditions/{expeditionId}/journal_entries")
    .fetch_one(&ctx.pool)
    .await
    .expect("count access_rule_patterns rows");
    assert_eq!(row_count, 1, "AC-17-205: re-importing must not create a duplicate access_rule_patterns row");

    let row = pattern_row(&ctx.pool, "trailmark-prod", "expeditions/{expeditionId}/journal_entries")
        .await
        .expect("pattern must still be active");
    assert_eq!(row.3.as_deref(), Some(EXPEDITIONS_OWNER_CONDITION), "AC-17-205: the pattern must remain unchanged");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-206: a file mixing a multi-segment pattern with a 4a-shaped
// single-wildcard block imports both correctly.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists
///   When:  Alex imports a file containing both a multi-segment pattern AND
///          a 4a-shaped single-collection/single-wildcard block
///   Then:  both are stored and active, each in its own table
///
/// AC-17-206
///
/// @driving_port @real-io @US-01 @AC-17-206
#[tokio::test]
async fn a_file_mixing_a_multi_segment_pattern_with_a_4a_shaped_block_imports_both() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let mixed_rules_file = r#"
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

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": mixed_rules_file }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(resp.status().as_u16(), 200, "AC-17-206: a file mixing both shapes must import cleanly");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["imported"].as_array().expect("array").len(), 2);

    let pattern = pattern_row(&ctx.pool, "trailmark-prod", "expeditions/{expeditionId}/journal_entries")
        .await
        .expect("AC-17-206: the multi-segment pattern must be stored in access_rule_patterns");
    assert_eq!(pattern.3.as_deref(), Some(EXPEDITIONS_OWNER_CONDITION));

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
        "AC-17-206: the 4a-shaped single-wildcard block must still be stored in access_rules exactly as before"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Error/edge: missing admin credentials rejected (mirrors cp01's own
// AC-17-178 precedent).
// ─────────────────────────────────────────────────────────────────────────────

/// @error @driving_port @real-io @US-01
#[tokio::test]
async fn import_of_a_multi_segment_pattern_without_valid_admin_credentials_is_rejected() {
    let ctx = SecurityRulesAdminContext::new().await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        // No Cookie header at all.
        .json(&serde_json::json!({ "rules_file": EXPEDITIONS_NESTED_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "missing session must be rejected exactly as every other admin endpoint rejects it"
    );
}
