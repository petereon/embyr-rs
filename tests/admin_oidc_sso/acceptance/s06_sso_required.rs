// @driving_port @real-io
//! admin-oidc-sso slice 6 (ADR-086 decision 6, O3): `PATCH /account {sso_required}` and its effect on
//! password sign-in, password invitations and password reset, plus the session probe fields.
//! CI only (testcontainers Postgres). Compile locally with
//! `cargo test -p embyr-server --test admin_oidc_sso_s06_sso_required --no-run`.
//!
//! ACs: AC-SSO-15 (toggle, lock-out guard, non-Owner 401, Owner exempt, audited), AC-SSO-14-shaped
//! "no oracle" checks, `GET /auth/session` `auth_method` / `sso_linked` / `sso_required`.

#[path = "../../admin_api_v2/common/mod.rs"]
mod common;
#[path = "../../common/mock_oidc_idp.rs"]
mod mock_oidc_idp;
#[path = "sso_support.rs"]
mod support;

use common::totp_code_for_secret;
use serde_json::{json, Value};
use support::*;
use uuid::Uuid;

async fn patch_account(s: &Sso, cookie: &str, body: Value) -> (u16, Value) {
    let r = s.ctx.client.patch(s.ctx.url("/admin/v1/account")).header("Cookie", cookie).json(&body).send().await.unwrap();
    (r.status().as_u16(), r.json::<Value>().await.unwrap_or(Value::Null))
}

async fn set_required(s: &Sso, on: bool) {
    sqlx::query("UPDATE accounts SET sso_required = $2 WHERE id = $1")
        .bind(uuid(&s.ctx.account_id))
        .bind(on)
        .execute(&s.ctx.pool)
        .await
        .unwrap();
}

async fn required(s: &Sso) -> bool {
    sqlx::query_scalar("SELECT sso_required FROM accounts WHERE id = $1")
        .bind(uuid(&s.ctx.account_id))
        .fetch_one(&s.ctx.pool)
        .await
        .unwrap()
}

async fn toggle_audits(s: &Sso) -> Vec<(String, Option<String>)> {
    sqlx::query_as(
        "SELECT outcome, reason FROM admin_audit_events WHERE event = 'sso_required_toggled' ORDER BY id",
    )
    .fetch_all(&s.ctx.pool)
    .await
    .unwrap()
}

async fn signin(s: &Sso, email: &str, password: &str, totp_secret: &str) -> reqwest::Response {
    s.ctx
        .client
        .post(s.ctx.url("/admin/v1/auth/signin"))
        .json(&json!({"email": email, "password": password, "totp_code": totp_code_for_secret(totp_secret)}))
        .send()
        .await
        .unwrap()
}

async fn session_json(s: &Sso, cookie: &str) -> Value {
    let r = s.ctx.client.get(s.ctx.url("/admin/v1/auth/session")).header("Cookie", cookie).send().await.unwrap();
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

// ── toggle: who, and the lock-out guard ──────────────────────────────────────

#[tokio::test]
async fn only_an_owner_can_toggle_sso_required() {
    let s = setup().await;
    link(&s, &s.ctx.user_id, "idp-owner").await;
    let admin = admin_cookie(&s.ctx).await;
    assert_eq!(patch_account(&s, &admin, json!({"sso_required": true})).await.0, 403);

    set_required(&s, true).await;
    assert_eq!(patch_account(&s, &admin, json!({"sso_required": false})).await.0, 403);
    assert!(required(&s).await, "a refused disable changes nothing");

    let no_cookie = s.ctx.client.patch(s.ctx.url("/admin/v1/account")).json(&json!({"sso_required": false})).send().await.unwrap();
    assert_eq!(no_cookie.status(), 401);
}

#[tokio::test]
async fn enabling_needs_an_owner_link_and_an_enabled_provider() {
    let s = setup().await;
    let owner = owner_cookie(&s.ctx).await;

    // provider enabled but the Owner has no link
    let (status, body) = patch_account(&s, &owner, json!({"sso_required": true})).await;
    assert_eq!((status, body["error"].as_str()), (422, Some("sso_required_prerequisites")));
    assert!(!required(&s).await);
    assert_eq!(toggle_audits(&s).await, vec![("failed".to_string(), Some("caller_not_linked".to_string()))]);

    // somebody else's link does not count
    link(&s, &s.ctx.admin_id, "idp-admin").await;
    assert_eq!(patch_account(&s, &owner, json!({"sso_required": true})).await.0, 422);

    // the Owner's own link, but the provider is disabled
    link(&s, &s.ctx.user_id, "idp-owner").await;
    sqlx::query("UPDATE oidc_providers SET enabled = false WHERE id = $1").bind(s.provider).execute(&s.ctx.pool).await.unwrap();
    assert_eq!(patch_account(&s, &owner, json!({"sso_required": true})).await.0, 422);
    assert_eq!(toggle_audits(&s).await.last().unwrap().1.as_deref(), Some("no_enabled_provider"));
    assert!(!required(&s).await);

    // enabled again: succeeds
    sqlx::query("UPDATE oidc_providers SET enabled = true WHERE id = $1").bind(s.provider).execute(&s.ctx.pool).await.unwrap();
    let (status, body) = patch_account(&s, &owner, json!({"sso_required": true})).await;
    assert_eq!((status, body), (200, json!({"sso_required": true})));
    assert!(required(&s).await);
}

#[tokio::test]
async fn toggles_are_audited_once_per_change_with_labels_only() {
    let s = setup().await;
    link(&s, &s.ctx.user_id, "idp-owner").await;
    let owner = owner_cookie(&s.ctx).await;

    assert_eq!(patch_account(&s, &owner, json!({"sso_required": true})).await.0, 200);
    assert_eq!(patch_account(&s, &owner, json!({"sso_required": true})).await.0, 200, "idempotent");
    assert_eq!(patch_account(&s, &owner, json!({"sso_required": false})).await.0, 200);
    assert_eq!(patch_account(&s, &owner, json!({"sso_required": false})).await.0, 200, "idempotent");

    assert_eq!(
        toggle_audits(&s).await,
        vec![
            ("ok".to_string(), Some("enabled:sessions_ended_0".to_string())),
            ("ok".to_string(), Some("disabled".to_string()))
        ]
    );
    let (actor, account): (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT actor_user_id, account_id FROM admin_audit_events WHERE event = 'sso_required_toggled' LIMIT 1",
    )
    .fetch_one(&s.ctx.pool)
    .await
    .unwrap();
    assert_eq!((actor, account), (Some(uuid(&s.ctx.user_id)), Some(uuid(&s.ctx.account_id))));
}

#[tokio::test]
async fn disabling_is_immediate_and_needs_no_prerequisites() {
    let s = setup().await;
    set_required(&s, true).await; // no link, provider removed: the guard must not block turning it off
    sqlx::query("DELETE FROM oidc_providers WHERE id = $1").bind(s.provider).execute(&s.ctx.pool).await.unwrap();
    let owner = owner_cookie(&s.ctx).await;
    assert_eq!(patch_account(&s, &owner, json!({"sso_required": false})).await.0, 200);
    assert!(!required(&s).await);
    assert_eq!(signin(&s, &s.ctx.admin_email, &s.ctx.admin_password, &s.ctx.admin_totp_secret_b32).await.status(), 200);
}

// ── effect on password sign-in ───────────────────────────────────────────────

#[tokio::test]
async fn required_makes_non_owner_password_signin_the_generic_401_and_owner_stays_exempt() {
    let s = setup().await;
    let wrong = signin(&s, &s.ctx.admin_email, "not-the-password", &s.ctx.admin_totp_secret_b32).await;
    let (wrong_status, wrong_body) = (wrong.status(), wrong.text().await.unwrap());
    assert_eq!(wrong_status, 401);

    set_required(&s, true).await;
    for (email, password, totp) in [
        (&s.ctx.admin_email, &s.ctx.admin_password, &s.ctx.admin_totp_secret_b32),
        (&s.ctx.viewer_email, &s.ctx.viewer_password, &s.ctx.viewer_totp_secret_b32),
    ] {
        let r = signin(&s, email, password, totp).await;
        assert_eq!(r.status(), wrong_status);
        assert!(r.headers().get("set-cookie").is_none());
        assert_eq!(r.text().await.unwrap(), wrong_body, "no body-level oracle");
    }
    assert_eq!(count(&s, &format!("SELECT count(*) FROM sessions WHERE user_id = '{}'", s.ctx.admin_id)).await, 0);

    // O3: the Owner keeps password + TOTP
    let r = signin(&s, &s.ctx.user_email, &s.ctx.user_password, &s.ctx.totp_secret_b32).await;
    assert_eq!(r.status(), 200);
    assert!(session_cookie_of(&r).is_some());
}

#[tokio::test]
async fn required_does_not_lock_a_non_owner_out_through_failed_totp_state() {
    // The refusal must not touch lockout counters (it is not a credential failure).
    let s = setup().await;
    set_required(&s, true).await;
    for _ in 0..4 {
        assert_eq!(signin(&s, &s.ctx.admin_email, &s.ctx.admin_password, &s.ctx.admin_totp_secret_b32).await.status(), 401);
    }
    set_required(&s, false).await;
    assert_eq!(signin(&s, &s.ctx.admin_email, &s.ctx.admin_password, &s.ctx.admin_totp_secret_b32).await.status(), 200);
}

// ── effect on invitations ────────────────────────────────────────────────────

#[tokio::test]
async fn required_refuses_password_invitations_for_non_owner_roles_only() {
    let s = setup().await;
    let owner = owner_cookie(&s.ctx).await;
    set_required(&s, true).await;

    for role in ["Admin", "Viewer"] {
        let (status, _) = invite(&s.ctx, &owner, &format!("pw-{role}@example.com"), role, None).await;
        assert_eq!(status, 422, "{role} default method");
        let (status, _) = invite(&s.ctx, &owner, &format!("pw2-{role}@example.com"), role, Some("password")).await;
        assert_eq!(status, 422, "{role} explicit password");
    }
    assert_eq!(count(&s, "SELECT count(*) FROM invitations WHERE email LIKE 'pw%'").await, 0);

    // SSO invitations and Owner (exempt) password invitations still work
    assert_eq!(invite(&s.ctx, &owner, "sso-v@example.com", "Viewer", Some("sso")).await.0, 202);
    assert_eq!(invite(&s.ctx, &owner, "pw-owner@example.com", "Owner", Some("password")).await.0, 202);

    set_required(&s, false).await;
    assert_eq!(invite(&s.ctx, &owner, "pw-later@example.com", "Viewer", None).await.0, 202);
}

// ── effect on password reset ─────────────────────────────────────────────────

async fn request_reset(s: &Sso, email: &str) -> (u16, Value) {
    let r = s.ctx.client.post(s.ctx.url("/admin/v1/auth/password_reset/request")).json(&json!({"email": email})).send().await.unwrap();
    (r.status().as_u16(), r.json::<Value>().await.unwrap())
}

#[tokio::test]
async fn required_makes_password_reset_a_neutral_no_op_for_non_owners() {
    let s = setup().await;
    set_required(&s, true).await;
    let unknown = request_reset(&s, "nobody@example.com").await;
    assert_eq!(request_reset(&s, &s.ctx.admin_email).await, unknown);
    assert_eq!(request_reset(&s, &s.ctx.viewer_email).await, unknown);
    // control: the exempt Owner is mailed, so the silence above means something
    assert_eq!(request_reset(&s, &s.ctx.user_email).await, unknown);
    for _ in 0..100 {
        if !s.ctx.emails.sent.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let sent: Vec<String> = s.ctx.emails.sent.lock().unwrap().iter().map(|m| m.to.clone()).collect();
    assert_eq!(sent, vec![s.ctx.user_email.clone()]);
    assert_eq!(count(&s, "SELECT count(*) FROM password_reset_tokens").await, 1, "only the Owner got a token");
}

#[tokio::test]
async fn a_reset_token_issued_before_required_stops_working_for_non_owners() {
    let s = setup().await;
    let token = embyr_core::admin::invitation::generate_token();
    sqlx::query(
        "INSERT INTO password_reset_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '1 hour')",
    )
    .bind(uuid(&s.ctx.admin_id))
    .bind(embyr_core::admin::invitation::hash_token(&token))
    .execute(&s.ctx.pool)
    .await
    .unwrap();
    set_required(&s, true).await;

    let before: Option<String> = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(uuid(&s.ctx.admin_id))
        .fetch_one(&s.ctx.pool)
        .await
        .unwrap();
    let r = s
        .ctx
        .client
        .post(s.ctx.url("/admin/v1/auth/password_reset/confirm"))
        .json(&json!({
            "token": token, "new_password": "a-brand-new-password",
            "totp_code": totp_code_for_secret(&s.ctx.admin_totp_secret_b32),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "answers like any invalid token");
    let after: Option<String> = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(uuid(&s.ctx.admin_id))
        .fetch_one(&s.ctx.pool)
        .await
        .unwrap();
    assert_eq!(before, after, "password untouched");
}

// ── session probe ────────────────────────────────────────────────────────────

#[tokio::test]
async fn session_probe_reports_auth_method_link_and_requirement() {
    let s = setup().await;
    let owner = owner_cookie(&s.ctx).await;
    let j = session_json(&s, &owner).await;
    assert_eq!((j["auth_method"].as_str(), j["sso_linked"].as_bool(), j["sso_required"].as_bool()), (Some("password"), Some(false), Some(false)));

    link(&s, &s.ctx.admin_id, "idp-admin").await;
    set_required(&s, true).await;
    let token = format!("sso-test-token-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO sessions (user_id, account_id, token_hash, expires_at, auth_method, sso_provider_id) \
         VALUES ($1, $2, $3, now() + interval '1 hour', 'sso', $4)",
    )
    .bind(uuid(&s.ctx.admin_id))
    .bind(uuid(&s.ctx.account_id))
    .bind(blake3::hash(token.as_bytes()).as_bytes().to_vec())
    .bind(s.provider)
    .execute(&s.ctx.pool)
    .await
    .unwrap();
    let j = session_json(&s, &format!("embyr_session={token}")).await;
    assert_eq!((j["auth_method"].as_str(), j["sso_linked"].as_bool(), j["sso_required"].as_bool()), (Some("sso"), Some(true), Some(true)));

    // the Owner's own (password) session sees the same account flag but its own method and link state
    let j = session_json(&s, &owner).await;
    assert_eq!((j["auth_method"].as_str(), j["sso_linked"].as_bool(), j["sso_required"].as_bool()), (Some("password"), Some(false), Some(true)));
}

// ── known-gap fixes (fix/sso-known-gaps) ─────────────────────────────────────

/// An SSO session of `user` through the seeded provider, inserted directly; returns its cookie.
async fn sso_session(s: &Sso, user: &str) -> String {
    let token = format!("sso-test-token-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO sessions (user_id, account_id, token_hash, expires_at, auth_method, sso_provider_id) \
         VALUES ($1, $2, $3, now() + interval '1 hour', 'sso', $4)",
    )
    .bind(uuid(user))
    .bind(uuid(&s.ctx.account_id))
    .bind(blake3::hash(token.as_bytes()).as_bytes().to_vec())
    .bind(s.provider)
    .execute(&s.ctx.pool)
    .await
    .unwrap();
    format!("embyr_session={token}")
}

async fn session_status(s: &Sso, cookie: &str) -> u16 {
    s.ctx.client.get(s.ctx.url("/admin/v1/auth/session")).header("Cookie", cookie).send().await.unwrap().status().as_u16()
}

async fn make_owner(s: &Sso, user: &str) {
    sqlx::query("UPDATE account_members SET role = 'Owner' WHERE user_id = $1")
        .bind(uuid(user))
        .execute(&s.ctx.pool)
        .await
        .unwrap();
}

async fn role_of(s: &Sso, user: &str) -> String {
    sqlx::query_scalar("SELECT role FROM account_members WHERE user_id = $1")
        .bind(uuid(user))
        .fetch_one(&s.ctx.pool)
        .await
        .unwrap()
}

async fn change_role(s: &Sso, cookie: &str, user: &str, role: &str) -> (u16, Value) {
    let r = s
        .ctx
        .client
        .patch(s.ctx.url(&format!("/admin/v1/members/{user}/role")))
        .header("Cookie", cookie)
        .json(&json!({ "role": role }))
        .send()
        .await
        .unwrap();
    (r.status().as_u16(), r.json::<Value>().await.unwrap_or(Value::Null))
}

async fn role_audits(s: &Sso) -> Vec<(String, Option<String>)> {
    sqlx::query_as("SELECT outcome, reason FROM admin_audit_events WHERE event = 'member_role_changed' ORDER BY id")
        .fetch_all(&s.ctx.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn demoting_an_owner_of_a_required_account_needs_an_sso_link_for_the_target() {
    let s = setup().await;
    let owner = owner_cookie(&s.ctx).await;
    make_owner(&s, &s.ctx.admin_id).await; // second Owner, password only
    set_required(&s, true).await;

    for role in ["Admin", "Viewer"] {
        let (status, body) = change_role(&s, &owner, &s.ctx.admin_id, role).await;
        assert_eq!((status, body["error"].as_str()), (422, Some("sso_link_required")), "{role}");
        assert!(body["message"].as_str().is_some_and(|m| !m.is_empty()));
        assert_eq!(role_of(&s, &s.ctx.admin_id).await, "Owner", "a refused change changes nothing");
    }
    assert_eq!(
        role_audits(&s).await,
        vec![("failed".to_string(), Some("sso_link_required".to_string())); 2]
    );

    // a link to a DISABLED provider does not count either
    link(&s, &s.ctx.admin_id, "idp-admin").await;
    sqlx::query("UPDATE oidc_providers SET enabled = false WHERE id = $1").bind(s.provider).execute(&s.ctx.pool).await.unwrap();
    assert_eq!(change_role(&s, &owner, &s.ctx.admin_id, "Viewer").await.0, 422);
    sqlx::query("UPDATE oidc_providers SET enabled = true WHERE id = $1").bind(s.provider).execute(&s.ctx.pool).await.unwrap();

    // linked to an enabled provider: allowed
    assert_eq!(change_role(&s, &owner, &s.ctx.admin_id, "Viewer").await, (200, json!({"ok": true})));
    assert_eq!(role_of(&s, &s.ctx.admin_id).await, "Viewer");
    assert_eq!(role_audits(&s).await.last().unwrap(), &("ok".to_string(), Some("demoted".to_string())));
}

#[tokio::test]
async fn demoting_an_owner_is_unrestricted_while_sso_is_not_required() {
    let s = setup().await;
    let owner = owner_cookie(&s.ctx).await;
    make_owner(&s, &s.ctx.admin_id).await;
    assert_eq!(change_role(&s, &owner, &s.ctx.admin_id, "Admin").await.0, 200);
    assert_eq!(role_of(&s, &s.ctx.admin_id).await, "Admin");
}

#[tokio::test]
async fn role_changes_that_do_not_cross_the_owner_boundary_are_unaffected_by_the_requirement() {
    let s = setup().await;
    let owner = owner_cookie(&s.ctx).await;
    set_required(&s, true).await;
    // Admin <-> Viewer, no link: nothing about how they sign in changes
    assert_eq!(change_role(&s, &owner, &s.ctx.admin_id, "Viewer").await.0, 200);
    assert_eq!(change_role(&s, &owner, &s.ctx.admin_id, "Admin").await.0, 200);
    // promoting a password member to Owner gives them the break-glass path
    assert_eq!(change_role(&s, &owner, &s.ctx.admin_id, "Owner").await.0, 200);
}

#[tokio::test]
async fn a_member_without_a_password_cannot_become_an_owner() {
    let s = setup().await;
    let owner = owner_cookie(&s.ctx).await;
    sqlx::query("UPDATE users SET password_hash = NULL WHERE id = $1")
        .bind(uuid(&s.ctx.viewer_id))
        .execute(&s.ctx.pool)
        .await
        .unwrap();
    for required in [false, true] {
        set_required(&s, required).await;
        let (status, body) = change_role(&s, &owner, &s.ctx.viewer_id, "Owner").await;
        assert_eq!((status, body["error"].as_str()), (422, Some("owner_requires_password")), "required={required}");
        assert_eq!(role_of(&s, &s.ctx.viewer_id).await, "Viewer");
    }
    assert_eq!(role_audits(&s).await.last().unwrap(), &("failed".to_string(), Some("owner_requires_password".to_string())));
}

#[tokio::test]
async fn enabling_ends_non_owner_password_sessions_only() {
    let s = setup().await;
    link(&s, &s.ctx.user_id, "idp-owner").await;
    let owner = owner_cookie(&s.ctx).await;
    let admin_pw = admin_cookie(&s.ctx).await;
    let viewer_pw = password_cookie(&s.ctx, &s.ctx.viewer_email, &s.ctx.viewer_password, &s.ctx.viewer_totp_secret_b32).await;
    link(&s, &s.ctx.admin_id, "idp-admin").await;
    let admin_sso = sso_session(&s, &s.ctx.admin_id).await;
    for c in [&owner, &admin_pw, &viewer_pw, &admin_sso] {
        assert_eq!(session_status(&s, c).await, 200);
    }

    // a refused enable ends nothing
    sqlx::query("UPDATE oidc_providers SET enabled = false WHERE id = $1").bind(s.provider).execute(&s.ctx.pool).await.unwrap();
    assert_eq!(patch_account(&s, &owner, json!({"sso_required": true})).await.0, 422);
    assert_eq!(session_status(&s, &admin_pw).await, 200);
    sqlx::query("UPDATE oidc_providers SET enabled = true WHERE id = $1").bind(s.provider).execute(&s.ctx.pool).await.unwrap();

    assert_eq!(patch_account(&s, &owner, json!({"sso_required": true})).await.0, 200);
    assert_eq!(session_status(&s, &admin_pw).await, 401, "non-Owner password session ended");
    assert_eq!(session_status(&s, &viewer_pw).await, 401, "non-Owner password session ended");
    assert_eq!(session_status(&s, &admin_sso).await, 200, "SSO session stays");
    assert_eq!(session_status(&s, &owner).await, 200, "the caller's (Owner) session stays");
    assert_eq!(toggle_audits(&s).await.last().unwrap().1.as_deref(), Some("enabled:sessions_ended_2"));

    // a repeat is a no-op: nothing more ended, no second audit row
    assert_eq!(patch_account(&s, &owner, json!({"sso_required": true})).await.0, 200);
    assert_eq!(toggle_audits(&s).await.len(), 1);

    // turning it off ends nothing
    assert_eq!(patch_account(&s, &owner, json!({"sso_required": false})).await.0, 200);
    assert_eq!((session_status(&s, &owner).await, session_status(&s, &admin_sso).await), (200, 200));
}

async fn provider_call(s: &Sso, cookie: &str, id: Uuid, method: &str, body: Option<Value>) -> (u16, Value) {
    let url = s.ctx.url(&format!("/admin/v1/oidc_providers/{id}"));
    let mut req = if method == "DELETE" { s.ctx.client.delete(url) } else { s.ctx.client.patch(url) };
    req = req.header("Cookie", cookie);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let r = req.send().await.unwrap();
    (r.status().as_u16(), r.json::<Value>().await.unwrap_or(Value::Null))
}

async fn provider_enabled(s: &Sso, id: Uuid) -> Option<bool> {
    sqlx::query_scalar("SELECT enabled FROM oidc_providers WHERE id = $1").bind(id).fetch_optional(&s.ctx.pool).await.unwrap()
}

#[tokio::test]
async fn the_last_enabled_provider_of_a_required_account_cannot_be_disabled_or_deleted() {
    let s = setup().await;
    link(&s, &s.ctx.user_id, "idp-owner").await;
    let owner = owner_cookie(&s.ctx).await;
    assert_eq!(patch_account(&s, &owner, json!({"sso_required": true})).await.0, 200);

    let (status, body) = provider_call(&s, &owner, s.provider, "PATCH", Some(json!({"enabled": false}))).await;
    assert_eq!((status, body["error"].as_str()), (422, Some("sso_required_active")));
    assert!(body["message"].as_str().unwrap().contains("Turn off the SSO requirement first"));
    let (status, body) = provider_call(&s, &owner, s.provider, "DELETE", None).await;
    assert_eq!((status, body["error"].as_str()), (422, Some("sso_required_active")));
    assert_eq!(provider_enabled(&s, s.provider).await, Some(true), "nothing changed");
    assert_eq!(
        sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT event, outcome, reason FROM admin_audit_events WHERE outcome = 'failed' ORDER BY id"
        )
        .fetch_all(&s.ctx.pool)
        .await
        .unwrap(),
        vec![
            ("sso_provider_disabled".to_string(), "failed".to_string(), Some("sso_required_last_provider".to_string())),
            ("sso_provider_deleted".to_string(), "failed".to_string(), Some("sso_required_last_provider".to_string())),
        ]
    );

    // with a second enabled provider either can go, the last one still cannot
    let other = insert_provider(&s.ctx, uuid(&s.ctx.account_id), "https://other.example.com", "other-client").await;
    assert_eq!(provider_call(&s, &owner, s.provider, "PATCH", Some(json!({"enabled": false}))).await.0, 200);
    assert_eq!(provider_call(&s, &owner, other, "PATCH", Some(json!({"enabled": false}))).await.0, 422);
    assert_eq!(provider_call(&s, &owner, other, "DELETE", None).await.0, 422);
    // a provider that is already disabled can be deleted: it was not a way in
    assert_eq!(provider_call(&s, &owner, s.provider, "DELETE", None).await.0, 204);

    // requirement off: the last one may go
    assert_eq!(patch_account(&s, &owner, json!({"sso_required": false})).await.0, 200);
    assert_eq!(provider_call(&s, &owner, other, "DELETE", None).await.0, 204);
}

#[tokio::test]
async fn providers_are_unrestricted_while_sso_is_not_required() {
    let s = setup().await;
    let owner = owner_cookie(&s.ctx).await;
    assert_eq!(provider_call(&s, &owner, s.provider, "PATCH", Some(json!({"enabled": false}))).await.0, 200);
    assert_eq!(provider_call(&s, &owner, s.provider, "DELETE", None).await.0, 204);
}

async fn unlink(s: &Sso, cookie: &str) -> (u16, Value) {
    let r = s.ctx.client.delete(s.ctx.url("/admin/v1/auth/sso_identity")).header("Cookie", cookie).send().await.unwrap();
    (r.status().as_u16(), r.json::<Value>().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_non_owner_of_a_required_account_cannot_unlink_their_last_enabled_link() {
    let s = setup().await;
    link(&s, &s.ctx.user_id, "idp-owner").await;
    link(&s, &s.ctx.admin_id, "idp-admin").await;
    let owner = owner_cookie(&s.ctx).await;
    let admin_sso = sso_session(&s, &s.ctx.admin_id).await;
    assert_eq!(patch_account(&s, &owner, json!({"sso_required": true})).await.0, 200);

    let (status, body) = unlink(&s, &admin_sso).await;
    assert_eq!((status, body["error"].as_str()), (409, Some("sso_link_required")));
    let links: i64 = sqlx::query_scalar("SELECT count(*) FROM admin_sso_identities WHERE user_id = $1")
        .bind(uuid(&s.ctx.admin_id))
        .fetch_one(&s.ctx.pool)
        .await
        .unwrap();
    assert_eq!(links, 1, "nothing changed");
    assert_eq!(
        last_audit(&s).await,
        ("sso_link_removed".to_string(), "failed".to_string(), Some("sso_required_last_link".to_string()))
    );

    // the Owner keeps password + TOTP, so the Owner may unlink
    assert_eq!(unlink(&s, &owner).await.0, 204);

    // requirement off: the Admin may unlink (they have a password to fall back on)
    sqlx::query("UPDATE accounts SET sso_required = false WHERE id = $1").bind(uuid(&s.ctx.account_id)).execute(&s.ctx.pool).await.unwrap();
    assert_eq!(unlink(&s, &admin_sso).await.0, 204);
}
