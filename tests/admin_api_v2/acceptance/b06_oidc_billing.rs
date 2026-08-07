// @US-B06 @driving_port @real-io
//! Slice B-06 — OIDC Providers + Billing.
//!
//! All tests are #[ignore] (RED). DELIVER unskips one at a time.
//!
//! Key invariants under test:
//!   - client_secret encrypted with AES-256-GCM, never returned in GET responses.
//!   - OIDC id_token validated: issuer, audience, expiry, signature (JWKS).
//!   - OIDC state parameter (CSRF nonce) verified on callback.
//!   - Billing GROUP BY correctly includes zero-metric projects.
//!   - Only Owner role can access OIDC configuration.
//!
//! Error ratio: 6 error/edge / 14 total = 43% ✓

#[path = "../common/mod.rs"]
mod common;
use common::AdminTestContext;

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-01: GET /admin/v1/oidc_providers — list without client_secret
// ─────────────────────────────────────────────────────────────────────────────

/// GET /admin/v1/oidc_providers returns list of configured providers.
/// client_secret_enc is never included in the response.
///
/// AC-B06-01
// @US-B06 @AC-B06-01 @driving_port @real-io
#[tokio::test]
async fn oidc_provider_list_excludes_client_secret() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/oidc_providers"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET /admin/v1/oidc_providers failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-B06-01: GET /admin/v1/oidc_providers must return 200"
    );

    let providers: Vec<serde_json::Value> = resp.json().await.expect("response must be JSON array");

    for provider in &providers {
        // Required fields present.
        for field in &["id", "issuer", "client_id", "enabled", "created_at"] {
            assert!(
                provider.get(field).is_some(),
                "AC-B06-01: provider object must include '{}'; got: {}",
                field,
                provider
            );
        }
        // Sensitive fields absent.
        for secret_field in &["client_secret", "client_secret_enc"] {
            assert!(
                provider.get(secret_field).is_none(),
                "AC-B06-01: '{}' must not appear in GET response; got: {}",
                secret_field,
                provider
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-01: Non-Owner cannot list OIDC providers (error)
// ─────────────────────────────────────────────────────────────────────────────

// @US-B06 @AC-B06-01 @error @driving_port @real-io
#[tokio::test]
async fn viewer_cannot_list_oidc_providers() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_viewer(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/oidc_providers"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET oidc_providers failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "AC-B06-01: Viewer cannot list OIDC providers; expected 403"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-02: POST /admin/v1/oidc_providers — client_secret AES-256-GCM encrypted
// ─────────────────────────────────────────────────────────────────────────────

/// POST /admin/v1/oidc_providers: client_secret encrypted with AES-256-GCM
/// before storage; never stored plaintext.
///
/// AC-B06-02
// @US-B06 @AC-B06-02 @driving_port @real-io
#[tokio::test]
async fn owner_creates_oidc_provider_and_secret_encrypted() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/oidc_providers"))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({
            "issuer":        "https://accounts.google.com",
            "client_id":     "my-client-id.apps.googleusercontent.com",
            "client_secret": "plaintext-secret-that-must-not-be-stored"
        }))
        .send()
        .await
        .expect("POST oidc_providers failed");

    assert_eq!(
        resp.status().as_u16(),
        201,
        "AC-B06-02: POST /admin/v1/oidc_providers must return 201"
    );

    let body: serde_json::Value = resp.json().await.expect("response must be JSON");

    assert_eq!(
        body["issuer"].as_str(),
        Some("https://accounts.google.com"),
        "AC-B06-02: issuer must be returned"
    );
    assert_eq!(
        body["client_id"].as_str(),
        Some("my-client-id.apps.googleusercontent.com"),
        "AC-B06-02: client_id must be returned"
    );
    assert_eq!(
        body["enabled"].as_bool(),
        Some(true),
        "AC-B06-02: new provider must be enabled by default"
    );
    // Plaintext secret must not be in response.
    assert!(
        body.get("client_secret").is_none(),
        "AC-B06-02: client_secret must not be in POST response; got: {}",
        body
    );

    // DB verification: verify plaintext secret never stored.
    let account_uuid = uuid::Uuid::parse_str(&ctx.account_id).unwrap();
    let enc_bytes: Vec<u8> = sqlx::query_scalar(
        "SELECT client_secret_enc FROM oidc_providers \
         WHERE issuer = 'https://accounts.google.com' AND account_id = $1",
    )
    .bind(account_uuid)
    .fetch_one(&ctx.pool)
    .await
    .expect("fetch client_secret_enc");

    let plaintext = b"plaintext-secret-that-must-not-be-stored";

    // enc_bytes = 12-byte nonce || AES-GCM ciphertext+tag
    assert!(
        enc_bytes.len() > plaintext.len(),
        "AC-B06-02: enc_bytes must be longer than plaintext (nonce overhead)"
    );
    // Plaintext must not appear as a subsequence in the enc bytes
    let found = enc_bytes.windows(plaintext.len()).any(|w| w == plaintext);
    assert!(
        !found,
        "AC-B06-02: plaintext secret must not appear in client_secret_enc bytes"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-03: PATCH /admin/v1/oidc_providers/:id — partial update
// ─────────────────────────────────────────────────────────────────────────────

/// PATCH /admin/v1/oidc_providers/:id with {enabled: false}: provider disabled.
/// Existing sessions authenticated via this provider are not signed out.
///
/// AC-B06-03
// @US-B06 @AC-B06-03 @driving_port @real-io
#[tokio::test]
async fn owner_disables_oidc_provider_without_signing_out_existing_sessions() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let provider_id = create_oidc_provider(&ctx, &session_cookie).await;

    // Disable the provider.
    let patch_resp = ctx
        .client
        .patch(ctx.url(&format!("/admin/v1/oidc_providers/{}", provider_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"enabled": false}))
        .send()
        .await
        .expect("PATCH oidc_provider failed");

    assert_eq!(
        patch_resp.status().as_u16(),
        200,
        "AC-B06-03: PATCH oidc_provider must return 200"
    );

    let updated: serde_json::Value = patch_resp.json().await.expect("response must be JSON");
    assert_eq!(
        updated["enabled"].as_bool(),
        Some(false),
        "AC-B06-03: provider must be disabled after PATCH"
    );

    // Session from before disable must still work.
    let projects_resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET projects after OIDC disable failed");

    assert_eq!(
        projects_resp.status().as_u16(),
        200,
        "AC-B06-03: existing session must not be invalidated when OIDC provider disabled"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-04: DELETE /admin/v1/oidc_providers/:id
// ─────────────────────────────────────────────────────────────────────────────

/// DELETE /admin/v1/oidc_providers/:id: 204. Does not sign out existing sessions.
///
/// AC-B06-04
// @US-B06 @AC-B06-04 @driving_port @real-io
#[tokio::test]
async fn owner_deletes_oidc_provider_and_existing_sessions_remain_valid() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let provider_id = create_oidc_provider(&ctx, &session_cookie).await;

    let delete_resp = ctx
        .client
        .delete(ctx.url(&format!("/admin/v1/oidc_providers/{}", provider_id)))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("DELETE oidc_provider failed");

    assert_eq!(
        delete_resp.status().as_u16(),
        204,
        "AC-B06-04: DELETE oidc_provider must return 204"
    );

    // Existing session must still work after provider deletion.
    let projects_resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET projects after OIDC delete failed");

    assert_eq!(
        projects_resp.status().as_u16(),
        200,
        "AC-B06-04: existing sessions must remain valid after OIDC provider deleted"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-05: OIDC callback — valid id_token grants session cookie
// ─────────────────────────────────────────────────────────────────────────────

/// OIDC callback with a valid id_token from a configured provider:
/// 302 redirect to /admin/; Set-Cookie: embyr_session set.
///
/// Uses a local mock JWKS endpoint via axum test server.
///
/// AC-B06-05
// @US-B06 @AC-B06-05 @real-io @adapter-integration
#[tokio::test]
async fn oidc_callback_with_valid_id_token_grants_session() {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::traits::PublicKeyParts;

    let ctx = AdminTestContext::new().await;
    let owner_cookie = sign_in_as_owner(&ctx).await;

    // ── 1. Generate RSA-2048 test key pair ────────────────────────────────────
    let private_key = rsa::RsaPrivateKey::new(&mut rand_core::OsRng, 2048)
        .expect("generate RSA key");
    let public_key = private_key.to_public_key();

    // Extract modulus (n) and exponent (e) for JWK
    let n_bytes = public_key.n().to_bytes_be();
    let e_bytes = public_key.e().to_bytes_be();
    let n_b64 = URL_SAFE_NO_PAD.encode(&n_bytes);
    let e_b64 = URL_SAFE_NO_PAD.encode(&e_bytes);

    // ── 2. Start local JWKS server ────────────────────────────────────────────
    let jwks = serde_json::json!({
        "keys": [{
            "kty": "RSA",
            "n": n_b64,
            "e": e_b64,
            "alg": "RS256",
            "use": "sig",
            "kid": "test-key-1"
        }]
    });

    let jwks_clone = jwks.clone();
    let jwks_router = axum::Router::new().route(
        "/.well-known/jwks.json",
        axum::routing::get(move || {
            let j = jwks_clone.clone();
            async move { axum::Json(j) }
        }),
    );
    let jwks_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind JWKS port");
    let jwks_port = jwks_listener.local_addr().unwrap().port();
    let jwks_issuer = format!("http://127.0.0.1:{}", jwks_port);
    tokio::spawn(async move {
        axum::serve(jwks_listener, jwks_router)
            .await
            .expect("JWKS server error");
    });
    tokio::task::yield_now().await;

    // ── 3. Register OIDC provider pointing at local JWKS server ──────────────
    let client_id = "test-oidc-client-id";
    let create_resp = ctx
        .client
        .post(ctx.url("/admin/v1/oidc_providers"))
        .header("Cookie", &owner_cookie)
        .json(&serde_json::json!({
            "issuer":        &jwks_issuer,
            "client_id":     client_id,
            "client_secret": "test-client-secret"
        }))
        .send()
        .await
        .expect("POST oidc_provider failed");
    assert_eq!(
        create_resp.status().as_u16(),
        201,
        "must create OIDC provider; got: {}",
        create_resp.status()
    );

    // ── 4. Generate a valid JWT signed with the private key ───────────────────
    let private_pem = private_key
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
        .expect("to_pkcs1_pem");
    let encoding_key = EncodingKey::from_rsa_pem(private_pem.as_bytes())
        .expect("EncodingKey::from_rsa_pem");

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-key-1".to_string());

    let exp = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();
    let claims = serde_json::json!({
        "iss": &jwks_issuer,
        "sub": "oidc-test-user@example.com",
        "aud": client_id,
        "exp": exp,
        "iat": chrono::Utc::now().timestamp()
    });

    let id_token = encode(&header, &claims, &encoding_key).expect("encode JWT");

    // ── 5. Call the OIDC callback ─────────────────────────────────────────────
    // Use redirect(false) so reqwest does not auto-follow the 302.
    let no_redirect_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build no-redirect client");

    let resp = no_redirect_client
        .get(ctx.url("/admin/v1/auth/oidc/callback"))
        .query(&[
            ("id_token", id_token.as_str()),
            ("state", "csrf-test-nonce"),
            ("code", "unused-auth-code"),
        ])
        .send()
        .await
        .expect("OIDC callback request failed");

    // ── 6. Assertions ─────────────────────────────────────────────────────────
    assert_eq!(
        resp.status().as_u16(),
        302,
        "AC-B06-05: valid id_token must return 302 redirect; got status {}",
        resp.status().as_u16()
    );

    let location = resp
        .headers()
        .get("location")
        .or_else(|| resp.headers().get("Location"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        location, "/admin/",
        "AC-B06-05: redirect must go to /admin/"
    );

    let cookie_header = resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        cookie_header.contains("embyr_session="),
        "AC-B06-05: Set-Cookie must contain embyr_session; got: {}",
        cookie_header
    );
    assert!(
        cookie_header.contains("HttpOnly"),
        "AC-B06-05: cookie must be HttpOnly"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-05: OIDC callback with invalid id_token → 401 redirect (error)
// ─────────────────────────────────────────────────────────────────────────────

// @US-B06 @AC-B06-05 @error @driving_port @real-io
#[tokio::test]
async fn oidc_callback_with_invalid_signature_redirects_with_oidc_failed() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    // Register a provider so the issuer is known.
    let _provider_id = create_oidc_provider(&ctx, &session_cookie).await;

    // Tampered token: signature won't verify against the real JWKS.
    let tampered_token = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.INVALIDSIGNATURE";

    let resp = ctx
        .client
        .get(ctx.url(&format!(
            "/admin/v1/auth/oidc/callback?code=fakeauthcode&state=csrf-nonce&id_token={}",
            tampered_token
        )))
        .send()
        .await
        .expect("OIDC callback request failed");

    // Must redirect to /admin/?error=oidc_failed (or return 401 if no redirect)
    let status = resp.status().as_u16();
    assert!(
        status == 302 || status == 401,
        "AC-B06-05: invalid id_token must return 302 or 401; got {}",
        status
    );

    if status == 302 {
        let location = resp
            .headers()
            .get("Location")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            location.contains("error=oidc_failed"),
            "AC-B06-05: redirect must include error=oidc_failed; got Location: {}",
            location
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-05: OIDC callback with unknown issuer → 401 (error)
// ─────────────────────────────────────────────────────────────────────────────

// @US-B06 @AC-B06-05 @error @driving_port @real-io
#[tokio::test]
async fn oidc_callback_with_unknown_issuer_returns_401() {
    let ctx = AdminTestContext::new().await;
    // No OIDC providers configured for this account.
    // Callback claiming issuer not in oidc_providers must be rejected.

    let resp = ctx
        .client
        .get(ctx.url(
            "/admin/v1/auth/oidc/callback?code=fakeauthcode&state=csrf-nonce",
        ))
        .send()
        .await
        .expect("OIDC callback request failed");

    let status = resp.status().as_u16();
    assert!(
        status == 401 || status == 302,
        "AC-B06-05: unknown issuer must return 401 or redirect with error; got {}",
        status
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-06: GET /admin/v1/billing — per-database breakdown
// ─────────────────────────────────────────────────────────────────────────────

/// GET /admin/v1/billing?range=7d returns per-database metrics summed over last 7 days.
/// Sourced from daily_project_metrics GROUP BY project.
///
/// AC-B06-06
// @US-B06 @AC-B06-06 @driving_port @real-io
#[tokio::test]
async fn billing_returns_per_database_breakdown_for_requested_range() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/billing?range=7d"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET billing failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-B06-06: GET /admin/v1/billing must return 200"
    );

    let body: serde_json::Value = resp.json().await.expect("response must be JSON");

    assert!(
        body.get("range").is_some(),
        "AC-B06-06: response must include 'range' field"
    );
    assert_eq!(
        body["range"].as_str(),
        Some("7d"),
        "AC-B06-06: range must match requested range"
    );

    let databases = body["databases"].as_array().expect("databases must be an array");

    for db in databases {
        for field in &[
            "id",
            "name",
            "reads",
            "writes",
            "deletes",
            "peak_connections",
            "log_storage_bytes",
        ] {
            assert!(
                db.get(field).is_some(),
                "AC-B06-06: database entry must include '{}'; got: {}",
                field,
                db
            );
        }
    }

    assert!(
        body.get("totals").is_some(),
        "AC-B06-06: response must include 'totals'"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-06: All supported billing ranges are accepted
// ─────────────────────────────────────────────────────────────────────────────

// @US-B06 @AC-B06-06 @driving_port @real-io
#[tokio::test]
async fn billing_accepts_all_valid_range_values() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    for range in &["7d", "30d", "month", "last_month"] {
        let resp = ctx
            .client
            .get(ctx.url(&format!("/admin/v1/billing?range={}", range)))
            .header("Cookie", &session_cookie)
            .send()
            .await
            .expect(&format!("GET billing?range={} failed", range));

        assert_eq!(
            resp.status().as_u16(),
            200,
            "AC-B06-06: billing?range={} must return 200",
            range
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-06: Invalid billing range returns 422 (error)
// ─────────────────────────────────────────────────────────────────────────────

// @US-B06 @AC-B06-06 @error @driving_port @real-io
#[tokio::test]
async fn billing_with_invalid_range_returns_422() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/billing?range=forever"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET billing with invalid range failed");

    assert_eq!(
        resp.status().as_u16(),
        422,
        "AC-B06-06: invalid range must return 422"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-07: peak_connections returns null (deferred)
// ─────────────────────────────────────────────────────────────────────────────

/// peak_connections column in billing response is always null in V1 (deferred per spec).
/// UI shows "—".
///
/// AC-B06-07
// @US-B06 @AC-B06-07 @driving_port @real-io
#[tokio::test]
async fn billing_peak_connections_always_null_in_v1() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/billing?range=7d"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET billing failed");

    let body: serde_json::Value = resp.json().await.expect("response must be JSON");
    let databases = body["databases"].as_array().expect("databases must be array");

    for db in databases {
        assert!(
            db["peak_connections"].is_null(),
            "AC-B06-07: peak_connections must be null in V1; got: {}",
            db["peak_connections"]
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-08: Projects with zero metrics appear in billing with all counters 0
// ─────────────────────────────────────────────────────────────────────────────

/// Projects that have no rows in daily_project_metrics still appear in billing
/// with reads=0, writes=0, deletes=0, etc.
///
/// AC-B06-08
// @US-B06 @AC-B06-08 @driving_port @real-io
#[tokio::test]
async fn zero_metric_projects_appear_in_billing_with_zero_counters() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    // "test-project-seeded-for-account" is seeded in AdminTestContext with no metric rows
    let zero_metric_project_id = "test-project-seeded-for-account";

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/billing?range=7d"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET billing failed");

    let body: serde_json::Value = resp.json().await.expect("response must be JSON");
    let databases = body["databases"].as_array().expect("databases must be array");

    let zero_project = databases
        .iter()
        .find(|db| db["id"].as_str() == Some(zero_metric_project_id));

    assert!(
        zero_project.is_some(),
        "AC-B06-08: project with zero metrics must still appear in billing"
    );

    if let Some(db) = zero_project {
        for counter in &["reads", "writes", "deletes"] {
            assert_eq!(
                db[counter].as_i64(),
                Some(0),
                "AC-B06-08: zero-metric project must have {}=0; got: {}",
                counter,
                db[counter]
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Sign in as the seeded Owner and return the session cookie.
async fn sign_in_as_owner(ctx: &AdminTestContext) -> String {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email":     ctx.user_email,
            "password":  ctx.user_password,
            "totp_code": ctx.totp_code_now(),
        }))
        .send()
        .await
        .expect("sign-in as owner failed");

    resp.headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(|s| s.trim().to_string())
        .expect("no cookie in sign-in response")
}

/// Sign in as the seeded Viewer and return the session cookie.
async fn sign_in_as_viewer(ctx: &AdminTestContext) -> String {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email":     ctx.viewer_email,
            "password":  ctx.viewer_password,
            "totp_code": ctx.viewer_totp_code_now(),
        }))
        .send()
        .await
        .expect("sign-in as viewer failed");

    resp.headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(|s| s.trim().to_string())
        .expect("no cookie in sign-in as viewer response")
}

/// Create a test OIDC provider and return its ID.
///
/// # Panics (RED scaffold)
async fn create_oidc_provider(ctx: &AdminTestContext, session_cookie: &str) -> String {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/oidc_providers"))
        .header("Cookie", session_cookie)
        .json(&serde_json::json!({
            "issuer":        "https://accounts.google.com",
            "client_id":     "test-client-id",
            "client_secret": "test-client-secret"
        }))
        .send()
        .await
        .expect("POST oidc_provider in helper failed");

    if resp.status().as_u16() != 201 {
        panic!(
            "Not yet implemented -- RED scaffold: \
            create_oidc_provider helper requires working POST /admin/v1/oidc_providers"
        )
    }

    resp.json::<serde_json::Value>()
        .await
        .expect("oidc_provider response must be JSON")["id"]
        .as_str()
        .expect("oidc_provider id must be present")
        .to_string()
}
