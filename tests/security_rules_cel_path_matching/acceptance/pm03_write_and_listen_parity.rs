//! PM03 (Slice 03, Walking Skeleton — Activity B, US-03, Release 1) — The
//! Same Routing Mechanism Gates Writes and Listen's Per-Event Re-Check.
//!
//! Acceptance criteria verified here (feature-delta.md US-03, ADR-063,
//! slice-03-write-and-listen-parity.md):
//!   AC-17-213: the routed pattern's own contributor (by bound variable
//!              match) can create/update/delete their own routed document.
//!   AC-17-214: a different signed-in end user is denied create/update
//!              /delete against a routed document that is not theirs,
//!              before the write reaches the storage adapter.
//!   AC-17-215: the routed binding resolves identically for create (no
//!              pre-existing document) as for update/delete — derived from
//!              the request's own target path, never from fetched document
//!              content.
//!   AC-17-216: Listen's per-event `Changed`/`Removed` re-check resolves the
//!              routed pattern's binding correctly (not `None`/always-deny)
//!              — closes `OQ-CP-04` from 4a.
//!   AC-17-217: a denied write or a denied live-update delivery has zero
//!              observable side effect (no partial write, no leaked event
//!              payload).
//!
//! Setup uses the REAL admin import endpoint from Slice 01
//! (`POST .../access_rules/import`), mirroring PM02's own setup exactly —
//! the SAME imported `expeditions/{expeditionId}/journal_entries/{entryId}`
//! pattern (`allow read, write`) proves both verbs AND Listen are gated by
//! ONE import.
//!
//! Driving port: admin HTTP :9090 (import) + gRPC :8080
//! `CreateDocument`/`UpdateDocument`/`DeleteDocument`/`Listen`, via
//! `SecurityRulesFullContext` — imported directly from
//! `security_rules_realtime`'s own fixture module (not re-exported from
//! this feature's own `common/mod.rs`, mirroring `cp03_path_variable_gates_writes.rs`'s
//! own documented reason: avoiding a duplicate, nominally-distinct
//! `SecurityRulesFullContext` type within this one test binary). This is
//! also this feature's own established real-Listen-stream fixture source
//! (per the dispatch's own instruction to reuse it rather than invent a new
//! one).
//!
//! Error ratio: 3 error/edge (AC-17-214/217 folded into 3 of the 5 tests)
//! out of 5 = 60%.

#![allow(unused_imports)]

#[path = "../../security_rules_realtime/common/mod.rs"]
mod common;
use common::{
    create_document, delete_document, drain_until_current, equality_where_filter,
    mint_client_identity_token, now_unix, open_listen_stream_filtered_as, string_field,
    try_recv_live_event, update_document, CapturedEvent, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use std::time::Duration;

// Separate read/write clauses (both bucket into the SAME pattern row's
// `read_condition`/`write_condition` columns, `rules_file::decompose_block`,
// unchanged by this slice) — NOT a combined `allow read, write:` clause,
// deliberately: `resource.data.<field>` (the PRE-write document) is always
// empty for Create (AC-17-28, `security-rules-write-path`'s own established,
// pre-existing fail-closed mechanism — out of THIS slice's scope to change).
// The write clause ORs the proposed-document check (satisfies Create) with
// the existing-document check (satisfies Update/Delete, whose
// `request.resource.data` is respectively partial/always-empty) — the SAME
// `||`-tolerates-one-side-FieldMissing composition ADR-034 already
// established, not a new evaluation shape.
const EXPEDITIONS_RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId}/journal_entries/{entryId} {
      allow read: if request.auth.uid == resource.data.owner_id;
      allow write: if request.auth.uid == request.resource.data.owner_id || request.auth.uid == resource.data.owner_id;
    }
  }
}
"#;

/// Shared setup: a fresh `SecurityRulesFullContext`, the multi-segment
/// `expeditions/{expeditionId}/journal_entries/{entryId}` pattern imported
/// via Slice 01's real admin endpoint (not hand-seeded), and a signing key
/// registered for client-identity token verification — mirrors PM02's own
/// `setup()` exactly (the SAME imported pattern gates read, write, and
/// Listen).
async fn setup(project_id: &str) -> (SecurityRulesFullContext, SigningKey) {
    let ctx = SecurityRulesFullContext::new(project_id).await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{project_id}/access_rules/import")))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EXPEDITIONS_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(resp.status().as_u16(), 200, "setup: expeditions pattern import must succeed");

    (ctx, signing_key)
}

fn journal_entry_resource_name(project_id: &str, expedition_id: &str, entry_id: &str) -> String {
    format!(
        "projects/{project_id}/databases/(default)/documents/expeditions/{expedition_id}/journal_entries/{entry_id}"
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-213: the routed pattern's own contributor can update their entry.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `expeditions/{expeditionId}/journal_entries` has the imported
///          pattern requiring `request.auth.uid == resource.data.owner_id`
///   And:   Maria Santos holds a verified identity and
///          `expeditions/trek-2026/journal_entries/entry-042` exists with
///          `owner_id: "maria-santos"`
///   When:  Maria calls `updateDoc()` on that entry
///   Then:  the write succeeds
///
/// AC-17-213
///
/// @walking_skeleton @driving_port @real-io @US-03 @AC-17-213
#[tokio::test]
async fn a_routed_patterns_own_contributor_can_update_their_entry() {
    let (ctx, signing_key) = setup("trailmark-prod-pm03-update-owner").await;
    ctx.seed_document(
        "expeditions/trek-2026/journal_entries",
        "entry-042",
        serde_json::json!({ "owner_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    let mut fields = std::collections::HashMap::new();
    fields.insert("caption".to_string(), string_field("summit day"));

    let resp = update_document(
        &ctx,
        &journal_entry_resource_name(&ctx.project_id, "trek-2026", "entry-042"),
        fields,
        Some(&marias_token),
    )
    .await;

    assert!(
        resp.is_ok(),
        "AC-17-213: the routed pattern's own contributor's update of her own entry must succeed: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-214/217: a different signed-in end user cannot update someone else's
// routed entry — denied before any change reaches storage.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-214, AC-17-217
///
/// @error @driving_port @real-io @US-03 @AC-17-214 @AC-17-217
#[tokio::test]
async fn a_different_signed_in_end_user_cannot_update_someone_elses_routed_entry() {
    let (ctx, signing_key) = setup("trailmark-prod-pm03-update-nonowner").await;
    ctx.seed_document(
        "expeditions/trek-2026/journal_entries",
        "entry-042",
        serde_json::json!({ "owner_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let mut fields = std::collections::HashMap::new();
    fields.insert("caption".to_string(), string_field("hijacked"));

    let resp = update_document(
        &ctx,
        &journal_entry_resource_name(&ctx.project_id, "trek-2026", "entry-042"),
        fields,
        Some(&danas_token),
    )
    .await;

    let err = resp.expect_err(
        "AC-17-214: a non-owner's update of someone else's routed entry must be denied",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-214/217: denial must be attributable to the routed pattern's rule — the same \
         status the handler returns BEFORE any write reaches the storage adapter, got {:?}",
        err.code()
    );

    // AC-17-217: zero observable side effect — Maria's own subsequent read
    // of the entry must still show the ORIGINAL caption-less state, proving
    // Dana's denied write never partially landed.
    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);
    let resp = ctx
        .get_document(
            &journal_entry_resource_name(&ctx.project_id, "trek-2026", "entry-042"),
            Some(&marias_token),
        )
        .await
        .expect("Maria's own read must succeed");
    assert!(
        !resp.get_ref().fields.contains_key("caption"),
        "AC-17-217: Dana's denied write must have zero observable side effect — no 'caption' \
         field must exist"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-213/214/215: creating a new entry under a routed pattern is gated
// identically to updating one — the binding resolves from the target path,
// never fetched content (there is nothing to fetch yet).
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-213, AC-17-214, AC-17-215
///
/// @driving_port @real-io @US-03 @AC-17-213 @AC-17-214 @AC-17-215
#[tokio::test]
async fn creating_a_new_entry_under_a_routed_pattern_is_gated_identically_to_updating_one() {
    let (ctx, signing_key) = setup("trailmark-prod-pm03-create").await;
    // expeditions/trek-2026/journal_entries/entry-099 deliberately NEVER
    // seeded — Create is the FIRST write to this document.

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    // Dana attempts to create the entry claiming MARIA as its owner (an
    // impersonation attempt, not a self-claim — mirrors
    // `security-rules-write-path`'s own AC-17-27 "proposed document fails
    // the write rule" shape): `request.auth.uid` ("dana-kim") never matches
    // the proposed `owner_id` ("maria-santos"), denied.
    let dana_resp = create_document(
        &ctx,
        "expeditions/trek-2026/journal_entries",
        "entry-099",
        std::collections::HashMap::from([("owner_id".to_string(), string_field("maria-santos"))]),
        Some(&danas_token),
    )
    .await;
    let err = dana_resp.expect_err(
        "AC-17-214: Dana creating a routed entry claiming another contributor's ownership must be denied",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-214: denial must be attributable to the routed pattern's rule, got {:?}",
        err.code()
    );

    let maria_resp = create_document(
        &ctx,
        "expeditions/trek-2026/journal_entries",
        "entry-099",
        std::collections::HashMap::from([("owner_id".to_string(), string_field("maria-santos"))]),
        Some(&marias_token),
    )
    .await;
    assert!(
        maria_resp.is_ok(),
        "AC-17-213/215: Maria creating her own routed entry (no pre-existing document) must \
         succeed — the binding resolves from the target path, not fetched content: {:?}",
        maria_resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-213/214: deleting a routed entry is gated identically.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-213, AC-17-214
///
/// @driving_port @real-io @US-03 @AC-17-213 @AC-17-214
#[tokio::test]
async fn deleting_a_routed_entry_is_gated_identically() {
    let (ctx, signing_key) = setup("trailmark-prod-pm03-delete").await;
    ctx.seed_document(
        "expeditions/trek-2026/journal_entries",
        "entry-042",
        serde_json::json!({ "owner_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let dana_resp = delete_document(
        &ctx,
        &journal_entry_resource_name(&ctx.project_id, "trek-2026", "entry-042"),
        Some(&danas_token),
    )
    .await;
    let err =
        dana_resp.expect_err("AC-17-214: Dana deleting Maria's routed entry must be denied");
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-214: denial must be attributable to the routed pattern's rule, got {:?}",
        err.code()
    );

    let maria_resp = delete_document(
        &ctx,
        &journal_entry_resource_name(&ctx.project_id, "trek-2026", "entry-042"),
        Some(&marias_token),
    )
    .await;
    assert!(
        maria_resp.is_ok(),
        "AC-17-213: Maria deleting her own routed entry must succeed: {:?}",
        maria_resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-216/217: Listen's per-event re-check honors the routed pattern —
// the contributor's own live update is delivered, a non-contributor's
// equivalent subscription never receives it (closes 4a's own OQ-CP-04).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Maria holds an active `onSnapshot()` subscription on
///          `expeditions/trek-2026/journal_entries`, filtered to her own
///          `owner_id`
///   When:  her own routed entry changes
///   Then:  the live event is delivered, re-checked against the routed
///          pattern's binding for `trek-2026`
///
/// AC-17-216
///
/// @driving_port @real-io @US-03 @AC-17-216
#[tokio::test]
async fn a_live_subscriptions_per_event_recheck_honors_the_routed_pattern_for_the_contributor() {
    let (ctx, signing_key) = setup("trailmark-prod-pm03-listen-allow").await;
    ctx.seed_document(
        "expeditions/trek-2026/journal_entries",
        "entry-042",
        serde_json::json!({ "owner_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    let filter = equality_where_filter(&[("owner_id", "maria-santos")]);
    let mut stream = open_listen_stream_filtered_as(
        &ctx,
        "expeditions/trek-2026/journal_entries",
        filter,
        Some(&marias_token),
    )
    .await;
    drain_until_current(&mut stream).await;

    update_document(
        &ctx,
        &journal_entry_resource_name(&ctx.project_id, "trek-2026", "entry-042"),
        std::collections::HashMap::from([
            ("owner_id".to_string(), string_field("maria-santos")),
            ("caption".to_string(), string_field("still hers")),
        ]),
        Some(&marias_token),
    )
    .await
    .expect("Maria's own compliant update should succeed");

    let event = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(event, Some(CapturedEvent::Changed { .. })),
        "AC-17-216: a live change event for the routed pattern's own contributor must be \
         delivered, re-checked against the routed binding, got: {event:?}"
    );
}

/// Journey:
///   Given: Dana holds an equivalent `onSnapshot()` subscription on
///          `expeditions/trek-2026/journal_entries`, filtered to Maria's own
///          `owner_id` (Dana does not contribute to `trek-2026`)
///   When:  that document changes
///   Then:  Dana's subscription does NOT receive the update — closes 4a's
///          own deferred OQ-CP-04 gap
///
/// AC-17-216, AC-17-217
///
/// @error @driving_port @real-io @US-03 @AC-17-216 @AC-17-217 @security-critical
#[tokio::test]
async fn a_live_subscriptions_per_event_recheck_denies_delivery_for_a_non_contributor() {
    let (ctx, signing_key) = setup("trailmark-prod-pm03-listen-deny").await;
    ctx.seed_document(
        "expeditions/trek-2026/journal_entries",
        "entry-042",
        serde_json::json!({ "owner_id": {"t": "S", "v": "maria-santos"} }),
    )
    .await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);
    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    // Dana's own filter never references her own uid (she has none in this
    // entry's data) — the filter alone would admit the subscription (Listen's
    // subscribe-time compliance gate remains pattern-blind, OQ-PM-07); it is
    // the PER-EVENT re-check (US-03/AC-17-216) that must withhold delivery.
    let filter = equality_where_filter(&[("owner_id", "maria-santos")]);
    let mut stream = open_listen_stream_filtered_as(
        &ctx,
        "expeditions/trek-2026/journal_entries",
        filter,
        Some(&danas_token),
    )
    .await;
    drain_until_current(&mut stream).await;

    // Written by Maria (the entry's own owner) — the write rule itself must
    // admit this write; ONLY Dana's per-event re-check, not the writer's own
    // permission, is under test here (AC-17-216).
    update_document(
        &ctx,
        &journal_entry_resource_name(&ctx.project_id, "trek-2026", "entry-042"),
        std::collections::HashMap::from([
            ("owner_id".to_string(), string_field("maria-santos")),
            ("caption".to_string(), string_field("not danas to see")),
        ]),
        Some(&marias_token),
    )
    .await
    .expect("Maria's own compliant write should succeed");

    let withheld = try_recv_live_event(&mut stream, Duration::from_millis(1500)).await;
    assert!(
        withheld.is_none(),
        "AC-17-216/217: a live change event for a routed entry Dana does not contribute to must \
         be withheld, never delivered, got: {withheld:?}"
    );
}
