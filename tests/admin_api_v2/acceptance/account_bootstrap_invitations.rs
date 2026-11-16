// @driving_port @real-io
//! account-bootstrap-and-invitations (audit #63; remainder of #60).
//! docs/feature/account-bootstrap-and-invitations/feature-delta.md — AC-ABI-01..16.
//! CI only (testcontainers Postgres).

#[path = "../common/mod.rs"]
mod common;
use common::AdminTestContext;
use serde_json::{json, Value};
use totp_rs::{Algorithm as TotpAlgorithm, TOTP};
use uuid::Uuid;

const OPERATOR_KEY: &str = "test-admin-key-from-env";
const PASSWORD: &str = "a-long-enough-password";

// ── helpers ──────────────────────────────────────────────────────────────────

async fn call(req: reqwest::RequestBuilder) -> (u16, Value) {
    let resp = req.send().await.expect("request failed");
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap_or(Value::Null))
}

async fn sign_in(ctx: &AdminTestContext, email: &str, password: &str, totp_code: String) -> (u16, Option<String>) {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&json!({"email": email, "password": password, "totp_code": totp_code}))
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

async fn owner(ctx: &AdminTestContext) -> String {
    sign_in(ctx, &ctx.user_email, &ctx.user_password, ctx.totp_code_now()).await.1.expect("owner cookie")
}
async fn admin(ctx: &AdminTestContext) -> String {
    sign_in(ctx, &ctx.admin_email, &ctx.admin_password, ctx.admin_totp_code_now()).await.1.expect("admin cookie")
}
async fn viewer(ctx: &AdminTestContext) -> String {
    sign_in(ctx, &ctx.viewer_email, &ctx.viewer_password, ctx.viewer_totp_code_now()).await.1.expect("viewer cookie")
}

async fn bootstrap(ctx: &AdminTestContext, name: &str, email: &str) -> (u16, Value) {
    call(
        ctx.client
            .post(ctx.url("/admin/v1/accounts"))
            .bearer_auth(OPERATOR_KEY)
            .json(&json!({"account_name": name, "owner_email": email})),
    )
    .await
}

async fn invite(ctx: &AdminTestContext, cookie: &str, email: &str, role: &str) -> (u16, Value) {
    call(
        ctx.client
            .post(ctx.url("/admin/v1/members/invite"))
            .header("Cookie", cookie)
            .json(&json!({"email": email, "role": role})),
    )
    .await
}

async fn accept(ctx: &AdminTestContext, token: &str, password: &str) -> (u16, Value) {
    call(
        ctx.client
            .post(ctx.url("/admin/v1/auth/invitations/accept"))
            .json(&json!({"token": token, "display_name": "New Person", "password": password})),
    )
    .await
}

async fn confirm(ctx: &AdminTestContext, token: &str, code: &str) -> (u16, Value) {
    call(
        ctx.client
            .post(ctx.url("/admin/v1/auth/invitations/confirm"))
            .json(&json!({"token": token, "totp_code": code})),
    )
    .await
}

fn base32_decode(s: &str) -> Vec<u8> {
    let (mut out, mut buf, mut bits) = (Vec::new(), 0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => panic!("not base32: {c}"),
        };
        buf = (buf << 5) | u32::from(v);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    out
}

fn code_for(secret_b32: &str) -> String {
    TOTP::new(TotpAlgorithm::SHA1, 6, 1, 30, base32_decode(secret_b32), None, String::new())
        .expect("totp")
        .generate_current()
        .expect("code")
}

/// accept + confirm with the correct code; returns (totp_secret_b32, recovery_codes).
async fn redeem(ctx: &AdminTestContext, token: &str) -> (String, Vec<String>) {
    let (status, accepted) = accept(ctx, token, PASSWORD).await;
    assert_eq!(status, 200, "accept: {accepted}");
    let secret = accepted["totp_secret"].as_str().expect("totp_secret").to_string();
    let (status, confirmed) = confirm(ctx, token, &code_for(&secret)).await;
    assert_eq!(status, 200, "confirm: {confirmed}");
    let codes = confirmed["recovery_codes"]
        .as_array()
        .expect("recovery_codes")
        .iter()
        .map(|c| c.as_str().expect("code").to_string())
        .collect();
    (secret, codes)
}

async fn count(ctx: &AdminTestContext, sql: &str, bind: &str) -> i64 {
    sqlx::query_scalar(sql).bind(bind).fetch_one(&ctx.pool).await.expect("count")
}

async fn invitation_id_for(ctx: &AdminTestContext, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM invitations WHERE email = $1")
        .bind(email)
        .fetch_one(&ctx.pool)
        .await
        .expect("invitation id")
}

// ── Operator bootstrap ───────────────────────────────────────────────────────

#[tokio::test]
async fn ac_abi_01_bootstrap_creates_account_and_owner_invitation_but_no_user() {
    let ctx = AdminTestContext::new().await;
    let (status, body) = bootstrap(&ctx, "Acme", "founder@acme.test").await;
    assert_eq!(status, 201, "{body}");
    for key in ["account_id", "invitation_id", "invite_token", "expires_at"] {
        assert!(body.get(key).is_some(), "missing {key}");
    }
    assert_eq!(count(&ctx, "SELECT count(*) FROM accounts WHERE name = $1", "Acme").await, 1);
    assert_eq!(count(&ctx, "SELECT count(*) FROM users WHERE email = $1", "founder@acme.test").await, 0);
    let account_id = Uuid::parse_str(body["account_id"].as_str().unwrap()).unwrap();
    let members: i64 = sqlx::query_scalar("SELECT count(*) FROM account_members WHERE account_id = $1")
        .bind(account_id)
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(members, 0);
    let (role, invited_by): (String, Option<Uuid>) =
        sqlx::query_as("SELECT role, invited_by FROM invitations WHERE account_id = $1")
            .bind(account_id)
            .fetch_one(&ctx.pool)
            .await
            .unwrap();
    assert_eq!((role.as_str(), invited_by), ("Owner", None));
}

#[tokio::test]
async fn ac_abi_02_bootstrap_auth_conflicts_and_validation() {
    let ctx = AdminTestContext::new().await;
    let body = json!({"account_name": "Acme", "owner_email": "founder@acme.test"});
    let (none, _) = call(ctx.client.post(ctx.url("/admin/v1/accounts")).json(&body)).await;
    let cookie = owner(&ctx).await;
    let (session, _) =
        call(ctx.client.post(ctx.url("/admin/v1/accounts")).header("Cookie", &cookie).json(&body)).await;
    assert!(matches!(none, 401 | 403) && matches!(session, 401 | 403), "{none} {session}");
    assert_eq!(count(&ctx, "SELECT count(*) FROM accounts WHERE name = $1", "Acme").await, 0);

    assert_eq!(bootstrap(&ctx, "Acme", "founder@acme.test").await.0, 201);
    assert_eq!(bootstrap(&ctx, "Acme 2", "Founder@Acme.test").await.0, 409, "open invitation");
    assert_eq!(bootstrap(&ctx, "Acme 3", &ctx.user_email).await.0, 409, "existing user");
    assert_eq!(bootstrap(&ctx, "Acme 4", "not-an-email").await.0, 422);
    assert_eq!(bootstrap(&ctx, "  ", "x@acme.test").await.0, 422);
    assert_eq!(count(&ctx, "SELECT count(*) FROM accounts WHERE name LIKE $1", "Acme %").await, 0);
}

// ── Invite ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ac_abi_03_invite_returns_token_once_and_stores_only_its_hash() {
    let ctx = AdminTestContext::new().await;
    let cookie = owner(&ctx).await;
    let (status, body) = invite(&ctx, &cookie, "new@example.com", "Admin").await;
    assert_eq!(status, 202, "{body}");
    let token = body["invite_token"].as_str().expect("invite_token");
    assert_eq!(token.len(), 43);
    assert!(body["invitation_id"].is_string() && body["expires_at"].is_string());
    let stored: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM invitations WHERE email = $1")
        .bind("new@example.com")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(stored, blake3::hash(token.as_bytes()).as_bytes().to_vec());
    assert_ne!(stored, token.as_bytes());
}

#[tokio::test]
async fn ac_abi_04_invite_role_cap_validation_and_existing_user() {
    let ctx = AdminTestContext::new().await;
    let admin_cookie = admin(&ctx).await;
    assert_eq!(invite(&ctx, &admin_cookie, "boss@example.com", "Owner").await.0, 403);
    assert_eq!(invite(&ctx, &admin_cookie, "ok@example.com", "Admin").await.0, 202);
    let owner_cookie = owner(&ctx).await;
    assert_eq!(invite(&ctx, &owner_cookie, "x@example.com", "superuser").await.0, 422);
    assert_eq!(invite(&ctx, &owner_cookie, "no-at-sign", "Viewer").await.0, 422);
    assert_eq!(invite(&ctx, &owner_cookie, &ctx.viewer_email, "Viewer").await.0, 409);
    assert_eq!(invite(&ctx, &owner_cookie, "OK@example.com", "Viewer").await.0, 409, "open invitation, any case");
}

#[tokio::test]
async fn ac_abi_05_viewer_cannot_invite() {
    let ctx = AdminTestContext::new().await;
    let cookie = viewer(&ctx).await;
    assert_eq!(invite(&ctx, &cookie, "x@example.com", "Viewer").await.0, 403);
    assert_eq!(count(&ctx, "SELECT count(*) FROM invitations WHERE email = $1", "x@example.com").await, 0);
}

// ── Redeem ───────────────────────────────────────────────────────────────────

async fn fresh_token(ctx: &AdminTestContext, email: &str) -> String {
    let cookie = owner(ctx).await;
    let (status, body) = invite(ctx, &cookie, email, "Viewer").await;
    assert_eq!(status, 202);
    body["invite_token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn ac_abi_06_accept_issues_totp_secret_and_creates_no_user() {
    let ctx = AdminTestContext::new().await;
    let token = fresh_token(&ctx, "p@example.com").await;
    let (status, body) = accept(&ctx, &token, PASSWORD).await;
    assert_eq!(status, 200, "{body}");
    assert!(body["otpauth_uri"].as_str().unwrap().starts_with("otpauth://totp/"));
    assert!(body["totp_secret"].is_string() && body["enrolment_expires_at"].is_string());
    assert!(body.get("password").is_none());
    assert_eq!(count(&ctx, "SELECT count(*) FROM users WHERE email = $1", "p@example.com").await, 0);
    assert_eq!(count(&ctx, "SELECT count(*) FROM pending_enrolments e JOIN invitations i ON i.id = e.invitation_id WHERE i.email = $1", "p@example.com").await, 1);
}

#[tokio::test]
async fn ac_abi_07_every_bad_token_gets_the_same_404() {
    let ctx = AdminTestContext::new().await;
    let cookie = owner(&ctx).await;
    let mut tokens = vec![
        ("unknown", "A".repeat(43)),
        ("malformed", "short".to_string()),
    ];
    let expired = fresh_token(&ctx, "expired@example.com").await;
    sqlx::query("UPDATE invitations SET expires_at = now() - interval '1 minute' WHERE email = 'expired@example.com'")
        .execute(&ctx.pool).await.unwrap();
    tokens.push(("expired", expired));
    let revoked = fresh_token(&ctx, "revoked@example.com").await;
    let id = invitation_id_for(&ctx, "revoked@example.com").await;
    let (status, _) = call(ctx.client.delete(ctx.url(&format!("/admin/v1/members/invitations/{id}"))).header("Cookie", &cookie)).await;
    assert_eq!(status, 204);
    tokens.push(("revoked", revoked));
    let used = fresh_token(&ctx, "used@example.com").await;
    redeem(&ctx, &used).await;
    tokens.push(("used", used));

    for (label, token) in tokens {
        let (s1, b1) = accept(&ctx, &token, PASSWORD).await;
        let (s2, b2) = confirm(&ctx, &token, "123456").await;
        assert_eq!((s1, s2), (404, 404), "{label}");
        assert_eq!(b1, json!({"message": "Invalid or expired invitation"}), "{label}");
        assert_eq!(b2, b1, "{label}");
    }
}

#[tokio::test]
async fn ac_abi_08_short_password_is_422_and_stores_nothing() {
    let ctx = AdminTestContext::new().await;
    let token = fresh_token(&ctx, "p@example.com").await;
    assert_eq!(accept(&ctx, &token, "elevenchars").await.0, 422);
    assert_eq!(count(&ctx, "SELECT count(*) FROM pending_enrolments WHERE display_name = $1", "New Person").await, 0);
}

#[tokio::test]
async fn ac_abi_09_confirm_creates_user_membership_recovery_codes_and_signin_works() {
    let ctx = AdminTestContext::new().await;
    let token = fresh_token(&ctx, "p@example.com").await;
    let (secret, codes) = redeem(&ctx, &token).await;
    assert_eq!(codes.len(), 10);

    let (role, joined): (String, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT am.role, am.joined_at FROM account_members am JOIN users u ON u.id = am.user_id WHERE u.email = $1",
    )
    .bind("p@example.com")
    .fetch_one(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(role, "Viewer");
    assert!(joined.is_some());
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM mfa_recovery_codes c JOIN users u ON u.id = c.user_id WHERE u.email = $1",
    )
    .bind("p@example.com")
    .fetch_one(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(stored, 10);
    assert_eq!(count(&ctx, "SELECT count(*) FROM invitations WHERE email = $1 AND accepted_at IS NOT NULL", "p@example.com").await, 1);
    assert_eq!(count(&ctx, "SELECT count(*) FROM pending_enrolments WHERE display_name = $1", "New Person").await, 0);

    let (status, cookie) = sign_in(&ctx, "p@example.com", PASSWORD, code_for(&secret)).await;
    assert_eq!(status, 200);
    assert!(cookie.is_some());
}

#[tokio::test]
async fn ac_abi_10_five_wrong_codes_invalidate_the_enrolment_but_not_the_token() {
    let ctx = AdminTestContext::new().await;
    let token = fresh_token(&ctx, "p@example.com").await;
    let (_, accepted) = accept(&ctx, &token, PASSWORD).await;
    let secret = accepted["totp_secret"].as_str().unwrap().to_string();
    let right = code_for(&secret);
    let wrong = if right == "000000" { "000001" } else { "000000" };
    for _ in 0..5 {
        assert_eq!(confirm(&ctx, &token, wrong).await.0, 401);
    }
    assert_eq!(confirm(&ctx, &token, &right).await.0, 404, "enrolment gone, even with the right code");
    assert_eq!(count(&ctx, "SELECT count(*) FROM users WHERE email = $1", "p@example.com").await, 0);
    // token still valid: accept again, then confirm
    let (status, again) = accept(&ctx, &token, PASSWORD).await;
    assert_eq!(status, 200);
    let code = code_for(again["totp_secret"].as_str().unwrap());
    assert_eq!(confirm(&ctx, &token, &code).await.0, 200);
}

#[tokio::test]
async fn ac_abi_11_concurrent_confirms_create_exactly_one_user() {
    let ctx = AdminTestContext::new().await;
    let token = fresh_token(&ctx, "p@example.com").await;
    let (_, accepted) = accept(&ctx, &token, PASSWORD).await;
    let code = code_for(accepted["totp_secret"].as_str().unwrap());
    let (a, b, c) = tokio::join!(confirm(&ctx, &token, &code), confirm(&ctx, &token, &code), confirm(&ctx, &token, &code));
    let mut statuses = [a.0, b.0, c.0];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 404, 404]);
    assert_eq!(count(&ctx, "SELECT count(*) FROM users WHERE email = $1", "p@example.com").await, 1);
}

#[tokio::test]
async fn ac_abi_12_redeemed_token_cannot_create_a_second_user() {
    let ctx = AdminTestContext::new().await;
    let token = fresh_token(&ctx, "p@example.com").await;
    redeem(&ctx, &token).await;
    assert_eq!(accept(&ctx, &token, "another-long-password").await.0, 404);
    assert_eq!(confirm(&ctx, &token, "123456").await.0, 404);
    assert_eq!(count(&ctx, "SELECT count(*) FROM users WHERE email = $1", "p@example.com").await, 1);
}

// ── Revoke and list ──────────────────────────────────────────────────────────

#[tokio::test]
async fn ac_abi_13_revoke_closes_the_token_and_enforces_scope() {
    let ctx = AdminTestContext::new().await;
    let cookie = owner(&ctx).await;
    let token = fresh_token(&ctx, "p@example.com").await;
    let id = invitation_id_for(&ctx, "p@example.com").await;
    let url = |id: &dyn std::fmt::Display| ctx.url(&format!("/admin/v1/members/invitations/{id}"));

    // a pending Owner invitation in the same account: Admin may not revoke it
    let (_, owner_invite) = invite(&ctx, &cookie, "co-owner@example.com", "Owner").await;
    let owner_inv_id = owner_invite["invitation_id"].as_str().unwrap().to_string();
    let admin_cookie = admin(&ctx).await;
    assert_eq!(call(ctx.client.delete(url(&owner_inv_id)).header("Cookie", &admin_cookie)).await.0, 403);
    let viewer_cookie = viewer(&ctx).await;
    assert_eq!(call(ctx.client.delete(url(&id)).header("Cookie", &viewer_cookie)).await.0, 403);

    // foreign account's invitation and unknown id -> 404
    let (_, foreign) = bootstrap(&ctx, "Other", "f@other.test").await;
    assert_eq!(call(ctx.client.delete(url(&foreign["invitation_id"].as_str().unwrap())).header("Cookie", &cookie)).await.0, 404);
    assert_eq!(call(ctx.client.delete(url(&Uuid::new_v4())).header("Cookie", &cookie)).await.0, 404);
    assert_eq!(call(ctx.client.delete(url(&"not-a-uuid")).header("Cookie", &cookie)).await.0, 404);

    assert_eq!(call(ctx.client.delete(url(&id)).header("Cookie", &cookie)).await.0, 204);
    assert_eq!(accept(&ctx, &token, PASSWORD).await.0, 404);
    // revoked email can be invited again
    assert_eq!(invite(&ctx, &cookie, "p@example.com", "Viewer").await.0, 202);
}

#[tokio::test]
async fn ac_abi_14_list_shows_open_invitations_only() {
    let ctx = AdminTestContext::new().await;
    let cookie = owner(&ctx).await;
    fresh_token(&ctx, "open@example.com").await;
    let revoked_id = {
        fresh_token(&ctx, "gone@example.com").await;
        invitation_id_for(&ctx, "gone@example.com").await
    };
    call(ctx.client.delete(ctx.url(&format!("/admin/v1/members/invitations/{revoked_id}"))).header("Cookie", &cookie)).await;
    let done = fresh_token(&ctx, "done@example.com").await;
    redeem(&ctx, &done).await;

    let (status, list) = call(ctx.client.get(ctx.url("/admin/v1/members")).header("Cookie", &cookie)).await;
    assert_eq!(status, 200);
    let rows = list.as_array().unwrap();
    let pending: Vec<&Value> = rows.iter().filter(|r| r["auth_method"] == "pending").collect();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["email"], "open@example.com");
    let open_id = invitation_id_for(&ctx, "open@example.com").await;
    assert_eq!(pending[0]["id"], open_id.to_string());
    assert!(rows.iter().any(|r| r["email"] == "done@example.com" && r["auth_method"] == "password"));
    assert!(!rows.iter().any(|r| r["email"] == "gone@example.com"));
}

// ── Abuse limits ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn ac_abi_15_accept_and_confirm_are_rate_limited_per_source_ip() {
    let ctx = AdminTestContext::new().await;
    let token = "A".repeat(43);
    let mut throttled = None;
    for _ in 0..170 {
        let resp = ctx
            .client
            .post(ctx.url("/admin/v1/auth/invitations/confirm"))
            .json(&json!({"token": token, "totp_code": "000000"}))
            .send()
            .await
            .unwrap();
        if resp.status().as_u16() == 429 {
            throttled = Some(resp);
            break;
        }
    }
    let resp = throttled.expect("expected a 429 within 170 attempts");
    assert!(resp.headers().contains_key("retry-after-ms"));
    // shared per-IP bucket: accept is throttled too
    assert_eq!(accept(&ctx, &token, PASSWORD).await.0, 429);
}

#[tokio::test]
async fn ac_abi_16_concurrent_accepts_all_complete_under_the_argon2_gate() {
    let ctx = AdminTestContext::new().await;
    let cookie = owner(&ctx).await;
    let mut tokens = Vec::new();
    for i in 0..4 {
        let (_, body) = invite(&ctx, &cookie, &format!("c{i}@example.com"), "Viewer").await;
        tokens.push(body["invite_token"].as_str().unwrap().to_string());
    }
    let (a, b, c, d) = tokio::join!(
        accept(&ctx, &tokens[0], PASSWORD),
        accept(&ctx, &tokens[1], PASSWORD),
        accept(&ctx, &tokens[2], PASSWORD),
        accept(&ctx, &tokens[3], PASSWORD)
    );
    assert_eq!([a.0, b.0, c.0, d.0], [200; 4]);
}

// ── End to end ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn fresh_install_to_first_signin_to_second_invitee() {
    let ctx = AdminTestContext::new().await;
    let (status, boot) = bootstrap(&ctx, "Fresh Co", "first@fresh.test").await;
    assert_eq!(status, 201);
    let (secret, _) = redeem(&ctx, boot["invite_token"].as_str().unwrap()).await;

    let (status, cookie) = sign_in(&ctx, "first@fresh.test", PASSWORD, code_for(&secret)).await;
    assert_eq!(status, 200);
    let cookie = cookie.expect("cookie");

    let (status, invited) = invite(&ctx, &cookie, "second@fresh.test", "Admin").await;
    assert_eq!(status, 202);
    let (second_secret, _) = redeem(&ctx, invited["invite_token"].as_str().unwrap()).await;
    let (status, _) = sign_in(&ctx, "second@fresh.test", PASSWORD, code_for(&second_secret)).await;
    assert_eq!(status, 200);

    let account: Uuid = sqlx::query_scalar("SELECT account_id FROM users WHERE email = 'second@fresh.test'")
        .fetch_one(&ctx.pool).await.unwrap();
    assert_eq!(account.to_string(), boot["account_id"].as_str().unwrap());
}
