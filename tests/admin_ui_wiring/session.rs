// @US-AUW-02 @driving_port @real-io
//! admin-ui-api-wiring slice 02 — `GET /admin/v1/auth/session` (AC-AUW-24).
//!
//! The SPA calls this on boot to restore its signed-in shell from the
//! HttpOnly cookie, and again after sign-in to prove the cookie was accepted.

#[path = "../admin_api_v2/common/mod.rs"]
mod common;
use common::AdminTestContext;

async fn sign_in_cookie(ctx: &AdminTestContext, email: &str, password: &str, totp: String) -> String {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({ "email": email, "password": password, "totp_code": totp }))
        .send()
        .await
        .expect("sign-in request failed");
    assert_eq!(resp.status().as_u16(), 200, "sign-in must succeed");
    resp.headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(|s| s.trim().to_string())
        .expect("no Set-Cookie in sign-in response")
}

/// AC-AUW-24: a valid session cookie yields the caller's identity, account and role.
#[tokio::test]
async fn session_returns_caller_identity_and_role() {
    let ctx = AdminTestContext::new().await;
    for (email, password, totp, id, role) in [
        (&ctx.user_email, &ctx.user_password, ctx.totp_code_now(), &ctx.user_id, "Owner"),
        (&ctx.viewer_email, &ctx.viewer_password, ctx.viewer_totp_code_now(), &ctx.viewer_id, "Viewer"),
    ] {
        let cookie = sign_in_cookie(&ctx, email, password, totp).await;
        let resp = ctx
            .client
            .get(ctx.url("/admin/v1/auth/session"))
            .header("Cookie", &cookie)
            .send()
            .await
            .expect("session request failed");
        assert_eq!(resp.status().as_u16(), 200);
        let body: serde_json::Value = resp.json().await.expect("JSON body");
        assert_eq!(body["account_id"], ctx.account_id.as_str());
        assert_eq!(body["account_name"], "Test Account");
        assert_eq!(body["user_id"], id.as_str());
        assert_eq!(body["email"], email.as_str());
        assert_eq!(body["role"], role);
        assert!(body["display_name"].as_str().is_some_and(|d| !d.is_empty()));
    }
}

/// AC-AUW-24: no cookie, an unknown cookie, or an operator Bearer key is 401.
#[tokio::test]
async fn session_without_valid_cookie_is_unauthorized() {
    let ctx = AdminTestContext::new().await;
    let no_cookie = ctx.client.get(ctx.url("/admin/v1/auth/session")).send().await.unwrap();
    assert_eq!(no_cookie.status().as_u16(), 401);
    let bad_cookie = ctx
        .client
        .get(ctx.url("/admin/v1/auth/session"))
        .header("Cookie", "embyr_session=not-a-real-token")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_cookie.status().as_u16(), 401);
    let bearer = ctx
        .client
        .get(ctx.url("/admin/v1/auth/session"))
        .header("Authorization", "Bearer test-admin-key-from-env")
        .send()
        .await
        .unwrap();
    assert_eq!(bearer.status().as_u16(), 401);
}
