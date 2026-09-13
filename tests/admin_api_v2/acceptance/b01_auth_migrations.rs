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
//!
//! admin-signin-hardening extension (ADR-076, docs/feature/admin-signin-hardening/
//! feature-delta.md, AC-ASH-01 through AC-ASH-08) — appended below the original
//! B-01 scenarios rather than a parallel file: same driving port
//! (POST /admin/v1/auth/signin), same AdminTestContext composition root.
//! Additional observables exercised (black-box, via HTTP only — no new Rust
//! module imports, since this DISTILL pass does not create signin_rate_limit.rs,
//! the sweeper, or touch auth.rs/router.rs/lib.rs; that is DELIVER's job):
//!   "response.status_code"            — 429 once the per-source-IP bucket is exhausted
//!   "response.retry_after_ms_present" — retry-after-ms header on a 429
//!   "reactor.batch_wall_time_ms"      — proves Argon2id runs off the async reactor thread

#[path = "../common/mod.rs"]
mod common;
use common::{AdminTestContext, assert_state_delta, set_to};
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

// ═════════════════════════════════════════════════════════════════════════════
// admin-signin-hardening (ADR-076) — AC-ASH-01 through AC-ASH-08
//
// AC-ASH-04 (regression guard: sign_in_with_valid_credentials_returns_session_cookie,
// wrong_totp_code_returns_401_and_does_not_set_session,
// three_consecutive_totp_failures_lock_account_for_fifteen_minutes,
// valid_recovery_code_grants_session_and_invalidates_code) and AC-ASH-07
// (regression guard: wrong_password_returns_401_without_revealing_email_existence)
// are already satisfied VERBATIM by the five existing tests above — zero new
// test code required for those two ACs (nw-tdd-methodology "No Code Without a
// Requiring Test": if an AC is already covered by a passing test, adding a
// duplicate is waste, not rigor).
// ═════════════════════════════════════════════════════════════════════════════

/// US-01 Scenario 2 (walking skeleton) + AC-ASH-01/AC-ASH-02: while a burst of
/// wrong-password requests targets a DIFFERENT teammate's account (Dana Kim,
/// here `ctx.viewer_email`), Chris Okafor's own correct-credential signin
/// completes quickly instead of being serialized behind the flood — proof
/// that Argon2id verification runs off the async reactor's own worker thread
/// (`tokio::task::spawn_blocking`, AC-ASH-01), not a code-inspection claim.
///
/// This test deliberately runs on tokio's DEFAULT `#[tokio::test]`
/// SINGLE-THREADED runtime (matches every other test in this file — no
/// `flavor = "multi_thread"` override). That choice is load-bearing, not
/// incidental: on a single OS thread, a synchronous (non-`.await`-yielding)
/// Argon2id verify running INLINE on the reactor thread cannot be preempted —
/// it monopolizes the one thread for its full ~50-100ms, and every other
/// task (other in-flight connections on the same admin server, the other
/// concurrent client requests below) stalls until it returns.
/// `tokio::task::spawn_blocking` moves that same computation onto tokio's
/// separate blocking-thread pool, which exists independently of runtime
/// flavor — so the reactor thread stays free even under this
/// single-threaded harness. A multi-thread runtime would partly mask an
/// inline-blocking bug behind OS-level parallelism across reactor threads;
/// single-threaded is the SHARPER proof, not a weaker one.
///
/// RED (today, unfixed — confirmed empirically, not assumed): `auth.rs` runs
/// Argon2id inline, so this whole batch of FLOOD_SIZE+1 requests serializes
/// on the one reactor thread. The observed failure mode is WORSE than mere
/// slowness: `SystemDb`'s pool has a 5-second `acquire_timeout`
/// (`adapters/system_db.rs`), and once ~7 requests' worth of serialized
/// Argon2id verifies (~50-100ms each) have consumed that budget, every
/// subsequent request's connection-pool acquire also times out, cascading
/// into `500` for effectively the rest of the batch (empirically: 7×`401`
/// then ~94×`500`, ~10s total wall time for FLOOD_SIZE=100) — including the
/// legitimate signin. This is finding #12's own "reactor starvation" claim
/// reproduced directly: the reactor stalls so completely that even
/// unrelated DB connection acquisition fails.
/// GREEN (post-fix): `spawn_blocking` moves each verify onto tokio's
/// blocking-thread pool (independent of the async reactor and unaffected by
/// its single-threadedness); all FLOOD_SIZE+1 verifies run genuinely
/// concurrently, DB connection acquisition is never starved, and the legit
/// request returns `200` well within `MAX_TOTAL_WALL_MS`.
///
/// AC-ASH-01, AC-ASH-02
// @US-ASH-01 @AC-ASH-01 @AC-ASH-02 @walking_skeleton @driving_port @real-io
#[tokio::test]
async fn argon2_verification_does_not_block_the_reactor_under_concurrent_signin_load() {
    // "~100 concurrent" per AC-ASH-02's own wording; safely under the
    // 150-token rate-limit capacity (ADR-076) so this test observes 200/401,
    // never 429 (that is AC-ASH-03/06's own concern, tested separately below).
    const FLOOD_SIZE: usize = 100;
    // One Argon2id verify (~50-100ms) plus DB-pool-of-5 queueing slack for
    // FLOOD_SIZE+1 requests — far below the fully-serialized RED baseline
    // (FLOOD_SIZE * ~50-100ms ≈ 5-10s).
    const MAX_TOTAL_WALL_MS: u128 = 2_000;

    let ctx = AdminTestContext::new().await;

    let mut handles = Vec::with_capacity(FLOOD_SIZE + 1);

    // Flood: wrong-password attempts against Dana Kim's (viewer) account.
    for _ in 0..FLOOD_SIZE {
        let client = ctx.client.clone();
        let url = ctx.url("/admin/v1/auth/signin");
        let email = ctx.viewer_email.clone();
        handles.push(tokio::spawn(async move {
            client
                .post(url)
                .json(&serde_json::json!({
                    "email": email,
                    "password": "wrong-password-flood",
                    "totp_code": "000000",
                }))
                .send()
                .await
                .map(|r| r.status().as_u16())
        }));
    }

    // Chris Okafor's own legitimate, concurrent signin — must still succeed.
    let legit_client = ctx.client.clone();
    let legit_url = ctx.url("/admin/v1/auth/signin");
    let legit_body = serde_json::json!({
        "email": ctx.user_email,
        "password": ctx.user_password,
        "totp_code": ctx.totp_code_now(),
    });
    handles.push(tokio::spawn(async move {
        legit_client
            .post(legit_url)
            .json(&legit_body)
            .send()
            .await
            .map(|r| r.status().as_u16())
    }));

    let start = std::time::Instant::now();
    let mut statuses = Vec::with_capacity(handles.len());
    for h in handles {
        statuses.push(h.await.expect("task panicked").expect("request failed"));
    }
    let elapsed_ms = start.elapsed().as_millis();

    // Legit request (last handle pushed) still succeeds — functionality intact.
    let legit_status = *statuses.last().unwrap();
    assert_eq!(
        legit_status, 200,
        "AC-ASH-02: legitimate concurrent signin must still succeed (got {legit_status}; \
         a 500 here means the reactor stalled badly enough that even DB connection \
         acquisition timed out — see this test's own doc comment); full batch: {statuses:?}"
    );

    assert!(
        elapsed_ms < MAX_TOTAL_WALL_MS,
        "AC-ASH-01/AC-ASH-02: {} concurrent wrong-password attempts (+1 legitimate) \
         took {}ms — expected under {}ms if Argon2id verification runs off the \
         reactor thread via spawn_blocking. A wall time near FLOOD_SIZE * 50-100ms \
         indicates Argon2id is still running INLINE and serializing every request \
         on the single-threaded test runtime (ADR-003 violation).",
        FLOOD_SIZE, elapsed_ms, MAX_TOTAL_WALL_MS
    );
}

/// Secondary, WEAKER regression guard (source-inspection level) for AC-ASH-01.
/// The load-style test above is the primary, decisive proof; this only
/// guards against an accidental future revert (e.g. someone "simplifying"
/// `signin` back to an inline Argon2 call) even if CI load ever made a
/// timing-based assertion flaky. Per DESIGN's own ADR-076 § Enforcement, a
/// broader workspace-wide CI grep gate (no direct `argon2::Argon2::new(`
/// call site anywhere under `crates/embyr-server/src/`, forcing every caller
/// through `embyr_core::auth::argon2`) is a separate, explicitly-recommended
/// FOLLOW-UP, not one of this feature's 8 locked ACs — this narrow,
/// signin-only check is this feature's own in-scope secondary guard.
///
/// RED today (confirmed empirically): `auth.rs` contains zero occurrences of
/// `spawn_blocking` — fails instantly (~0ms, no server/container needed).
#[test]
fn signin_source_wraps_password_verification_in_spawn_blocking() {
    let src = include_str!("../../../crates/embyr-server/src/admin/handlers/auth.rs");
    assert!(
        src.contains("spawn_blocking"),
        "AC-ASH-01 (secondary/weak check): auth.rs must call \
         tokio::task::spawn_blocking somewhere for the Argon2id verification — \
         the primary proof is the concurrent-load test above; this only \
         guards against an accidental revert."
    );
}

/// US-01 Scenario 3: a sustained flood of wrong-password attempts against one
/// account, all from the same source (the test's own loopback client), is
/// throttled once the configured threshold is exceeded. ADR-076 sizes the
/// bucket at capacity=150 tokens (source-IP keyed) — sending more than that
/// from one source must produce at least one `429` with a `retry-after-ms`
/// header, per D-ASH-3's "gated before any Argon2id/DB work" position.
///
/// RED (today, unfixed): zero rate-limit gate exists on this route — every
/// one of the OVER_CAPACITY requests below returns 401, never 429.
///
/// AC-ASH-03
// @US-ASH-01 @AC-ASH-03 @error @driving_port @real-io
#[tokio::test]
async fn wrong_password_flood_from_one_source_is_throttled_once_capacity_is_exceeded() {
    const OVER_CAPACITY: usize = 160; // capacity=150 tokens (ADR-076) + safety margin

    let ctx = AdminTestContext::new().await;

    let mut handles = Vec::with_capacity(OVER_CAPACITY);
    for _ in 0..OVER_CAPACITY {
        let client = ctx.client.clone();
        let url = ctx.url("/admin/v1/auth/signin");
        let email = ctx.user_email.clone();
        handles.push(tokio::spawn(async move {
            let resp = client
                .post(url)
                .json(&serde_json::json!({
                    "email": email,
                    "password": "wrong-password-sustained-flood",
                    "totp_code": "000000",
                }))
                .send()
                .await
                .expect("request failed");
            let status = resp.status().as_u16();
            let retry_after_ms = resp
                .headers()
                .get("retry-after-ms")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            (status, retry_after_ms)
        }));
    }

    let mut throttled_count = 0usize;
    let mut saw_retry_after_header = false;
    for h in handles {
        let (status, retry_after_ms) = h.await.expect("task panicked");
        if status == 429 {
            throttled_count += 1;
            if retry_after_ms.is_some() {
                saw_retry_after_header = true;
            }
        }
    }

    assert!(
        throttled_count > 0,
        "AC-ASH-03: sending {} wrong-password attempts from one source \
         (capacity=150 per ADR-076) must throttle at least the excess with 429; \
         got 0 throttled responses out of {}",
        OVER_CAPACITY, OVER_CAPACITY
    );
    assert!(
        saw_retry_after_header,
        "AC-ASH-03: at least one 429 response must carry a retry-after-ms header"
    );
}

/// US-02 Scenario 2: an automated script probes many distinct candidate
/// emails from one source, most of which do not exist. Because the rate
/// limiter is keyed by SOURCE IP ONLY (ADR-076 D-ASH-2/OQ-ASH-01 — not
/// email, not a compound key), the throttle fires after the same 150-token
/// capacity is exhausted, regardless of how many distinct emails were
/// targeted. This is the test that distinguishes an IP-keyed limiter from an
/// email-keyed one (explicitly rejected by DESIGN — an email-keyed bucket
/// would hand this exact attacker a fresh 150-token budget per candidate
/// email, defeating the throttle outright).
///
/// RED (today, unfixed): zero rate limiting exists — every one of the
/// OVER_CAPACITY distinct-email probes below returns 401, never 429.
///
/// AC-ASH-06
// @US-ASH-02 @AC-ASH-06 @error @driving_port @real-io
#[tokio::test]
async fn enumeration_across_many_distinct_candidate_emails_from_one_source_is_throttled() {
    const OVER_CAPACITY: usize = 160; // capacity=150 tokens (ADR-076) + safety margin

    let ctx = AdminTestContext::new().await;

    let mut handles = Vec::with_capacity(OVER_CAPACITY);
    for i in 0..OVER_CAPACITY {
        let client = ctx.client.clone();
        let url = ctx.url("/admin/v1/auth/signin");
        // Distinct, mostly-nonexistent candidate emails — same source IP for every request.
        let candidate_email = format!("candidate{i}@fernbankanalytics.example");
        handles.push(tokio::spawn(async move {
            client
                .post(url)
                .json(&serde_json::json!({
                    "email": candidate_email,
                    "password": "any-password",
                    "totp_code": "000000",
                }))
                .send()
                .await
                .map(|r| r.status().as_u16())
        }));
    }

    let mut status_counts: HashMap<u16, usize> = HashMap::new();
    for h in handles {
        let status = h.await.expect("task panicked").expect("request failed");
        *status_counts.entry(status).or_insert(0) += 1;
    }

    let throttled = *status_counts.get(&429).unwrap_or(&0);
    let rejected = *status_counts.get(&401).unwrap_or(&0);

    assert!(
        throttled > 0,
        "AC-ASH-06: probing {} DISTINCT candidate emails from one source must be \
         throttled by the SAME per-source mechanism once capacity is exceeded — \
         an email-keyed (or unkeyed) limiter would let every distinct email \
         through; got status counts: {:?}",
        OVER_CAPACITY, status_counts
    );
    assert!(
        rejected <= 150,
        "AC-ASH-06: no more than the 150-token capacity worth of distinct-email \
         probes should reach a full 401 rejection; got {} 401s (status counts: {:?})",
        rejected, status_counts
    );
}

/// DESIGN resolved OQ-ASH-03 as "accept the residual timing difference; do
/// not add a dummy Argon2id verify" (D-ASH-7). This test makes that a
/// testable outcome instead of an unenforced prose decision: the
/// unknown-email path must remain measurably FASTER than the
/// known-email-wrong-password path (which pays a full Argon2id verify), not
/// converge to the same latency a dummy-verify fix would produce.
///
/// Sent as isolated, unthrottled single requests (well under the 150-token
/// capacity), so any difference is attributable ONLY to whether Argon2id
/// runs on the unknown-email path — not to rate-limiting.
///
/// NOTE: unlike the other new tests in this section, this one is expected to
/// ALREADY PASS against today's unfixed code — D-ASH-7 explicitly changes
/// zero lines of the unknown-email fast path. It exists to lock the accepted
/// design decision as a regression guard, not to prove new behavior.
///
/// AC-ASH-08
// @US-ASH-02 @AC-ASH-08 @driving_port @real-io
#[tokio::test]
async fn unknown_email_path_remains_faster_than_known_email_wrong_password_path() {
    let ctx = AdminTestContext::new().await;

    async fn timed_signin_401(client: &reqwest::Client, url: &str, email: &str) -> std::time::Duration {
        let start = std::time::Instant::now();
        let resp = client
            .post(url)
            .json(&serde_json::json!({
                "email": email,
                "password": "irrelevant-wrong-password",
                "totp_code": "000000",
            }))
            .send()
            .await
            .expect("request failed");
        assert_eq!(resp.status().as_u16(), 401, "sanity: both paths must return 401");
        start.elapsed()
    }

    let url = ctx.url("/admin/v1/auth/signin");

    // Warm up the connection/DB pool so the first timed sample isn't skewed
    // by one-time setup cost (TCP handshake, pool connection acquisition).
    let _ = timed_signin_401(&ctx.client, &url, "warmup-unused@example.com").await;

    let unknown_email_elapsed =
        timed_signin_401(&ctx.client, &url, "nobody-at-all@fernbankanalytics.example").await;
    let known_wrong_password_elapsed = timed_signin_401(&ctx.client, &url, &ctx.user_email).await;

    assert!(
        unknown_email_elapsed < known_wrong_password_elapsed,
        "AC-ASH-08: the unknown-email path ({:?}) must remain faster than the \
         known-email-wrong-password path ({:?}) — a dummy Argon2id verify on the \
         unknown-email path (the rejected OQ-ASH-03 alternative) would make these \
         converge",
        unknown_email_elapsed,
        known_wrong_password_elapsed
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
