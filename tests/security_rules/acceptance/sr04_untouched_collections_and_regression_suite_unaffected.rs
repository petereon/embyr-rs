//! SR04 (Slice 04, Walking Skeleton, US-04, Release 1) — A Collection With
//! No Rule Defined Keeps Reading Exactly as Before.
//!
//! This is this feature's AC-16-08-equivalent regression discipline
//! (feature-delta.md § Prioritization, Slice 04 rationale) — the single
//! highest-consequence regression risk in `security-rules`, proved
//! structurally (ADR-029 § Structural no-rule-defined guardrail) AND by
//! actually re-running the full pre-existing suite (this file's third
//! scenario, AC-17-16).
//!
//! Acceptance criteria verified here (feature-delta.md US-04):
//!   AC-17-14: a collection with no rule defined behaves identically to
//!             pre-feature behavior — unrestricted by identity, gated only
//!             by the existing `api_key`.
//!   AC-17-15: a rule defined for one collection has zero observable
//!             effect on any other collection in the same project that has
//!             no rule of its own.
//!   AC-17-16: the full 113-scenario pre-existing regression suite (72
//!             `embyr-rs` + 41 `client-auth`) passes unmodified — a proof
//!             obligation satisfied by DELIVER's own GREEN-phase re-run of
//!             the full suite (mirroring `client-auth`'s own AC-16-08(a)
//!             discipline, feature-delta.md client-auth § DISTILL
//!             Pre-DELIVER Gate), NOT by a self-contained Rust assertion —
//!             see the marker test below.
//!
//! Driving port: gRPC :8080 `GetDocument` (via `SecurityRulesFullContext`,
//! Pillar 3) for AC-17-14/15; the full pre-existing acceptance suite
//! (subprocess `cargo test`, out-of-band) for AC-17-16.
//!
//! Error ratio: 2 edge/guardrail (AC-17-15 isolation, AC-17-16 full-suite
//! proof) out of 3 = 67%.
//!
//! One scenario enabled at a time (RED scaffold discipline, ADR-025 D2).
//! AC-17-16's marker test is `#[ignore]`d unconditionally (see its own doc
//! comment) — it is never meant to execute as a Rust assertion.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesFullContext;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-14: a collection that has never had a rule defined is unaffected
// (WALKING SKELETON — this file's primary happy path, per the DISTILL
// dispatch instructions' Slices 01-04 WS treatment)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — this file's own opening Given; scenario 2 below
/// reuses this file's zero-rules-on-app_config precondition shape, per
/// Pillar 2):
///   Given: `app_config` has never had a rule defined
///   When:  any caller calls `getDoc()` on an `app_config` document, using
///          only the existing project `api_key`
///   Then:  the read succeeds exactly as it did before this feature shipped
///
/// AC-17-14
///
/// @walking_skeleton @driving_port @real-io @US-04 @AC-17-14
#[tokio::test]
async fn a_collection_that_has_never_had_a_rule_defined_is_unaffected_by_this_feature() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sr04-untouched").await;
    // Deliberately: zero rows in access_rules for this project at all — no
    // rule seeded on ANY collection, matching the structural no-rule
    // -defined guardrail's own precondition (ADR-029).

    // No client-identity header — this scenario is specifically about the
    // existing api_key-only path being unaffected (AC-17-14's own wording:
    // "gated only by the existing api_key").
    let resp = ctx
        .get_document(&ctx.document_resource_name("app_config", "config-doc"), None)
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-14: a collection with no rule defined must read exactly as it did before \
         this feature shipped: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-15: a rule on one collection does not affect a sibling collection
// without its own rule (error/edge — isolation guardrail)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-15
///
/// @error @driving_port @real-io @US-04 @AC-17-15
#[tokio::test]
async fn a_rule_on_one_collection_does_not_affect_a_sibling_collection_without_its_own_rule() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sr04-isolation").await;
    // journal_entries HAS a strict ownership rule; trail_guides has NONE.
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    // A caller with NO matching rule-satisfying identity for
    // journal_entries (no client-identity header presented at all) reads
    // trail_guides — a DIFFERENT collection with no rule of its own —
    // which must succeed, completely unaffected by journal_entries's rule.
    let resp = ctx
        .get_document(&ctx.document_resource_name("trail_guides", "guide-doc"), None)
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-15: journal_entries's rule must have ZERO observable effect on trail_guides, \
         which has no rule of its own: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-16: full 113-scenario pre-existing regression suite (proof
// obligation — marker/documentation test, NOT a self-contained assertion)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-16 is satisfied by literally re-running the full pre-existing
/// regression suite (72 `embyr-rs` + 41 `client-auth` = 113 scenarios)
/// UNMODIFIED against a build that includes this feature's changes — NOT
/// by any assertion this function's body could make. This mirrors
/// `client-auth`'s own AC-16-08(a) discipline exactly: that DISTILL wave
/// documented the regression-suite command in a module doc comment and ran
/// it out-of-band (see `docs/feature/client-auth/feature-delta.md` §
/// DISTILL Pre-DELIVER Gate), rather than writing a single Rust test that
/// asserts over other test binaries — this feature's own DISTILL run does
/// the identical thing; see `docs/feature/security-rules/distill/red-classification.md`
/// for THIS wave's captured run.
///
/// This function is a structural MARKER (Mandate 7's "RED-ready scaffold"
/// concept applied to a proof-obligation rather than an implementation
/// gap) — it exists so `AC-17-16` is discoverable in the test suite
/// (`cargo test -- --list` surfaces it, traceable to this AC) without
/// pretending a 113-scenario regression run can be expressed as one
/// in-process assertion. It is `#[ignore]`d UNCONDITIONALLY (not part of
/// the one-scenario-at-a-time RED progression above) — DELIVER's own
/// GREEN phase for this slice satisfies AC-17-16 by re-running the command
/// below, not by unskipping and passing this function.
///
/// Command (from workspace root — the full 113-scenario suite plus this
/// feature's own new `security_rules_sr0N_*` binaries):
/// ```text
/// cargo test -p embyr-server \
///   --test us_01_configure_sdk --test us_02_write_document --test us_03_read_document \
///   --test us_04_query_collection --test us_05_listen_realtime --test us_06_transactions \
///   --test us_07_provision_project --test us_08_monitor_project --test us_09_suspend_project \
///   --test us_10_aws_secrets --test us_11_gcp_secrets --test us_12_agent_backend \
///   --test us_13_browser_transport --test us_14_rate_limiting --test walking_skeleton \
///   --test client_auth_ca01_register_verification_credential \
///   --test client_auth_ca02_signin_and_reject_invalid_tokens \
///   --test client_auth_ca03_rotate_verification_credential \
///   --test client_auth_ca04_standalone_verify_debug_check \
///   --test client_auth_ca05_algorithm_confusion_defense
/// ```
///
/// AC-17-16
///
/// @driving_port @real-io @US-04 @AC-17-16 @regression-proof-obligation
#[test]
#[ignore = "structural marker, not an executable assertion — AC-17-16 is satisfied by \
            re-running the full 113-scenario pre-existing regression suite unmodified, \
            per this function's own doc comment; see docs/feature/security-rules/distill/red-classification.md"]
fn full_113_scenario_regression_suite_passes_unmodified() {
    unreachable!(
        "AC-17-16 is a proof obligation over EXTERNAL test binaries, not an in-process \
         assertion this function's body could make — see its doc comment for the exact \
         command DISTILL already ran once and DELIVER re-runs at GREEN for this slice."
    );
}
