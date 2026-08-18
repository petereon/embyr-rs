//! Slice 07 (US-07, LAST slice, ADR-030) — Alex Tests a Write Rule Against
//! Concrete Old/New Examples Before Publishing It.
//!
//! Extends `security-rules`' own `simulate_access_rule` admin action
//! (`sr05_simulate_rule_before_publish.rs`) rather than adding a new
//! endpoint (ADR-030 § Decision — Composition, "Simulation: extend
//! `simulate_access_rule`, not a new endpoint"). `SimulateAccessRuleBody`
//! gains `operation` (documentation-only, NOT consumed by `evaluate()`) and
//! `request_resource` (the proposed new state) — the same
//! `resource`/`request_resource` populated-vs-empty differentiation real
//! write enforcement (Slices 02-04) uses drives create/update/delete
//! semantics here too, with zero `operation`-based branch anywhere.
//!
//! Acceptance criteria verified here (feature-delta.md US-07):
//!   AC-17-46: simulating a candidate write rule against synthetic old/new
//!             document payloads returns the same allow/deny outcome real
//!             write-evaluation would produce, across create/update/delete
//!             shapes, including an INTENTIONALLY-BACKWARDS immutable-field
//!             condition that proves simulation genuinely catches authoring
//!             bugs (mirroring `security-rules`' own US-05 "surfaces an
//!             over-permissive rule bug" precedent).
//!   AC-17-47: simulating a write rule has zero effect on live/published
//!             traffic — never calls `upsert_write_access_rule`, never
//!             touches a live document.
//!   AC-17-48: simulation supports the anonymous (no synthetic identity)
//!             case for write rules, matching Slice 05's real
//!             anonymous-write evaluation exactly.
//!
//! Driving ports: Admin HTTP :9090 (`SecurityRulesAdminContext`, AC-17-46/48)
//! + Admin HTTP :9090 AND gRPC :8080 together (`SecurityRulesFullContext`,
//! AC-17-47 — needs BOTH the simulate action and a REAL updateDoc call on
//! the SAME composition root to prove zero cross-effect, mirroring sr05's
//! own scenario 3 shape).
//!
//! `simulate_access_rule` itself needs no new branch to support write-rule
//! simulation (ADR-030): it never reads from `access_rules` or
//! `write_access_rules` at all, so the single existing handler already
//! supports both rule types — only the two-field-map `evaluate()` call
//! needed updating, identically to real enforcement.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    mint_client_identity_token, now_unix, seed_write_access_rule_full, string_field,
    update_document, write_access_rule_condition_source_full, SecurityRulesAdminContext,
    SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-46 (create shape): a candidate write rule referencing only
// `request.resource.data.<field>` (the proposed new state) simulates
// correctly against a create-shaped payload (`resource` empty, matching real
// `handle_create_document`'s own empty-map convention).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate write rule requiring the proposed
///          document's owner field to match the caller's uid
///   When:  Alex simulates it against a CREATE-shaped payload (`resource`
///          empty — no existing document — `request_resource` populated)
///   Then:  the response shows "allow," matching what real
///          `handle_create_document` evaluation would produce for an
///          identical create attempt
///
/// AC-17-46 (create shape)
///
/// @driving_port @real-io @US-07 @AC-17-46
#[tokio::test]
async fn simulating_a_create_shape_write_rule_returns_the_correct_allow_outcome() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.resource.data.owner_id == request.auth.uid",
            "operation": "create",
            "auth": {"uid": "maria-santos"},
            "resource": {},
            "request_resource": {"owner_id": "maria-santos"},
        }))
        .send()
        .await
        .expect("simulate request failed");

    assert_eq!(resp.status().as_u16(), 200, "AC-17-46: a valid simulation request must return 200");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "allow",
        "AC-17-46: a create-shaped payload matching the candidate write rule must simulate as \
         'allow' — the same outcome real `handle_create_document` evaluation would produce"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-46 (update shape, bug-surfacing): an intentionally-backwards
// immutable-field condition proves simulation genuinely catches authoring
// bugs (mirrors `security-rules`' own US-05 "surfaces an over-permissive
// rule bug" precedent).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex MEANT to write an immutable-owner rule
///          (`resource.data.owner_id == request.resource.data.owner_id`,
///          deny any change to the owner field) but ACCIDENTALLY wrote `!=`
///          instead of `==`
///   When:  Alex simulates it against an UPDATE-shaped payload (both
///          `resource` and `request_resource` populated) where the proposed
///          new document CHANGES the protected `owner_id` field — the exact
///          shape the intended rule exists to deny
///   Then:  the response shows "allow" instead of the "deny" Alex expected —
///          the reversed-equality bug surfaces BEFORE publishing, exactly
///          mirroring `update_gated_by_both_resource_states.rs`'s own
///          AC-17-32 real-evaluation semantics for the CORRECT rule (which
///          denies this exact payload)
///
/// AC-17-46 (update shape, backwards-condition bug)
///
/// @error @driving_port @real-io @US-07 @AC-17-46
#[tokio::test]
async fn simulating_an_update_shape_write_rule_with_a_backwards_immutable_field_condition_surfaces_the_authoring_bug(
) {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            // INTENTIONALLY BACKWARDS: Alex meant `==` (immutable field) but
            // wrote `!=` — the exact class of authoring bug simulation
            // exists to catch before publishing.
            "condition": "resource.data.owner_id != request.resource.data.owner_id",
            "operation": "update",
            "resource": {"owner_id": "maria-santos"},
            "request_resource": {"owner_id": "dana-kim"},
        }))
        .send()
        .await
        .expect("simulate request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "allow",
        "AC-17-46: the reversed-equality bug must surface as 'allow' — a proposed update that \
         CHANGES the protected owner_id field must have been denied under Alex's INTENDED rule, \
         but the backwards `!=` he actually wrote allows it. That discrepancy IS the bug \
         simulation is meant to catch pre-publish."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-46 (delete shape): a candidate write rule referencing
// `request.resource.data.<field>` fails closed against a delete-shaped
// payload (`request_resource` empty — no proposed new state, matching real
// `handle_delete_document`'s own empty-map convention).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate write rule referencing
///          `request.resource.data.owner_id` (nonsensical for a delete, but
///          grammar-legal)
///   When:  Alex simulates it against a DELETE-shaped payload (`resource`
///          populated — the existing document — `request_resource` empty —
///          no proposed new state)
///   Then:  the response shows "deny" — the fail-closed mechanism denies
///          via the missing-field short-circuit, matching what real
///          `handle_delete_document` evaluation would produce (AC-17-37's
///          precedent, never a crash)
///
/// AC-17-46 (delete shape, fail-closed)
///
/// @error @driving_port @real-io @US-07 @AC-17-46
#[tokio::test]
async fn simulating_a_delete_shape_write_rule_fails_closed_when_the_proposed_document_is_absent() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.resource.data.owner_id == request.auth.uid",
            "operation": "delete",
            "auth": {"uid": "maria-santos"},
            "resource": {"owner_id": "maria-santos"},
            "request_resource": {},
        }))
        .send()
        .await
        .expect("simulate request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "deny",
        "AC-17-46: a delete-shaped payload (empty request_resource) referencing \
         request.resource.data.<field> must fail closed as 'deny' — the same fail-closed outcome \
         real `handle_delete_document` evaluation would produce, never a crash"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-47: simulating a write rule has zero effect on live/published
// traffic — never calls `upsert_write_access_rule`, never touches a live
// document.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an ACTIVE, PUBLISHED write rule (owner-only)
///          and Maria Santos (the owner) holds a verified identity
///   When:  Maria's real `updateDoc()` succeeds against the published rule,
///          THEN Alex simulates a COMPLETELY DIFFERENT candidate write rule
///          (unconditional deny) with synthetic old/new payloads
///   Then:  the stored `write_access_rules` row is byte-identical before and
///          after the simulation, and Maria's real `updateDoc()` STILL
///          succeeds afterward — simulation never wrote to
///          `write_access_rules` and never touched a live document
///
/// AC-17-47
///
/// @error @driving_port @real-io @US-07 @AC-17-47
#[tokio::test]
async fn simulating_a_candidate_write_rule_has_zero_effect_on_live_write_traffic_or_stored_rules() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp07-liveunaffected").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    // journal_entries has an ACTIVE, PUBLISHED write rule (Maria owns her
    // own doc, and does not change the protected field on this update).
    const PUBLISHED_RULE: &str = "request.auth.uid == resource.data.owner_id";
    seed_write_access_rule_full(&ctx, "journal_entries", PUBLISHED_RULE).await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));

    // Real caller succeeds against the PUBLISHED rule before simulation.
    let before = update_document(
        &ctx,
        &ctx.document_resource_name("journal_entries", "maria-doc"),
        fields.clone(),
        Some(&marias_token),
    )
    .await;
    assert!(before.is_ok(), "precondition: Maria's real update must succeed before simulation");

    let stored_before = write_access_rule_condition_source_full(&ctx, "journal_entries").await;
    assert_eq!(
        stored_before.as_deref(),
        Some(PUBLISHED_RULE),
        "precondition: the published write rule must be stored as seeded"
    );

    // Alex simulates a COMPLETELY DIFFERENT candidate write rule
    // (unconditional deny) with synthetic old/new payloads — this must
    // never touch `write_access_rules` or any live document.
    let simulate_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/simulate",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "false",
            "operation": "update",
            "auth": {"uid": "test-user-999"},
            "resource": {"owner_id": "someone-else"},
            "request_resource": {"owner_id": "someone-else-still"},
        }))
        .send()
        .await
        .expect("simulate request failed");
    assert_eq!(simulate_resp.status().as_u16(), 200);

    // The stored write rule row is byte-identical to before the simulation —
    // `upsert_write_access_rule` was never called.
    let stored_after = write_access_rule_condition_source_full(&ctx, "journal_entries").await;
    assert_eq!(
        stored_before, stored_after,
        "AC-17-47: the stored write_access_rules row must be UNCHANGED after simulation — \
         simulate_access_rule must never call upsert_write_access_rule"
    );

    // Real callers' updateDoc calls continue to be evaluated against the
    // PUBLISHED rule, completely unaffected by the simulation just run.
    let after = update_document(
        &ctx,
        &ctx.document_resource_name("journal_entries", "maria-doc"),
        fields,
        Some(&marias_token),
    )
    .await;
    assert!(
        after.is_ok(),
        "AC-17-47: Maria's real update must STILL succeed after the simulation — simulation must \
         have zero effect on live traffic: {:?}",
        after.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-48: simulation supports the anonymous (no synthetic identity) case
// for write rules, matching Slice 05's real anonymous-write evaluation
// exactly.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate write rule requiring
///          `request.auth != null` (mirrors `anonymous_write_sessions.rs`'s
///          own AC-17-39 real-evaluation Given exactly)
///   When:  Alex simulates it with NO "auth" field at all (anonymous) against
///          a CREATE-shaped payload
///   Then:  the response shows "deny," matching exactly what a real
///          never-signed-in session's createDoc() receives under AC-17-39
///
/// AC-17-48
///
/// @error @driving_port @real-io @US-07 @AC-17-48
#[tokio::test]
async fn simulating_a_write_rule_supports_the_anonymous_case_identically_to_real_anonymous_write_evaluation(
) {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    // No "auth" field at all — represents an anonymous caller, identical in
    // shape to real evaluation's `Option<AuthContext>` (AC-17-39's own
    // never-signed-in-session semantics).
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.auth != null",
            "operation": "create",
            "resource": {},
            "request_resource": {"owner_id": "nobody"},
        }))
        .send()
        .await
        .expect("simulate request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "deny",
        "AC-17-48: simulating with no synthetic identity against a write rule must match what a \
         real never-signed-in session receives on createDoc() under AC-17-39 (denied by an \
         auth-required write rule)"
    );
}
