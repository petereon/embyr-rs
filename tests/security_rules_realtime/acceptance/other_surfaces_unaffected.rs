//! Slice 06 (US-06, ADR-033) — GetDocument, Writes, and RunQuery Remain
//! Unaffected, and an Unruled Collection's Content Stays Unrestricted.
//!
//! Pure regression/independence-proof slice (per its own slice doc, § OUT
//! Scope): no new production logic is expected — Slices 01-05 already built
//! this feature's own entire mechanism, structurally isolated inside
//! `crates/embyr-server/src/realtime/*` plus two visibility-only widenings
//! (`fn` -> `pub(crate) fn`, zero behavior change) in `grpc/handler.rs`
//! (`translate_filter`, `query_compliance_rejection`) — confirmed directly by
//! reading `git diff 95979a0^..a44e5c7 -- crates/embyr-server/src/grpc/handler.rs`
//! before writing this file. Mirrors
//! `security_rules_collection_group_rules::acceptance::other_surfaces_unaffected`
//! (Slice 05) and `security_rules_query_path`'s own Slice 06 discipline
//! exactly, extended here with the STRONGEST possible contrast this feature
//! introduces: a collection carrying an ACTIVE rule AND an ACTIVE Listen
//! subscription SIMULTANEOUSLY.
//!
//! Confirmed directly (not assumed) by reading `handle_get_document`
//! (lines 506-629), `handle_create_document`/`handle_update_document`/
//! `handle_delete_document`, and BOTH arms of `handle_run_query`
//! (lines 1131-1364): none of the five references `listen_registry`/
//! `ListenRegistry` anywhere — the only three call sites in the whole file
//! are the `ListenRegistry` field declaration/import and `handle_listen`
//! itself (lines 57, 71, 1412, 1461, 1470).
//!
//! Acceptance criteria verified here (slice-06-other-surfaces-unaffected.md):
//!   AC-17-126: `GetDocument`'s rule-lookup behavior is byte-for-byte
//!              unmodified — a real admit/deny domain proof, unaffected by a
//!              concurrently open Listen subscription on the SAME collection.
//!   AC-17-127: write-path's rule-lookup behavior is byte-for-byte unmodified
//!              — same admit/deny proof shape for `CreateDocument`, with a
//!              concurrently open Listen subscription.
//!   AC-17-128: `RunQuery`'s rule-lookup behavior (BOTH the non-group
//!              `access_rules` arm and the group `group_access_rules` arm)
//!              is byte-for-byte unmodified — same admit/deny proof shape for
//!              both variants, each with a concurrently open Listen
//!              subscription.
//!   AC-17-129: an unruled collection's (`app_config`) Listen subscription
//!              CONTENT remains fully unrestricted — the initial snapshot AND
//!              a subsequent live `Changed` and `Removed` event are all
//!              delivered exactly as before this feature shipped, content-wise
//!              — proving Slice 01's cross-collection SCOPE fix is genuinely
//!              orthogonal to content restriction.
//!
//! Driving ports: gRPC :8080 `GetDocument`/`CreateDocument`/`RunQuery`/
//! `Listen` — all via the real production composition root
//! (`SecurityRulesFullContext`), real Postgres NOTIFY fan-out, no mocks.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, delete_document, drain_until_current, equality_where_filter,
    mint_client_identity_token, now_unix, open_listen_stream, open_listen_stream_filtered_as,
    run_query_raw, seed_group_access_rule_full, seed_write_access_rule_full, string_field,
    try_recv_live_event, CapturedEvent, SecurityRulesFullContext,
};

use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use std::{collections::HashMap, time::Duration};

/// The ownership-equality condition every prior epic in this initiative has
/// used for `journal_entries` (`security-rules`'s own original rule text) —
/// reused verbatim, never a new grammar shape.
const OWNERSHIP_READ_RULE: &str = "request.auth.uid == resource.data.owner_id";

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-126: GetDocument's rule-lookup behavior is byte-for-byte unmodified,
// unaffected by a concurrently open Listen subscription on the SAME
// collection.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (parametrized — 2 input variations of the SAME behavior, per
/// Mandate 5):
///   Given: `journal_entries` has an active read rule AND an active Listen
///          subscription open on it simultaneously
///   When:  a caller who satisfies/does not satisfy the rule calls
///          `getDoc()` on Maria's own document
///   Then:  the same admit/deny outcome `security-rules` has always given is
///          produced, completely unaffected by the concurrent Listen
///          subscription
///
/// AC-17-126
///
/// @driving_port @real-io @US-06 @AC-17-126 @security-critical
#[tokio::test]
async fn get_document_unaffected_by_this_features_own_new_code_with_active_listen_subscription() {
    for (case, tag, caller_uid, expect_ok) in [
        ("caller satisfies the rule", "allow", "maria-santos", true),
        ("caller does not satisfy the rule", "deny", "dana-kim", false),
    ] {
        let ctx = SecurityRulesFullContext::new(&format!("trailmark-prod-srrt06-getdoc-{tag}"))
            .await;
        let signing_key = SigningKey::generate(&mut OsRng);
        ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
            .await;
        ctx.seed_access_rule("journal_entries", OWNERSHIP_READ_RULE)
            .await;

        // Active Listen subscription on the SAME collection, open for the
        // entire GetDocument exchange below — the strongest possible
        // contrast this feature introduces (US-06's own fixture).
        let marias_token = mint_client_identity_token(
            &signing_key,
            "maria-santos",
            &ctx.project_id,
            now_unix() + 3600,
        );
        let filter = equality_where_filter(&[("owner_id", "maria-santos")]);
        let mut stream = open_listen_stream_filtered_as(
            &ctx,
            "journal_entries",
            filter,
            Some(&marias_token),
        )
        .await;
        drain_until_current(&mut stream).await;

        let callers_token = mint_client_identity_token(
            &signing_key,
            caller_uid,
            &ctx.project_id,
            now_unix() + 3600,
        );
        let marias_doc = ctx.document_resource_name("journal_entries", "maria-doc");
        let result = ctx.get_document(&marias_doc, Some(&callers_token)).await;

        if expect_ok {
            assert!(
                result.is_ok(),
                "AC-17-126 ({case}): GetDocument must succeed exactly as security-rules already \
                 left it, unaffected by the concurrently open Listen subscription: {:?}",
                result.err()
            );
        } else {
            assert_eq!(
                result
                    .expect_err(
                        "AC-17-126 ({case}): GetDocument must deny exactly as security-rules \
                         already left it"
                    )
                    .code(),
                tonic::Code::PermissionDenied,
                "AC-17-126 ({case}): denial must be attributable to the rule alone"
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-127: write-path's rule-lookup behavior is byte-for-byte unmodified,
// unaffected by a concurrently open Listen subscription on the SAME
// collection.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (parametrized — 2 input variations of the SAME behavior, per
/// Mandate 5):
///   Given: `journal_entries` has an active write rule AND an active Listen
///          subscription open on it simultaneously
///   When:  Maria creates a new entry whose proposed `owner_id`
///          satisfies/does not satisfy the write rule
///   Then:  the same admit/deny outcome `security-rules-write-path` has
///          always given is produced, completely unaffected by the
///          concurrent Listen subscription
///
/// AC-17-127
///
/// @driving_port @real-io @US-06 @AC-17-127 @security-critical
#[tokio::test]
async fn write_path_unaffected_by_this_features_own_new_code_with_active_listen_subscription() {
    for (case, tag, proposed_owner, expect_ok) in [
        ("proposed document satisfies the write rule", "allow", "maria-santos", true),
        ("proposed document fails the write rule", "deny", "dana-kim", false),
    ] {
        let ctx = SecurityRulesFullContext::new(&format!("trailmark-prod-srrt06-write-{tag}"))
            .await;
        let signing_key = SigningKey::generate(&mut OsRng);
        ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
            .await;
        seed_write_access_rule_full(
            &ctx,
            "journal_entries",
            "request.resource.data.owner_id == request.auth.uid",
        )
        .await;

        // Active Listen subscription on the SAME collection, open for the
        // entire CreateDocument exchange below.
        let marias_token = mint_client_identity_token(
            &signing_key,
            "maria-santos",
            &ctx.project_id,
            now_unix() + 3600,
        );
        let filter = equality_where_filter(&[("owner_id", "maria-santos")]);
        let mut stream = open_listen_stream_filtered_as(
            &ctx,
            "journal_entries",
            filter,
            Some(&marias_token),
        )
        .await;
        drain_until_current(&mut stream).await;

        let fields = HashMap::from([("owner_id".to_string(), string_field(proposed_owner))]);
        let resp = create_document(
            &ctx,
            "journal_entries",
            &format!("srrt06-write-{tag}-doc"),
            fields,
            Some(&marias_token),
        )
        .await;

        if expect_ok {
            assert!(
                resp.is_ok(),
                "AC-17-127 ({case}): create must succeed exactly as security-rules-write-path \
                 already left it, unaffected by the concurrently open Listen subscription: {:?}",
                resp.err()
            );
        } else {
            assert_eq!(
                resp.expect_err(
                    "AC-17-127 ({case}): create must be denied exactly as \
                     security-rules-write-path already left it"
                )
                .code(),
                tonic::Code::PermissionDenied,
                "AC-17-127 ({case}): denial must be attributable to the write rule alone"
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-128: RunQuery's rule-lookup behavior (both access_rules non-group
// AND group_access_rules group-query arms) is byte-for-byte unmodified,
// unaffected by a concurrently open Listen subscription on the SAME
// collection.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (parametrized — 2 input variations of the SAME behavior across
/// RunQuery's two mutually-exclusive arms, per Mandate 5):
///   Given: `journal_entries` has an active rule of the arm-appropriate kind
///          (non-group `access_rules` or group `group_access_rules`) AND an
///          active Listen subscription open on it simultaneously
///   When:  Maria issues a compliant query and Dana issues a non-compliant
///          query, in the arm-appropriate shape (`all_descendants`
///          false/true)
///   Then:  the same admit/deny outcome `security-rules-query-path`/
///          `security-rules-collection-group-rules` have always given is
///          produced, completely unaffected by the concurrent Listen
///          subscription
///
/// AC-17-128
///
/// @driving_port @real-io @US-06 @AC-17-128 @security-critical
#[tokio::test]
async fn run_query_group_and_non_group_unaffected_by_this_features_own_new_code_with_active_listen_subscription()
{
    for (case, tag, all_descendants) in [
        ("non-group RunQuery (access_rules)", "nongroup", false),
        ("group RunQuery (group_access_rules)", "group", true),
    ] {
        let ctx = SecurityRulesFullContext::new(&format!("trailmark-prod-srrt06-query-{tag}"))
            .await;
        let signing_key = SigningKey::generate(&mut OsRng);
        ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
            .await;
        if all_descendants {
            seed_group_access_rule_full(&ctx, "journal_entries", OWNERSHIP_READ_RULE).await;
        } else {
            ctx.seed_access_rule("journal_entries", OWNERSHIP_READ_RULE)
                .await;
        }

        // Active Listen subscription on the SAME collection, open for the
        // entire RunQuery exchange below.
        let marias_token = mint_client_identity_token(
            &signing_key,
            "maria-santos",
            &ctx.project_id,
            now_unix() + 3600,
        );
        let filter = equality_where_filter(&[("owner_id", "maria-santos")]);
        let mut stream = open_listen_stream_filtered_as(
            &ctx,
            "journal_entries",
            filter,
            Some(&marias_token),
        )
        .await;
        drain_until_current(&mut stream).await;

        let danas_token = mint_client_identity_token(
            &signing_key,
            "dana-kim",
            &ctx.project_id,
            now_unix() + 3600,
        );

        let dana_result = run_query_raw(
            &ctx,
            "journal_entries",
            all_descendants,
            equality_where_filter(&[("owner_id", "maria-santos")]),
            Some(&danas_token),
        )
        .await;
        assert_eq!(
            dana_result
                .expect_err(&format!(
                    "AC-17-128 ({case}): Dana's non-compliant query must be denied exactly as \
                     the owning prior epic already left it, unaffected by the concurrently open \
                     Listen subscription"
                ))
                .code(),
            tonic::Code::PermissionDenied,
            "AC-17-128 ({case}): denial must be attributable to the rule alone"
        );

        let maria_result = run_query_raw(
            &ctx,
            "journal_entries",
            all_descendants,
            equality_where_filter(&[("owner_id", "maria-santos")]),
            Some(&marias_token),
        )
        .await;
        assert!(
            maria_result.is_ok(),
            "AC-17-128 ({case}): Maria's compliant query must succeed exactly as the owning \
             prior epic already left it, unaffected by the concurrently open Listen \
             subscription: {:?}",
            maria_result.err()
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-129: an unruled collection's (app_config) Listen subscription
// CONTENT remains fully unrestricted — initial snapshot AND every subsequent
// live event, exactly as before this feature shipped.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `app_config` has never had any rule (`access_rules` returns
///          `None` for it — the ONLY path US-06's own "content fully
///          unrestricted" behavior reaches)
///   When:  an anonymous session (no client-identity token at all) opens a
///          Listen subscription to `app_config`
///   Then:  the initial snapshot delivers the context's own default-seeded
///          `app_config` document, unfiltered by identity
///   When:  a new `app_config` document is created, then deleted
///   Then:  both the live `Changed` and `Removed` events are delivered,
///          unwithheld — proving Slice 01's cross-collection SCOPE fix
///          (US-01) is genuinely orthogonal to content restriction: an
///          unruled collection's Listen subscription is not accidentally
///          gated by anything Slices 03-05 added
///
/// AC-17-129
///
/// @driving_port @real-io @US-06 @AC-17-129
#[tokio::test]
async fn unruled_collections_listen_content_remains_fully_unrestricted() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srrt06-unruled").await;

    // No rule of any kind is ever seeded for app_config — the structural
    // guardrail (get_access_rule -> None) US-06's own technical note names
    // as the ONLY path that reaches this behavior.
    let mut stream = open_listen_stream(&ctx, "app_config").await;
    let initial_snapshot = common::collect_initial_snapshot(&mut stream).await;

    let seeded_config_doc = format!(
        "projects/{}/databases/(default)/documents/app_config/{}-config-doc",
        ctx.project_id, ctx.project_id
    );
    assert!(
        initial_snapshot.contains(&seeded_config_doc),
        "AC-17-129: the context's own default-seeded app_config document must be delivered in \
         the initial snapshot, unrestricted by any identity/filter, got: {initial_snapshot:?}"
    );

    create_document(
        &ctx,
        "app_config",
        "srrt06-new-config-doc",
        HashMap::from([("note".to_string(), string_field("unrestricted content check"))]),
        None,
    )
    .await
    .expect("create against an unruled collection should succeed unrestricted");
    let created = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(created, Some(CapturedEvent::Changed { .. })),
        "AC-17-129: a new document's live Changed event must be delivered to an unruled \
         collection's subscriber, content unrestricted, got: {created:?}"
    );

    let new_doc_resource_name = format!(
        "projects/{}/databases/(default)/documents/app_config/srrt06-new-config-doc",
        ctx.project_id
    );
    delete_document(&ctx, &new_doc_resource_name, None)
        .await
        .expect("delete against an unruled collection should succeed unrestricted");
    let removed = try_recv_live_event(&mut stream, Duration::from_millis(2000)).await;
    assert!(
        matches!(removed, Some(CapturedEvent::Removed { .. })),
        "AC-17-129: a deleted document's live Removed event must be delivered to an unruled \
         collection's subscriber, never withheld by the delete-event non-leakage mechanism \
         (US-05, which only applies when a rule exists), got: {removed:?}"
    );
}
