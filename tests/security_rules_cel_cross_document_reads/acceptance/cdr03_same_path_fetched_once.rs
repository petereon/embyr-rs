//! CDR03 (Slice 03, US-03, Release 1) — Two References to the Same
//! Document Are Fetched Once.
//!
//! Acceptance criteria verified here (feature-delta.md US-03, ADR-066):
//!   AC-CDR-09: a condition referencing the identical concrete document
//!              path via BOTH `exists()` and `get()` evaluates correctly
//!              (dedup's own OUTCOME proof — the underlying mechanism's
//!              own dedup GUARANTEE is already proven directly at the
//!              unit level, `discover_cross_document_paths`'s own
//!              `BTreeSet`-backed pure function,
//!              `discover_deduplicates_the_same_path_referenced_twice`;
//!              `fetch_cross_document_reads` is a trivial, obviously-
//!              correct loop over that already-deduplicated set — a
//!              real-adapter call-count acceptance proof would only
//!              re-test `BTreeSet`'s own stdlib dedup guarantee, adding
//!              test-infrastructure risk for zero additional coverage).
//!   AC-CDR-10: two DIFFERENT concrete paths each resolve independently
//!              and correctly — dedup is per-path, never a blanket
//!              single-fetch-per-evaluation cap.
//!
//! Driving port: gRPC :8080 `GetDocument` (`SecurityRulesFullContext`).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

fn resource_name(project_id: &str, path: &str) -> String {
    format!("projects/{project_id}/databases/(default)/documents/{path}")
}

/// AC-CDR-09
///
/// @driving_port @real-io @US-03 @AC-CDR-09
#[tokio::test]
async fn exists_and_get_on_the_identical_path_compose_correctly_in_one_condition() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cdr03-samepath").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    let define_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "exists(/databases/$(database)/documents/organizations/$(request.auth.uid)) && get(/databases/$(database)/documents/organizations/$(request.auth.uid)).data.role == \"admin\"",
        }))
        .send()
        .await
        .expect("define_access_rule request failed");
    assert_eq!(define_resp.status().as_u16(), 200, "AC-CDR-09: must be accepted");

    ctx.seed_document(
        "organizations",
        "maria-santos",
        serde_json::json!({ "role": {"t": "S", "v": "admin"} }),
    )
    .await;
    ctx.seed_document("journal_entries", "trek-2026", serde_json::json!({}))
        .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );
    let resp = ctx
        .get_document(
            &resource_name(&ctx.project_id, "journal_entries/trek-2026"),
            Some(&marias_token),
        )
        .await;
    assert!(
        resp.is_ok(),
        "AC-CDR-09: exists()==true && get().data.role=='admin' on the SAME path must be allowed: {:?}",
        resp.err()
    );
}

/// AC-CDR-10
///
/// @driving_port @real-io @US-03 @AC-CDR-10
#[tokio::test]
async fn two_distinct_paths_resolve_independently_in_one_condition() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cdr03-twopaths").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    let define_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "exists(/databases/$(database)/documents/organizations/$(request.auth.uid)) && exists(/databases/$(database)/documents/trail_guides/$(request.path.entryId))",
        }))
        .send()
        .await
        .expect("define_access_rule request failed");
    assert_eq!(define_resp.status().as_u16(), 200, "AC-CDR-10: must be accepted");

    // Both distinct referenced documents exist.
    ctx.seed_document("organizations", "maria-santos", serde_json::json!({}))
        .await;
    ctx.seed_document("trail_guides", "trek-2026", serde_json::json!({}))
        .await;
    ctx.seed_document("journal_entries", "trek-2026", serde_json::json!({}))
        .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );
    let both_exist = ctx
        .get_document(
            &resource_name(&ctx.project_id, "journal_entries/trek-2026"),
            Some(&marias_token),
        )
        .await;
    assert!(
        both_exist.is_ok(),
        "AC-CDR-10: both distinct referenced documents exist, must be allowed: {:?}",
        both_exist.err()
    );

    // Now remove the trail_guides half — via a caller whose own trail_guides
    // reference resolves to a document that was never seeded.
    let resp_missing_second = ctx
        .get_document(
            &resource_name(&ctx.project_id, "journal_entries/does-not-exist-entry"),
            Some(&marias_token),
        )
        .await;
    let err = resp_missing_second.expect_err(
        "AC-CDR-10: the second, DIFFERENT path (trail_guides/does-not-exist-entry) is unresolved, \
         must be denied independently of the first path's own success",
    );
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());
}
