//! Slice 06 (US-06, ADR-030) — Untouched Collections and the Read/Write
//! Independence Guardrail Hold.
//!
//! This is this feature's own AC-17-14/15/16-equivalent regression
//! discipline (feature-delta.md US-06 Technical Notes: "mirrors
//! `security-rules`' own US-04/AC-17-14/15/16 discipline") — the single
//! highest-consequence regression risk in `security-rules-write-path`,
//! proved structurally (`handler.rs`'s write handlers query
//! `write_access_rules` ONLY, never `access_rules` — see
//! `handle_create_document`/`handle_update_document`/
//! `handle_delete_document`'s own "structural mechanism behind AC-17-43"
//! comments) AND by real gRPC calls (this file's first three scenarios) AND
//! by actually re-running the full pre-existing suite (this file's fourth,
//! marker-only scenario, AC-17-45).
//!
//! Acceptance criteria verified here (feature-delta.md US-06):
//!   AC-17-42: a collection with no write rule defined continues writing
//!             exactly as before this feature shipped.
//!   AC-17-43: a collection with a READ rule (from `security-rules`) but no
//!             WRITE rule of its own has fully unrestricted writes — the
//!             read rule never silently governs writes. This feature's
//!             single most important test.
//!   AC-17-44: a write rule defined for one collection has zero observable
//!             effect on any other collection's writes (sibling-collection
//!             isolation, mirrors `security-rules`' own AC-17-15).
//!   AC-17-45: the full pre-existing regression suite (113 `embyr-rs`/
//!             `client-auth` + 20 `security-rules` acceptance = 133
//!             scenarios) passes unmodified — a proof obligation satisfied
//!             by DELIVER's own GREEN-phase re-run of the full suite
//!             (mirroring `security-rules`' own `sr04` AC-17-16 discipline
//!             exactly), NOT by a self-contained Rust assertion — see the
//!             marker test below.
//!
//! Driving port: gRPC :8080 `CreateDocument`/`UpdateDocument`/
//! `DeleteDocument` (via `SecurityRulesFullContext` + this feature's own
//! `create_document`/`update_document`/`delete_document`/
//! `seed_write_access_rule_full` helpers — Pillar 3, real Postgres, real
//! gRPC, no mocks) for AC-17-42/43/44; the full pre-existing acceptance
//! suite (subprocess `cargo test`, out-of-band) for AC-17-45.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, delete_document, mint_client_identity_token, now_unix,
    seed_write_access_rule_full, string_field, update_document, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-42: a collection with no write rule defined continues writing
// exactly as before this feature shipped (create/update/delete, unauth'd —
// same behavior, parametrized by operation, mirrors AC-17-39's own
// three-operations-in-one-test shape).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `app_config` has never had a read or write rule defined
///   When:  an unauthenticated caller creates, updates, and deletes
///          `app_config` documents using only the existing project `api_key`
///   Then:  all three operations succeed exactly as they did before this
///          feature shipped
///
/// AC-17-42
///
/// @driving_port @real-io @US-06 @AC-17-42
#[tokio::test]
async fn a_collection_with_no_write_rule_defined_continues_writing_exactly_as_before() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp06-untouched").await;
    // Deliberately: zero rows in write_access_rules (or access_rules) for
    // this project at all — matches AC-17-42's own "never had any rule"
    // precondition.

    let mut fields = std::collections::HashMap::new();
    fields.insert("enabled".to_string(), string_field("true"));

    let create_resp =
        create_document(&ctx, "app_config", "swp06-untouched-doc", fields.clone(), None).await;
    let update_resp = update_document(
        &ctx,
        &ctx.document_resource_name("app_config", "config-doc"),
        fields,
        None,
    )
    .await;
    let delete_resp =
        delete_document(&ctx, &ctx.document_resource_name("app_config", "config-doc"), None)
            .await;

    assert!(
        create_resp.is_ok(),
        "AC-17-42: a create against a collection with no write rule must succeed unchanged: {:?}",
        create_resp.err()
    );
    assert!(
        update_resp.is_ok(),
        "AC-17-42: an update against a collection with no write rule must succeed unchanged: {:?}",
        update_resp.err()
    );
    assert!(
        delete_resp.is_ok(),
        "AC-17-42: a delete against a collection with no write rule must succeed unchanged: {:?}",
        delete_resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-43: a collection with a READ rule (from `security-rules`) but no
// WRITE rule of its own has fully unrestricted writes. THE central lock —
// given real scrutiny via two identity variants (anonymous AND a
// signed-in-but-non-owner caller who would specifically FAIL the read
// rule's own condition if it were mistakenly consulted), parametrized
// (Mandate 5) rather than duplicated as separate tests since both prove the
// identical behavior: the read rule's restrictive condition has zero
// bearing on writes.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an active READ rule
///          (`request.auth.uid == resource.data.owner_id`, seeded via
///          `security-rules`' own read-rule table) and NO write rule of its
///          own
///   When:  (a) a caller with NO identity at all, and (b) Dana Kim — a
///          signed-in caller who does NOT own the existing seed document
///          `maria-doc` and would therefore be DENIED by this exact
///          condition were it a read — each create, update, and delete a
///          `journal_entries` document
///   Then:  every operation succeeds for both callers — the read rule's
///          restrictive ownership condition never silently governs writes
///
/// AC-17-43
///
/// @driving_port @real-io @US-06 @AC-17-43
#[tokio::test]
async fn a_collection_with_a_read_rule_but_no_write_rule_has_fully_unrestricted_writes() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp06-readonly-independence").await;
    // A READ rule (security-rules' own table/mechanism) that would deny
    // Dana specifically, since she does not own `maria-doc`. No write rule
    // is ever defined for journal_entries in this test.
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("nobody"));

    for (label, identity) in [("anonymous", None), ("non-owner (Dana)", Some(danas_token.as_str()))]
    {
        let create_resp = create_document(
            &ctx,
            "journal_entries",
            &format!("swp06-readonly-independence-{label}"),
            fields.clone(),
            identity,
        )
        .await;
        let update_resp = update_document(
            &ctx,
            &ctx.document_resource_name("journal_entries", "maria-doc"),
            fields.clone(),
            identity,
        )
        .await;
        let delete_resp = delete_document(
            &ctx,
            &ctx.document_resource_name("journal_entries", "maria-doc"),
            identity,
        )
        .await;

        assert!(
            create_resp.is_ok(),
            "AC-17-43 ({label}): journal_entries's READ rule must have ZERO effect on create, \
             since no write rule is defined for it: {:?}",
            create_resp.err()
        );
        assert!(
            update_resp.is_ok(),
            "AC-17-43 ({label}): journal_entries's READ rule must have ZERO effect on update, \
             since no write rule is defined for it: {:?}",
            update_resp.err()
        );
        assert!(
            delete_resp.is_ok(),
            "AC-17-43 ({label}): journal_entries's READ rule must have ZERO effect on delete, \
             since no write rule is defined for it: {:?}",
            delete_resp.err()
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-44: a write rule defined for one collection has zero observable
// effect on any other collection's writes (sibling-collection isolation,
// mirrors `security-rules`' own AC-17-15).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an active WRITE rule requiring the caller
///          to own the referenced document; `trail_guides` in the same
///          project has none
///   When:  a caller with NO client-identity token at all — who would be
///          denied by journal_entries's own rule — creates, updates, and
///          deletes `trail_guides` documents
///   Then:  all three succeed, completely unaffected by journal_entries's
///          write rule
///
/// AC-17-44
///
/// @error @driving_port @real-io @US-06 @AC-17-44
#[tokio::test]
async fn a_write_rule_on_one_collection_has_zero_effect_on_a_sibling_collections_writes() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp06-sibling-isolation").await;
    seed_write_access_rule_full(
        &ctx,
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;
    // trail_guides has an active READ rule from security-rules fixtures?
    // No — deliberately none of any kind, isolating this test to proving
    // ONE sibling collection's WRITE rule has zero cross-collection effect.

    let mut fields = std::collections::HashMap::new();
    fields.insert("title".to_string(), string_field("Untouched by journal_entries"));

    let create_resp = create_document(
        &ctx,
        "trail_guides",
        "swp06-sibling-isolation-doc",
        fields.clone(),
        None,
    )
    .await;
    let update_resp = update_document(
        &ctx,
        &ctx.document_resource_name("trail_guides", "guide-doc"),
        fields,
        None,
    )
    .await;
    let delete_resp =
        delete_document(&ctx, &ctx.document_resource_name("trail_guides", "guide-doc"), None)
            .await;

    assert!(
        create_resp.is_ok(),
        "AC-17-44: journal_entries's write rule must have ZERO effect on trail_guides's create: {:?}",
        create_resp.err()
    );
    assert!(
        update_resp.is_ok(),
        "AC-17-44: journal_entries's write rule must have ZERO effect on trail_guides's update: {:?}",
        update_resp.err()
    );
    assert!(
        delete_resp.is_ok(),
        "AC-17-44: journal_entries's write rule must have ZERO effect on trail_guides's delete: {:?}",
        delete_resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-45: full 133-scenario pre-existing regression suite (proof
// obligation — marker/documentation test, NOT a self-contained assertion)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-45 is satisfied by literally re-running the full pre-existing
/// regression suite (113 `embyr-rs`/`client-auth` scenarios + 20
/// `security-rules` acceptance scenarios = 133) UNMODIFIED, alongside this
/// feature's own six `security_rules_write_path_*` acceptance test
/// binaries, against a build that includes this feature's changes — NOT by
/// any assertion this function's body could make. This mirrors
/// `security-rules`' own `sr04`
/// (`full_113_scenario_regression_suite_passes_unmodified`) discipline
/// exactly, extended to this feature's own additional six targets.
///
/// This function is a structural MARKER — it exists so `AC-17-45` is
/// discoverable in the test suite (`cargo test -- --list` surfaces it,
/// traceable to this AC) without pretending a 133+-scenario regression run
/// can be expressed as one in-process assertion. It is `#[ignore]`d
/// UNCONDITIONALLY (not part of the one-scenario-at-a-time RED progression
/// above) — DELIVER's own GREEN phase for this slice satisfies AC-17-45 by
/// re-running the command below, not by unskipping and passing this
/// function.
///
/// Command (from workspace root — the 19 non-`us_10_aws_secrets`
/// `embyr-rs`/`client-auth` targets, `security-rules`' own 5
/// `security_rules_sr0N` targets, and ALL SIX
/// `security_rules_write_path_*` targets built so far, Slices 01-06;
/// `us_10_aws_secrets` is excluded — confirmed pre-existing environmental
/// failure, unrelated to this feature):
/// ```text
/// cargo test -p embyr-server \
///   --test us_01_configure_sdk --test us_02_write_document --test us_03_read_document \
///   --test us_04_query_collection --test us_05_listen_realtime --test us_06_transactions \
///   --test us_07_provision_project --test us_08_monitor_project --test us_09_suspend_project \
///   --test us_11_gcp_secrets --test us_12_agent_backend --test us_13_browser_transport \
///   --test us_14_rate_limiting --test walking_skeleton \
///   --test client_auth_ca01_register_verification_credential \
///   --test client_auth_ca02_signin_and_reject_invalid_tokens \
///   --test client_auth_ca03_rotate_verification_credential \
///   --test client_auth_ca04_standalone_verify_debug_check \
///   --test client_auth_ca05_algorithm_confusion_defense \
///   --test security_rules_sr01_define_and_redefine_rule \
///   --test security_rules_sr02_signed_in_read_gated_by_rule \
///   --test security_rules_sr03_anonymous_session_evaluated_as_null_auth \
///   --test security_rules_sr04_untouched_collections_and_regression_suite_unaffected \
///   --test security_rules_sr05_simulate_rule_before_publish \
///   --test security_rules_write_path_define_write_rule \
///   --test security_rules_write_path_create_gated_by_write_rule \
///   --test security_rules_write_path_update_gated_by_both_resource_states \
///   --test security_rules_write_path_delete_gated_by_resource \
///   --test security_rules_write_path_anonymous_write_sessions \
///   --test security_rules_write_path_read_write_independence_and_regression
/// ```
///
/// AC-17-45
///
/// @driving_port @real-io @US-06 @AC-17-45 @regression-proof-obligation
#[test]
#[ignore = "structural marker, not an executable assertion — AC-17-45 is satisfied by \
            re-running the full 133-scenario pre-existing regression suite (plus this \
            feature's own 6 write-path targets) unmodified, per this function's own doc \
            comment"]
fn full_133_scenario_regression_suite_passes_unmodified() {
    unreachable!(
        "AC-17-45 is a proof obligation over EXTERNAL test binaries, not an in-process \
         assertion this function's body could make — see its doc comment for the exact \
         command DELIVER runs at GREEN for this slice."
    );
}
