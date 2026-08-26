//! Slice 05 (US-05, ADR-033) — A Delete Event for a Document the
//! Subscriber's Rule Would Deny Is Not Delivered.
//!
//! Extends existence non-leakage (`security-rules`/`security-rules-write-path`'s
//! own established discipline, AC-17-10/34/38) to `Listen`'s own delete-event
//! delivery. `fetch_event()` (`postgres_notify_listener.rs`) now widens its
//! existing SQL query to also select soft-deleted rows and their pre-deletion
//! `fields` (ADR-033 § Decision — Delete Non-Leakage, US-05, resolves
//! Handoff Package flag 7 / OQ-SRRT-02); `handle_add_target`'s `Removed`
//! arm applies `evaluate()` to that snapshot identically to Slice 04's own
//! `Changed`-arm gate — same loop-lifetime `condition`/`auth_ctx` locals,
//! same `Deny` -> `continue` withholding. `fields` is NEVER serialized into
//! the wire-level `DocumentDelete` response; it exists only as `evaluate()`'s
//! own decision input.
//!
//! Acceptance criteria verified here (slice-05-delete-event-non-leakage.md):
//!   AC-17-122: a delete of a document the subscriber's own rule WOULD have
//!              admitted is delivered as a real `DocumentDelete` event.
//!   AC-17-123: a delete of a document the subscriber's own rule would have
//!              DENIED is NOT delivered — real, timeout-based "no event
//!              arrives" assertion, mirroring Slice 04's own withheld-event
//!              test pattern (Production-Data Taste Test).
//!   AC-17-124: a document that became non-compliant for a subscriber
//!              BEFORE deletion (Slice 04's own scenario — an `owner_id`
//!              reassignment) does not deliver its subsequent deletion
//!              either — a real two-step domain proof.
//!   AC-17-125: a content-blind rule correctly decides and delivers delete
//!              events WITHOUT requiring the deleted document's own field
//!              data.
//!
//! Driving port: gRPC :8080 `Listen` (via `SecurityRulesFullContext` + this
//! feature's own `open_listen_stream_filtered_as`/`create_document`/
//! `update_document`/`delete_document` helpers) — Pillar 3, real Postgres
//! NOTIFY fan-out, real gRPC, no mocks.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, delete_document, drain_until_current, equality_where_filter,
    mint_client_identity_token, now_unix, open_listen_stream_filtered_as, string_field,
    try_recv_live_event, update_document, CapturedEvent, SecurityRulesFullContext,
};

use std::time::Duration;

use ed25519_dalek::SigningKey;
use rand_core::OsRng;

/// Journey shared by AC-17-122/123/124: `journal_entries` carries a READ
/// rule requiring `request.auth.uid == resource.data.owner_id`; Maria
/// subscribes with a filter admitted by Slice 03's own subscribe-time gate
/// (`owner_id == "maria-santos"`), and the initial snapshot is drained
/// before any live-event assertion begins — mirrors
/// `per_event_compliance_recheck.rs`'s own `compliant_maria_subscription`
/// exactly (Pillar 3 reuse discipline).
async fn compliant_maria_subscription(
    project_suffix: &str,
) -> (SecurityRulesFullContext, tonic::Streaming<embyr_proto::firestore::ListenResponse>, String) {
    let ctx =
        SecurityRulesFullContext::new(&format!("trailmark-prod-srr05-{project_suffix}")).await;
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
// AC-17-122: a delete of a document the subscriber's own rule would have
// admitted is delivered as a real DocumentDelete event.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Maria's compliant subscription; her own default-seeded document
///          (`owner_id == "maria-santos"`)
///   When:  that document is deleted
///   Then:  a `DocumentDelete` event for it arrives on Maria's stream
///
/// AC-17-122
///
/// @driving_port @real-io @US-05 @AC-17-122
#[tokio::test]
async fn admitted_delete_is_delivered() {
    let (ctx, mut stream, maria_doc) = compliant_maria_subscription("admit").await;

    delete_document(&ctx, &maria_doc, None)
        .await
        .expect("delete of own compliant document should succeed");

    let event = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(&event, Some(CapturedEvent::Removed { document_name }) if document_name == &maria_doc),
        "AC-17-122: a delete of a document the subscriber's own rule would have admitted must \
         be delivered as a DocumentDelete event, got: {event:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-123: a delete of a document the subscriber's own rule would have
// DENIED is not delivered (Production-Data Taste Test).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Maria's compliant subscription
///   When:  a document she was NEVER entitled to see (`owner_id ==
///          "dana-kim"` from creation) is deleted
///   Then:  no removal event ever reaches Maria's stream — real,
///          timeout-based "no event arrives" assertion
///   And:   the stream is still alive: a subsequent compliant delete DOES
///          arrive
///
/// AC-17-123
///
/// @driving_port @real-io @US-05 @AC-17-123 @security-critical
#[tokio::test]
async fn denied_delete_is_withheld() {
    let (ctx, mut stream, maria_doc) = compliant_maria_subscription("deny").await;

    let danas_doc = format!(
        "projects/{}/databases/(default)/documents/journal_entries/srr05-danas-doc",
        ctx.project_id
    );
    create_document(
        &ctx,
        "journal_entries",
        "srr05-danas-doc",
        std::collections::HashMap::from([("owner_id".to_string(), string_field("dana-kim"))]),
        None,
    )
    .await
    .expect("write of dana's own document should succeed");

    delete_document(&ctx, &danas_doc, None)
        .await
        .expect("delete of dana's document should succeed");

    let withheld = try_recv_live_event(&mut stream, Duration::from_millis(1500)).await;
    assert!(
        withheld.is_none(),
        "AC-17-123: a delete of a document the subscriber's own rule would have denied must \
         never be delivered, got: {withheld:?}"
    );

    // Liveness check: the stream is still alive — a subsequent compliant
    // delete (her own doc) DOES arrive.
    delete_document(&ctx, &maria_doc, None)
        .await
        .expect("delete of own compliant document should succeed");
    let live = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(live, Some(CapturedEvent::Removed { .. })),
        "AC-17-123: the withheld delete must not have broken the stream — a subsequent \
         compliant delete must still be delivered, got: {live:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-124: a document that became non-compliant BEFORE deletion does not
// deliver its subsequent deletion either.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Maria's compliant subscription
///   When:  a document she DID own (`owner_id == "maria-santos"` at
///          creation) has its `owner_id` reassigned to `"dana-kim"`
///          (withheld per Slice 04), then is deleted
///   Then:  neither the reassignment NOR the subsequent delete is ever
///          delivered to Maria's stream
///   And:   the stream is still alive: a subsequent compliant delete DOES
///          arrive
///
/// AC-17-124
///
/// @driving_port @real-io @US-05 @AC-17-124 @security-critical
#[tokio::test]
async fn delete_of_a_document_that_lost_compliance_before_deletion_is_withheld() {
    let (ctx, mut stream, maria_doc) = compliant_maria_subscription("reassign-then-delete").await;

    let reassigned_doc = format!(
        "projects/{}/databases/(default)/documents/journal_entries/srr05-reassigned-doc",
        ctx.project_id
    );
    create_document(
        &ctx,
        "journal_entries",
        "srr05-reassigned-doc",
        std::collections::HashMap::from([("owner_id".to_string(), string_field("maria-santos"))]),
        None,
    )
    .await
    .expect("write of maria's own document should succeed");

    // Drain the creation's own DocumentChange (admitted at creation time).
    let created_event = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(created_event, Some(CapturedEvent::Changed { .. })),
        "setup: creation of maria's own document must be delivered, got: {created_event:?}"
    );

    update_document(
        &ctx,
        &reassigned_doc,
        std::collections::HashMap::from([("owner_id".to_string(), string_field("dana-kim"))]),
        None,
    )
    .await
    .expect("owner_id reassignment write should succeed");

    let reassignment_event = try_recv_live_event(&mut stream, Duration::from_millis(1500)).await;
    assert!(
        reassignment_event.is_none(),
        "AC-17-124 setup: the reassignment itself must be withheld (Slice 04's own guarantee), \
         got: {reassignment_event:?}"
    );

    delete_document(&ctx, &reassigned_doc, None)
        .await
        .expect("delete of the now-non-compliant document should succeed");

    let withheld = try_recv_live_event(&mut stream, Duration::from_millis(1500)).await;
    assert!(
        withheld.is_none(),
        "AC-17-124: a document that became non-compliant before deletion must not deliver its \
         subsequent deletion either, got: {withheld:?}"
    );

    // Liveness check: the stream is still alive — a subsequent compliant
    // delete (her own doc) DOES arrive.
    delete_document(&ctx, &maria_doc, None)
        .await
        .expect("delete of own compliant document should succeed");
    let live = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(live, Some(CapturedEvent::Removed { .. })),
        "AC-17-124: the withheld delete must not have broken the stream — a subsequent \
         compliant delete must still be delivered, got: {live:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-125: a content-blind rule correctly decides and delivers delete
// events WITHOUT requiring the deleted document's own field data.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: a subscriber whose rule is content-blind (`request.auth !=
///          null` — never dereferences `resource.data`), subscribed to
///          `journal_entries` with no filter
///   When:  the default-seeded, field-EMPTY `{project}-no-owner-doc` is
///          deleted
///   Then:  the `DocumentDelete` event is still delivered correctly — the
///          content-blind rule's decision never needed the deleted
///          document's own field data
///
/// AC-17-125
///
/// @driving_port @real-io @US-05 @AC-17-125
#[tokio::test]
async fn content_blind_rule_delivers_delete_without_field_data() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srr05-blind").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("journal_entries", "request.auth != null")
        .await;

    let token = mint_client_identity_token(
        &signing_key,
        "any-authenticated-user",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut stream =
        open_listen_stream_filtered_as(&ctx, "journal_entries", None, Some(&token)).await;
    drain_until_current(&mut stream).await;

    let no_owner_doc = format!(
        "projects/{}/databases/(default)/documents/journal_entries/{}-no-owner-doc",
        ctx.project_id, ctx.project_id
    );

    delete_document(&ctx, &no_owner_doc, None)
        .await
        .expect("delete of the field-empty document should succeed");

    let event = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(&event, Some(CapturedEvent::Removed { document_name }) if document_name == &no_owner_doc),
        "AC-17-125: a content-blind rule must correctly decide and deliver a delete event \
         without requiring the deleted document's own field data, got: {event:?}"
    );
}
