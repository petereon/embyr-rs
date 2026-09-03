//! RW03 (Slice 03, Walking Skeleton, US-03, Release 1) — The Same
//! Precedence-Aware Mechanism Gates Writes and Listen's Per-Event Re-Check.
//!
//! Acceptance criteria verified here (feature-delta.md US-03, ADR-064,
//! slice-03-write-and-listen-parity.md):
//!   AC-17-245: a signed-in end user whose identity satisfies a
//!              precedence-resolved recursive-wildcard pattern's own write
//!              condition can create/update/delete a document governed
//!              exclusively by that pattern.
//!   AC-17-246: a caller whose identity does not satisfy that condition is
//!              denied create/update/delete against a document governed
//!              exclusively by that pattern, before the write reaches the
//!              storage adapter.
//!   AC-17-247: Listen's per-event `Changed`/`Removed` re-check resolves the
//!              precedence-resolved pattern's own outcome correctly (not
//!              `None`/always-deny) for a catch-all-governed document.
//!   AC-17-248: a denied write or a denied live-update delivery has zero
//!              observable side effect.
//!   AC-17-249: a specific rule's own write behavior (4a or 4b) is
//!              completely unaffected by the existence of a co-existing,
//!              structurally-overlapping recursive-wildcard catch-all — the
//!              specific rule always wins, on writes exactly as on reads
//!              (AC-17-239, extended).
//!
//! Setup uses the REAL admin import endpoint (Slice 01), mirroring both
//! RW02's and PM03's own setup discipline exactly — the SAME imported
//! `expeditions/{expeditionId}/{path=**}` catch-all (plus a co-existing 4b
//! `journal_entries` pattern) gates write, and Listen's per-event re-check.
//!
//! Driving port: admin HTTP :9090 (import) + gRPC :8080
//! `CreateDocument`/`UpdateDocument`/`DeleteDocument`/`Listen`, imported
//! directly from `security_rules_realtime`'s own fixture module (mirrors
//! PM03's own documented reason: avoiding a duplicate, nominally-distinct
//! `SecurityRulesFullContext` type within this one test binary — the same
//! real-Listen-stream fixture pattern PM03 established, reused rather than
//! invented anew, per the dispatch's own explicit instruction).

#![allow(unused_imports)]

#[path = "../../security_rules_realtime/common/mod.rs"]
mod common;
use common::{
    create_document, delete_document, drain_until_current, mint_client_identity_token, now_unix,
    open_listen_stream_filtered_as, string_field, try_recv_live_event, update_document,
    CapturedEvent, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use std::time::Duration;

/// Mirrors RW02's own `PRECEDENCE_RULES_FILE` exactly: a 4b fixed-depth
/// pattern (owner-only on `journal_entries`) co-exists with a broader
/// recursive-wildcard catch-all (`expeditions/{expeditionId}/{path=**}`,
/// any signed-in user) that structurally reaches the SAME `journal_entries`
/// documents too — proving the 4b pattern always wins on writes exactly as
/// RW02 proved it wins on reads (AC-17-249), while `photos` (governed ONLY
/// by the catch-all) proves the catch-all itself gates writes and Listen
/// (AC-17-245/246/247/248).
// Separate read/write clauses on `journal_entries` (mirrors PM03's own
// `EXPEDITIONS_RULES_FILE` exactly): `resource.data.<field>` is always empty
// for Create, so the write clause ORs the proposed-document check (Create)
// with the existing-document check (Update/Delete) — the SAME
// `||`-tolerates-one-side-FieldMissing composition ADR-034 established.
const PRECEDENCE_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId}/journal_entries/{entryId} {
      allow read: if request.auth.uid == resource.data.owner_id;
      allow write: if request.auth.uid == request.resource.data.owner_id || request.auth.uid == resource.data.owner_id;
    }
    match /expeditions/{expeditionId}/{path=**} {
      allow read, write: if request.auth.uid != "";
    }
  }
}
"#;

async fn setup(project_id: &str) -> (SecurityRulesFullContext, SigningKey) {
    let ctx = SecurityRulesFullContext::new(project_id).await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{project_id}/access_rules/import")))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": PRECEDENCE_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(resp.status().as_u16(), 200, "setup: rules-file import must succeed");

    (ctx, signing_key)
}

fn photo_resource_name(project_id: &str, expedition_id: &str, photo_id: &str) -> String {
    format!(
        "projects/{project_id}/databases/(default)/documents/expeditions/{expedition_id}/photos/{photo_id}"
    )
}

fn journal_entry_resource_name(project_id: &str, expedition_id: &str, entry_id: &str) -> String {
    format!(
        "projects/{project_id}/databases/(default)/documents/expeditions/{expedition_id}/journal_entries/{entry_id}"
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-245: a signed-in end user satisfying the catch-all's own write
// condition can create, update, and delete a document governed exclusively
// by the recursive-wildcard pattern.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `expeditions/{expeditionId}/{path=**}` allows any signed-in user
///          to write (no exact-match rule, no 4b pattern governs `photos`)
///   When:  Maria calls `setDoc()` to create a photo, then `updateDoc()`,
///          then `deleteDoc()` on that same document
///   Then:  all three succeed — the catch-all governs the entire write
///          lifecycle, not merely reads
///
/// AC-17-245
///
/// @walking_skeleton @driving_port @real-io @US-03 @AC-17-245
#[tokio::test]
async fn a_signed_in_user_satisfying_the_catch_all_can_create_update_and_delete_a_governed_document() {
    let (ctx, signing_key) = setup("trailmark-prod-rw03-cud").await;
    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);
    let resource = photo_resource_name(&ctx.project_id, "trek-2026", "img-042");

    let create_resp = create_document(
        &ctx,
        "expeditions/trek-2026/photos",
        "img-042",
        std::collections::HashMap::new(),
        Some(&marias_token),
    )
    .await;
    assert!(
        create_resp.is_ok(),
        "AC-17-245: create under a catch-all-governed collection must succeed: {:?}",
        create_resp.err()
    );

    let update_resp = update_document(
        &ctx,
        &resource,
        std::collections::HashMap::from([("caption".to_string(), string_field("summit"))]),
        Some(&marias_token),
    )
    .await;
    assert!(
        update_resp.is_ok(),
        "AC-17-245: update under a catch-all-governed collection must succeed: {:?}",
        update_resp.err()
    );

    let delete_resp = delete_document(&ctx, &resource, Some(&marias_token)).await;
    assert!(
        delete_resp.is_ok(),
        "AC-17-245: delete under a catch-all-governed collection must succeed: {:?}",
        delete_resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-246/248: a caller not satisfying the catch-all's own write condition
// is denied create/update/delete, before any write reaches storage, with
// zero observable side effect.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: the SAME catch-all, an existing photo Maria created
///   When:  an anonymous caller (no client-identity token) attempts to
///          create a sibling photo, update Maria's photo, and delete it
///   Then:  all three are denied with `PermissionDenied`, and Maria's own
///          document is completely unaffected by the denied attempts
///
/// AC-17-246, AC-17-248
///
/// @error @driving_port @real-io @US-03 @AC-17-246 @AC-17-248
#[tokio::test]
async fn an_unsatisfying_caller_is_denied_create_update_delete_with_zero_observable_side_effect() {
    let (ctx, signing_key) = setup("trailmark-prod-rw03-denied").await;
    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);
    let resource = photo_resource_name(&ctx.project_id, "trek-2026", "img-042");

    create_document(
        &ctx,
        "expeditions/trek-2026/photos",
        "img-042",
        std::collections::HashMap::new(),
        Some(&marias_token),
    )
    .await
    .expect("setup: Maria's own create must succeed");

    let anon_create = create_document(
        &ctx,
        "expeditions/trek-2026/photos",
        "img-099",
        std::collections::HashMap::new(),
        None,
    )
    .await;
    let err = anon_create.expect_err("AC-17-246: anonymous create under the catch-all must be denied");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());

    let anon_update = update_document(
        &ctx,
        &resource,
        std::collections::HashMap::from([("caption".to_string(), string_field("hijacked"))]),
        None,
    )
    .await;
    let err = anon_update.expect_err("AC-17-246: anonymous update under the catch-all must be denied");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());

    let anon_delete = delete_document(&ctx, &resource, None).await;
    let err = anon_delete.expect_err("AC-17-246: anonymous delete under the catch-all must be denied");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());

    // AC-17-248: zero observable side effect — the document still exists,
    // with none of the anonymous caller's attempted mutations landed.
    let resp = ctx
        .get_document(&resource, Some(&marias_token))
        .await
        .expect("AC-17-248: the document must still exist — no denied delete landed");
    assert!(
        !resp.get_ref().fields.contains_key("caption"),
        "AC-17-248: the denied update must have zero observable side effect"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-247: Listen's per-event re-check honors the precedence-resolved
// catch-all — delivered for a satisfying user, withheld for a non-satisfying
// one.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Maria holds an active `onSnapshot()` subscription on
///          `expeditions/trek-2026/photos`, a catch-all-governed collection
///   When:  her own photo changes
///   Then:  the live event is delivered, re-checked against the
///          precedence-resolved catch-all pattern
///
/// AC-17-247
///
/// @driving_port @real-io @US-03 @AC-17-247
#[tokio::test]
async fn a_live_subscriptions_per_event_recheck_delivers_for_a_satisfying_user_under_the_catch_all() {
    let (ctx, signing_key) = setup("trailmark-prod-rw03-listen-allow").await;
    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    create_document(
        &ctx,
        "expeditions/trek-2026/photos",
        "img-042",
        std::collections::HashMap::new(),
        Some(&marias_token),
    )
    .await
    .expect("setup: Maria's own create must succeed");

    let mut stream = open_listen_stream_filtered_as(
        &ctx,
        "expeditions/trek-2026/photos",
        None,
        Some(&marias_token),
    )
    .await;
    drain_until_current(&mut stream).await;

    update_document(
        &ctx,
        &photo_resource_name(&ctx.project_id, "trek-2026", "img-042"),
        std::collections::HashMap::from([("caption".to_string(), string_field("summit day"))]),
        Some(&marias_token),
    )
    .await
    .expect("Maria's own compliant update should succeed");

    let event = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(event, Some(CapturedEvent::Changed { .. })),
        "AC-17-247: a live change event for a catch-all-governed document must be delivered, \
         re-checked against the precedence-resolved pattern, got: {event:?}"
    );
}

/// Journey:
///   Given: an anonymous subscriber (no verified identity at all) holds an
///          `onSnapshot()` subscription on the catch-all-governed
///          collection — the catch-all's own condition is
///          `request.auth.uid != ""`, which an anonymous caller never
///          satisfies
///   When:  a photo in the catch-all-governed collection changes
///   Then:  the anonymous subscriber's stream never receives the update —
///          the catch-all's own condition is correctly enforced per event,
///          not silently defaulted to unrestricted (the pre-Slice-03 gap)
///
/// AC-17-247, AC-17-248
///
/// @error @driving_port @real-io @US-03 @AC-17-247 @AC-17-248 @security-critical
#[tokio::test]
async fn a_live_subscriptions_per_event_recheck_withholds_delivery_for_a_non_satisfying_subscriber() {
    let (ctx, signing_key) = setup("trailmark-prod-rw03-listen-deny").await;
    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    create_document(
        &ctx,
        "expeditions/trek-2026/photos",
        "img-042",
        std::collections::HashMap::new(),
        Some(&marias_token),
    )
    .await
    .expect("setup: Maria's own create must succeed");

    // No verified identity at all — does NOT satisfy `request.auth.uid != ""`.
    let mut stream =
        open_listen_stream_filtered_as(&ctx, "expeditions/trek-2026/photos", None, None).await;
    drain_until_current(&mut stream).await;

    update_document(
        &ctx,
        &photo_resource_name(&ctx.project_id, "trek-2026", "img-042"),
        std::collections::HashMap::from([("caption".to_string(), string_field("not for anon"))]),
        Some(&marias_token),
    )
    .await
    .expect("Maria's own compliant update should succeed");

    let withheld = try_recv_live_event(&mut stream, Duration::from_millis(1500)).await;
    assert!(
        withheld.is_none(),
        "AC-17-247/248: a live change event for a catch-all-governed document must be withheld \
         from a subscriber whose identity does not satisfy the catch-all's own condition — proving \
         real per-event enforcement, not a silent unrestricted default, got: {withheld:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-249: a specific (4b) rule's own write behavior is completely
// unaffected by the co-existing, structurally-overlapping recursive-wildcard
// catch-all — the specific rule always wins, on writes exactly as on reads.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has BOTH a 4b owner-only write pattern AND the
///          broader catch-all that would allow any signed-in user to write
///   When:  Dana (signed in, NOT the owner — satisfies the catch-all's own
///          condition but not the 4b pattern's own owner-only condition)
///          attempts to update Maria's journal entry
///   Then:  the write is denied — the 4b pattern's own owner-only condition
///          governs, proving the catch-all was never even consulted (it
///          would have allowed Dana, since she IS signed in)
///
/// AC-17-249
///
/// @error @driving_port @real-io @US-03 @AC-17-249
#[tokio::test]
async fn a_4b_specific_rules_write_behavior_is_unaffected_by_the_co_existing_catch_all() {
    let (ctx, signing_key) = setup("trailmark-prod-rw03-4bwins-write").await;
    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    create_document(
        &ctx,
        "expeditions/trek-2026/journal_entries",
        "entry-042",
        std::collections::HashMap::from([("owner_id".to_string(), string_field("maria-santos"))]),
        Some(&marias_token),
    )
    .await
    .expect("setup: Maria's own create of her own entry must succeed");

    let dana_resp = update_document(
        &ctx,
        &journal_entry_resource_name(&ctx.project_id, "trek-2026", "entry-042"),
        std::collections::HashMap::from([
            ("owner_id".to_string(), string_field("maria-santos")),
            ("caption".to_string(), string_field("hijacked")),
        ]),
        Some(&danas_token),
    )
    .await;
    let err = dana_resp.expect_err(
        "AC-17-249: the 4b owner-only pattern must govern the write — Dana is signed in (which \
         the overlapping catch-all alone would allow) but is NOT the owner",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-249: denial must come from the 4b pattern's own owner-only condition, got {:?}",
        err.code()
    );

    let maria_resp = update_document(
        &ctx,
        &journal_entry_resource_name(&ctx.project_id, "trek-2026", "entry-042"),
        std::collections::HashMap::from([
            ("owner_id".to_string(), string_field("maria-santos")),
            ("caption".to_string(), string_field("summit day")),
        ]),
        Some(&marias_token),
    )
    .await;
    assert!(
        maria_resp.is_ok(),
        "AC-17-249: the owner's own write, gated by the SAME 4b pattern, must still succeed \
         exactly as before the catch-all was imported: {:?}",
        maria_resp.err()
    );
}
