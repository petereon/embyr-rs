// @driving_port @real-io
//! admin-password-reset (go-live gap S7): email-link reset gated by a second factor.
//! POST /admin/v1/auth/password_reset/{request,confirm}. CI only (testcontainers Postgres).
//!
//! Rate-limit scenarios rely on the limiters' slow refill (+1 per 60 s / 20 min):
//! within a test's lifetime no token comes back, so the burst boundary is exact.

#[path = "../common/mod.rs"]
mod common;
use common::AdminTestContext;
use serde_json::{json, Value};
use uuid::Uuid;

const RECOVERY_CODE: &str = "RECOV-0001"; // seeded by the harness for the Owner
const NEW_PASSWORD: &str = "a-brand-new-password";

// ── helpers ──────────────────────────────────────────────────────────────────

async fn post(ctx: &AdminTestContext, path: &str, body: Value) -> (u16, Value, reqwest::header::HeaderMap) {
    let resp = ctx.client.post(ctx.url(path)).json(&body).send().await.expect("request failed");
    let (status, headers) = (resp.status().as_u16(), resp.headers().clone());
    (status, resp.json().await.unwrap_or(Value::Null), headers)
}

async fn request_reset(ctx: &AdminTestContext, email: &str) -> (u16, Value) {
    let (s, b, _) = post(ctx, "/admin/v1/auth/password_reset/request", json!({ "email": email })).await;
    (s, b)
}

async fn confirm_reset(ctx: &AdminTestContext, token: &str, extra: Value) -> (u16, Value) {
    let mut body = json!({ "token": token, "new_password": NEW_PASSWORD });
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    let (s, b, _) = post(ctx, "/admin/v1/auth/password_reset/confirm", body).await;
    (s, b)
}

async fn sign_in(ctx: &AdminTestContext, email: &str, password: &str, totp: String) -> (u16, Option<String>) {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&json!({"email": email, "password": password, "totp_code": totp}))
        .send()
        .await
        .expect("signin failed");
    let cookie = resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(|s| s.trim().to_string());
    (resp.status().as_u16(), cookie)
}

async fn session_status(ctx: &AdminTestContext, cookie: &str) -> u16 {
    ctx.client
        .get(ctx.url("/admin/v1/auth/session"))
        .header("Cookie", cookie)
        .send()
        .await
        .expect("session probe failed")
        .status()
        .as_u16()
}

/// The reset email is sent from a spawned task: wait (bounded) for it and
/// return the token from its link.
async fn emailed_token(ctx: &AdminTestContext, to: &str) -> String {
    for _ in 0..100 {
        let found = ctx.emails.sent.lock().unwrap().iter().rev().find(|m| m.to == to).cloned();
        if let Some(m) = found {
            let at = m.body_text.find("?reset=").expect("reset link in email") + "?reset=".len();
            return m.body_text[at..at + 43].to_string();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("no reset email to {to}");
}

fn hash(token: &str) -> Vec<u8> {
    blake3::hash(token.as_bytes()).as_bytes().to_vec()
}

fn owner_id(ctx: &AdminTestContext) -> Uuid {
    Uuid::parse_str(&ctx.user_id).unwrap()
}

async fn token_rows(ctx: &AdminTestContext, user: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM password_reset_tokens WHERE user_id = $1")
        .bind(user)
        .fetch_one(&ctx.pool)
        .await
        .unwrap()
}

async fn password_hash(ctx: &AdminTestContext, user: Uuid) -> String {
    sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(user)
        .fetch_one(&ctx.pool)
        .await
        .unwrap()
}

async fn seed_token(ctx: &AdminTestContext, user: Uuid, token: &str, expires_sql: &str) {
    sqlx::query(&format!(
        "INSERT INTO password_reset_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, {expires_sql}) \
         ON CONFLICT (user_id) DO UPDATE SET token_hash = EXCLUDED.token_hash, expires_at = EXCLUDED.expires_at"
    ))
    .bind(user)
    .bind(hash(token))
    .execute(&ctx.pool)
    .await
    .expect("seed token");
}

// ── Request ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn request_answers_identically_for_known_and_unknown_emails_and_stores_only_a_hash() {
    let ctx = AdminTestContext::new().await;
    let unknown = request_reset(&ctx, "nobody@example.com").await;
    let malformed = request_reset(&ctx, "not-an-email").await;
    let known = request_reset(&ctx, &ctx.user_email.to_uppercase()).await; // case-insensitive
    assert_eq!(unknown.0, 202);
    assert_eq!(unknown, malformed);
    assert_eq!(unknown, known, "no enumeration: same status and body");

    // unknown / malformed addresses create nothing and send nothing
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM password_reset_tokens")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(total, 1);
    let token = emailed_token(&ctx, &ctx.user_email).await;
    assert_eq!(ctx.emails.sent.lock().unwrap().len(), 1);

    let (stored, one_hour): (Vec<u8>, bool) = sqlx::query_as(
        "SELECT token_hash, expires_at > now() + interval '55 minutes' AND expires_at <= now() + interval '1 hour' \
         FROM password_reset_tokens WHERE user_id = $1",
    )
    .bind(owner_id(&ctx))
    .fetch_one(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(stored, hash(&token), "only the BLAKE3 hash is stored");
    assert!(one_hour, "expiry is one hour");
}

#[tokio::test]
async fn a_second_request_replaces_the_first_token() {
    let ctx = AdminTestContext::new().await;
    request_reset(&ctx, &ctx.user_email).await;
    let first = emailed_token(&ctx, &ctx.user_email).await;
    request_reset(&ctx, &ctx.user_email).await;
    for _ in 0..100 {
        if ctx.emails.sent.lock().unwrap().len() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let second = emailed_token(&ctx, &ctx.user_email).await;
    assert_ne!(first, second);
    assert_eq!(token_rows(&ctx, owner_id(&ctx)).await, 1, "at most one live token per user");

    let totp = ctx.totp_code_now();
    assert_eq!(confirm_reset(&ctx, &first, json!({"totp_code": totp})).await.0, 404, "replaced token is dead");
    assert_eq!(confirm_reset(&ctx, &second, json!({"totp_code": ctx.totp_code_now()})).await.0, 204);
}

// ── Confirm ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn confirm_with_totp_sets_password_revokes_sessions_and_consumes_the_token() {
    let ctx = AdminTestContext::new().await;
    let old_session = sign_in(&ctx, &ctx.user_email, &ctx.user_password, ctx.totp_code_now()).await.1.unwrap();
    let other_session =
        sign_in(&ctx, &ctx.admin_email, &ctx.admin_password, ctx.admin_totp_code_now()).await.1.unwrap();
    assert_eq!(session_status(&ctx, &old_session).await, 200);

    request_reset(&ctx, &ctx.user_email).await;
    let token = emailed_token(&ctx, &ctx.user_email).await;
    let (status, body) = confirm_reset(&ctx, &token, json!({"totp_code": ctx.totp_code_now()})).await;
    assert_eq!(status, 204, "{body}");

    assert_eq!(session_status(&ctx, &old_session).await, 401, "all of the user's sessions die");
    assert_eq!(session_status(&ctx, &other_session).await, 200, "other users' sessions survive");
    assert_eq!(token_rows(&ctx, owner_id(&ctx)).await, 0, "token consumed");
    let old = sign_in(&ctx, &ctx.user_email, &ctx.user_password, ctx.totp_code_now()).await.0;
    assert_eq!(old, 401, "old password no longer works");
    let new = sign_in(&ctx, &ctx.user_email, NEW_PASSWORD, ctx.totp_code_now()).await.0;
    assert_eq!(new, 200, "new password works");

    // single use
    assert_eq!(confirm_reset(&ctx, &token, json!({"totp_code": ctx.totp_code_now()})).await.0, 404);
}

#[tokio::test]
async fn confirm_without_a_valid_second_factor_fails_keeps_the_token_and_counts_attempts() {
    let ctx = AdminTestContext::new().await;
    let user = owner_id(&ctx);
    let before = password_hash(&ctx, user).await;
    request_reset(&ctx, &ctx.user_email).await;
    let token = emailed_token(&ctx, &ctx.user_email).await;

    assert_eq!(confirm_reset(&ctx, &token, json!({})).await.0, 401, "email alone is not enough");
    assert_eq!(confirm_reset(&ctx, &token, json!({"totp_code": "000000"})).await.0, 401);
    assert_eq!(confirm_reset(&ctx, &token, json!({"totp_code": "000000"})).await.0, 401);
    let weak = post(
        &ctx,
        "/admin/v1/auth/password_reset/confirm",
        json!({"token": token, "new_password": "short", "totp_code": ctx.totp_code_now()}),
    )
    .await;
    assert_eq!(weak.0, 422, "password strength is checked (and does not consume)");

    let failed: i32 = sqlx::query_scalar("SELECT failed_totp_attempts FROM users WHERE id = $1")
        .bind(user)
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(failed, 2, "wrong codes counted by the signin lockout policy");
    assert_eq!(token_rows(&ctx, user).await, 1, "token not consumed");
    assert_eq!(password_hash(&ctx, user).await, before, "password unchanged");

    // third wrong code locks the account; even a right code is refused until it ends
    assert_eq!(confirm_reset(&ctx, &token, json!({"totp_code": "000000"})).await.0, 401);
    assert_eq!(confirm_reset(&ctx, &token, json!({"totp_code": ctx.totp_code_now()})).await.0, 429);
    assert_eq!(token_rows(&ctx, user).await, 1);
    assert_eq!(password_hash(&ctx, user).await, before);
}

#[tokio::test]
async fn a_recovery_code_is_an_accepted_second_factor_exactly_once() {
    let ctx = AdminTestContext::new().await;
    request_reset(&ctx, &ctx.user_email).await;
    let token = emailed_token(&ctx, &ctx.user_email).await;
    let (status, body) = confirm_reset(&ctx, &token, json!({"recovery_code": RECOVERY_CODE})).await;
    assert_eq!(status, 204, "{body}");
    let used: bool = sqlx::query_scalar("SELECT used_at IS NOT NULL FROM mfa_recovery_codes WHERE user_id = $1")
        .bind(owner_id(&ctx))
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert!(used);

    // a fresh reset cannot reuse the burned code
    seed_token(&ctx, owner_id(&ctx), &"C".repeat(43), "now() + interval '1 hour'").await;
    assert_eq!(confirm_reset(&ctx, &"C".repeat(43), json!({"recovery_code": RECOVERY_CODE})).await.0, 401);
}

#[tokio::test]
async fn unknown_malformed_expired_and_used_tokens_are_indistinguishable() {
    let ctx = AdminTestContext::new().await;
    let user = owner_id(&ctx);
    let expired = "E".repeat(43);
    seed_token(&ctx, user, &expired, "now() - interval '1 minute'").await;

    let totp = || json!({"totp_code": ctx.totp_code_now()});
    let unknown = confirm_reset(&ctx, &"U".repeat(43), totp()).await;
    let malformed = confirm_reset(&ctx, "short", totp()).await;
    let expired_r = confirm_reset(&ctx, &expired, totp()).await;
    assert_eq!(unknown.0, 404);
    assert_eq!(unknown, malformed);
    assert_eq!(unknown, expired_r);
    assert_eq!(token_rows(&ctx, user).await, 1, "expired row untouched, password unchanged");
}

// ── Abuse limits ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn requests_are_throttled_per_email_and_per_source_ip() {
    let ctx = AdminTestContext::new().await;
    // per email (burst 3, +1 per 20 min): identical for a known and an unknown address
    for email in [ctx.user_email.as_str(), "ghost@example.com"] {
        let statuses = [
            request_reset(&ctx, email).await.0,
            request_reset(&ctx, email).await.0,
            request_reset(&ctx, email).await.0,
            request_reset(&ctx, email).await.0,
        ];
        assert_eq!(statuses, [202, 202, 202, 429], "{email}");
    }

    // per IP (burst 10, +1 per min): 2 requests are already spent above
    let mut throttled = None;
    for i in 0..10 {
        let (status, _, headers) =
            post(&ctx, "/admin/v1/auth/password_reset/request", json!({"email": format!("u{i}@example.com")})).await;
        if status == 429 {
            throttled = Some((i, headers));
            break;
        }
        assert_eq!(status, 202);
    }
    // 8 of the 10 burst tokens were spent above, so the third further request is refused
    let (at, headers) = throttled.expect("expected a per-IP 429");
    assert!(headers.contains_key("retry-after-ms"));
    assert_eq!(at, 2, "per-IP burst of 10 exhausted");
}
