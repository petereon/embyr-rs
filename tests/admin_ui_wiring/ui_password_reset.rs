//! Admin password reset screens — request link, `?reset=<token>` confirm. Wires the SPA to
//! `POST /admin/v1/auth/password_reset/{request,confirm}` (admin-password-reset backend).
//! Pure host tests: `update` + `api::{to_request, decode, reset_token_from_query}`; no browser, no wasm.
//!
//! bypass: example-based rather than property-based — each case pins one documented status/body
//! contract of the reset endpoints, which a generator would only restate.

use embyr_admin_ui::api::{
    decode, reset_token_from_query, to_request, ApiCall, ApiError, CallKind, HttpResponse, Method, SignInCode,
};
use embyr_admin_ui::model::{AppModel, AuthScreen};
use embyr_admin_ui::msg::{Effect, Msg};
use embyr_admin_ui::update::update;

fn response(status: u16, body: &str) -> HttpResponse {
    HttpResponse { status, retry_after_ms: None, body: body.to_string() }
}

fn confirm(code: SignInCode) -> ApiCall {
    ApiCall::ConfirmPasswordReset { token: "tok-123".into(), new_password: "correct horse battery".into(), code }
}

fn request() -> ApiCall {
    ApiCall::RequestPasswordReset { email: "ada@example.com".into() }
}

/// Feed the decoded response of `call` through `update`.
fn settle(model: &mut AppModel, call: &ApiCall, resp: HttpResponse) {
    for msg in decode(call, resp) {
        update(model, msg);
    }
}

// ── request construction ─────────────────────────────────────────────────────

#[test]
fn request_posts_only_the_email() {
    let req = to_request(&request());
    assert_eq!((req.method, req.path.as_str()), (Method::Post, "/admin/v1/auth/password_reset/request"));
    assert_eq!(req.body.unwrap(), serde_json::json!({ "email": "ada@example.com" }));
}

#[test]
fn confirm_sends_exactly_one_second_factor() {
    let totp = to_request(&confirm(SignInCode::Totp("123456".into())));
    assert_eq!((totp.method, totp.path.as_str()), (Method::Post, "/admin/v1/auth/password_reset/confirm"));
    let body = totp.body.unwrap();
    assert_eq!(body["token"], "tok-123");
    assert_eq!(body["new_password"], "correct horse battery");
    assert_eq!(body["totp_code"], "123456");
    assert!(body.get("recovery_code").is_none());

    let recovery = to_request(&confirm(SignInCode::Recovery("abcd-efgh".into()))).body.unwrap();
    assert_eq!(recovery["recovery_code"], "abcd-efgh");
    assert!(recovery.get("totp_code").is_none());
}

#[test]
fn confirm_secrets_never_appear_in_debug_output() {
    let shown = format!("{:?}", Msg::Request(confirm(SignInCode::Totp("123456".into()))));
    for secret in ["tok-123", "correct horse battery", "123456"] {
        assert!(!shown.contains(secret), "{secret} leaked: {shown}");
    }
}

#[test]
fn reset_token_is_read_from_the_query_string() {
    assert_eq!(reset_token_from_query("?reset=abc_-DEF123"), Some("abc_-DEF123".into()));
    assert_eq!(reset_token_from_query("?x=1&reset=tok&y=2"), Some("tok".into()));
    for none in ["", "?", "?reset=", "?resetx=tok", "?other=reset=tok"] {
        assert_eq!(reset_token_from_query(none), None, "{none:?}");
    }
}

// ── request outcome ──────────────────────────────────────────────────────────

#[test]
fn accepted_request_shows_a_neutral_notice_and_clears_pending() {
    let mut model = AppModel::default();
    assert_eq!(update(&mut model, Msg::Request(request())), vec![Effect::Http(request())]);
    assert!(model.pending.contains(&CallKind::ResetRequest));

    settle(&mut model, &request(), response(202, r#"{"message":"If that address belongs to an account, a reset link has been sent"}"#));

    assert!(model.pending.is_empty() && model.reset_error.is_none());
    assert!(model.auth_notice.as_deref().is_some_and(|n| n.contains("If that address belongs to an account")));
}

#[test]
fn throttled_request_shows_the_retry_time() {
    let mut model = AppModel::default();
    let throttled = HttpResponse { retry_after_ms: Some(2300), ..response(429, "") };
    settle(&mut model, &request(), throttled);
    assert_eq!(model.reset_error.as_deref(), Some("Too many attempts, retry in 3 s"));
    assert!(model.auth_notice.is_none());
}

// ── confirm outcome ──────────────────────────────────────────────────────────

#[test]
fn successful_confirm_returns_to_sign_in_with_a_notice() {
    let mut model = AppModel { auth_screen: AuthScreen::ResetPassword, ..AppModel::default() };
    update(&mut model, Msg::Request(confirm(SignInCode::Totp("123456".into()))));

    settle(&mut model, &confirm(SignInCode::Totp("123456".into())), response(204, ""));

    assert_eq!(model.auth_screen, AuthScreen::SignIn);
    assert!(model.auth_notice.as_deref().is_some_and(|n| n.contains("Password updated")));
    assert!(model.pending.is_empty() && model.reset_error.is_none() && !model.authed);
}

#[test]
fn wrong_second_factor_stays_on_the_form_and_is_not_a_session_expiry() {
    let mut model = AppModel { auth_screen: AuthScreen::ResetPassword, ..AppModel::default() };
    settle(&mut model, &confirm(SignInCode::Totp("000000".into())), response(401, r#"{"message":"Invalid or expired code"}"#));

    assert_eq!(model.auth_screen, AuthScreen::ResetPassword);
    assert_eq!(model.reset_error.as_deref(), Some("Invalid or expired code"));
    assert!(model.sign_in_error.is_none(), "must not masquerade as 'session expired'");
}

#[test]
fn bad_link_and_short_password_get_specific_messages() {
    let mut model = AppModel::default();
    settle(&mut model, &confirm(SignInCode::Totp("1".into())), response(404, r#"{"message":"Invalid or expired reset link"}"#));
    assert!(model.reset_error.as_deref().is_some_and(|m| m.contains("invalid or has expired")));

    settle(&mut model, &confirm(SignInCode::Totp("1".into())), response(422, ""));
    assert_eq!(model.reset_error.as_deref(), Some("Password must be at least 12 characters"));
}

#[test]
fn locked_account_and_server_errors_are_reported_on_the_form() {
    let mut model = AppModel::default();
    settle(&mut model, &confirm(SignInCode::Totp("1".into())), response(429, ""));
    assert!(model.reset_error.as_deref().is_some_and(|m| m.contains("locked")));

    update(&mut model, Msg::ApiFailed { call: CallKind::ResetConfirm, error: ApiError::Server(500) });
    assert_eq!(model.reset_error.as_deref(), Some("server error (500)"));
    assert!(model.toasts.is_empty());
}

#[test]
fn a_new_attempt_clears_the_previous_verdict() {
    let mut model = AppModel { auth_screen: AuthScreen::ResetPassword, ..AppModel::default() };
    settle(&mut model, &confirm(SignInCode::Totp("1".into())), response(401, r#"{"message":"nope"}"#));
    assert!(model.reset_error.is_some());

    update(&mut model, Msg::Request(confirm(SignInCode::Totp("2".into()))));
    assert!(model.reset_error.is_none());
}

// ── navigation between signed-out screens ────────────────────────────────────

#[test]
fn show_auth_switches_screen_and_drops_stale_messages() {
    let mut model = AppModel::default();
    model.reset_error = Some("old".into());
    model.auth_notice = Some("old".into());

    update(&mut model, Msg::ShowAuth(AuthScreen::ForgotPassword));
    assert_eq!(model.auth_screen, AuthScreen::ForgotPassword);
    assert!(model.reset_error.is_none() && model.auth_notice.is_none());

    update(&mut model, Msg::ShowAuth(AuthScreen::SignIn));
    assert_eq!(model.auth_screen, AuthScreen::SignIn);
}

#[test]
fn sign_out_resets_to_the_sign_in_screen() {
    let mut model = AppModel { auth_screen: AuthScreen::ResetPassword, ..AppModel::default() };
    update(&mut model, Msg::SignOut);
    assert_eq!(model.auth_screen, AuthScreen::SignIn);
}
