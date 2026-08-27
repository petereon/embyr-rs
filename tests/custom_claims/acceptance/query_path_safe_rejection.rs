//! Slice 05 (US-05, ADR-034) — A Claim-Referencing Rule Fails Safely, Not
//! Silently, Against Query-Path Surfaces.
//!
//! **Pure proof-obligation slice**: per ADR-034 § Decision — Query-Path
//! Safety, `decompose_decidable`'s existing wildcard arm (`_ =>
//! Err(Undecidable)`, `crates/embyr-core/src/access_control/mod.rs`) is
//! ALREADY the sole path to `RejectedUnsupportedRuleShape` for any
//! `Condition::Compare` involving `Operand::AuthTokenClaim` — a Rust
//! exhaustive-match guarantee (the new operand variant cannot structurally
//! match `decompose_decidable`'s named arms), not a new rejection branch.
//! This file proves that already-correct behavior end-to-end through the
//! real `RunQuery` (non-group + group)/`Listen` composition. Zero
//! `embyr_core::access_control` code is expected to change (verified via
//! `git diff --stat` after GREEN, not merely asserted here).
//!
//! Acceptance criteria verified here (feature-delta.md US-05):
//!   AC-17-148: a RunQuery (non-group AND group) against a collection whose
//!              rule references a claim is rejected outright
//!              (`RejectedUnsupportedRuleShape`), never silently admitted or
//!              silently mis-filtered. Covered by three tests: a bare
//!              claim-Compare (non-group), the same shape inside a
//!              `group_access_rules` entry (collectionGroup), and a claim
//!              conjunct composed with an otherwise-decidable ownership
//!              conjunct via `&&` — ADR-034's own designated
//!              mutation-testing-relevant surface (§ Handoff Package flag
//!              4), proving the wildcard's exhaustiveness holds through
//!              `Condition::And`'s recursive `?`-propagation too, not just
//!              at the top level.
//!   AC-17-149: a Listen subscription's subscribe-time compliance check
//!              against the SAME claim-referencing rule is likewise
//!              rejected outright, consistent with RunQuery's own behavior.
//!   AC-17-150: GetDocument and the write path (both `evaluate()`-based, not
//!              `check_query_compliance()`-based) remain FULLY FUNCTIONAL
//!              against the IDENTICAL claim-referencing rule that AC-17-148
//!              proves is query-path-rejected — proving the limitation is
//!              scoped precisely to `check_query_compliance()`, not
//!              `evaluate()`. Per feature-delta.md's own UAT scenario for
//!              this AC, and Domain Example 3's own footnote ("this
//!              scenario documents the boundary; it does not require a
//!              subscription to actually succeed against a claim-gated
//!              collection, since US-05's own first scenario proves
//!              subscribe-time admission is itself rejected"), Listen's
//!              per-event recheck is NOT independently exercised here: a
//!              claim-gated collection's Listen subscription can never be
//!              opened in the first place (AC-17-149 proves subscribe-time
//!              always rejects it), so there is no reachable state in which
//!              a per-event recheck against this exact rule could run.
//!
//! Driving ports: gRPC :8080 `RunQuery` (non-group + group) and `Listen`,
//! reused from `security_rules_realtime`'s own common module — the ONE
//! path tree that already carries `run_query`/`run_query_raw`/
//! `seed_group_access_rule_full`/`open_listen_stream_filtered_as` together
//! with `SecurityRulesFullContext`, avoiding the cross-`#[path]`-tree
//! type-identity pitfall `security_rules_realtime/common/mod.rs::run_query`
//! documents (a `SecurityRulesFullContext` reached via a DIFFERENT
//! `#[path]` inclusion point is a structurally distinct type even from the
//! identical source file). `mint_client_identity_token_with_claims` is
//! imported separately from `custom_claims`'s own common module — a free
//! function with no `SecurityRulesFullContext` coupling, safe to mix
//! across `#[path]` trees. Mirrors `security_rules_query_path`'s own Slice
//! 05 precedent (`undecidable_rule_shape_rejects_outright.rs`) and
//! `security_rules_realtime`'s own Slice 03 precedent
//! (`subscribe_time_compliance_check.rs`) — Pillar 3, real Postgres, real
//! gRPC, real Ed25519-signed tokens, no mocks.

#![allow(unused_imports)]

#[path = "../../security_rules_realtime/common/mod.rs"]
mod common;
use common::{
    create_document, now_unix, open_listen_stream_filtered_as, run_query, run_query_raw,
    seed_group_access_rule_full, seed_write_access_rule_full, string_field, update_document,
    SecurityRulesFullContext,
};

#[path = "../common/mod.rs"]
mod custom_claims_common;
use custom_claims_common::mint_client_identity_token_with_claims;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use std::time::Duration;
use tokio_stream::StreamExt;

const CLAIM_RULE: &str = "request.auth.token.is_moderator == true";

fn resource_name(ctx: &SecurityRulesFullContext, collection: &str, doc_id: &str) -> String {
    format!(
        "projects/{}/databases/(default)/documents/{}/{}",
        ctx.project_id, collection, doc_id
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-148 (non-group): a RunQuery against a claim-referencing rule is
// rejected outright — even for a caller whose claim WOULD satisfy the rule
// under evaluate(), proving check_query_compliance() never fetches a
// document to test that.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `flagged_content` has the claim-referencing rule
///          `request.auth.token.is_moderator == true` (US-02's own rule)
///   And:   Priya Nair holds a verified identity with claim
///          `is_moderator: true` — a caller `evaluate()` WOULD admit
///   When:  Priya issues a `RunQuery` against `flagged_content`
///   Then:  the query is rejected outright, the same
///          `RejectedUnsupportedRuleShape` outcome an already-undecidable
///          rule shape (Or/Not) already produces today
///
/// AC-17-148
///
/// @error @driving_port @real-io @US-05 @AC-17-148 @security-critical
#[tokio::test]
async fn a_run_query_against_a_claim_referencing_rule_is_rejected_outright() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc05-runquery").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("flagged_content", CLAIM_RULE).await;

    let priyas_token = mint_client_identity_token_with_claims(
        &signing_key,
        "priya-nair",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"is_moderator": true}),
    );

    let result = run_query(&ctx, "flagged_content", &[], Some(&priyas_token)).await;

    let err = result.expect_err(
        "AC-17-148: a RunQuery against a claim-referencing rule must be rejected outright, even \
         for a caller whose claim WOULD satisfy the rule under evaluate()",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-148: rejection must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[UNSUPPORTED_RULE_SHAPE]"),
        "AC-17-148: rejection message must name the whole-rule-undecidable reason, got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-148 (group): a collectionGroup RunQuery against a claim-referencing
// GROUP rule is likewise rejected outright.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: a `group_access_rules` entry for `flagged_content`'s collection
///          id references `request.auth.token.is_moderator`
///   When:  Priya issues a `collectionGroup()` query against that
///          collection id
///   Then:  the query is rejected with `RejectedUnsupportedRuleShape`
///
/// AC-17-148
///
/// @error @driving_port @real-io @US-05 @AC-17-148
#[tokio::test]
async fn a_collection_group_run_query_against_a_claim_referencing_group_rule_is_rejected_outright()
{
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc05-group").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_group_access_rule_full(&ctx, "flagged_content", CLAIM_RULE).await;

    let priyas_token = mint_client_identity_token_with_claims(
        &signing_key,
        "priya-nair",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"is_moderator": true}),
    );

    let result = run_query_raw(&ctx, "flagged_content", true, None, Some(&priyas_token)).await;

    let err = result.expect_err(
        "AC-17-148: a collectionGroup RunQuery against a claim-referencing GROUP rule must be \
         rejected outright",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-148: group-query rejection must be attributable to the rule (PermissionDenied), \
         got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[UNSUPPORTED_RULE_SHAPE]"),
        "AC-17-148: group-query rejection message must name the whole-rule-undecidable reason, \
         got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-148 (AND-composed): a claim conjunct combined with an otherwise-
// decidable ownership conjunct undecidabilizes the WHOLE rule for
// query-path purposes — ADR-034's own designated mutation-testing-relevant
// surface (proves the wildcard's exhaustiveness through Condition::And's
// recursive `?`-propagation, not just a bare top-level Compare).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `flagged_content` has the rule
///          `request.auth.token.is_moderator == true && request.auth.uid == resource.data.owner_id`
///          — an otherwise fully-decidable ownership conjunct, ANDed with a
///          claim-referencing conjunct
///   And:   Priya Nair holds a verified identity with claim
///          `is_moderator: true`, and issues a query WITH a filter that
///          would satisfy the ownership conjunct in isolation
///          (`owner_id == "priya-nair"`, her own uid)
///   When:  Priya issues that filtered `RunQuery` against `flagged_content`
///   Then:  the query is STILL rejected outright — one claim-referencing
///          conjunct anywhere in an AND-chain undecidabilizes the entire
///          rule, exactly mirroring how any other unsupported shape inside
///          an `And` already behaves today
///
/// AC-17-148
///
/// @error @driving_port @real-io @US-05 @AC-17-148 @security-critical
#[tokio::test]
async fn a_claim_conjunct_inside_an_and_chain_rejects_the_whole_query_outright() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc05-and").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "flagged_content",
        "request.auth.token.is_moderator == true && request.auth.uid == resource.data.owner_id",
    )
    .await;

    let priyas_token = mint_client_identity_token_with_claims(
        &signing_key,
        "priya-nair",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"is_moderator": true}),
    );

    let result = run_query(
        &ctx,
        "flagged_content",
        &[("owner_id", "priya-nair")],
        Some(&priyas_token),
    )
    .await;

    let err = result.expect_err(
        "AC-17-148: a claim conjunct anywhere inside an And-chain must reject the whole query \
         outright, even when a filter satisfying the OTHER (decidable) conjunct is present",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-148: AND-composed rejection must be attributable to the rule (PermissionDenied), \
         got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[UNSUPPORTED_RULE_SHAPE]"),
        "AC-17-148: AND-composed rejection message must name the whole-rule-undecidable reason \
         — never [OWNERSHIP_FILTER_MISSING], which would indicate the claim conjunct was \
         silently dropped rather than undecidabilizing the whole rule — got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-149: a Listen subscription's subscribe-time compliance check against
// the SAME claim-referencing rule is likewise rejected outright.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `flagged_content` has the SAME claim-referencing rule
///   And:   Priya Nair holds a verified identity with claim
///          `is_moderator: true` — a caller `evaluate()` WOULD admit
///   When:  Priya issues an `AddTarget` Listen subscription against
///          `flagged_content`
///   Then:  the subscription is rejected before any row is read, with the
///          SAME reason code `RunQuery`'s own rejection for the identical
///          rule uses
///
/// AC-17-149
///
/// @error @driving_port @real-io @US-05 @AC-17-149 @security-critical
#[tokio::test]
async fn a_listen_subscription_against_a_claim_referencing_rule_is_rejected_outright() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc05-listen").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("flagged_content", CLAIM_RULE).await;

    let priyas_token = mint_client_identity_token_with_claims(
        &signing_key,
        "priya-nair",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"is_moderator": true}),
    );

    let mut stream =
        open_listen_stream_filtered_as(&ctx, "flagged_content", None, Some(&priyas_token)).await;

    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("AC-17-149: timed out waiting for the terminal rejection")
        .expect("AC-17-149: stream ended with no message at all");

    let err = first.expect_err(
        "AC-17-149: a claim-referencing rule must reject the Listen subscription at subscribe \
         time, before any row is read — the FIRST message must be the terminal error, never a \
         DocumentChange from the initial snapshot",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-149: rejection must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[UNSUPPORTED_RULE_SHAPE]"),
        "AC-17-149: rejection message must carry the SAME reason-code token RunQuery's own \
         rejection for the identical rule uses, got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-150: GetDocument and the write path (both evaluate()-based, not
// check_query_compliance()-based) remain FULLY FUNCTIONAL against the
// IDENTICAL claim-referencing rule that AC-17-148 proves is
// query-path-rejected — in the SAME context/rule as the tests above, to
// make the contrast maximally strong.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `flagged_content` has the SAME claim-referencing READ rule
///          (`request.auth.token.is_moderator == true`) AND the SAME
///          claim-referencing WRITE rule, already proven functional in
///          US-02/US-03
///   And:   Priya Nair holds a verified identity with claim
///          `is_moderator: true`
///   When:  Priya calls `getDoc()` then `updateDoc()` on a
///          `flagged_content` document
///   Then:  both succeed exactly as US-02/US-03 already prove — the
///          query-path limitation proven above does not regress any
///          already-shipped `evaluate()`-based enforcement surface
///
/// AC-17-150
///
/// @driving_port @real-io @US-05 @AC-17-150
#[tokio::test]
async fn get_document_and_write_path_remain_functional_against_the_same_rule() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc05-evaluate-unaffected").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    // Seed the document BEFORE the write rule exists, unauthenticated — the
    // write rule below must not gate this setup call (mirrors
    // write_path_reuse_proof.rs's own precedent exactly).
    let mut fields = std::collections::HashMap::new();
    fields.insert("text".to_string(), string_field("reported content"));
    create_document(
        &ctx,
        "flagged_content",
        "cc05-evaluate-doc",
        fields.clone(),
        None,
    )
    .await
    .expect("seed flagged_content document");

    // The IDENTICAL rule the tests above prove is query-path-rejected,
    // authored on BOTH the read and write tables (mirrors US-02/US-03's
    // own rule text exactly).
    ctx.seed_access_rule("flagged_content", CLAIM_RULE).await;
    seed_write_access_rule_full(&ctx, "flagged_content", CLAIM_RULE).await;

    let priyas_token = mint_client_identity_token_with_claims(
        &signing_key,
        "priya-nair",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"is_moderator": true}),
    );

    let doc_name = resource_name(&ctx, "flagged_content", "cc05-evaluate-doc");

    let read = ctx.get_document(&doc_name, Some(&priyas_token)).await;
    assert!(
        read.is_ok(),
        "AC-17-150: GetDocument against the SAME claim-referencing rule that rejects RunQuery \
         must remain fully functional: {:?}",
        read.err()
    );

    let write = update_document(&ctx, &doc_name, fields, Some(&priyas_token)).await;
    assert!(
        write.is_ok(),
        "AC-17-150: the write path against the SAME claim-referencing rule must remain fully \
         functional: {:?}",
        write.err()
    );
}
