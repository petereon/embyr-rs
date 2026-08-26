//! Slice 04 (US-04, ADR-033) — Each Individually-Delivered Live Change Event
//! Is Re-Checked Against the Rule Using the Already-Fetched Document.
//!
//! Extends `evaluate()` (ADR-027/030, unchanged) to `Listen`'s own ongoing,
//! per-event delivery decision inside `handle_add_target`'s `Changed` arm —
//! the SAME mechanism `GetDocument`/writes already use, now applied
//! continuously across a subscription's entire lifetime rather than once at
//! subscribe time (Slice 03's own gate).
//!
//! Acceptance criteria verified here (slice-04-per-event-compliance-recheck.md):
//!   AC-17-117: a live change event for a document that STILL satisfies the
//!              subscriber's own rule is delivered.
//!   AC-17-118: a live change event for a document that NO LONGER satisfies
//!              the subscriber's own rule (its `owner_id` field is
//!              reassigned mid-subscription, after the initial snapshot) is
//!              withheld — a real NOTIFY-triggered write followed by a
//!              real, timeout-based "no event arrives" assertion, mirroring
//!              `collection_scoped_fanout.rs`'s own AC-17-105/106 pattern.
//!   AC-17-119: a live change event referencing a document with a MISSING
//!              rule-referenced field fails closed (withheld), never
//!              crashes/panics — proven by a liveness follow-up write that
//!              still arrives on the same, still-open stream.
//!   AC-17-120: `evaluate()` (ADR-027/030) is reused completely unmodified
//!              — a structural/reuse property, verified via `git diff`
//!              confirming zero changes to
//!              `crates/embyr-core/src/access_control/mod.rs`, NOT via a
//!              runtime assertion here (mirrors
//!              `subscribe_time_compliance_check.rs`'s own AC-17-115
//!              precedent).
//!   AC-17-121: the per-event re-check adds zero additional I/O beyond
//!              `fetch_event()`'s own existing fetch — a structural claim,
//!              not directly assertable at the driving-port boundary
//!              (mirrors `security_rules_query_path`'s own AC-17-91/112
//!              precedent). Proven by code inspection: the `evaluate()` call
//!              added to `listen_handler.rs`'s `Changed` arm consumes
//!              `doc.fields`, already in memory from the `ListenEvent::
//!              Changed(FirestoreDocument)` `fetch_event()` produced BEFORE
//!              `fan_out()` was ever called (`postgres_notify_listener.rs`)
//!              — no new adapter call, no new port, no new DB round-trip
//!              anywhere in this slice's diff.
//!
//! Driving port: gRPC :8080 `Listen` (via `SecurityRulesFullContext` +
//! this feature's own `open_listen_stream_filtered_as`/`update_document`
//! helpers) — Pillar 3, real Postgres NOTIFY fan-out, real gRPC, no mocks.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, drain_until_current, equality_where_filter, mint_client_identity_token,
    now_unix, open_listen_stream_filtered_as, string_field, try_recv_live_event, update_document,
    CapturedEvent, SecurityRulesFullContext,
};

use std::time::Duration;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;

/// Journey shared by all three scenarios below: `journal_entries` carries a
/// READ rule requiring `request.auth.uid == resource.data.owner_id`; Maria
/// subscribes with a filter admitted by Slice 03's own subscribe-time gate
/// (`owner_id == "maria-santos"`), and the initial snapshot (containing the
/// context's own default-seeded `{project}-maria-doc`) is drained before any
/// live-event assertion begins.
async fn compliant_maria_subscription(
    project_suffix: &str,
) -> (SecurityRulesFullContext, tonic::Streaming<embyr_proto::firestore::ListenResponse>, String) {
    let ctx =
        SecurityRulesFullContext::new(&format!("trailmark-prod-srr04-{project_suffix}")).await;
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
    let mut stream =
        open_listen_stream_filtered_as(&ctx, "journal_entries", filter, Some(&marias_token))
            .await;
    drain_until_current(&mut stream).await;

    let maria_doc_resource_name = format!(
        "projects/{}/databases/(default)/documents/journal_entries/{}-maria-doc",
        ctx.project_id, ctx.project_id
    );

    (ctx, stream, maria_doc_resource_name)
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-117: a live change event for a document that STILL satisfies the
// subscriber's own rule is delivered.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Maria's compliant subscription (see `compliant_maria_subscription`)
///   When:  her OWN document (`owner_id` still `"maria-santos"`) is updated —
///          only an unrelated field changes
///   Then:  the live `DocumentChange` event arrives on her stream
///
/// AC-17-117
///
/// @driving_port @real-io @US-04 @AC-17-117
#[tokio::test]
async fn compliant_live_change_event_is_delivered() {
    let (ctx, mut stream, maria_doc) = compliant_maria_subscription("allow").await;

    update_document(
        &ctx,
        &maria_doc,
        std::collections::HashMap::from([
            ("owner_id".to_string(), string_field("maria-santos")),
            ("note".to_string(), string_field("still hers")),
        ]),
        None,
    )
    .await
    .expect("update to still-compliant document should succeed");

    let event = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(event, Some(CapturedEvent::Changed { .. })),
        "AC-17-117: a live change event for a document that still satisfies the subscriber's \
         own rule must be delivered, got: {event:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-118: a live change event for a document that NO LONGER satisfies the
// subscriber's own rule is withheld (Production-Data Taste Test).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Maria's compliant subscription, already past the initial
///          snapshot for her own `owner_id`-matched document
///   When:  that SAME document's `owner_id` is reassigned to `"dana-kim"`
///          mid-subscription (a real NOTIFY-triggered write)
///   Then:  the NEXT live event for that document never arrives on Maria's
///          stream — real, timeout-based "no event arrives" assertion
///   And:   the stream is still alive: a subsequent compliant write (a new
///          document she still owns) DOES arrive
///
/// AC-17-118
///
/// @driving_port @real-io @US-04 @AC-17-118 @security-critical
#[tokio::test]
async fn live_change_event_for_a_document_that_lost_compliance_is_withheld() {
    let (ctx, mut stream, maria_doc) = compliant_maria_subscription("deny-reassign").await;

    update_document(
        &ctx,
        &maria_doc,
        std::collections::HashMap::from([("owner_id".to_string(), string_field("dana-kim"))]),
        None,
    )
    .await
    .expect("owner_id reassignment write should succeed (no write-rule seeded)");

    let withheld = try_recv_live_event(&mut stream, Duration::from_millis(1500)).await;
    assert!(
        withheld.is_none(),
        "AC-17-118: a live change event for a document reassigned away from the subscriber's \
         own uid mid-subscription must be withheld, never delivered, got: {withheld:?}"
    );

    // Liveness check: the stream is still alive — a fresh, still-compliant
    // document (her own uid) must still arrive.
    create_document(
        &ctx,
        "journal_entries",
        "still-compliant-doc",
        std::collections::HashMap::from([("owner_id".to_string(), string_field("maria-santos"))]),
        None,
    )
    .await
    .expect("write of a still-compliant document should succeed");
    let live = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(live, Some(CapturedEvent::Changed { .. })),
        "AC-17-118: the withheld event must not have broken the stream — a subsequent \
         compliant document must still be delivered, got: {live:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-119: a live change event referencing a document with a MISSING
// rule-referenced field fails closed (withheld), never crashes/panics.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Maria's compliant subscription; the context's own default-seeded
///          `{project}-no-owner-doc` has NO `owner_id` field at all
///   When:  that document is updated (still without an `owner_id` field)
///   Then:  the live event is withheld — fail-closed, never a panic
///   And:   the stream remains healthy: a subsequent compliant write still
///          arrives on the same connection
///
/// AC-17-119
///
/// @error @driving_port @real-io @US-04 @AC-17-119
#[tokio::test]
async fn live_change_event_for_a_document_missing_the_rule_field_fails_closed() {
    let (ctx, mut stream, _maria_doc) = compliant_maria_subscription("missing-field").await;

    let no_owner_doc = format!(
        "projects/{}/databases/(default)/documents/journal_entries/{}-no-owner-doc",
        ctx.project_id, ctx.project_id
    );
    update_document(
        &ctx,
        &no_owner_doc,
        std::collections::HashMap::from([("title".to_string(), string_field("still no owner"))]),
        None,
    )
    .await
    .expect("update of the field-missing document should succeed");

    let withheld = try_recv_live_event(&mut stream, Duration::from_millis(1500)).await;
    assert!(
        withheld.is_none(),
        "AC-17-119: a live change event referencing a document missing the rule-referenced \
         field must fail closed (withheld), never delivered, got: {withheld:?}"
    );

    // Liveness / no-panic check: the server did not crash — a subsequent
    // compliant write still arrives on the SAME still-open stream.
    create_document(
        &ctx,
        "journal_entries",
        "post-missing-field-doc",
        std::collections::HashMap::from([("owner_id".to_string(), string_field("maria-santos"))]),
        None,
    )
    .await
    .expect("write of a compliant document should succeed");
    let live = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(live, Some(CapturedEvent::Changed { .. })),
        "AC-17-119: the server must not crash/panic on a fail-closed evaluation — a subsequent \
         compliant document must still be delivered, got: {live:?}"
    );
}
