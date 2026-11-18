//! Shared helpers for the slice 4 and 5 acceptance files (each is its own test crate and declares
//! `mod common;` and `mod mock_oidc_idp;` before including this one with `#[path]`).
//! Mirrors the helpers of `s03_start_and_callback.rs`, which stays untouched.
#![allow(dead_code)]

use crate::common::{totp_code_for_secret, AdminTestContext};
use crate::mock_oidc_idp::MockIdp;
use embyr_server::middleware::signin_rate_limit::SigninRateLimiter;
use serde_json::{json, Value};
use uuid::Uuid;

pub struct Sso {
    pub ctx: AdminTestContext,
    pub idp: MockIdp,
    pub provider: Uuid,
}

pub fn uuid(s: &str) -> Uuid {
    Uuid::parse_str(s).unwrap()
}

/// Encrypted exactly as the server encrypts a provider secret (the test key is `[0u8; 32]`).
fn secret_enc(secret: &str) -> Vec<u8> {
    embyr_server::adapters::encryption::encrypt(&[0u8; 32], secret.as_bytes()).unwrap()
}

pub async fn insert_provider(ctx: &AdminTestContext, account: Uuid, issuer: &str, client_id: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO oidc_providers (account_id, issuer, client_id, client_secret_enc, enabled) \
         VALUES ($1, $2, $3, $4, true) RETURNING id",
    )
    .bind(account)
    .bind(issuer)
    .bind(client_id)
    .bind(secret_enc("mock-secret"))
    .fetch_one(&ctx.pool)
    .await
    .expect("insert provider")
}

pub async fn setup() -> Sso {
    let ctx = AdminTestContext::with_sso(SigninRateLimiter::new(150.0, 10.0 / 60.0)).await;
    let idp = MockIdp::start().await;
    let provider = insert_provider(&ctx, uuid(&ctx.account_id), &idp.issuer(), &idp.client_id).await;
    Sso { ctx, idp, provider }
}

pub async fn link(s: &Sso, user: &str, subject: &str) {
    sqlx::query(
        "INSERT INTO admin_sso_identities (provider_id, subject, account_id, user_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(s.provider)
    .bind(subject)
    .bind(uuid(&s.ctx.account_id))
    .bind(uuid(user))
    .execute(&s.ctx.pool)
    .await
    .expect("insert identity");
}

pub fn no_redirect() -> reqwest::Client {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap()
}

pub fn set_cookies(resp: &reqwest::Response) -> Vec<String> {
    resp.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect()
}

pub fn location_of(r: &reqwest::Response) -> String {
    r.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string()
}

pub fn session_cookie_of(r: &reqwest::Response) -> Option<String> {
    set_cookies(r)
        .into_iter()
        .find(|c| c.starts_with("embyr_session=") && !c.contains("Max-Age=0"))
        .map(|c| c.split(';').next().unwrap().to_string())
}

pub struct Started {
    pub authorization_url: String,
    /// `embyr_sso=<value>` as a browser would send it back.
    pub cookie: String,
}

/// POST /auth/oidc/start with an arbitrary JSON body and an optional session cookie.
pub async fn start_raw(ctx: &AdminTestContext, body: Value, session_cookie: Option<&str>) -> reqwest::Response {
    let mut req = ctx.client.post(ctx.url("/admin/v1/auth/oidc/start")).json(&body);
    if let Some(c) = session_cookie {
        req = req.header("Cookie", c);
    }
    req.send().await.expect("start request")
}

/// Parses a successful start response.
pub async fn started(resp: reqwest::Response) -> Started {
    assert_eq!(resp.status(), 200);
    let cookie = set_cookies(&resp)
        .into_iter()
        .find(|c| c.starts_with("embyr_sso="))
        .expect("embyr_sso cookie")
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let body: Value = resp.json().await.unwrap();
    Started { authorization_url: body["authorization_url"].as_str().expect("authorization_url").to_string(), cookie }
}

pub async fn is_unavailable(resp: reqwest::Response) -> bool {
    resp.status() == 200 && resp.json::<Value>().await.unwrap() == json!({ "error": "sso_unavailable" })
}

pub async fn callback_at(location: &str, cookie: Option<&str>) -> reqwest::Response {
    let mut req = no_redirect().get(location);
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    req.send().await.expect("callback request")
}

/// Browser trip: IdP authorize, then the callback with the binding cookie.
pub async fn finish(s: &Sso, st: &Started) -> reqwest::Response {
    let location = s.idp.follow_authorize(&st.authorization_url).await;
    callback_at(&location, Some(&st.cookie)).await
}

/// What a browser can observe of a failed flow: identical for every cause (AC-SSO-19).
pub fn assert_failed(r: &reqwest::Response) {
    assert_eq!(r.status().as_u16(), 302);
    assert_eq!(location_of(r), "/admin/?error=sso_failed");
    assert!(session_cookie_of(r).is_none(), "no session on failure");
}

pub async fn count(s: &Sso, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&s.ctx.pool).await.expect("count")
}

pub async fn last_audit(s: &Sso) -> (String, String, Option<String>) {
    sqlx::query_as("SELECT event, outcome, reason FROM admin_audit_events ORDER BY id DESC LIMIT 1")
        .fetch_one(&s.ctx.pool)
        .await
        .expect("an audit row")
}

/// Password + TOTP sign-in of one of the seeded users; returns `embyr_session=<token>`.
pub async fn password_cookie(ctx: &AdminTestContext, email: &str, password: &str, totp_secret: &str) -> String {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&json!({"email": email, "password": password, "totp_code": totp_code_for_secret(totp_secret)}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "password sign-in");
    set_cookies(&resp)[0].split(';').next().unwrap().to_string()
}

pub async fn owner_cookie(ctx: &AdminTestContext) -> String {
    password_cookie(ctx, &ctx.user_email, &ctx.user_password, &ctx.totp_secret_b32).await
}

pub async fn admin_cookie(ctx: &AdminTestContext) -> String {
    password_cookie(ctx, &ctx.admin_email, &ctx.admin_password, &ctx.admin_totp_secret_b32).await
}

pub async fn session_user(ctx: &AdminTestContext, cookie: &str) -> String {
    let resp = ctx.client.get(ctx.url("/admin/v1/auth/session")).header("Cookie", cookie).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    resp.json::<Value>().await.unwrap()["user_id"].as_str().unwrap().to_string()
}

/// Creates an invitation through the real API. Returns the response status and body.
pub async fn invite(ctx: &AdminTestContext, cookie: &str, email: &str, role: &str, auth_method: Option<&str>) -> (u16, Value) {
    let mut body = json!({ "email": email, "role": role });
    if let Some(m) = auth_method {
        body["auth_method"] = json!(m);
    }
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/members/invite"))
        .header("Cookie", cookie)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    (status, resp.json::<Value>().await.unwrap_or(Value::Null))
}

/// An open SSO invitation for `email` (Viewer unless `role` says otherwise); returns the raw token.
pub async fn sso_invitation(s: &Sso, email: &str, role: &str) -> String {
    let cookie = owner_cookie(&s.ctx).await;
    let (status, body) = invite(&s.ctx, &cookie, email, role, Some("sso")).await;
    assert_eq!(status, 202, "{body}");
    body["invite_token"].as_str().unwrap().to_string()
}

pub async fn start_invite(s: &Sso, token: &str) -> reqwest::Response {
    start_raw(&s.ctx, json!({ "provider_id": s.provider.to_string(), "intent": "invite", "invite_token": token }), None)
        .await
}

/// A whole SSO-invitation redemption with the IdP asserting `sub` / `email`; returns the callback response.
pub async fn redeem(s: &Sso, token: &str, sub: &str, email: &str) -> reqwest::Response {
    s.idp.set(|b| {
        b.sub = sub.into();
        b.email = Some(email.into());
    });
    let st = started(start_invite(s, token).await).await;
    finish(s, &st).await
}
