// @driving_port @real-io
//! admin-oidc-sso slice 5 (ADR-086): self-link (`start` intent `link`, password + TOTP re-entered)
//! and `DELETE /auth/sso_identity`, against the real router and the in-process mock IdP.
//! CI only (testcontainers Postgres). Compile locally with
//! `cargo test -p embyr-server --test admin_oidc_sso_s05_self_link --no-run`.
//!
//! ACs: AC-SSO-12 (link, sub already linked elsewhere, unlink, 409 for SSO-only), AC-SSO-16
//! (self-link needs password + TOTP; SSO-only users cannot), AC-SSO-18/19, #58-style cross-user cases.

#[path = "../../admin_api_v2/common/mod.rs"]
mod common;
#[path = "../../common/mock_oidc_idp.rs"]
mod mock_oidc_idp;
#[path = "sso_support.rs"]
mod support;

use common::totp_code_for_secret;
use serde_json::json;
use support::*;
use uuid::Uuid;

/// `start {intent:"link"}` as the SPA sends it: session cookie + password + current TOTP.
async fn start_link(s: &Sso, cookie: Option<&str>, password: &str, totp: Option<String>) -> reqwest::Response {
    let mut body = json!({ "provider_id": s.provider.to_string(), "intent": "link", "password": password });
    if let Some(t) = totp {
        body["totp_code"] = json!(t);
    }
    start_raw(&s.ctx, body, cookie).await
}

async fn owner_start_link(s: &Sso, cookie: &str) -> reqwest::Response {
    start_link(s, Some(cookie), &s.ctx.user_password, Some(s.ctx.totp_code_now())).await
}

async fn links_of(s: &Sso, user: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT subject FROM admin_sso_identities WHERE user_id = $1 ORDER BY subject")
        .bind(uuid(user))
        .fetch_all(&s.ctx.pool)
        .await
        .unwrap()
}

async fn unlink(s: &Sso, cookie: Option<&str>, query: &str) -> u16 {
    let mut req = s.ctx.client.delete(s.ctx.url(&format!("/admin/v1/auth/sso_identity{query}")));
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    req.send().await.unwrap().status().as_u16()
}

/// A session as the callback mints it for `user` through the provider.
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

async fn session_alive(s: &Sso, cookie: &str) -> bool {
    s.ctx.client.get(s.ctx.url("/admin/v1/auth/session")).header("Cookie", cookie).send().await.unwrap().status() == 200
}

// ── AC-SSO-12: self-link ─────────────────────────────────────────────────────

#[tokio::test]
async fn a_member_links_their_identity_after_re_entering_password_and_totp() {
    let s = setup().await;
    let cookie = owner_cookie(&s.ctx).await;
    s.idp.set(|b| b.sub = "idp-owner".into());

    let st = started(owner_start_link(&s, &cookie).await).await;
    // the state row carries the user and the intent, nothing else identifies who is linking
    let (intent, user, invitation): (String, Option<Uuid>, Option<Uuid>) =
        sqlx::query_as("SELECT intent, user_id, invitation_id FROM sso_auth_requests")
            .fetch_one(&s.ctx.pool)
            .await
            .unwrap();
    assert_eq!((intent.as_str(), user, invitation), ("link", Some(uuid(&s.ctx.user_id)), None));

    let resp = finish(&s, &st).await;
    assert_eq!(resp.status(), 302);
    assert_eq!(location_of(&resp), "/admin/?sso_linked=1");
    assert!(session_cookie_of(&resp).is_none(), "the user keeps their own session; none is minted");
    assert_eq!(links_of(&s, &s.ctx.user_id).await, vec!["idp-owner".to_string()]);
    assert_eq!(count(&s, "SELECT count(*) FROM sso_auth_requests").await, 0);
    assert_eq!(last_audit(&s).await, ("sso_link_created".into(), "ok".into(), None));

    // and the link works: SSO sign-in lands on the owner
    let st = started(start_raw(&s.ctx, json!({ "provider_id": s.provider.to_string() }), None).await).await;
    let signin = finish(&s, &st).await;
    assert_eq!(location_of(&signin), "/admin/");
    assert_eq!(session_user(&s.ctx, &session_cookie_of(&signin).unwrap()).await, s.ctx.user_id);
}

#[tokio::test]
async fn link_start_needs_a_session_and_both_credentials() {
    let s = setup().await;
    let cookie = owner_cookie(&s.ctx).await;

    // no session cookie
    let r = start_link(&s, None, &s.ctx.user_password, Some(s.ctx.totp_code_now())).await;
    assert_eq!(r.status(), 401);
    // a made-up session cookie
    let r = start_link(&s, Some("embyr_session=nope"), &s.ctx.user_password, Some(s.ctx.totp_code_now())).await;
    assert_eq!(r.status(), 401);
    // wrong password
    let r = start_link(&s, Some(&cookie), "not the password", Some(s.ctx.totp_code_now())).await;
    assert_eq!(r.status(), 401);
    // right password, no TOTP; right password, wrong TOTP
    assert_eq!(start_link(&s, Some(&cookie), &s.ctx.user_password, None).await.status(), 401);
    assert_eq!(start_link(&s, Some(&cookie), &s.ctx.user_password, Some(s.ctx.totp_code_wrong())).await.status(), 401);
    // a session cookie is not enough on its own
    let r = start_raw(&s.ctx, json!({ "provider_id": s.provider.to_string(), "intent": "link" }), Some(&cookie)).await;
    assert_eq!(r.status(), 401);

    assert_eq!(count(&s, "SELECT count(*) FROM sso_auth_requests").await, 0, "no state row without re-authentication");
}

#[tokio::test]
async fn an_sso_only_member_cannot_self_link() {
    let s = setup().await;
    let cookie = password_cookie(&s.ctx, &s.ctx.viewer_email, &s.ctx.viewer_password, &s.ctx.viewer_totp_secret_b32).await;
    // the viewer becomes SSO-only afterwards (as an SSO invitation would have made them)
    sqlx::query("UPDATE users SET password_hash = NULL WHERE id = $1")
        .bind(uuid(&s.ctx.viewer_id))
        .execute(&s.ctx.pool)
        .await
        .unwrap();

    let r = start_link(&s, Some(&cookie), &s.ctx.viewer_password, Some(s.ctx.viewer_totp_code_now())).await;
    assert_eq!(r.status(), 401);
    assert_eq!(count(&s, "SELECT count(*) FROM sso_auth_requests").await, 0);
}

#[tokio::test]
async fn link_start_is_scoped_to_the_sessions_own_account_and_unlinked_providers() {
    let s = setup().await;
    let cookie = owner_cookie(&s.ctx).await;
    let other_account: Uuid = sqlx::query_scalar("INSERT INTO accounts (name) VALUES ('Other') RETURNING id")
        .fetch_one(&s.ctx.pool)
        .await
        .unwrap();
    let foreign = insert_provider(&s.ctx, other_account, &s.idp.issuer(), &s.idp.client_id).await;

    let body = json!({
        "provider_id": foreign.to_string(), "intent": "link",
        "password": s.ctx.user_password, "totp_code": s.ctx.totp_code_now(),
    });
    assert!(is_unavailable(start_raw(&s.ctx, body, Some(&cookie)).await).await, "another account's provider");
    assert_eq!(count(&s, "SELECT count(*) FROM sso_auth_requests").await, 0);

    // already linked at this provider: 409, no second row
    link(&s, &s.ctx.user_id, "idp-owner").await;
    let r = owner_start_link(&s, &cookie).await;
    assert_eq!(r.status(), 409);
    assert_eq!(count(&s, "SELECT count(*) FROM sso_auth_requests").await, 0);
}

#[tokio::test]
async fn a_subject_linked_to_another_user_cannot_be_linked_again() {
    let s = setup().await;
    // the admin already owns "shared-sub"; the owner's IdP login asserts the same subject
    link(&s, &s.ctx.admin_id, "shared-sub").await;
    let cookie = owner_cookie(&s.ctx).await;
    s.idp.set(|b| b.sub = "shared-sub".into());

    let st = started(owner_start_link(&s, &cookie).await).await;
    let resp = finish(&s, &st).await;

    assert_failed(&resp);
    assert_eq!(last_audit(&s).await, ("sso_link_created".into(), "failed".into(), Some("link_conflict".into())));
    assert!(links_of(&s, &s.ctx.user_id).await.is_empty(), "nothing linked to the owner");
    assert_eq!(links_of(&s, &s.ctx.admin_id).await, vec!["shared-sub".to_string()], "the admin's link is untouched");
    assert_eq!(count(&s, "SELECT count(*) FROM sessions WHERE auth_method = 'sso'").await, 0);
}

#[tokio::test]
async fn the_link_callback_needs_the_browser_binding_and_a_live_membership() {
    let s = setup().await;
    let cookie = owner_cookie(&s.ctx).await;
    s.idp.set(|b| b.sub = "idp-owner".into());

    // state started in one browser, callback in another (no / wrong binding cookie)
    let st = started(owner_start_link(&s, &cookie).await).await;
    let location = s.idp.follow_authorize(&st.authorization_url).await;
    assert_failed(&callback_at(&location, None).await);
    assert_eq!(last_audit(&s).await, ("sso_link_created".into(), "failed".into(), Some("cookie_missing".into())));
    let st = started(owner_start_link(&s, &cookie).await).await;
    let location = s.idp.follow_authorize(&st.authorization_url).await;
    assert_failed(&callback_at(&location, Some("embyr_sso=someone-elses")).await);
    assert_eq!(last_audit(&s).await, ("sso_link_created".into(), "failed".into(), Some("cookie_mismatch".into())));

    // removed from the account between start and callback
    let st = started(owner_start_link(&s, &cookie).await).await;
    sqlx::query("DELETE FROM account_members WHERE user_id = $1")
        .bind(uuid(&s.ctx.user_id))
        .execute(&s.ctx.pool)
        .await
        .unwrap();
    assert_failed(&finish(&s, &st).await);
    assert_eq!(last_audit(&s).await, ("sso_link_created".into(), "failed".into(), Some("member_removed".into())));
    assert!(links_of(&s, &s.ctx.user_id).await.is_empty());
}

// ── AC-SSO-12: unlink ────────────────────────────────────────────────────────

#[tokio::test]
async fn a_password_member_unlinks_their_own_identity_only() {
    let s = setup().await;
    link(&s, &s.ctx.user_id, "idp-owner").await;
    link(&s, &s.ctx.admin_id, "idp-admin").await;
    let cookie = owner_cookie(&s.ctx).await;
    let sso_cookie = sso_session(&s, &s.ctx.user_id).await;
    let admin_sso_cookie = sso_session(&s, &s.ctx.admin_id).await;

    assert_eq!(unlink(&s, None, "").await, 401, "needs a session");
    assert_eq!(unlink(&s, Some(&cookie), "").await, 204);

    assert!(links_of(&s, &s.ctx.user_id).await.is_empty());
    assert_eq!(links_of(&s, &s.ctx.admin_id).await, vec!["idp-admin".to_string()], "someone else's link is not touched");
    assert!(session_alive(&s, &cookie).await, "the requesting session survives");
    assert!(!session_alive(&s, &sso_cookie).await, "SSO sessions minted through the removed link end");
    assert!(session_alive(&s, &admin_sso_cookie).await, "other users' sessions are not touched");
    assert_eq!(last_audit(&s).await, ("sso_link_removed".into(), "ok".into(), None));

    // nothing left to remove; and the removed identity can no longer sign in
    assert_eq!(unlink(&s, Some(&cookie), "").await, 404);
    s.idp.set(|b| b.sub = "idp-owner".into());
    let st = started(start_raw(&s.ctx, json!({ "provider_id": s.provider.to_string() }), None).await).await;
    assert_failed(&finish(&s, &st).await);
    assert_eq!(last_audit(&s).await, ("sso_signin".into(), "failed".into(), Some("identity_unlinked".into())));
}

#[tokio::test]
async fn unlink_can_name_a_provider() {
    let s = setup().await;
    link(&s, &s.ctx.user_id, "idp-owner").await;
    let cookie = owner_cookie(&s.ctx).await;

    assert_eq!(unlink(&s, Some(&cookie), &format!("?provider_id={}", Uuid::new_v4())).await, 404);
    assert_eq!(links_of(&s, &s.ctx.user_id).await.len(), 1);
    assert_eq!(unlink(&s, Some(&cookie), &format!("?provider_id={}", s.provider)).await, 204);
    assert!(links_of(&s, &s.ctx.user_id).await.is_empty());
}

#[tokio::test]
async fn an_sso_only_member_cannot_unlink_themselves_out() {
    let s = setup().await;
    link(&s, &s.ctx.viewer_id, "idp-viewer").await;
    sqlx::query("UPDATE users SET password_hash = NULL WHERE id = $1")
        .bind(uuid(&s.ctx.viewer_id))
        .execute(&s.ctx.pool)
        .await
        .unwrap();
    let cookie = sso_session(&s, &s.ctx.viewer_id).await;

    assert_eq!(unlink(&s, Some(&cookie), "").await, 409);
    assert_eq!(links_of(&s, &s.ctx.viewer_id).await, vec!["idp-viewer".to_string()]);
    assert!(session_alive(&s, &cookie).await);
    assert_eq!(count(&s, "SELECT count(*) FROM admin_audit_events WHERE event = 'sso_link_removed'").await, 0);
}

// ── AC-SSO-16: no reauth path for SSO-only users ─────────────────────────────

#[tokio::test]
async fn an_sso_only_session_gets_no_account_deletion_even_with_a_guessed_password() {
    let s = setup().await;
    link(&s, &s.ctx.viewer_id, "idp-viewer").await;
    sqlx::query("UPDATE users SET password_hash = NULL WHERE id = $1")
        .bind(uuid(&s.ctx.viewer_id))
        .execute(&s.ctx.pool)
        .await
        .unwrap();
    let cookie = sso_session(&s, &s.ctx.viewer_id).await;
    let del = s
        .ctx
        .client
        .delete(s.ctx.url("/admin/v1/account"))
        .header("Cookie", &cookie)
        .json(&json!({
            "password": s.ctx.viewer_password,
            "totp_code": totp_code_for_secret(&s.ctx.viewer_totp_secret_b32),
            "confirm_account_name": "Test Account",
        }))
        .send()
        .await
        .unwrap();
    assert!(del.status() == 401 || del.status() == 403, "{}", del.status());
    let deleted: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT deleted_at FROM accounts WHERE id = $1")
            .bind(uuid(&s.ctx.account_id))
            .fetch_one(&s.ctx.pool)
            .await
            .unwrap();
    assert!(deleted.is_none());
}
