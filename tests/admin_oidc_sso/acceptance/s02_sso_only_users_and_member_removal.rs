// @driving_port @real-io
//! admin-oidc-sso slice 2 (ADR-086): nullable `users.password_hash`, SSO session bookkeeping and
//! `remove_member` link cleanup. CI only (testcontainers Postgres): compile locally with
//! `cargo test -p embyr-server --test admin_oidc_sso_s02_sso_only_users --no-run`.
//!
//! ACs: AC-SSO-10, AC-SSO-13 (session part, O6), AC-SSO-14, AC-SSO-16, AC-SSO-19-shaped
//! "no oracle" checks. No start/callback here (slice 3): SSO state is seeded with SQL.

#[path = "../../admin_api_v2/common/mod.rs"]
mod common;
#[path = "../../common/mock_oidc_idp.rs"]
mod mock_oidc_idp;
use common::{totp_code_for_secret, AdminTestContext};
use serde_json::{json, Value};
use uuid::Uuid;

// ── helpers ──────────────────────────────────────────────────────────────────

async fn sign_in_raw(ctx: &AdminTestContext, email: &str, password: &str, totp_secret: &str) -> reqwest::Response {
    ctx.client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&json!({"email": email, "password": password, "totp_code": totp_code_for_secret(totp_secret)}))
        .send()
        .await
        .expect("signin failed")
}

fn cookie_of(resp: &reqwest::Response) -> String {
    resp.headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(|s| s.trim().to_string())
        .expect("no session cookie")
}

async fn owner_cookie(ctx: &AdminTestContext) -> String {
    cookie_of(&sign_in_raw(ctx, &ctx.user_email, &ctx.user_password, &ctx.totp_secret_b32).await)
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

fn uuid(s: &str) -> Uuid {
    Uuid::parse_str(s).unwrap()
}

async fn make_provider(ctx: &AdminTestContext, issuer: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO oidc_providers (account_id, issuer, client_id, client_secret_enc) \
         VALUES ($1, $2, 'client', '\\x00'::bytea) RETURNING id",
    )
    .bind(uuid(&ctx.account_id))
    .bind(issuer)
    .fetch_one(&ctx.pool)
    .await
    .expect("insert provider")
}

async fn link(ctx: &AdminTestContext, provider: Uuid, user: &str, subject: &str) {
    sqlx::query(
        "INSERT INTO admin_sso_identities (provider_id, subject, account_id, user_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(provider)
    .bind(subject)
    .bind(uuid(&ctx.account_id))
    .bind(uuid(user))
    .execute(&ctx.pool)
    .await
    .expect("insert identity");
}

/// A session as the slice-3 callback will mint it (`auth_method='sso'`, provider recorded).
async fn sso_session(ctx: &AdminTestContext, user: &str, provider: Uuid) -> String {
    let token = format!("sso-test-token-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO sessions (user_id, account_id, token_hash, expires_at, auth_method, sso_provider_id) \
         VALUES ($1, $2, $3, now() + interval '1 hour', 'sso', $4)",
    )
    .bind(uuid(user))
    .bind(uuid(&ctx.account_id))
    .bind(blake3::hash(token.as_bytes()).as_bytes().to_vec())
    .bind(provider)
    .execute(&ctx.pool)
    .await
    .expect("insert sso session");
    format!("embyr_session={token}")
}

async fn request_reset(ctx: &AdminTestContext, email: &str) -> (u16, Value) {
    let r = ctx
        .client
        .post(ctx.url("/admin/v1/auth/password_reset/request"))
        .json(&json!({ "email": email }))
        .send()
        .await
        .unwrap();
    (r.status().as_u16(), r.json::<Value>().await.unwrap())
}

async fn make_sso_only(ctx: &AdminTestContext, user: &str) {
    sqlx::query("UPDATE users SET password_hash = NULL WHERE id = $1")
        .bind(uuid(user))
        .execute(&ctx.pool)
        .await
        .expect("null the password hash");
}

async fn count(ctx: &AdminTestContext, sql: &str, bind: Uuid) -> i64 {
    sqlx::query_scalar(sql).bind(bind).fetch_one(&ctx.pool).await.expect("count")
}

// ── AC-SSO-14: NULL password_hash is handled everywhere it is read ───────────

#[tokio::test]
async fn sso_only_user_gets_the_same_401_as_a_wrong_password() {
    let ctx = AdminTestContext::new().await;
    let wrong = sign_in_raw(&ctx, &ctx.admin_email, "not-the-password", &ctx.admin_totp_secret_b32).await;
    let (wrong_status, wrong_body) = (wrong.status(), wrong.text().await.unwrap());
    assert_eq!(wrong_status, 401);

    make_sso_only(&ctx, &ctx.viewer_id).await;
    // even with the password the user used to have, and a valid TOTP code
    let resp = sign_in_raw(&ctx, &ctx.viewer_email, &ctx.viewer_password, &ctx.viewer_totp_secret_b32).await;
    assert_eq!(resp.status(), wrong_status);
    assert!(resp.headers().get("set-cookie").is_none());
    assert_eq!(resp.text().await.unwrap(), wrong_body, "no body-level oracle");
    assert_eq!(count(&ctx, "SELECT count(*) FROM sessions WHERE user_id = $1", uuid(&ctx.viewer_id)).await, 0);
}

#[tokio::test]
async fn password_signin_records_a_password_session_without_a_provider() {
    let ctx = AdminTestContext::new().await;
    assert!(owner_cookie(&ctx).await.starts_with("embyr_session="));
    let row: (String, Option<Uuid>) =
        sqlx::query_as("SELECT auth_method, sso_provider_id FROM sessions WHERE user_id = $1")
            .bind(uuid(&ctx.user_id))
            .fetch_one(&ctx.pool)
            .await
            .unwrap();
    assert_eq!(row, ("password".to_string(), None));
}

#[tokio::test]
async fn reset_request_for_an_sso_only_user_looks_unknown_and_sends_nothing() {
    let ctx = AdminTestContext::new().await;
    make_sso_only(&ctx, &ctx.viewer_id).await;
    let unknown = request_reset(&ctx, "nobody@example.com").await;
    let sso_only = request_reset(&ctx, &ctx.viewer_email).await;
    assert_eq!(sso_only, unknown);

    // control: a password user does get the email, so the silence above means something
    assert_eq!(request_reset(&ctx, &ctx.admin_email).await, unknown);
    for _ in 0..100 {
        if !ctx.emails.sent.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let sent: Vec<String> = ctx.emails.sent.lock().unwrap().iter().map(|m| m.to.clone()).collect();
    assert_eq!(sent, vec![ctx.admin_email.clone()], "only the password user is mailed");
    assert_eq!(count(&ctx, "SELECT count(*) FROM password_reset_tokens WHERE user_id = $1", uuid(&ctx.viewer_id)).await, 0);
}

#[tokio::test]
async fn reset_confirm_never_sets_a_password_on_an_sso_only_user() {
    let ctx = AdminTestContext::new().await;
    make_sso_only(&ctx, &ctx.viewer_id).await;
    // a token that should never exist; planted to prove confirm refuses it anyway
    let token = embyr_core::admin::invitation::generate_token();
    sqlx::query(
        "INSERT INTO password_reset_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '1 hour')",
    )
    .bind(uuid(&ctx.viewer_id))
    .bind(embyr_core::admin::invitation::hash_token(&token))
    .execute(&ctx.pool)
    .await
    .unwrap();

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/password_reset/confirm"))
        .json(&json!({
            "token": token, "new_password": "a-brand-new-password",
            "totp_code": totp_code_for_secret(&ctx.viewer_totp_secret_b32),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404, "answers like any invalid token");
    let hash: Option<String> = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(uuid(&ctx.viewer_id))
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(hash, None);
}

// ── AC-SSO-16: re-authentication that needs a password refuses an SSO-only user ──

#[tokio::test]
async fn account_deletion_reauth_refuses_a_user_without_a_password() {
    let ctx = AdminTestContext::new().await;
    let cookie = owner_cookie(&ctx).await;
    make_sso_only(&ctx, &ctx.user_id).await; // (cannot happen for an Owner in V1; exercises verify_credentials)
    let resp = ctx
        .client
        .delete(ctx.url("/admin/v1/account"))
        .header("Cookie", &cookie)
        .json(&json!({
            "password": ctx.user_password,
            "totp_code": totp_code_for_secret(&ctx.totp_secret_b32),
            "confirm_account_name": "Test Account",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    let deleted: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT deleted_at FROM accounts WHERE id = $1")
            .bind(uuid(&ctx.account_id))
            .fetch_one(&ctx.pool)
            .await
            .unwrap();
    assert!(deleted.is_none(), "account must be untouched");
}

// ── AC-SSO-10: remove_member deletes links, pending link requests, ends sessions ──

#[tokio::test]
async fn removing_a_member_deletes_their_links_and_ends_their_sessions_only() {
    let ctx = AdminTestContext::new().await;
    let provider = make_provider(&ctx, "https://idp.example.test").await;
    link(&ctx, provider, &ctx.viewer_id, "viewer-sub").await;
    link(&ctx, provider, &ctx.admin_id, "admin-sub").await;
    sqlx::query(
        "INSERT INTO sso_auth_requests (state_hash, provider_id, intent, user_id, nonce, code_verifier_enc, browser_binding, expires_at) \
         VALUES ('\\x01'::bytea, $1, 'link', $2, 'n', '\\x02'::bytea, '\\x03'::bytea, now() + interval '10 minutes')",
    )
    .bind(provider)
    .bind(uuid(&ctx.viewer_id))
    .execute(&ctx.pool)
    .await
    .unwrap();
    let viewer_session = sso_session(&ctx, &ctx.viewer_id, provider).await;
    let admin_session = sso_session(&ctx, &ctx.admin_id, provider).await;
    assert_eq!(session_status(&ctx, &viewer_session).await, 200, "seeded SSO session is usable");

    let owner = owner_cookie(&ctx).await;
    let resp = ctx
        .client
        .delete(ctx.url(&format!("/admin/v1/members/{}", ctx.viewer_id)))
        .header("Cookie", &owner)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    let viewer = uuid(&ctx.viewer_id);
    assert_eq!(count(&ctx, "SELECT count(*) FROM admin_sso_identities WHERE user_id = $1", viewer).await, 0);
    assert_eq!(count(&ctx, "SELECT count(*) FROM sso_auth_requests WHERE user_id = $1", viewer).await, 0);
    assert_eq!(count(&ctx, "SELECT count(*) FROM users WHERE id = $1", viewer).await, 1, "users row survives");
    assert_ne!(session_status(&ctx, &viewer_session).await, 200, "removed member's SSO session is dead");
    // other members untouched
    assert_eq!(count(&ctx, "SELECT count(*) FROM admin_sso_identities WHERE user_id = $1", uuid(&ctx.admin_id)).await, 1);
    assert_eq!(session_status(&ctx, &admin_session).await, 200);
}

// ── O6: disabling or deleting a provider ends its SSO sessions, never password sessions ──

#[tokio::test]
async fn disabling_a_provider_ends_its_sso_sessions_but_not_password_sessions() {
    // Enabling validates the issuer (slice 3, AC-SSO-07), so `other` points at the in-process mock IdP.
    std::env::set_var("EMBYR_ALLOW_PRIVATE_DSN_HOSTS", "true");
    let idp = mock_oidc_idp::MockIdp::start().await;
    let ctx = AdminTestContext::new().await;
    let provider = make_provider(&ctx, "https://idp.example.test").await;
    let other = make_provider(&ctx, &idp.issuer()).await;
    let viewer_sso = sso_session(&ctx, &ctx.viewer_id, provider).await;
    let admin_sso_other = sso_session(&ctx, &ctx.admin_id, other).await;
    let owner = owner_cookie(&ctx).await; // password session

    let resp = ctx
        .client
        .patch(ctx.url(&format!("/admin/v1/oidc_providers/{provider}")))
        .header("Cookie", &owner)
        .json(&json!({ "enabled": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    assert_ne!(session_status(&ctx, &viewer_sso).await, 200, "sessions of the disabled provider end");
    assert_eq!(session_status(&ctx, &admin_sso_other).await, 200, "another provider's sessions stay");
    assert_eq!(session_status(&ctx, &owner).await, 200, "password sessions stay");

    // re-enabling or a no-op patch must not end anything
    let resp = ctx
        .client
        .patch(ctx.url(&format!("/admin/v1/oidc_providers/{other}")))
        .header("Cookie", &owner)
        .json(&json!({ "enabled": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(session_status(&ctx, &admin_sso_other).await, 200);
}

#[tokio::test]
async fn deleting_a_provider_ends_its_sso_sessions_and_removes_its_links() {
    let ctx = AdminTestContext::new().await;
    let provider = make_provider(&ctx, "https://idp.example.test").await;
    link(&ctx, provider, &ctx.viewer_id, "viewer-sub").await;
    let viewer_sso = sso_session(&ctx, &ctx.viewer_id, provider).await;
    let owner = owner_cookie(&ctx).await;
    assert_eq!(session_status(&ctx, &viewer_sso).await, 200);

    let resp = ctx
        .client
        .delete(ctx.url(&format!("/admin/v1/oidc_providers/{provider}")))
        .header("Cookie", &owner)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    assert_ne!(session_status(&ctx, &viewer_sso).await, 200);
    assert_eq!(count(&ctx, "SELECT count(*) FROM admin_sso_identities WHERE provider_id = $1", provider).await, 0);
    assert_eq!(session_status(&ctx, &owner).await, 200);
}

// ── Schema invariants the later slices rely on ───────────────────────────────

#[tokio::test]
async fn schema_makes_cross_account_links_and_inconsistent_sessions_unrepresentable() {
    let ctx = AdminTestContext::new().await;
    let provider = make_provider(&ctx, "https://idp.example.test").await;
    let other_account: Uuid = sqlx::query_scalar("SELECT id FROM accounts WHERE name = 'Other Account'")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();

    // a link must carry the provider's account AND the user's account
    let cross = sqlx::query(
        "INSERT INTO admin_sso_identities (provider_id, subject, account_id, user_id) VALUES ($1, 's', $2, $3)",
    )
    .bind(provider)
    .bind(other_account)
    .bind(uuid(&ctx.viewer_id))
    .execute(&ctx.pool)
    .await;
    assert!(cross.is_err(), "link with a foreign account_id must be rejected");

    // one subject per provider, one link per (provider, user)
    link(&ctx, provider, &ctx.viewer_id, "same-sub").await;
    let dup_subject = sqlx::query(
        "INSERT INTO admin_sso_identities (provider_id, subject, account_id, user_id) VALUES ($1, 'same-sub', $2, $3)",
    )
    .bind(provider)
    .bind(uuid(&ctx.account_id))
    .bind(uuid(&ctx.admin_id))
    .execute(&ctx.pool)
    .await;
    assert!(dup_subject.is_err());

    // sessions: auth_method and sso_provider_id must agree
    for (method, provider_id) in [("sso", None), ("password", Some(provider))] {
        let bad = sqlx::query(
            "INSERT INTO sessions (user_id, account_id, token_hash, expires_at, auth_method, sso_provider_id) \
             VALUES ($1, $2, $3, now() + interval '1 hour', $4, $5)",
        )
        .bind(uuid(&ctx.viewer_id))
        .bind(uuid(&ctx.account_id))
        .bind(Uuid::new_v4().as_bytes().to_vec())
        .bind(method)
        .bind(provider_id)
        .execute(&ctx.pool)
        .await;
        assert!(bad.is_err(), "{method} session with provider {provider_id:?} must be rejected");
    }

    // state rows: intent decides which reference is required
    let bad_request = sqlx::query(
        "INSERT INTO sso_auth_requests (state_hash, provider_id, intent, nonce, code_verifier_enc, browser_binding, expires_at) \
         VALUES ('\\x09'::bytea, $1, 'invite', 'n', '\\x02'::bytea, '\\x03'::bytea, now())",
    )
    .bind(provider)
    .execute(&ctx.pool)
    .await;
    assert!(bad_request.is_err(), "invite intent without invitation_id must be rejected");
}
