//! security-rules-collection-group-rules Slice 06 (US-06, ADR-032) —
//! Untouched Collections and the Full Regression Baseline Are Unaffected.
//!
//! Pure regression/consistency proof slice (per its own slice doc, § OUT
//! Scope): NO production code change is expected here — Slices 01-05
//! already built the disjoint-table, disjoint-call-site-branch design
//! (Resolution 1) and its "no group rule ⇒ reject outright" default
//! (Resolution 2). This slice's own DELIVER confirms both claims hold:
//! (a) additively, over the full pre-existing regression baseline, and
//! (b) consistently, across collection ids this feature's own prior
//! slices never touched.
//!
//! Acceptance criteria verified here (feature-delta.md
//! `security-rules-collection-group-rules` US-06,
//! slice-06-regression-baseline-unaffected.md):
//!   AC-17-97 + AC-17-98: satisfied by literally re-running the 133-scenario
//!             FINALIZED `security-rules` baseline plus
//!             `security-rules-query-path`'s own delivered scenarios
//!             UNMODIFIED, against a build that includes this feature's
//!             changes — a proof obligation over EXTERNAL test binaries, not
//!             expressible as an in-process Rust assertion. Per this slice's
//!             own DELIVER instructions, NOT represented by any function in
//!             this file (not even an `#[ignore]`d marker) — satisfied by
//!             the out-of-band regression sweep reported alongside this
//!             slice's commit.
//!   AC-17-99: `app_config` (no rule of any kind — exact-path, write, or
//!             group) retains fully unrestricted `GetDocument`/write/
//!             non-group-`RunQuery` behavior; only its group-query behavior
//!             is newly gated. `GetDocument` and non-group `RunQuery`
//!             coverage on `app_config` already exists — Slice 04's own
//!             `non_group_query_and_get_document_remain_unrestricted_on_the_same_ungoverned_collection`
//!             (`reject_ungoverned_group_query.rs`) proves both, alongside
//!             the same collection id's group-query rejection, in one test.
//!             The genuine gap this slice closes is write-path coverage on
//!             `app_config` specifically — no prior slice in this feature
//!             exercises `CreateDocument`/`UpdateDocument`/`DeleteDocument`
//!             against `app_config`.
//!   AC-17-100: group-query rejection is consistent across MULTIPLE
//!             different ungoverned collection ids in the SAME project —
//!             `app_config` and `trail_guides` (both already referenced by
//!             this feature's own prior slices) plus `trip_photos` (a THIRD
//!             collection id never referenced anywhere in this feature's
//!             own prior slices) — all reject with the byte-identical
//!             `[GROUP_RULE_NOT_DEFINED]` reason, no exemptions.
//!
//! Driving port: gRPC :8080 `CreateDocument`/`UpdateDocument`/
//! `DeleteDocument`/`RunQuery` via the real production composition root
//! (`SecurityRulesFullContext`).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{create_document, delete_document, run_query, string_field, SecurityRulesFullContext};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-99: app_config's write-path remains fully unrestricted (no write
// rule, no exact-path rule, no group rule — this feature's own genuine
// coverage gap); its group-query behavior alone is newly gated, proven in
// the same test as the direct contrast the AC itself describes.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `app_config` has never had any rule of any kind
///   When:  an anonymous caller creates, updates, and deletes a document in
///          `app_config`
///   Then:  every write succeeds unrestricted
///   When:  the same caller issues a `collectionGroup('app_config')` query
///   Then:  it alone is rejected — a NEW capability being gated, not a
///          regression of the write-path behavior just proven unrestricted
///
/// AC-17-99
///
/// @driving_port @real-io @US-06 @AC-17-99
#[tokio::test]
async fn app_configs_write_path_remains_unrestricted_while_only_its_group_query_behavior_is_newly_gated(
) {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-scgr06-appconfig-writes").await;
    // Deliberately: no exact-path rule, no write rule, no group rule for
    // app_config in this project at all.

    let mut fields = HashMap::new();
    fields.insert("enabled".to_string(), string_field("true"));
    create_document(&ctx, "app_config", "scgr06-write-doc", fields.clone(), None)
        .await
        .expect(
            "AC-17-99: CreateDocument on app_config must succeed unrestricted — no rule of any \
             kind is defined for this collection",
        );
    let resource_name = format!(
        "projects/{}/databases/(default)/documents/app_config/scgr06-write-doc",
        ctx.project_id
    );

    let mut updated_fields = HashMap::new();
    updated_fields.insert("enabled".to_string(), string_field("false"));
    common::update_document(&ctx, &resource_name, updated_fields, None)
        .await
        .expect("AC-17-99: UpdateDocument on app_config must succeed unrestricted");

    delete_document(&ctx, &resource_name, None)
        .await
        .expect("AC-17-99: DeleteDocument on app_config must succeed unrestricted");

    // Contrast: only app_config's group-query behavior is newly gated — a
    // new capability being gated, never a regression of the unrestricted
    // write-path behavior just proven above.
    let group_result = run_query(&ctx, "app_config", true, &[], None).await;
    let group_err = group_result.expect_err(
        "AC-17-99: app_config's collection-group query must still be rejected — this feature's \
         new default gates ONLY all_descendants=true querying, never writes",
    );
    assert_eq!(group_err.code(), tonic::Code::PermissionDenied);
    assert!(
        group_err.message().contains("[GROUP_RULE_NOT_DEFINED]"),
        "AC-17-99: rejection must name the distinguishable no-group-rule reason, got: {}",
        group_err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-100: group-query rejection is consistent across multiple ungoverned
// collection ids in the same project — no exemptions, no special-cased
// collection names.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: a project with no group rule defined for any collection id —
///          `app_config` and `trail_guides` (both used by this feature's
///          own prior slices) and `trip_photos` (never referenced anywhere
///          in this feature's own prior slices)
///   When:  a collection-group query is issued against each of the three
///   Then:  all three are rejected identically, with the byte-identical
///          `[GROUP_RULE_NOT_DEFINED]` reason — no collection id is
///          silently exempted
///
/// AC-17-100
///
/// @error @driving_port @real-io @US-06 @AC-17-100
#[tokio::test]
async fn group_query_rejection_is_consistent_across_multiple_ungoverned_collection_ids_in_the_same_project(
) {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-scgr06-consistency").await;
    // Deliberately: zero rows in group_access_rules for this project at all.

    let mut messages = Vec::new();
    for collection_id in ["app_config", "trail_guides", "trip_photos"] {
        let result = run_query(&ctx, collection_id, true, &[], None).await;
        let err = result.expect_err(&format!(
            "AC-17-100: a collection-group query against ungoverned collection id \
             '{collection_id}' must be rejected outright, no exemptions"
        ));
        assert_eq!(
            err.code(),
            tonic::Code::PermissionDenied,
            "AC-17-100: rejection for '{collection_id}' must be attributable to the missing \
             group rule (PermissionDenied), got {:?}: {}",
            err.code(),
            err.message()
        );
        assert!(
            err.message().contains("[GROUP_RULE_NOT_DEFINED]"),
            "AC-17-100: rejection for '{collection_id}' must name the distinguishable \
             no-group-rule reason, got: {}",
            err.message()
        );
        messages.push(err.message().to_string());
    }

    assert_eq!(
        messages[0], messages[1],
        "AC-17-100: rejection must be byte-identical across ungoverned collection ids — no \
         exemptions, no special-cased collection names"
    );
    assert_eq!(
        messages[1], messages[2],
        "AC-17-100: rejection must be byte-identical across ungoverned collection ids — no \
         exemptions, no special-cased collection names"
    );
}
