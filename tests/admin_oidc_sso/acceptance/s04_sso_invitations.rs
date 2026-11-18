// @driving_port @real-io
//! admin-oidc-sso slice 4 (ADR-086): invitations with `auth_method='sso'` against the real router
//! and the in-process mock IdP. CI only (testcontainers Postgres). Compile locally with
//! `cargo test -p embyr-server --test admin_oidc_sso_s04_sso_invitations --no-run`.
//!
//! ACs: AC-SSO-11 (create, lookup, start, callback, every failure creates nothing), AC-SSO-16
//! (an SSO-only member cannot delete the account), AC-SSO-18/19 (audit labels, one observable
//! failure shape), audit #58-style cross-user attempts.

#[path = "../../admin_api_v2/common/mod.rs"]
mod common;
#[path = "../../common/mock_oidc_idp.rs"]
mod mock_oidc_idp;
#[path = "sso_support.rs"]
mod support;

use serde_json::{json, Value};
use support::*;
use uuid::Uuid;

async fn invitation_open(s: &Sso, token: &str) -> bool {
    count(
        s,
        &format!(
            "SELECT count(*) FROM invitations WHERE token_hash = '\\x{}'::bytea AND accepted_at IS NULL",
            hex(&embyr_core::admin::invitation::hash_token(token))
        ),
    )
    .await
        == 1
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

async fn users(s: &Sso) -> i64 {
    count(s, "SELECT count(*) FROM users").await
}

// ── creation ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn creating_an_sso_invitation_is_validated_and_worded_for_sso() {
    let s = setup().await;
    let cookie = owner_cookie(&s.ctx).await;

    // unknown method, Owner role: 422
    assert_eq!(invite(&s.ctx, &cookie, "a@example.com", "Viewer", Some("magic")).await.0, 422);
    assert_eq!(invite(&s.ctx, &cookie, "boss@example.com", "Owner", Some("sso")).await.0, 422);
    assert_eq!(count(&s, "SELECT count(*) FROM invitations WHERE email IN ('a@example.com','boss@example.com')").await, 0);

    // default stays password
    let (status, body) = invite(&s.ctx, &cookie, "pw@example.com", "Viewer", None).await;
    assert_eq!((status, body["auth_method"].as_str()), (202, Some("password")));

    // sso with an enabled provider
    let (status, body) = invite(&s.ctx, &cookie, "sso@example.com", "Admin", Some("sso")).await;
    assert_eq!((status, body["auth_method"].as_str()), (202, Some("sso")), "{body}");
    let method: String = sqlx::query_scalar("SELECT auth_method FROM invitations WHERE email = 'sso@example.com'")
        .fetch_one(&s.ctx.pool)
        .await
        .unwrap();
    assert_eq!(method, "sso");
    let mail = s.ctx.emails.sent.lock().unwrap().last().cloned().expect("invite mail");
    assert!(
        mail.body_text.contains("Continue with SSO") && !mail.body_text.contains("enrol an authenticator"),
        "{}",
        mail.body_text
    );

    // disabled provider => none enabled => 422
    sqlx::query("UPDATE oidc_providers SET enabled = false").execute(&s.ctx.pool).await.unwrap();
    assert_eq!(invite(&s.ctx, &cookie, "late@example.com", "Viewer", Some("sso")).await.0, 422);
}

// ── lookup ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn lookup_tells_the_spa_how_to_redeem_without_echoing_the_address() {
    let s = setup().await;
    let cookie = owner_cookie(&s.ctx).await;
    let (_, pw) = invite(&s.ctx, &cookie, "pw@example.com", "Viewer", None).await;
    let sso_token = sso_invitation(&s, "sso@example.com", "Viewer").await;
    let lookup = |t: String| {
        let (c, u) = (s.ctx.client.clone(), s.ctx.url("/admin/v1/auth/invitations/lookup"));
        async move {
            let r = c.post(u).json(&json!({ "token": t })).send().await.unwrap();
            (r.status().as_u16(), r.json::<Value>().await.unwrap_or(Value::Null))
        }
    };

    assert_eq!(lookup(pw["invite_token"].as_str().unwrap().to_string()).await, (200, json!({ "auth_method": "password" })));
    assert_eq!(
        lookup(sso_token.clone()).await,
        (200, json!({ "auth_method": "sso", "sso_provider_ids": [s.provider.to_string()] }))
    );
    assert_eq!(lookup("garbage".into()).await.0, 404);
    assert_eq!(lookup(embyr_core::admin::invitation::generate_token()).await.0, 404);
    // a used invitation looks like any other dead token
    sqlx::query("UPDATE invitations SET accepted_at = now()").execute(&s.ctx.pool).await.unwrap();
    assert_eq!(lookup(sso_token).await.0, 404);
}

// ── an SSO invitation cannot be redeemed with a password ─────────────────────

#[tokio::test]
async fn an_sso_invitation_cannot_be_redeemed_with_password_and_totp() {
    let s = setup().await;
    let token = sso_invitation(&s, "sso@example.com", "Viewer").await;
    let users_before = users(&s).await;

    let accept = s
        .ctx
        .client
        .post(s.ctx.url("/admin/v1/auth/invitations/accept"))
        .json(&json!({ "token": token, "display_name": "Eve", "password": "correct horse battery staple 1!" }))
        .send()
        .await
        .unwrap();
    assert_eq!(accept.status(), 404);
    let confirm = s
        .ctx
        .client
        .post(s.ctx.url("/admin/v1/auth/invitations/confirm"))
        .json(&json!({ "token": token, "totp_code": "123456" }))
        .send()
        .await
        .unwrap();
    assert_eq!(confirm.status(), 404);
    assert_eq!(count(&s, "SELECT count(*) FROM pending_enrolments").await, 0);
    assert_eq!(users(&s).await, users_before);
    assert!(invitation_open(&s, &token).await);
}

// ── happy path ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn invitee_redeems_an_sso_invitation_and_gets_a_session() {
    let s = setup().await;
    let token = sso_invitation(&s, "newperson@example.com", "Admin").await;

    // e-mail case differs between the invitation and the IdP: still equal
    let resp = redeem(&s, &token, "idp-new", "NewPerson@Example.COM").await;

    assert_eq!(resp.status(), 302);
    assert_eq!(location_of(&resp), "/admin/", "no query string, no token in the URL");
    let cookie = session_cookie_of(&resp).expect("session cookie");
    let (user_id, pw, totp, name): (Uuid, Option<String>, Option<Vec<u8>>, String) = sqlx::query_as(
        "SELECT id, password_hash, totp_secret_enc, display_name FROM users WHERE email = 'newperson@example.com'",
    )
    .fetch_one(&s.ctx.pool)
    .await
    .expect("user created");
    assert_eq!((pw, totp, name.as_str()), (None, None, "newperson"), "SSO-only: no password, no TOTP");
    let role: String = sqlx::query_scalar("SELECT role FROM account_members WHERE user_id = $1 AND account_id = $2")
        .bind(user_id)
        .bind(uuid(&s.ctx.account_id))
        .fetch_one(&s.ctx.pool)
        .await
        .expect("member created");
    assert_eq!(role, "Admin");
    assert_eq!(
        count(&s, "SELECT count(*) FROM admin_sso_identities WHERE subject = 'idp-new'").await,
        1,
        "link created"
    );
    assert!(!invitation_open(&s, &token).await, "invitation accepted");
    assert_eq!(count(&s, "SELECT count(*) FROM mfa_recovery_codes WHERE user_id IN (SELECT id FROM users WHERE email = 'newperson@example.com')").await, 0);
    assert_eq!(session_user(&s.ctx, &cookie).await, user_id.to_string());
    let (method, provider): (String, Option<Uuid>) =
        sqlx::query_as("SELECT auth_method, sso_provider_id FROM sessions WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&s.ctx.pool)
            .await
            .unwrap();
    assert_eq!((method.as_str(), provider), ("sso", Some(s.provider)));

    // audit: invite accepted + link created, labels only
    let events: Vec<(String, String)> =
        sqlx::query_as("SELECT event, outcome FROM admin_audit_events WHERE actor_user_id = $1 ORDER BY id")
            .bind(user_id)
            .fetch_all(&s.ctx.pool)
            .await
            .unwrap();
    assert_eq!(
        events,
        vec![("sso_invite_accepted".into(), "ok".into()), ("sso_link_created".into(), "ok".into())]
    );

    // the token is spent: starting again is "unavailable", and the member signs in through the link
    assert!(is_unavailable(start_invite(&s, &token).await).await);
    s.idp.reset();
    s.idp.set(|b| b.sub = "idp-new".into());
    let st = started(start_raw(&s.ctx, json!({ "provider_id": s.provider.to_string() }), None).await).await;
    let again = finish(&s, &st).await;
    assert_eq!(location_of(&again), "/admin/");
    assert_eq!(session_user(&s.ctx, &session_cookie_of(&again).unwrap()).await, user_id.to_string());
}

// ── every failure creates nothing and looks the same ─────────────────────────

#[tokio::test]
async fn every_failed_redemption_creates_nothing_and_leaves_the_invitation_open() {
    let s = setup().await;
    let viewer_email = s.ctx.viewer_email.clone();
    let users_before = users(&s).await;
    let members_before = count(&s, "SELECT count(*) FROM account_members").await;
    let links_before = count(&s, "SELECT count(*) FROM admin_sso_identities").await;

    async fn check(s: &Sso, resp: reqwest::Response, reason: &str, token: Option<&str>, users_before: i64, members_before: i64, links_before: i64) {
        assert_failed(&resp);
        assert_eq!(last_audit(s).await, ("sso_invite_accepted".into(), "failed".into(), Some(reason.into())));
        assert_eq!(users(s).await, users_before, "{reason}: no user");
        assert_eq!(count(s, "SELECT count(*) FROM account_members").await, members_before, "{reason}: no member");
        assert_eq!(count(s, "SELECT count(*) FROM admin_sso_identities").await, links_before, "{reason}: no link");
        assert_eq!(count(s, "SELECT count(*) FROM sessions WHERE auth_method = 'sso'").await, 0, "{reason}: no session");
        assert_eq!(count(s, "SELECT count(*) FROM sso_auth_requests").await, 0, "{reason}: state consumed");
        if let Some(t) = token {
            assert!(invitation_open(s, t).await, "{reason}: invitation still open");
        }
    }
    macro_rules! case {
        ($reason:expr, $token:expr, $resp:expr) => {{
            let t: &str = &$token;
            let r = $resp;
            check(&s, r, $reason, Some(t), users_before, members_before, links_before).await;
        }};
    }

    // unverified e-mail: false, and the string "true" (only the JSON boolean counts)
    let t = sso_invitation(&s, "unverified@example.com", "Viewer").await;
    s.idp.set(|b| b.email_verified = json!(false));
    case!("invite_email_unverified", t, redeem(&s, &t, "sub-a", "unverified@example.com").await);
    s.idp.set(|b| b.email_verified = json!("true"));
    case!("invite_email_unverified", t, redeem(&s, &t, "sub-a", "unverified@example.com").await);
    s.idp.set(|b| b.email_verified = json!(true));

    // e-mail of someone else (an unrelated address)
    let t = sso_invitation(&s, "mismatch@example.com", "Viewer").await;
    case!("invite_email_mismatch", t, redeem(&s, &t, "sub-b", "other@example.com").await);

    // audit #58 shape: the IdP asserts the e-mail of an EXISTING member; that member gets no session
    // and no second link, and nothing is created for the invitee either
    let t = sso_invitation(&s, "victim-target@example.com", "Viewer").await;
    case!("invite_email_mismatch", t, redeem(&s, &t, "sub-c", &viewer_email).await);
    let viewer_sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE user_id = $1")
        .bind(uuid(&s.ctx.viewer_id))
        .fetch_one(&s.ctx.pool)
        .await
        .unwrap();
    assert_eq!(viewer_sessions, 0);

    // no e-mail claim at all
    let t = sso_invitation(&s, "noclaim@example.com", "Viewer").await;
    s.idp.set(|b| {
        b.sub = "sub-d".into();
        b.email = None;
    });
    let st = started(start_invite(&s, &t).await).await;
    case!("invite_email_missing", t, finish(&s, &st).await);

    // the invitation changes between start and callback: revoked, expired, already accepted
    for (reason, change) in [
        ("invitation_unavailable", "revoked_at = now()"),
        ("invitation_unavailable", "expires_at = now() - interval '1 minute'"),
        ("invitation_unavailable", "accepted_at = now()"),
    ] {
        let email = format!("late-{}@example.com", change.split(' ').next().unwrap());
        let t = sso_invitation(&s, &email, "Viewer").await;
        s.idp.set(|b| {
            b.sub = "sub-e".into();
            b.email = Some(email.clone());
        });
        let st = started(start_invite(&s, &t).await).await;
        sqlx::query(&format!("UPDATE invitations SET {change} WHERE email = $1"))
            .bind(&email)
            .execute(&s.ctx.pool)
            .await
            .unwrap();
        let resp = finish(&s, &st).await;
        assert_failed(&resp);
        assert_eq!(last_audit(&s).await, ("sso_invite_accepted".into(), "failed".into(), Some(reason.into())), "{change}");
        assert_eq!(users(&s).await, users_before, "{change}");
        assert_eq!(count(&s, "SELECT count(*) FROM admin_sso_identities").await, links_before, "{change}");
    }

    // an Owner invitation marked sso (cannot be created through the API) is refused at start and,
    // if it was started before the role changed, at the callback
    let t = sso_invitation(&s, "promoted@example.com", "Viewer").await;
    s.idp.set(|b| {
        b.sub = "sub-f".into();
        b.email = Some("promoted@example.com".into());
    });
    let st = started(start_invite(&s, &t).await).await;
    sqlx::query("UPDATE invitations SET role = 'Owner' WHERE email = 'promoted@example.com'")
        .execute(&s.ctx.pool)
        .await
        .unwrap();
    case!("owner_invitation", t, finish(&s, &st).await);
    assert!(is_unavailable(start_invite(&s, &t).await).await, "Owner + sso cannot even start");

    // the invitation address became a user between start and callback (any case): nothing is merged
    let t = sso_invitation(&s, "race@example.com", "Viewer").await;
    s.idp.set(|b| {
        b.sub = "sub-g".into();
        b.email = Some("race@example.com".into());
    });
    let st = started(start_invite(&s, &t).await).await;
    sqlx::query(
        "INSERT INTO users (account_id, email, display_name, password_hash) VALUES ($1, 'RACE@example.com', 'Squatter', 'x')",
    )
    .bind(uuid(&s.ctx.account_id))
    .execute(&s.ctx.pool)
    .await
    .unwrap();
    let resp = finish(&s, &st).await;
    assert_failed(&resp);
    assert_eq!(last_audit(&s).await, ("sso_invite_accepted".into(), "failed".into(), Some("email_taken".into())));
    assert_eq!(users(&s).await, users_before + 1, "only the squatter row");
    assert_eq!(count(&s, "SELECT count(*) FROM admin_sso_identities").await, links_before);
    sqlx::query("DELETE FROM users WHERE email = 'RACE@example.com'").execute(&s.ctx.pool).await.unwrap();

    // the IdP subject is already linked to another user: the link is never moved
    link(&s, &s.ctx.admin_id, "taken-sub").await;
    let t = sso_invitation(&s, "dupe@example.com", "Viewer").await;
    let resp = redeem(&s, &t, "taken-sub", "dupe@example.com").await;
    assert_failed(&resp);
    assert_eq!(last_audit(&s).await, ("sso_invite_accepted".into(), "failed".into(), Some("link_conflict".into())));
    assert_eq!(users(&s).await, users_before, "rolled back with the transaction");
    assert!(invitation_open(&s, &t).await);
    let owner_of_sub: Uuid =
        sqlx::query_scalar("SELECT user_id FROM admin_sso_identities WHERE subject = 'taken-sub'")
            .fetch_one(&s.ctx.pool)
            .await
            .unwrap();
    assert_eq!(owner_of_sub, uuid(&s.ctx.admin_id));
}

// ── scoping: provider and invitation must belong together ────────────────────

#[tokio::test]
async fn start_refuses_foreign_providers_password_invitations_and_garbage() {
    let s = setup().await;
    let cookie = owner_cookie(&s.ctx).await;
    let sso_token = sso_invitation(&s, "sso@example.com", "Viewer").await;
    let (_, pw) = invite(&s.ctx, &cookie, "pw@example.com", "Viewer", None).await;

    // a second account with its own enabled provider (same issuer string is fine across accounts)
    let other_account: Uuid = sqlx::query_scalar("INSERT INTO accounts (name) VALUES ('Other') RETURNING id")
        .fetch_one(&s.ctx.pool)
        .await
        .unwrap();
    let foreign = insert_provider(&s.ctx, other_account, &s.idp.issuer(), &s.idp.client_id).await;

    let go = |provider: String, token: Value| {
        start_raw(&s.ctx, json!({ "provider_id": provider, "intent": "invite", "invite_token": token }), None)
    };
    assert!(is_unavailable(go(foreign.to_string(), json!(sso_token)).await).await, "foreign provider");
    assert!(is_unavailable(go(s.provider.to_string(), pw["invite_token"].clone()).await).await, "password invitation");
    assert!(is_unavailable(go(s.provider.to_string(), json!("not-a-token")).await).await, "malformed token");
    assert!(is_unavailable(go(s.provider.to_string(), Value::Null).await).await, "no token");
    assert!(is_unavailable(go(s.provider.to_string(), json!(embyr_core::admin::invitation::generate_token())).await).await, "unknown token");
    assert_eq!(count(&s, "SELECT count(*) FROM sso_auth_requests").await, 0, "none of them stored a row");
    // and the legitimate pair still works
    assert_eq!(go(s.provider.to_string(), json!(sso_token)).await.status(), 200);
}

// ── AC-SSO-16: an SSO-only member cannot delete the account ──────────────────

#[tokio::test]
async fn an_sso_only_member_cannot_delete_the_account() {
    let s = setup().await;
    let token = sso_invitation(&s, "member@example.com", "Admin").await;
    let resp = redeem(&s, &token, "idp-member", "member@example.com").await;
    let cookie = session_cookie_of(&resp).expect("SSO session");

    let del = s
        .ctx
        .client
        .delete(s.ctx.url("/admin/v1/account"))
        .header("Cookie", &cookie)
        .json(&json!({ "password": "anything", "totp_code": "000000", "confirm_account_name": "Test Account" }))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 403, "SSO-only members are never Owners");
    let deleted: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT deleted_at FROM accounts WHERE id = $1")
            .bind(uuid(&s.ctx.account_id))
            .fetch_one(&s.ctx.pool)
            .await
            .unwrap();
    assert!(deleted.is_none());
}
