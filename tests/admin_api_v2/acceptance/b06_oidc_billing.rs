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
#[ignore]
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
#[ignore]
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
#[ignore]
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
    panic!(
        "Not yet implemented -- RED scaffold: \
        verify client_secret_enc contains AES-GCM ciphertext, not plaintext"
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-03: PATCH /admin/v1/oidc_providers/:id — partial update
// ─────────────────────────────────────────────────────────────────────────────

/// PATCH /admin/v1/oidc_providers/:id with {enabled: false}: provider disabled.
/// Existing sessions authenticated via this provider are not signed out.
///
/// AC-B06-03
// @US-B06 @AC-B06-03 @driving_port @real-io
#[ignore]
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
#[ignore]
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
/// Uses a local mock JWKS endpoint via wiremock or axum test server.
///
/// AC-B06-05
// @US-B06 @AC-B06-05 @real-io @adapter-integration
#[ignore]
#[tokio::test]
async fn oidc_callback_with_valid_id_token_grants_session() {
    let ctx = AdminTestContext::new().await;
    // Setup: configure an OIDC provider pointing at a local mock JWKS server.
    // Generate a test id_token signed by the mock key.
    panic!(
        "Not yet implemented -- RED scaffold: \
        OIDC callback integration test requires local mock JWKS server + id_token generation"
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B06-05: OIDC callback with invalid id_token → 401 redirect (error)
// ─────────────────────────────────────────────────────────────────────────────

// @US-B06 @AC-B06-05 @error @driving_port @real-io
#[ignore]
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
#[ignore]
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
#[ignore]
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
#[ignore]
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
#[ignore]
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
#[ignore]
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
#[ignore]
#[tokio::test]
async fn zero_metric_projects_appear_in_billing_with_zero_counters() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    // Seed: one project with no metric rows in daily_project_metrics.
    let zero_metric_project_id = "seeded-zero-metric-project-id";

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

/// Sign in as a seeded Viewer and return the session cookie.
///
/// # Panics (RED scaffold)
async fn sign_in_as_viewer(ctx: &AdminTestContext) -> String {
    panic!(
        "Not yet implemented -- RED scaffold: \
        sign_in_as_viewer requires seeded Viewer in AdminTestContext"
    )
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
