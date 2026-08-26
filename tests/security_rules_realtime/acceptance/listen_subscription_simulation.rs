//! Slice 08 (US-08, LAST slice of security-rules-realtime, ADR-033 §
//! Decision — Admin Surface) — Alex Can Pre-Check Whether a Candidate Listen
//! Subscription Would Be Admitted.
//!
//! Per DESIGN's own resolution (ADR-033), this is a pure documentation
//! /framing slice: `simulate_query_compliance` (already built by
//! `security-rules-query-path`, ADR-031, unchanged) applies identically to a
//! candidate `Listen` subscription's own initial-snapshot filter shape,
//! since `Listen`'s subscribe-time gate (Slice 03 of THIS feature,
//! `realtime::listen_handler::handle_add_target`) calls `check_query_compliance()`
//! with the IDENTICAL `(condition, filter, auth)` input shape
//! `handle_run_query` already uses. ZERO new route/handler/response type —
//! confirmed against ADR-033's own text and `access_rules.rs`'s existing,
//! unmodified handler contract before writing this file.
//!
//! Acceptance criteria verified here (feature-delta.md US-08):
//!   AC-17-134: simulating a candidate query filter against a candidate
//!              rule, framed as a Listen subscription's own initial-snapshot
//!              filter, returns the SAME admit outcome real Listen
//!              subscribe-time enforcement (Slice 03) would produce for an
//!              identical filter/rule/identity — cross-checked here against
//!              an ACTUAL Listen subscription opened with the identical
//!              filter/rule/identity, proving the two mechanisms agree (not
//!              merely asserted from the simulate response alone).
//!   AC-17-135: simulation correctly reports "missing required filter"
//!              (`OWNERSHIP_FILTER_MISSING`, the SAME reason code Slice 03's
//!              real rejection uses) for a candidate filter lacking a
//!              required conjunct.
//!   AC-17-136: simulation correctly reports "admitted" for a candidate
//!              collection id with NO rule. As ADR-033 §  Decision — Admin
//!              Surface / feature-delta.md OQ-SRRT-06 resolves: `simulate_
//!              query_compliance` takes a candidate CONDITION TEXT, never a
//!              collection reference, and never reads `access_rules` — there
//!              is no candidate rule text to simulate for "no rule," so
//!              there is nothing for a simulate call to test. This AC is
//!              therefore proven STRUCTURALLY: a real Listen subscription to
//!              an unruled collection is admitted, unconditionally, exactly
//!              as `RunQuery`'s own US-06 default already establishes — NOT
//!              via a new simulate-endpoint scenario referencing a
//!              nonexistent "candidate collection id" input.
//!   AC-17-137: simulating a Listen-subscription shape has ZERO effect on
//!              live/open Listen traffic — a real gRPC :8080 Listen stream
//!              stays open and keeps delivering real events, completely
//!              unaffected by a `simulate_query_compliance` call with a
//!              COMPLETELY DIFFERENT candidate condition/filter, on the SAME
//!              composition root (mirrors `security_rules_query_path`'s own
//!              AC-17-76 and `security_rules_collection_group_rules`'s own
//!              AC-17-104 precedent shape exactly).
//!
//! Driving ports: Admin HTTP :9090 (`SecurityRulesAdminContext`, AC-17-135)
//! + Admin HTTP :9090 AND gRPC :8080 together on the SAME composition root
//! (`SecurityRulesFullContext`, AC-17-134/136/137 — each needs a REAL Listen
//! stream to cross-check or prove non-interference against the simulate
//! action).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, drain_until_current, equality_where_filter, mint_client_identity_token,
    now_unix, open_listen_stream, open_listen_stream_filtered_as, try_recv_live_event,
    CapturedEvent, SecurityRulesAdminContext, SecurityRulesFullContext,
};

use std::time::Duration;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-134: simulating a candidate query filter against a candidate rule,
// framed as a Listen subscription, returns the SAME admit outcome a real
// Listen subscription with the identical filter/rule/identity produces.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` carries the real ownership rule
///          (`request.auth.uid == resource.data.owner_id`), and Maria's real
///          `Listen` subscription filtered by her own `owner_id` is admitted
///          (proven by the real stream opening and delivering its initial
///          snapshot)
///   When:  Alex simulates the IDENTICAL candidate rule/filter/identity via
///          `simulate_query_compliance`, framed as testing that same Listen
///          subscription's own initial-snapshot filter shape
///   Then:  the response shows `compliant: true` — the same admit outcome
///          the real, already-open Listen subscription just proved
///
/// AC-17-134
///
/// @driving_port @real-io @US-08 @AC-17-134
#[tokio::test]
async fn simulating_a_filter_matching_a_real_admitted_listen_subscription_reports_compliant() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srr08-admitted").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    // Real Listen subscription, identical rule/filter/identity — the cross
    // -check oracle. `open_listen_stream_filtered_as` panics on a rejected
    // subscription (`.expect("listen should succeed")`), and
    // `drain_until_current` further proves the initial snapshot was actually
    // delivered — together, direct proof of REAL admission.
    let filter = equality_where_filter(&[("owner_id", "maria-santos")]);
    let mut real_stream = open_listen_stream_filtered_as(
        &ctx,
        "journal_entries",
        filter,
        Some(&marias_token),
    )
    .await;
    drain_until_current(&mut real_stream).await;

    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/simulate_query",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.auth.uid == resource.data.owner_id",
            "auth": {"uid": "maria-santos"},
            "query_filters": [
                {"field_path": "owner_id", "op": "==", "value": "maria-santos"},
            ],
        }))
        .send()
        .await
        .expect("simulate_query request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-134: a valid simulate_query request must return 200"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["compliant"], true,
        "AC-17-134: simulating the IDENTICAL rule/filter/identity a real, already-admitted \
         Listen subscription just proved compliant must also report compliant: true — the two \
         mechanisms must agree, got: {body}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-135: simulation correctly reports "missing required filter" for a
// candidate filter lacking a required conjunct.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a candidate rule requiring an ownership-equality
///          conjunct
///   When:  Alex simulates it framed as a candidate Listen subscription's
///          own initial-snapshot filter, with NO query filters at all — the
///          shape his real-time list screen would issue before he fixes it
///   Then:  the response shows `compliant: false` with reason
///          `OWNERSHIP_FILTER_MISSING` — the SAME reason code Slice 03's own
///          real subscribe-time rejection (`query_compliance_rejection`)
///          uses, since it's literally the same `check_query_compliance()`
///          function's output
///
/// AC-17-135
///
/// @error @driving_port @real-io @US-08 @AC-17-135
#[tokio::test]
async fn simulating_a_listen_subscription_filter_missing_the_ownership_conjunct_reports_ownership_filter_missing(
) {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url(
            "/admin/v1/projects/trailmark-prod/access_rules/simulate_query",
        ))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.auth.uid == resource.data.owner_id",
            "auth": {"uid": "maria-santos"},
            "query_filters": [],
        }))
        .send()
        .await
        .expect("simulate_query request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["compliant"], false,
        "AC-17-135: a candidate Listen subscription filter missing the required ownership \
         conjunct must be reported non-compliant, got: {body}"
    );
    assert_eq!(
        body["reasons"],
        serde_json::json!(["OWNERSHIP_FILTER_MISSING"]),
        "AC-17-135: the missing conjunct must be reported via the SAME reason-code vocabulary \
         Slice 03's real subscribe-time rejection uses, got: {body}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-136: a candidate collection id with NO rule is admitted — proven
// structurally (ADR-033 § Decision — Admin Surface / OQ-SRRT-06), since
// `simulate_query_compliance` takes a candidate CONDITION TEXT, never a
// collection reference, and never reads `access_rules` — there is no
// candidate rule text to simulate for "no rule."
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `app_config` (a real, seeded collection in this domain) carries
///          NO access rule at all
///   When:  Alex opens a real Listen subscription against it, with no query
///          filter
///   Then:  the subscription is admitted unconditionally and its initial
///          snapshot is delivered — matching US-06's own real "no rule ⇒
///          unrestricted" default, the SAME structural guarantee `RunQuery`
///          already has. This is "admitted because unruled," distinct from
///          AC-17-134's "admitted because compliant" case (which required a
///          filter satisfying an ACTIVE rule) — no `simulate_query_compliance`
///          call is made for this scenario because the endpoint's own
///          contract has no candidate-collection-id input to represent "no
///          rule" with (see module doc comment, OQ-SRRT-06).
///
/// AC-17-136
///
/// @driving_port @real-io @US-08 @AC-17-136
#[tokio::test]
async fn a_real_listen_subscription_to_an_unruled_collection_is_admitted_unconditionally() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srr08-unruled").await;

    // `app_config` is seeded by `SecurityRulesFullContext::new` but never
    // carries an `access_rules` row in this test — no `seed_access_rule`
    // call for it anywhere above, matching the domain's own "app_config
    // never protected by any rule" convention.
    let mut real_stream = open_listen_stream(&ctx, "app_config").await;
    let snapshot = common::collect_initial_snapshot(&mut real_stream).await;

    assert_eq!(
        snapshot.len(),
        1,
        "AC-17-136: a Listen subscription to an unruled collection must be admitted \
         unconditionally and deliver its real initial snapshot — content unrestricted, \
         matching US-06's own default, got snapshot: {snapshot:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-137: simulating a Listen-subscription shape has ZERO effect on
// live/open Listen traffic.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` carries a REAL, PUBLISHED ownership rule, and
///          Maria holds a real, OPEN Listen subscription filtered by her own
///          `owner_id`, already past its initial snapshot, and receiving
///          real live events normally
///   When:  Alex simulates a COMPLETELY DIFFERENT candidate rule/filter
///          /identity via `simulate_query_compliance`, framed as testing an
///          unrelated candidate Listen subscription
///   Then:  Maria's real, open Listen subscription STILL receives real live
///          events normally afterward — `simulate_query_compliance` never
///          issues a real `Listen`/`RunQuery` call and never touches
///          `access_rules`, mirroring `security_rules_query_path`'s own
///          AC-17-76 and `security_rules_collection_group_rules`'s own
///          AC-17-104 precedent shape exactly
///
/// AC-17-137
///
/// @error @driving_port @real-io @US-08 @AC-17-137
#[tokio::test]
async fn simulating_a_different_candidate_listen_shape_has_zero_effect_on_a_real_open_listen_subscription(
) {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srr08-liveunaffected").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let filter = equality_where_filter(&[("owner_id", "maria-santos")]);
    let mut real_stream = open_listen_stream_filtered_as(
        &ctx,
        "journal_entries",
        filter,
        Some(&marias_token),
    )
    .await;
    drain_until_current(&mut real_stream).await;

    // Real caller receives real events normally BEFORE simulation.
    create_document(
        &ctx,
        "journal_entries",
        "before-simulation-doc",
        std::collections::HashMap::from([(
            "owner_id".to_string(),
            embyr_proto::firestore::Value {
                value_type: Some(
                    embyr_proto::firestore::value::ValueType::StringValue(
                        "maria-santos".to_string(),
                    ),
                ),
            },
        )]),
        None,
    )
    .await
    .expect("precondition: real create against the published rule must succeed");
    let before = try_recv_live_event(&mut real_stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(before, Some(CapturedEvent::Changed { .. })),
        "precondition: Maria's real, open Listen subscription must receive real live events \
         before simulation, got: {before:?}"
    );

    // Alex simulates a COMPLETELY DIFFERENT candidate rule/filter/identity —
    // this must never touch `access_rules` or issue a real `Listen`/`RunQuery`.
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let simulate_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/simulate_query",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "false",
            "auth": {"uid": "test-user-999"},
            "query_filters": [
                {"field_path": "owner_id", "op": "==", "value": "someone-else"},
            ],
        }))
        .send()
        .await
        .expect("simulate_query request failed");
    assert_eq!(simulate_resp.status().as_u16(), 200);

    // Maria's real, open Listen subscription STILL receives real live events
    // normally afterward, completely unaffected by the simulation just run.
    create_document(
        &ctx,
        "journal_entries",
        "after-simulation-doc",
        std::collections::HashMap::from([(
            "owner_id".to_string(),
            embyr_proto::firestore::Value {
                value_type: Some(
                    embyr_proto::firestore::value::ValueType::StringValue(
                        "maria-santos".to_string(),
                    ),
                ),
            },
        )]),
        None,
    )
    .await
    .expect("real create against the published rule must still succeed after simulation");
    let after = try_recv_live_event(&mut real_stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(after, Some(CapturedEvent::Changed { .. })),
        "AC-17-137: Maria's real, open Listen subscription must STILL receive real live events \
         after the simulation — simulate_query_compliance must have zero effect on live Listen \
         traffic, got: {after:?}"
    );
}
