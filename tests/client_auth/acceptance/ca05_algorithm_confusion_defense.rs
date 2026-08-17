//! CA05 — Algorithm-Confusion Defense Regression (ADR-024 Enforcement,
//! security-review-verified per the DISTILL dispatch instructions).
//!
//! A dedicated acceptance-level regression test for ADR-024's algorithm
//! -pinning claim: a verifier that accepts multiple algorithms can be
//! tricked into treating a registered PUBLIC key's bytes as an HMAC secret
//! (`alg: HS256`), letting an attacker who knows the public key — not
//! secret, by Option C's own design — forge a valid-looking signature. This
//! must be rejected as Malformed, never accepted.
//!
//! This is the direct acceptance-level proof of the security-review claim;
//! `crates/embyr-core/src/client_identity/mod.rs`'s own unit test
//! (`hs256_token_using_the_public_key_bytes_as_hmac_secret_is_rejected_as_malformed`)
//! is the layer-1 proof. Both must independently hold.
//!
//! Driving port: REST :8081 (sign-in — the primary attacker-facing surface;
//! the debug-verify endpoint shares the identical verification routine per
//! ADR-025, so a pass here is evidence for both call sites).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_hs256_confusion_token, now_unix, ClientAuthFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

/// @error @driving_port @real-io @security-regression @ADR-024
#[tokio::test]
#[ignore = "RED — enable after ca01/ca02's walking skeletons are GREEN"]
async fn an_hs256_token_forged_from_the_registered_public_key_bytes_is_rejected_as_malformed() {
    let ctx = ClientAuthFullContext::new("trailmark-prod-ca05-alg-confusion").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let public_key_bytes = signing_key.verifying_key().to_bytes();
    ctx.seed_credential(&public_key_bytes).await;

    // An attacker who has learned the registered PUBLIC key (not secret, by
    // ADR-024's own design) crafts an HS256 token using those bytes as the
    // HMAC secret — the textbook JWT algorithm-confusion attack.
    let forged_token =
        mint_hs256_confusion_token(&public_key_bytes, "maria-santos", &ctx.project_id, now_unix() + 3600);

    let resp = reqwest::Client::new()
        .post(ctx.rest_url(&format!(
            "/v1/projects/{}/accounts:signInWithCustomToken",
            ctx.project_id
        )))
        .json(&serde_json::json!({"token": forged_token}))
        .send()
        .await
        .expect("sign-in request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "algorithm-confusion attack must be rejected, never accepted as a successful sign-in"
    );
    let body: serde_json::Value = resp.json().await.expect("sign-in response must be JSON");
    assert_eq!(
        body["reason"], "MALFORMED_TOKEN",
        "ADR-024 Enforcement: a non-EdDSA-algorithm token must be Malformed, never any other \
         reason — the verifier must be strictly single-algorithm (EdDSA only)"
    );
}
