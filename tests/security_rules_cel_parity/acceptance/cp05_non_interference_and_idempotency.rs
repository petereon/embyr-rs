//! CP05 (Slice 05, US-05, Release 1) — Importing Protects Only the
//! Collections Named, and Re-Importing Is Idempotent.
//!
//! This is a PROOF slice (feature-delta.md § OUT Scope: "New production
//! logic" is not expected) — these acceptance tests prove real,
//! already-built behavior (Slices 01-04) holds, mirroring
//! `security-rules`'s own US-04/AC-17-14/15/16 discipline and
//! `security-rules-query-path`'s own sqp06 marker-test pattern for the
//! full-suite regression obligation.
//!
//! Acceptance criteria verified here (feature-delta.md US-05):
//!   AC-17-194: importing a file has zero observable effect on any
//!              collection's rule not named in that file, whether that rule
//!              was defined via import or via the existing JSON API.
//!   AC-17-195: re-importing a byte-identical file produces no duplicate
//!              history entry — the underlying `upsert_*` call's own
//!              idempotent-redefine semantics are reused, not bypassed.
//!   AC-17-196: the full pre-existing regression suite (133+ scenarios
//!              across all 7 prior JOB-17 epics) passes unmodified — a
//!              proof obligation satisfied by DELIVER's own GREEN-phase
//!              re-run of the full suite, NOT by a self-contained Rust
//!              assertion (see the marker test below).
//!   AC-17-197: mixing import-authored and JSON-API-authored rules within
//!              the same project produces no observable inconsistency —
//!              both are the identical underlying row shape, retrievable
//!              identically via the existing history endpoint.
//!
//! Driving port: Admin HTTP :9090 (`SecurityRulesAdminContext`, real
//! `build_admin_router` composition root — Pillar 3, reused unchanged from
//! Slice 01's own cp01 fixture).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesAdminContext;

const PROFILES_AND_JOURNAL_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /profiles/{userId} {
      allow read, write: if request.auth.uid == userId;
    }
    match /journal_entries {
      allow read: if request.auth.uid == resource.data.owner_id;
    }
  }
}
"#;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-194: a hand-defined (JSON API) rule on a collection NOT named in an
// import is completely unaffected by that import.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trail_guides` has an active rule defined via the existing
///          `POST .../access_rules` JSON API (not import)
///   When:  Alex imports a file naming only `profiles` and `journal_entries`
///   Then:  `trail_guides`'s rule remains exactly as it was — same
///          condition, same `updated_at`, and no new history entry
///
/// AC-17-194
///
/// @driving_port @real-io @US-05 @AC-17-194
#[tokio::test]
async fn importing_a_file_has_zero_effect_on_a_json_api_defined_rule_for_an_unnamed_collection() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    // trail_guides defined via the EXISTING JSON API, not import.
    let define_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "trail_guides",
            "condition": "true",
        }))
        .send()
        .await
        .expect("define request failed");
    assert_eq!(define_resp.status().as_u16(), 200);

    let before_updated_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT updated_at FROM access_rules WHERE project_id = $1 AND collection_path = $2",
    )
    .bind("trailmark-prod")
    .bind("trail_guides")
    .fetch_one(&ctx.pool)
    .await
    .expect("trail_guides row must exist before import");

    // Import a file that names other collections entirely — trail_guides
    // never appears in it.
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": PROFILES_AND_JOURNAL_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(resp.status().as_u16(), 200, "AC-17-194: the import itself must succeed");

    let after_condition = ctx.access_rule_condition_source("trailmark-prod", "trail_guides").await;
    let after_updated_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT updated_at FROM access_rules WHERE project_id = $1 AND collection_path = $2",
    )
    .bind("trailmark-prod")
    .bind("trail_guides")
    .fetch_one(&ctx.pool)
    .await
    .expect("trail_guides row must still exist after import");

    assert_eq!(
        after_condition.as_deref(),
        Some("true"),
        "AC-17-194: trail_guides's condition must be byte-for-byte unchanged by an unrelated import"
    );
    assert_eq!(
        after_updated_at, before_updated_at,
        "AC-17-194: trail_guides's row must not even be touched (updated_at unchanged) by an import naming other collections"
    );

    let history_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM access_rule_history WHERE project_id = $1 AND collection_path = $2",
    )
    .bind("trailmark-prod")
    .bind("trail_guides")
    .fetch_one(&ctx.pool)
    .await
    .expect("count trail_guides history rows");
    assert_eq!(
        history_count, 1,
        "AC-17-194: an import naming other collections must not add a history entry for trail_guides"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-195: re-importing a byte-identical file produces no duplicate
// history entry.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `profiles` already has an active rule from a prior import
///   When:  Alex imports the byte-identical file again
///   Then:  `profiles`'s rule is confirmed unchanged and exactly ONE history
///          entry exists for it — no duplicate entry for the no-op
///          re-import (same for its write-rule counterpart)
///
/// AC-17-195
///
/// @error @driving_port @real-io @US-05 @AC-17-195
#[tokio::test]
async fn reimporting_a_byte_identical_file_produces_exactly_one_history_entry_not_two() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    for _ in 0..2 {
        let resp = ctx
            .client
            .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
            .header("Cookie", &cookie)
            .json(&serde_json::json!({ "rules_file": PROFILES_AND_JOURNAL_RULES_FILE }))
            .send()
            .await
            .expect("import request failed");
        assert_eq!(resp.status().as_u16(), 200, "AC-17-195: each import of the identical file must succeed");
    }

    let read_history_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM access_rule_history WHERE project_id = $1 AND collection_path = $2",
    )
    .bind("trailmark-prod")
    .bind("profiles")
    .fetch_one(&ctx.pool)
    .await
    .expect("count profiles read-history rows");
    assert_eq!(
        read_history_count, 1,
        "AC-17-195: re-importing an unchanged file must not create a duplicate access_rule_history entry"
    );

    let write_history_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM write_access_rule_history WHERE project_id = $1 AND collection_path = $2",
    )
    .bind("trailmark-prod")
    .bind("profiles")
    .fetch_one(&ctx.pool)
    .await
    .expect("count profiles write-history rows");
    assert_eq!(
        write_history_count, 1,
        "AC-17-195: re-importing an unchanged file must not create a duplicate write_access_rule_history entry"
    );

    let stored = ctx.access_rule_condition_source("trailmark-prod", "profiles").await;
    assert_eq!(
        stored.as_deref(),
        Some("request.auth.uid == request.path.userId"),
        "AC-17-195: the rule must remain active and unchanged after re-import"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-197: mixing import-authored and JSON-API-authored rules in the same
// project is observably consistent — identical row shape either way.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trailmark-prod` has one collection (`trail_guides`) defined via
///          the JSON API and, after an import, another (`journal_entries`)
///          defined via import
///   When:  Alex retrieves each collection's history via the existing,
///          identical `GET .../access_rules/:collection_path/history` action
///   Then:  both are retrievable with the identical response shape (id,
///          condition, actor_account_id, captured_at) — no observable
///          inconsistency between the two authoring paths
///
/// AC-17-197
///
/// @driving_port @real-io @US-05 @AC-17-197
#[tokio::test]
async fn mixing_import_and_json_api_authored_rules_produces_no_observable_inconsistency() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let define_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "trail_guides",
            "condition": "true",
        }))
        .send()
        .await
        .expect("define request failed");
    assert_eq!(define_resp.status().as_u16(), 200);

    let import_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": PROFILES_AND_JOURNAL_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(import_resp.status().as_u16(), 200);

    for collection in ["trail_guides", "journal_entries"] {
        let history_resp = ctx
            .client
            .get(ctx.url(&format!(
                "/admin/v1/projects/trailmark-prod/access_rules/{collection}/history"
            )))
            .header("Cookie", &cookie)
            .send()
            .await
            .expect("history request failed");
        assert_eq!(
            history_resp.status().as_u16(),
            200,
            "AC-17-197: history retrieval must succeed identically for {collection}, regardless of authoring path"
        );
        let body: serde_json::Value = history_resp.json().await.expect("history response must be JSON");
        let history = body["history"].as_array().expect("history must be an array");
        assert_eq!(
            history.len(),
            1,
            "AC-17-197: {collection} must have exactly one history entry"
        );
        let entry = &history[0];
        assert!(
            entry["id"].is_number() && entry["condition"].is_string()
                && entry["actor_account_id"].is_string() && entry["captured_at"].is_string(),
            "AC-17-197: {collection}'s history entry must have the identical shape regardless of \
             whether the rule was defined via import or the JSON API, got {entry:?}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-196: full cumulative pre-existing regression suite (proof obligation
// — marker/documentation test, NOT a self-contained assertion). Mirrors
// security-rules-query-path's sqp06 `full_198_scenario_regression_suite_
// passes_unmodified` pattern exactly.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-196 is satisfied by literally re-running the full pre-existing
/// `security_rules_*` regression suite (all targets across the 7 prior
/// JOB-17 sibling epics: `security-rules`, `-write-path`, `-query-path`,
/// `-realtime`, `-collection-group-rules`, `-operations`, plus this
/// feature's own cp01-cp04) UNMODIFIED, alongside this slice's own new
/// scenarios above, against a build that includes this feature's changes —
/// NOT by any assertion this function's body could make.
///
/// This function is a structural MARKER — it exists so `AC-17-196` is
/// discoverable in the test suite (`cargo test -- --list` surfaces it,
/// traceable to this AC) without pretending a 44+-target regression run can
/// be expressed as one in-process assertion. It is `#[ignore]`d
/// UNCONDITIONALLY — DELIVER's own GREEN phase for this slice satisfies
/// AC-17-196 by re-running the full `security_rules_*` cargo test targets,
/// not by unskipping and passing this function.
///
/// AC-17-196
///
/// @driving_port @real-io @US-05 @AC-17-196 @regression-proof-obligation
#[test]
#[ignore = "structural marker, not an executable assertion — AC-17-196 is satisfied by \
            re-running the full pre-existing security_rules_* regression suite (all 7 sibling \
            epics + this feature's own cp01-cp04) unmodified, per this function's own doc \
            comment"]
fn full_security_rules_regression_suite_passes_unmodified() {
    unreachable!(
        "AC-17-196 is a proof obligation over EXTERNAL test binaries, not an in-process \
         assertion this function's body could make — see its doc comment for what DELIVER runs \
         at GREEN for this slice."
    );
}
