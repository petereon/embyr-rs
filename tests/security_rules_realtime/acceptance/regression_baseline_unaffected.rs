//! Slice 07 (US-07, ADR-033) — Untouched Collections and the Full Regression
//! Baseline Are Unaffected.
//!
//! Pure regression/consistency-proof slice (per its own slice doc, § OUT
//! Scope): no new production code is expected here — Slices 01-06 already
//! built this feature's entire mechanism, structurally isolated inside
//! `crates/embyr-server/src/realtime/*` (confirmed directly by
//! `other_surfaces_unaffected.rs`, Slice 06's own `git diff` read). This file
//! proves ONLY AC-17-132 in-process (a project with no rules anywhere
//! retains fully unrestricted Listen content while gaining correctly-scoped
//! delivery, consistently across MULTIPLE collections).
//!
//! AC-17-130 + AC-17-131 (the 133-scenario FINALIZED `security-rules`
//! baseline, plus `security-rules-write-path`/`security-rules-query-path`/
//! `security-rules-collection-group-rules`'s own delivered scenarios) are a
//! proof obligation over EXTERNAL test binaries, not expressible as an
//! in-process Rust assertion — satisfied by an out-of-band full regression
//! sweep reported alongside this slice's own commit, mirroring
//! `security_rules_collection_group_rules::acceptance::
//! regression_baseline_unaffected`'s (Slice 06) identical discipline. NOT
//! represented by any function in this file, not even an `#[ignore]`d
//! marker.
//!
//! AC-17-133's own specific fixtures, identified by direct read of
//! `tests/acceptance/us_05_listen_realtime.rs` (the base walking-skeleton's
//! own resume-token/keepalive/RESET Listen mechanics coverage):
//!   - `reconnect_with_resume_token_delivers_only_delta` (resume-token delta)
//!   - `stale_resume_token_triggers_full_resnapshot_not_error` (resume-token
//!     staleness)
//!   - `idle_listen_stream_receives_no_change_keep_alive` (keepalive)
//!   - `slow_consumer_overflow_triggers_reset` (RESET)
//! all four are re-run, unmodified, as part of the out-of-band sweep — no
//! genuine gap was found in what they already cover, so no new test is added
//! here for AC-17-133 (mirroring this slice's own doc's explicit
//! expectation).
//!
//! Driving port: gRPC :8080 `Listen`/`CreateDocument`, via the real
//! production composition root (`SecurityRulesFullContext`), real Postgres
//! NOTIFY fan-out, no mocks.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    collect_initial_snapshot, create_document, open_listen_stream, string_field,
    try_recv_live_event, CapturedEvent, SecurityRulesFullContext,
};

use embyr_proto::firestore::ListenResponse;
use std::{collections::HashMap, time::Duration};

/// Wait a bounded, short window for a live event on `stream` that must NEVER
/// arrive — the cross-collection non-leak half of AC-17-132's own proof.
/// Mirrors `try_recv_live_event`'s own "None means nothing arrived" contract
/// (`common/mod.rs`), named here for the specific negative-assertion role
/// this file repeats across all three collection pairs.
async fn assert_no_cross_collection_leak(
    stream: &mut tonic::Streaming<ListenResponse>,
    leaking_collection: &str,
    victim_collection: &str,
) {
    let event = try_recv_live_event(stream, Duration::from_millis(800)).await;
    assert!(
        event.is_none(),
        "AC-17-132: {victim_collection}'s subscriber must never receive {leaking_collection}'s \
         live event — scoping (US-01) must hold identically in a ruleless project too, got: \
         {event:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-132: a project with NO rules anywhere (zero rows in access_rules,
// write_access_rules, AND group_access_rules) retains fully unrestricted
// Listen content — initial snapshot AND live events — while gaining
// correctly-scoped delivery (Slice 01's own fix), consistently across
// MULTIPLE collections in that same ruleless project.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: a fresh project with zero rows in `access_rules`,
///          `write_access_rules`, and `group_access_rules` —
///          `SecurityRulesFullContext::new` seeds only documents, never a
///          rule of any kind, so this is the structural "ruleless project"
///          case by construction
///   When:  three separate Listen subscriptions are opened, one each for
///          `journal_entries`, `trail_guides`, and `app_config`
///   Then:  each subscription's initial snapshot delivers its own
///          collection's pre-seeded document(s) in full, unrestricted by
///          any identity (no client-identity token is presented at all)
///   When:  a new document is created in EACH of the three collections in
///          turn
///   Then:  each collection's own subscriber receives that collection's own
///          live `Changed` event unrestricted, and NEITHER of the other two
///          subscribers ever receives it — proving content stays fully
///          unrestricted while scope (US-01) stays correct, consistently,
///          across all three collections in the same ruleless project
///
/// AC-17-132
///
/// @driving_port @real-io @US-07 @AC-17-132
#[tokio::test]
async fn ruleless_project_retains_unrestricted_content_with_correctly_scoped_delivery_across_multiple_collections()
{
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srrt07-ruleless").await;
    // Deliberately: zero rows in access_rules, write_access_rules, and
    // group_access_rules for this project — SecurityRulesFullContext::new
    // seeds only documents (journal_entries x2, trail_guides x1,
    // app_config x1), never a rule of any kind.

    let mut journal_stream = open_listen_stream(&ctx, "journal_entries").await;
    let mut guides_stream = open_listen_stream(&ctx, "trail_guides").await;
    let mut config_stream = open_listen_stream(&ctx, "app_config").await;

    // --- initial snapshot: each collection's own pre-seeded content is
    // delivered in full, unrestricted (no identity token presented).
    let journal_snapshot = collect_initial_snapshot(&mut journal_stream).await;
    let guides_snapshot = collect_initial_snapshot(&mut guides_stream).await;
    let config_snapshot = collect_initial_snapshot(&mut config_stream).await;

    assert_eq!(
        journal_snapshot.len(),
        2,
        "AC-17-132: journal_entries' own 2 pre-seeded documents must be delivered unrestricted, \
         got: {journal_snapshot:?}"
    );
    assert_eq!(
        guides_snapshot.len(),
        1,
        "AC-17-132: trail_guides' own 1 pre-seeded document must be delivered unrestricted, got: \
         {guides_snapshot:?}"
    );
    assert_eq!(
        config_snapshot.len(),
        1,
        "AC-17-132: app_config's own 1 pre-seeded document must be delivered unrestricted, got: \
         {config_snapshot:?}"
    );

    // --- live delivery: journal_entries' own write reaches ONLY its own
    // subscriber, content unrestricted.
    create_document(
        &ctx,
        "journal_entries",
        "srrt07-ruleless-journal-doc",
        HashMap::from([("note".to_string(), string_field("ruleless content check"))]),
        None,
    )
    .await
    .expect("create against a ruleless journal_entries should succeed unrestricted");
    let journal_expected_name = format!(
        "projects/{}/databases/(default)/documents/journal_entries/srrt07-ruleless-journal-doc",
        ctx.project_id
    );
    let journal_event = try_recv_live_event(&mut journal_stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(&journal_event, Some(CapturedEvent::Changed { document_name }) if document_name == &journal_expected_name),
        "AC-17-132: journal_entries' own subscriber must receive its own collection's Changed \
         event unrestricted, got: {journal_event:?}"
    );
    assert_no_cross_collection_leak(&mut guides_stream, "journal_entries", "trail_guides").await;
    assert_no_cross_collection_leak(&mut config_stream, "journal_entries", "app_config").await;

    // --- live delivery: trail_guides' own write reaches ONLY its own
    // subscriber, content unrestricted.
    create_document(
        &ctx,
        "trail_guides",
        "srrt07-ruleless-guides-doc",
        HashMap::from([("note".to_string(), string_field("ruleless content check"))]),
        None,
    )
    .await
    .expect("create against a ruleless trail_guides should succeed unrestricted");
    let guides_expected_name = format!(
        "projects/{}/databases/(default)/documents/trail_guides/srrt07-ruleless-guides-doc",
        ctx.project_id
    );
    let guides_event = try_recv_live_event(&mut guides_stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(&guides_event, Some(CapturedEvent::Changed { document_name }) if document_name == &guides_expected_name),
        "AC-17-132: trail_guides' own subscriber must receive its own collection's Changed event \
         unrestricted, got: {guides_event:?}"
    );
    assert_no_cross_collection_leak(&mut journal_stream, "trail_guides", "journal_entries").await;
    assert_no_cross_collection_leak(&mut config_stream, "trail_guides", "app_config").await;

    // --- live delivery: app_config's own write reaches ONLY its own
    // subscriber, content unrestricted.
    create_document(
        &ctx,
        "app_config",
        "srrt07-ruleless-config-doc",
        HashMap::from([("note".to_string(), string_field("ruleless content check"))]),
        None,
    )
    .await
    .expect("create against a ruleless app_config should succeed unrestricted");
    let config_expected_name = format!(
        "projects/{}/databases/(default)/documents/app_config/srrt07-ruleless-config-doc",
        ctx.project_id
    );
    let config_event = try_recv_live_event(&mut config_stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(&config_event, Some(CapturedEvent::Changed { document_name }) if document_name == &config_expected_name),
        "AC-17-132: app_config's own subscriber must receive its own collection's Changed event \
         unrestricted, got: {config_event:?}"
    );
    assert_no_cross_collection_leak(&mut journal_stream, "app_config", "journal_entries").await;
    assert_no_cross_collection_leak(&mut guides_stream, "app_config", "trail_guides").await;
}
