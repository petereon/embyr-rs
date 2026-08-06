// @walking_skeleton @driving_port @real-io @US-B01 @US-B02
//! Walking skeleton — admin-api-v2
//!
//! Proves the two auth stacks coexist on :9090.
//!
//! Journey: sign in → view databases → sign out.
//!
//! This is the only test NOT marked #[ignore]. It must be RED (panic from the scaffold),
//! not BROKEN (compile error or setup failure). DELIVER unskips and implements.
//!
//! Classification target: RED — AdminTestContext::new panics with "Not yet implemented"

#[path = "../common/mod.rs"]
mod common;
use common::AdminTestContext;

/// Walking skeleton: user admin signs in and views their databases, then signs out.
///
/// AC-B01-01, AC-B01-06, AC-B02-01
///
/// Chained journey:
///   Given: fresh Postgres with one account, one user (TOTP enrolled), one Owner member
///   When:  POST /admin/v1/auth/signin {email, password, totp_code}
///   Then:  200 + Set-Cookie: embyr_session (HttpOnly, Secure, SameSite=Strict)
///   ---
///   Given: session cookie from previous step
///   When:  GET /admin/v1/projects
///   Then:  200 + JSON array (account-scoped, excludes deleted)
///   ---
///   Given: session cookie
///   When:  POST /admin/v1/auth/signout
///   Then:  204 + Set-Cookie: embyr_session; Max-Age=0 (cookie cleared)
///
/// @walking_skeleton — litmus: a non-technical stakeholder can confirm
///   "yes, signing in and seeing my databases is what I need to do"
// @walking_skeleton @driving_port @real-io @US-B01 @US-B02 @AC-B01-01 @AC-B01-06 @AC-B02-01
#[tokio::test]
async fn admin_user_signs_in_views_databases_and_signs_out() {
    let ctx = AdminTestContext::new().await;

    // ── Step 1: Sign in ───────────────────────────────────────────────────────
    let signin_body = serde_json::json!({
        "email":     ctx.user_email,
        "password":  ctx.user_password,
        "totp_code": ctx.totp_code_now(),
    });

    let signin_resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&signin_body)
        .send()
        .await
        .expect("POST /admin/v1/auth/signin: request failed");

    assert_eq!(
        signin_resp.status().as_u16(),
        200,
        "AC-B01-01: sign-in with valid credentials must return 200; got {}",
        signin_resp.status()
    );

    let set_cookie = signin_resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    assert!(
        set_cookie.contains("embyr_session="),
        "AC-B01-01: Set-Cookie must include embyr_session; got: {}",
        set_cookie
    );
    assert!(
        set_cookie.contains("HttpOnly"),
        "AC-B01-01: session cookie must be HttpOnly; got: {}",
        set_cookie
    );
    assert!(
        set_cookie.contains("SameSite=Strict"),
        "AC-B01-01: session cookie must be SameSite=Strict; got: {}",
        set_cookie
    );

    // Extract the session cookie value for subsequent requests.
    let session_cookie = set_cookie
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    let signin_json: serde_json::Value = signin_resp
        .json()
        .await
        .expect("AC-B01-01: sign-in response must be JSON");

    assert!(
        signin_json.get("account_id").is_some(),
        "AC-B01-01: response must contain account_id"
    );
    assert!(
        signin_json.get("display_name").is_some(),
        "AC-B01-01: response must contain display_name"
    );
    assert!(
        signin_json.get("role").is_some(),
        "AC-B01-01: response must contain role"
    );

    // ── Step 2: View databases ────────────────────────────────────────────────
    let projects_resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET /admin/v1/projects: request failed");

    assert_eq!(
        projects_resp.status().as_u16(),
        200,
        "AC-B02-01: GET /admin/v1/projects must return 200; got {}",
        projects_resp.status()
    );

    let projects_json: serde_json::Value = projects_resp
        .json()
        .await
        .expect("AC-B02-01: projects response must be JSON");

    assert!(
        projects_json.is_array(),
        "AC-B02-01: response must be a JSON array; got: {}",
        projects_json
    );

    // ── Step 3: Sign out ──────────────────────────────────────────────────────
    let signout_resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signout"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("POST /admin/v1/auth/signout: request failed");

    assert_eq!(
        signout_resp.status().as_u16(),
        204,
        "AC-B01-06: sign-out must return 204; got {}",
        signout_resp.status()
    );

    let signout_cookie = signout_resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    assert!(
        signout_cookie.contains("Max-Age=0"),
        "AC-B01-06: sign-out must clear cookie with Max-Age=0; got: {}",
        signout_cookie
    );
}
