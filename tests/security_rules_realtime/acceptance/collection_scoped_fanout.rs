//! Slice 01 (US-01, ADR-033) — A Listen Subscriber's Live Delivery Never
//! Crosses Into Another Collection (Walking Skeleton).
//!
//! Fixes a currently-shipped, project-wide bug: `ListenRegistry::fan_out()`
//! delivers every document-change event on a project's Postgres NOTIFY
//! channel to every subscriber on that channel, regardless of which
//! collection each subscriber's own `AddTarget` actually named. This is
//! independent of access-rule enforcement (Slices 03-05) — rule checking
//! is out of scope here.
//!
//! Acceptance criteria verified here (slice-01-collection-scoped-fanout.md):
//!   AC-17-105: a subscriber's live stream never receives a `DocumentChange`
//!              event for a real concurrent write to a DIFFERENT collection.
//!   AC-17-106: a subscriber's live stream never receives a
//!              `DocumentDelete`/Removed event for a DIFFERENT collection.
//!   AC-17-107: two subscribers to the SAME collection both correctly
//!              continue to receive that collection's own events (the fix
//!              must not over-narrow).
//!   AC-17-108: the guarantee holds identically whether or not either
//!              collection has any access rule defined.
//!
//! Driving port: gRPC :8080 `Listen` (via `SecurityRulesFullContext` +
//! this feature's own `open_listen_stream`/`create_document` helpers —
//! Pillar 3, real Postgres NOTIFY fan-out, real gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, delete_document, drain_until_current, open_listen_stream, try_recv_live_event,
    CapturedEvent, SecurityRulesFullContext,
};
use std::{collections::HashMap, time::Duration};

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-105: a subscriber to collection A never receives a DocumentChange
// event for a real concurrent write to collection B in the SAME project.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Maria subscribes to `journal_entries` only
///   When:  a document is written to the UNRELATED `trip_photos` collection
///          in the same project
///   Then:  Maria's stream never receives a DocumentChange for it
///   And:   Maria's stream still receives DocumentChange for her OWN
///          collection's writes (proves the stream is genuinely alive, not
///          silently broken)
///
/// AC-17-105
///
/// @driving_port @real-io @US-01 @AC-17-105
#[tokio::test]
async fn subscriber_to_collection_a_never_receives_changed_event_for_collection_b() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srr01-scope-changed").await;

    let mut stream = open_listen_stream(&ctx, "journal_entries").await;
    drain_until_current(&mut stream).await;

    create_document(&ctx, "trip_photos", "leak-check-photo", HashMap::new(), None)
        .await
        .expect("write to unrelated collection should succeed");

    let leaked = try_recv_live_event(&mut stream, Duration::from_millis(1500)).await;
    assert!(
        leaked.is_none(),
        "AC-17-105: subscriber to journal_entries must never receive an event for \
         trip_photos, got: {leaked:?}"
    );

    // Liveness check: a write to the SUBSCRIBED collection itself must
    // still arrive — proves the prior assertion was a real fix, not a
    // silently-broken stream.
    create_document(&ctx, "journal_entries", "own-collection-doc", HashMap::new(), None)
        .await
        .expect("write to own collection should succeed");
    let own_event = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(own_event, Some(CapturedEvent::Changed { .. })),
        "subscriber must still receive events for its OWN collection, got: {own_event:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-106: a subscriber to collection A never receives a
// DocumentDelete/Removed event for collection B.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Maria subscribes to `journal_entries` only; a document already
///          exists in the UNRELATED `trip_photos` collection
///   When:  that `trip_photos` document is deleted
///   Then:  Maria's stream never receives a DocumentDelete for it
///   And:   Maria's stream still receives DocumentDelete for her OWN
///          collection's deletes
///
/// AC-17-106
///
/// @driving_port @real-io @US-01 @AC-17-106
#[tokio::test]
async fn subscriber_to_collection_a_never_receives_removed_event_for_collection_b() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srr01-scope-removed").await;

    create_document(&ctx, "trip_photos", "leak-check-delete-photo", HashMap::new(), None)
        .await
        .expect("seed document in unrelated collection");

    let mut stream = open_listen_stream(&ctx, "journal_entries").await;
    drain_until_current(&mut stream).await;

    let other_resource_name = format!(
        "projects/{}/databases/(default)/documents/trip_photos/leak-check-delete-photo",
        ctx.project_id
    );
    delete_document(&ctx, &other_resource_name, None)
        .await
        .expect("delete of unrelated collection's document should succeed");

    let leaked = try_recv_live_event(&mut stream, Duration::from_millis(1500)).await;
    assert!(
        leaked.is_none(),
        "AC-17-106: subscriber to journal_entries must never receive a Removed event for \
         trip_photos, got: {leaked:?}"
    );

    // Liveness check: create then delete a document in the SUBSCRIBED
    // collection — the Removed event must still arrive.
    create_document(&ctx, "journal_entries", "own-delete-doc", HashMap::new(), None)
        .await
        .expect("write to own collection should succeed");
    let created = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(created, Some(CapturedEvent::Changed { .. })),
        "expected Changed event for own collection's create, got: {created:?}"
    );

    let own_resource_name = format!(
        "projects/{}/databases/(default)/documents/journal_entries/own-delete-doc",
        ctx.project_id
    );
    delete_document(&ctx, &own_resource_name, None)
        .await
        .expect("delete of own collection's document should succeed");
    let removed = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(removed, Some(CapturedEvent::Removed { .. })),
        "subscriber must still receive Removed events for its OWN collection, got: {removed:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-107: two subscribers to the SAME collection both correctly receive
// that collection's own events (the fix must not over-narrow).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: two independent Listen streams both subscribe to
///          `journal_entries`
///   When:  a document is written to `journal_entries`
///   Then:  BOTH streams receive a DocumentChange for it
///
/// AC-17-107
///
/// @driving_port @real-io @US-01 @AC-17-107
#[tokio::test]
async fn two_subscribers_to_the_same_collection_both_receive_its_events() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srr01-same-collection").await;

    let mut stream_a = open_listen_stream(&ctx, "journal_entries").await;
    drain_until_current(&mut stream_a).await;
    let mut stream_b = open_listen_stream(&ctx, "journal_entries").await;
    drain_until_current(&mut stream_b).await;

    create_document(&ctx, "journal_entries", "shared-doc", HashMap::new(), None)
        .await
        .expect("write to shared collection should succeed");

    let event_a = try_recv_live_event(&mut stream_a, Duration::from_millis(2000)).await;
    let event_b = try_recv_live_event(&mut stream_b, Duration::from_millis(2000)).await;

    assert!(
        matches!(event_a, Some(CapturedEvent::Changed { .. })),
        "AC-17-107: subscriber A to journal_entries must receive its own collection's event, \
         got: {event_a:?}"
    );
    assert!(
        matches!(event_b, Some(CapturedEvent::Changed { .. })),
        "AC-17-107: subscriber B to journal_entries must receive its own collection's event, \
         got: {event_b:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-108: the guarantee holds identically whether or not either
// collection has any access rule defined at all.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (parametrized — 2 input variations of the SAME behavior, per
/// Mandate 5):
///   Given: one collection carries a `security-rules`-era exact-path rule
///          and the other carries none, in either assignment
///   When:  a document is written to the OTHER (non-subscribed) collection
///   Then:  the subscriber never receives that event — rule presence never
///          affects the collection-scoping guarantee
///
/// AC-17-108
///
/// @driving_port @real-io @US-01 @AC-17-108
#[tokio::test]
async fn collection_scoping_holds_regardless_of_which_collection_has_a_rule() {
    for (case, tag, subscribed, ruled_is_subscribed, other) in [
        ("subscribed collection carries the rule", "sub-ruled", "journal_entries", true, "app_config"),
        ("other collection carries the rule", "other-ruled", "app_config", false, "journal_entries"),
    ] {
        let project_id = format!("trailmark-prod-srr01-rule-indep-{tag}");
        let ctx = SecurityRulesFullContext::new(&project_id).await;

        let ruled_collection = if ruled_is_subscribed { subscribed } else { other };
        ctx.seed_access_rule(ruled_collection, "request.auth.uid == resource.data.owner_id")
            .await;

        let mut stream = open_listen_stream(&ctx, subscribed).await;
        drain_until_current(&mut stream).await;

        create_document(&ctx, other, "leak-check-doc", HashMap::new(), None)
            .await
            .expect("write to the other collection should succeed");

        let leaked = try_recv_live_event(&mut stream, Duration::from_millis(1500)).await;
        assert!(
            leaked.is_none(),
            "AC-17-108 ({case}): collection-scoping must hold regardless of rule presence, \
             got: {leaked:?}"
        );
    }
}
