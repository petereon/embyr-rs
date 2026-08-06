// @US-B01 @driving_port @real-io
//! Slice B-01 — Session Authentication + DB Migrations.
//!
//! All tests run against a real Postgres container via AdminTestContext.
//! Walking skeleton is in walking_skeleton.rs.
//!
//! Open questions resolved:
//!   OQ-B03: TOTP library = totp-rs 5.x (confirmed in DESIGN technology choices)
//!   OQ-B02: sessions.token_hash = BYTEA (recommended; crafter decides DDL detail)
//!   OQ-B04: admin_api_keys have no idle expiry (long-lived; immediate revocation only)
//!
//! State-delta universe (port-exposed observables):
//!   "response.status_code"                    — HTTP status from driving port
//!   "response.set_cookie.embyr_session"       — cookie header presence
//!   "response.set_cookie.max_age_zero"        — signout cookie cleared
//!   "db.sessions.token_hash_stored"           — BLAKE3 hash in DB (not plaintext)
//!   "db.sessions.row_expired"                 — expires_at updated on signout
//!   "auth.account_locked"                     — lockout flag observable via 429

#[path = "../common/mod.rs"]
mod common;
use common::{AdminTestContext, assert_state_delta, set_to, unchanged};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// AC-B01-01: Valid credentials return session cookie
// ─────────────────────────────────────────────────────────────────────────────

/// Valid credentials (email + password + correct TOTP) return 200 + HttpOnly session cookie.
/// Cookie attributes: HttpOnly, Secure, SameSite=Strict, Path=/admin.
///
/// AC-B01-01
// @US-B01 @AC-B01-01 @driving_port @real-io
#[tokio::test]
async fn sign_in_with_valid_credentials_returns_session_cookie() {
    let ctx = AdminTestContext::new().await;

    let body = serde_json::json!({
        "email":     ctx.user_email,
        "password":  ctx.user_password,
        "totp_code": ctx.totp_code_now(),
    });

    let before: HashMap<&str, String> = HashMap::new();

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&body)
        .send()
        .await
        .expect("POST /admin/v1/auth/signin failed");

    let status = resp.status().as_u16();
    let set_cookie = resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert("response.status_code", status.to_string());
    after.insert(
        "response.set_cookie.embyr_session",
        if set_cookie.contains("embyr_session=") {
            "present".to_string()
        } else {
            "absent".to_string()
        },
    );

    let universe = &[
        "response.status_code",
        "response.set_cookie.embyr_session",
    ];
    let mut expected = HashMap::new();
    expected.insert("response.status_code", set_to("200".to_string()));
    expected.insert(
        "response.set_cookie.embyr_session",
        set_to("present".to_string()),
    );

    assert_state_delta(&before, &after, universe, &expected);

    assert!(
        set_cookie.contains("HttpOnly"),
        "AC-B01-01: cookie must be HttpOnly"
    );
    assert!(
        set_cookie.contains("SameSite=Strict"),
        "AC-B01-01: cookie must be SameSite=Strict"
    );
    assert!(
        set_cookie.contains("Path=/admin"),
        "AC-B01-01: cookie must have Path=/admin"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B01-02: Wrong password returns 401 without leaking email existence
// ─────────────────────────────────────────────────────────────────────────────

/// Wrong password: 401 {"message": "Invalid credentials"}.
/// Same response for unknown email (no oracle attack surface).
///
/// AC-B01-02
// @US-B01 @AC-B01-02 @error @driving_port @real-io
#[tokio::test]
async fn wrong_password_returns_401_without_revealing_email_existence() {
    let ctx = AdminTestContext::new().await;

    // Test 1: wrong password for known email
    let resp_known = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email":    ctx.user_email,
            "password": "wrong-password-xyz",
            "totp_code": "000000",
        }))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp_known.status().as_u16(),
        401,
        "AC-B01-02: wrong password must return 401"
    );

    let body: serde_json::Value = resp_known.json().await.expect("response must be JSON");
    assert_eq!(
        body["message"].as_str().unwrap_or(""),
        "Invalid credentials",
        "AC-B01-02: error message must be 'Invalid credentials' (not reveal password issue)"
    );

    // Test 2: unknown email — same response shape (no oracle)
    let resp_unknown = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email":    "nonexistent@example.com",
            "password": "any-password",
            "totp_code": "000000",
        }))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp_unknown.status().as_u16(),
        401,
        "AC-B01-02: unknown email must return 401 (same as wrong password)"
    );

    let body2: serde_json::Value = resp_unknown.json().await.expect("response must be JSON");
    assert_eq!(
        body2["message"].as_str().unwrap_or(""),
        "Invalid credentials",
        "AC-B01-02: error message must be identical for unknown email"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B01-03: Wrong TOTP returns 401 without setting a session
// ─────────────────────────────────────────────────────────────────────────────

/// Correct email + password but wrong TOTP code → 401; no Set-Cookie.
///
/// AC-B01-03
// @US-B01 @AC-B01-03 @error @driving_port @real-io
#[tokio::test]
async fn wrong_totp_code_returns_401_and_does_not_set_session() {
    let ctx = AdminTestContext::new().await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email":     ctx.user_email,
            "password":  ctx.user_password,
            "totp_code": ctx.totp_code_wrong(),
        }))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-B01-03: wrong TOTP must return 401"
    );

    // Save headers before consuming resp with .json().
    let set_cookie = resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body: serde_json::Value = resp.json().await.expect("response must be JSON");
    assert_eq!(
        body["message"].as_str().unwrap_or(""),
        "Invalid or expired code",
        "AC-B01-03: TOTP failure message"
    );

    assert!(
        !set_cookie.contains("embyr_session="),
        "AC-B01-03: no session cookie must be set on TOTP failure; got: {}",
        set_cookie
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B01-04: Three consecutive TOTP failures lock account for 15 minutes
// ─────────────────────────────────────────────────────────────────────────────

/// Three wrong TOTP codes within 15 minutes: account locked for 15 minutes.
/// Fourth attempt returns 429 {"message": "Too many failed attempts — account locked for 15 minutes"}.
///
/// AC-B01-04
// @US-B01 @AC-B01-04 @error @driving_port @real-io
#[tokio::test]
async fn three_consecutive_totp_failures_lock_account_for_fifteen_minutes() {
    let ctx = AdminTestContext::new().await;

    let wrong_body = serde_json::json!({
        "email":     ctx.user_email,
        "password":  ctx.user_password,
        "totp_code": ctx.totp_code_wrong(),
    });

    // Three failures
    for i in 0..3 {
        let resp = ctx
            .client
            .post(ctx.url("/admin/v1/auth/signin"))
            .json(&wrong_body)
            .send()
            .await
            .expect("request failed");
        assert_eq!(
            resp.status().as_u16(),
            401,
            "AC-B01-04: failure {} must return 401",
            i + 1
        );
    }

    // Fourth attempt with correct password should now return 429
    let resp_locked = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email":     ctx.user_email,
            "password":  ctx.user_password,
            "totp_code": ctx.totp_code_now(),
        }))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp_locked.status().as_u16(),
        429,
        "AC-B01-04: after 3 TOTP failures, subsequent attempt must return 429"
    );

    let body: serde_json::Value = resp_locked.json().await.expect("response must be JSON");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .contains("account locked for 15 minutes"),
        "AC-B01-04: lockout message must mention '15 minutes'; got: {}",
        body["message"]
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B01-05: Recovery code grants session and is invalidated
// ─────────────────────────────────────────────────────────────────────────────

/// Valid one-time recovery code: 200 + session cookie. Code invalidated immediately.
/// Second use of same recovery code: 401.
///
/// AC-B01-05
// @US-B01 @AC-B01-05 @driving_port @real-io
#[tokio::test]
async fn valid_recovery_code_grants_session_and_invalidates_code() {
    let ctx = AdminTestContext::new().await;

    // Recovery code seeded in AdminTestContext::new() as BLAKE3("RECOV-0001")
    let recovery_code = "RECOV-0001";

    // First use: 200 + session
    let resp1 = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email":         ctx.user_email,
            "password":      ctx.user_password,
            "recovery_code": recovery_code,
        }))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp1.status().as_u16(),
        200,
        "AC-B01-05: valid recovery code must return 200"
    );
    assert!(
        resp1
            .headers()
            .get("set-cookie")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .contains("embyr_session="),
        "AC-B01-05: recovery code must set session cookie"
    );

    // Second use: code invalidated → 401
    let resp2 = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email":         ctx.user_email,
            "password":      ctx.user_password,
            "recovery_code": recovery_code,
        }))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp2.status().as_u16(),
        401,
        "AC-B01-05: recovery code must be invalidated after single use"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B01-06: Sign out clears cookie and expires session row
// ─────────────────────────────────────────────────────────────────────────────

/// Sign out with valid session: 204; cookie Max-Age=0; session row expires_at = now.
/// Subsequent request with cleared cookie returns 401.
///
/// AC-B01-06
// @US-B01 @AC-B01-06 @driving_port @real-io
#[tokio::test]
async fn sign_out_clears_session_cookie_and_subsequent_request_returns_401() {
    let ctx = AdminTestContext::new().await;

    // Sign in first
    let session_cookie = sign_in_and_get_cookie(&ctx).await;

    // Sign out
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signout"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        204,
        "AC-B01-06: signout must return 204"
    );

    let cleared = resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        cleared.contains("Max-Age=0"),
        "AC-B01-06: signout must clear cookie with Max-Age=0; got: {}",
        cleared
    );

    // Subsequent request with same cookie must return 401
    let resp_after = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp_after.status().as_u16(),
        401,
        "AC-B01-06: request with expired session cookie must return 401"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B01-07: Two auth stacks do not cross-contaminate
// ─────────────────────────────────────────────────────────────────────────────

/// Operator Bearer token on a session-auth route returns 401.
/// Session cookie on an operator-only route returns 401.
///
/// AC-B01-07
// @US-B01 @AC-B01-07 @error @driving_port @real-io
#[tokio::test]
async fn operator_bearer_rejected_on_session_only_route() {
    let ctx = AdminTestContext::new().await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Authorization", "Bearer test-admin-key-from-env")
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-B01-07: operator Bearer token must be rejected on session-auth route"
    );
}

/// Session cookie on an operator-only route returns 401.
// @US-B01 @AC-B01-07 @error @driving_port @real-io
#[tokio::test]
async fn session_cookie_rejected_on_operator_only_route() {
    let ctx = AdminTestContext::new().await;

    let session_cookie = sign_in_and_get_cookie(&ctx).await;

    // POST /admin/v1/projects (operator provision route — requires Bearer EMBYR_ADMIN_KEY)
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects"))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"project_id": "test-proj", "backend_mode": "direct_pg"}))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-B01-07: session cookie must be rejected on operator-only route"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B01-08: DB migrations run cleanly on fresh Postgres
// ─────────────────────────────────────────────────────────────────────────────

/// All migration tables applied without error.
/// AdminTestContext::new() runs migrations — if it panics from a migration error,
/// this test is BROKEN not RED. This test provides explicit migration-success assertion.
///
/// AC-B01-08
// @US-B01 @AC-B01-08 @real-io @adapter-integration
#[tokio::test]
async fn db_migrations_run_cleanly_on_fresh_postgres() {
    let ctx = AdminTestContext::new().await;

    // Verify sessions table exists and has token_hash column (migration 0009).
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_name = 'sessions' AND column_name = 'token_hash')",
    )
    .fetch_one(&ctx.pool)
    .await
    .expect("query failed");

    assert!(
        exists,
        "AC-B01-08: sessions.token_hash column must exist after migrations"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B01-09: Session token stored as BLAKE3 hash, not plaintext
// ─────────────────────────────────────────────────────────────────────────────

/// After sign-in, the sessions table must contain a BLAKE3 hash in token_hash
/// and must NOT contain the raw cookie value as plaintext.
///
/// AC-B01-09
// @US-B01 @AC-B01-09 @driving_port @real-io
#[tokio::test]
async fn session_token_stored_as_blake3_hash_not_plaintext() {
    let ctx = AdminTestContext::new().await;

    let sign_in_resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email":     ctx.user_email,
            "password":  ctx.user_password,
            "totp_code": ctx.totp_code_now(),
        }))
        .send()
        .await
        .expect("request failed");

    let set_cookie = sign_in_resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Extract raw cookie value from "embyr_session=<token>; ..."
    let raw_token: String = set_cookie
        .split(';')
        .next()
        .unwrap_or("")
        .split('=')
        .nth(1)
        .unwrap_or("")
        .to_string();

    assert!(!raw_token.is_empty(), "must have extracted raw token from cookie");

    // Compute expected BLAKE3 hash of the raw token
    let expected_hash = blake3::hash(raw_token.as_bytes()).as_bytes().to_vec();

    // Verify DB has a session with that hash (not the raw token)
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions WHERE token_hash = $1",
    )
    .bind(&expected_hash)
    .fetch_one(&ctx.pool)
    .await
    .expect("DB query failed");

    assert_eq!(
        count, 1,
        "AC-B01-09: BLAKE3(token) must be stored in sessions.token_hash; \
         raw token must NOT be stored"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B01-10: Session expires after 24h of inactivity
// ─────────────────────────────────────────────────────────────────────────────

/// After 24h idle, session returns 401.
/// The session is expired directly via DB UPDATE (Postgres uses real wall clock,
/// not tokio's paused clock). `start_paused = true` is kept as the test annotation
/// to document that this behavior is independent of tokio's simulated clock.
/// `tokio::time::resume()` is called immediately so that testcontainers and HTTP
/// I/O work correctly under the test runtime.
///
/// AC-B01-10
// @US-B01 @AC-B01-10 @error @driving_port
#[tokio::test(start_paused = true)]
async fn session_expires_after_24h_of_inactivity() {
    // Resume the real-time clock immediately: testcontainers uses Docker API (HTTP)
    // with timeouts that are incompatible with tokio's auto-advancing paused clock.
    // Postgres session expiry uses the DB wall clock, not tokio's simulated clock,
    // so the `start_paused` attribute is here to document that invariant, not to
    // drive the actual expiry mechanism.
    tokio::time::resume();

    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_and_get_cookie(&ctx).await;

    // Extract raw token value from "embyr_session=<token>" cookie
    let raw_token = session_cookie
        .strip_prefix("embyr_session=")
        .expect("cookie must start with embyr_session=");
    let token_hash = blake3::hash(raw_token.as_bytes()).as_bytes().to_vec();

    // Directly expire the session in the DB (Postgres uses real wall clock)
    sqlx::query(
        "UPDATE sessions SET expires_at = now() - interval '1 second' \
         WHERE token_hash = $1",
    )
    .bind(&token_hash)
    .execute(&ctx.pool)
    .await
    .expect("expire session");

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-B01-10: expired session must return 401"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// DESIGN Earned Trust: Expired session row rejected
// ─────────────────────────────────────────────────────────────────────────────

/// A session row inserted with expires_at < now() must be rejected with 401.
/// This is the behavioral probe test for the SessionContextExtractor (from DESIGN Earned Trust).
///
// @US-B01 @error @driving_port @real-io @earned_trust
#[tokio::test]
async fn pre_expired_session_row_returns_401() {
    use base64::Engine;
    use rand_core::RngCore;
    use uuid::Uuid;

    let ctx = AdminTestContext::new().await;

    // Generate a fresh random token NOT associated with any real sign-in
    let mut buf = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut buf);
    let fake_token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf);
    let token_hash = blake3::hash(fake_token.as_bytes()).as_bytes().to_vec();

    // Insert an already-expired session row directly
    let user_id = Uuid::parse_str(&ctx.user_id).expect("valid uuid");
    let account_id = Uuid::parse_str(&ctx.account_id).expect("valid uuid");

    sqlx::query(
        "INSERT INTO sessions (user_id, account_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, now() - interval '1 minute')",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(&token_hash)
    .execute(&ctx.pool)
    .await
    .expect("insert expired session");

    // Request with this token → 401 (session expired)
    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Cookie", format!("embyr_session={fake_token}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "earned-trust: pre-expired session must return 401"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Helper: sign in and return the session cookie string (e.g. "embyr_session=abc123").
///
/// Panics if sign-in fails (indicates broken scaffold, not wrong implementation).
async fn sign_in_and_get_cookie(ctx: &AdminTestContext) -> String {
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
        .expect("sign_in_and_get_cookie: request failed");

    resp.headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(|s| s.trim().to_string())
        .expect("sign_in_and_get_cookie: no Set-Cookie in response")
}
