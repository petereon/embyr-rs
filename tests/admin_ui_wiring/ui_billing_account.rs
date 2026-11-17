//! admin-ui-api-wiring slices 13-15 (billing, account deletion, invitation accept) — Stripe Checkout /
//! portal redirects, server-owned billing status, Owner-only account deletion, two-step invitation redemption.
//! Pure host tests: `update` + `api::{to_request, decode, *_from_query}`; no browser, no wasm.
//!
//! bypass: example-based rather than property-based — each case pins one documented status/body
//! contract of the billing, account and invitation endpoints, which a generator would only restate.

use embyr_admin_ui::api::{
    checkout_return_from_query, decode, invite_token_from_query, to_request, ApiCall, ApiError, CallKind,
    HttpResponse, Method, SignInCode,
};
use embyr_admin_ui::model::{
    confirms_account_delete, AppModel, AuthScreen, CheckoutReturn, CurrentUser, EffectiveStatus, InviteStep, Plan,
    Role, Section, SubscriptionStatus,
};
use embyr_admin_ui::msg::{Effect, Msg};
use embyr_admin_ui::update::update;

// ── helpers ──────────────────────────────────────────────────────────────────

fn response(status: u16, body: &str) -> HttpResponse {
    HttpResponse { status, retry_after_ms: None, body: body.to_string() }
}

fn user(role: Role) -> CurrentUser {
    CurrentUser {
        user_id: "u-1".into(),
        account_id: "a-1".into(),
        account_name: "Acme".into(),
        email: "ada@example.com".into(),
        display_name: "Ada".into(),
        role,
    }
}

/// A signed-in model (session established, boot loads not executed).
fn signed_in(role: Role) -> AppModel {
    let mut model = AppModel::default();
    update(&mut model, Msg::SessionEstablished(user(role)));
    model.pending.clear();
    model
}

fn apply(model: &mut AppModel, call: &ApiCall, resp: HttpResponse) -> Vec<Effect> {
    let mut effects = Vec::new();
    for msg in decode(call, resp) {
        effects.extend(update(model, msg));
    }
    effects
}

fn subscription_json(plan: &str, status: &str, caps: &str) -> String {
    format!(
        r#"{{"plan":"{plan}","status":"{status}","stripe_customer_id":"cus_1","current_period_end":"2026-12-01T00:00:00Z","card":null{caps}}}"#
    )
}

const CAPS: &str = r#","cap_status":[{"dimension":"reads","used":2100000,"cap":2000000,"pct":105},{"dimension":"writes","used":10,"cap":500000,"pct":0}]"#;

fn toast_texts(model: &AppModel) -> Vec<&str> {
    model.toasts.iter().map(|t| t.message.as_str()).collect()
}

// ── billing: request shapes ──────────────────────────────────────────────────

#[test]
fn billing_requests_use_the_documented_routes() {
    let sub = to_request(&ApiCall::LoadSubscription);
    assert_eq!((sub.method, sub.path.as_str(), sub.body), (Method::Get, "/admin/v1/billing/subscription", None));

    let usage = to_request(&ApiCall::LoadUsage);
    assert_eq!((usage.method, usage.path.as_str()), (Method::Get, "/admin/v1/billing?range=month"));

    let checkout = to_request(&ApiCall::StartCheckout);
    assert_eq!((checkout.method, checkout.path.as_str()), (Method::Post, "/admin/v1/billing/checkout_session"));
    assert_eq!(checkout.body.unwrap(), serde_json::json!({ "plan": "pro" }));

    let portal = to_request(&ApiCall::OpenPortal);
    assert_eq!((portal.method, portal.path.as_str(), portal.body), (Method::Post, "/admin/v1/billing/portal_session", None));
}

#[test]
fn checkout_and_portal_share_one_in_flight_slot() {
    let mut model = signed_in(Role::Owner);
    assert_eq!(update(&mut model, Msg::Request(ApiCall::StartCheckout)), vec![Effect::Http(ApiCall::StartCheckout)]);
    assert!(update(&mut model, Msg::Request(ApiCall::OpenPortal)).is_empty(), "double click must not open two sessions");
}

// ── billing: server-owned status ─────────────────────────────────────────────

#[test]
fn signing_in_loads_the_subscription_for_the_banner_on_every_screen() {
    let mut model = AppModel::default();
    let effects = update(&mut model, Msg::SessionEstablished(user(Role::Owner)));
    assert!(effects.contains(&Effect::Http(ApiCall::LoadSubscription)));
}

#[test]
fn opening_billing_loads_subscription_and_usage_once() {
    let mut model = signed_in(Role::Owner);
    assert_eq!(
        update(&mut model, Msg::NavigateTo(Section::Billing)),
        vec![Effect::Http(ApiCall::LoadSubscription), Effect::Http(ApiCall::LoadUsage)]
    );
    assert!(update(&mut model, Msg::NavigateTo(Section::Billing)).is_empty(), "already in flight");
}

#[test]
fn free_plan_over_its_limit_shows_the_server_status_caps_and_an_upgrade_banner() {
    let mut model = signed_in(Role::Owner);
    apply(&mut model, &ApiCall::LoadSubscription, response(200, &subscription_json("free", "free_cap_exceeded", CAPS)));

    assert_eq!(model.subscription.plan, Plan::Free);
    assert_eq!(model.subscription.stripe_customer_id, "cus_1");
    assert!(model.subscription.current_period_end.is_some());
    assert_eq!(model.billing.status, Some(SubscriptionStatus::FreeCapExceeded));
    let caps = model.billing.cap_status.as_ref().expect("caps");
    assert_eq!((caps[0].dimension.as_str(), caps[0].used, caps[0].cap, caps[0].pct), ("reads", 2_100_000, 2_000_000, 105));
    assert_eq!(model.effective_status(), EffectiveStatus::FreeCapExceeded);
    let banner = model.suspension_banner_view().expect("billing-hold banner");
    assert!(banner.cta_starts_checkout, "a Free-plan limit is resolved by upgrading");
}

#[test]
fn past_due_shows_the_payment_banner_that_leads_to_the_portal() {
    let mut model = signed_in(Role::Owner);
    apply(&mut model, &ApiCall::LoadSubscription, response(200, &subscription_json("pro", "past_due", "")));

    assert_eq!(model.subscription.plan, Plan::Pro);
    assert!(model.billing.cap_status.is_none());
    let banner = model.suspension_banner_view().expect("banner");
    assert!(!banner.cta_starts_checkout);
}

#[test]
fn active_and_canceled_subscriptions_show_no_banner() {
    let mut model = signed_in(Role::Owner);
    for status in ["active", "canceled"] {
        apply(&mut model, &ApiCall::LoadSubscription, response(200, &subscription_json("pro", status, "")));
        assert!(model.suspension_banner_view().is_none(), "{status}");
    }
}

#[test]
fn a_recovered_account_drops_the_banner_on_reload() {
    let mut model = signed_in(Role::Owner);
    apply(&mut model, &ApiCall::LoadSubscription, response(200, &subscription_json("free", "free_cap_exceeded", CAPS)));
    assert!(model.suspension_banner_view().is_some());
    apply(&mut model, &ApiCall::LoadSubscription, response(200, &subscription_json("pro", "active", "")));
    assert!(model.suspension_banner_view().is_none());
}

#[test]
fn only_the_owner_can_manage_billing() {
    for (role, allowed) in [(Role::Owner, true), (Role::Admin, false), (Role::Viewer, false)] {
        assert_eq!(signed_in(role.clone()).can_manage_billing(), allowed, "{role:?}");
    }
}

#[test]
fn unknown_plan_or_status_is_a_decode_error_not_a_guess() {
    let mut model = signed_in(Role::Owner);
    apply(&mut model, &ApiCall::LoadSubscription, response(200, &subscription_json("enterprise", "active", "")));
    apply(&mut model, &ApiCall::LoadSubscription, response(200, &subscription_json("pro", "paused", "")));
    assert_eq!(model.toasts.len(), 2);
    assert!(model.billing.status.is_none());
}

#[test]
fn stripe_not_configured_is_a_screen_state_not_a_toast_and_clears_on_success() {
    let mut model = signed_in(Role::Owner);
    apply(&mut model, &ApiCall::LoadSubscription, response(502, ""));
    assert!(model.billing.unavailable && model.toasts.is_empty() && model.authed);

    apply(&mut model, &ApiCall::LoadSubscription, response(200, &subscription_json("free", "active", "")));
    assert!(!model.billing.unavailable);
}

#[test]
fn month_usage_lists_every_database_with_its_name_or_id() {
    let mut model = signed_in(Role::Viewer);
    let body = r#"{"range":"month","databases":[
        {"id":"shop","name":"Shop","reads":5,"writes":4,"deletes":3,"peak_connections":null,"log_storage_bytes":0},
        {"id":"raw-id","name":null,"reads":0,"writes":0,"deletes":0,"peak_connections":null,"log_storage_bytes":0}],
        "totals":{"reads":5,"writes":4,"deletes":3,"log_storage_bytes":0}}"#;
    apply(&mut model, &ApiCall::LoadUsage, response(200, body));

    let rows = model.usage_table_rows();
    assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Shop", "raw-id"]);
    assert_eq!((rows[0].usage.reads, rows[0].usage.writes, rows[0].usage.deletes), (5, 4, 3));
}

// ── billing: hosted Stripe redirects ─────────────────────────────────────────

#[test]
fn a_created_session_redirects_to_the_stripe_hosted_page() {
    let mut model = signed_in(Role::Owner);
    update(&mut model, Msg::Request(ApiCall::StartCheckout));
    let effects = apply(
        &mut model,
        &ApiCall::StartCheckout,
        response(200, r#"{"url":"https://checkout.stripe.com/c/pay/cs_test_1"}"#),
    );
    assert_eq!(effects, vec![Effect::Redirect("https://checkout.stripe.com/c/pay/cs_test_1".into())]);
    assert!(model.toasts.is_empty());

    let effects = apply(
        &mut model,
        &ApiCall::OpenPortal,
        response(200, r#"{"url":"https://billing.stripe.com/p/session/x"}"#),
    );
    assert_eq!(effects, vec![Effect::Redirect("https://billing.stripe.com/p/session/x".into())]);
}

#[test]
fn a_non_https_link_is_never_followed() {
    let mut model = signed_in(Role::Owner);
    for url in ["http://checkout.stripe.com/x", "javascript:alert(1)", "//evil.example/x", ""] {
        let effects = apply(&mut model, &ApiCall::StartCheckout, response(200, &format!(r#"{{"url":"{url}"}}"#)));
        assert!(effects.is_empty(), "{url:?} must not redirect");
    }
    assert_eq!(model.toasts.len(), 4);
    assert!(model.pending.is_empty());
}

#[test]
fn checkout_and_portal_errors_get_specific_messages() {
    let mut model = signed_in(Role::Owner);
    apply(&mut model, &ApiCall::StartCheckout, response(409, ""));
    apply(&mut model, &ApiCall::OpenPortal, response(409, ""));
    apply(&mut model, &ApiCall::StartCheckout, response(503, ""));
    apply(&mut model, &ApiCall::OpenPortal, response(502, ""));
    apply(&mut model, &ApiCall::StartCheckout, response(403, ""));

    let texts = toast_texts(&model);
    assert!(texts[0].contains("already has an active subscription"), "{texts:?}");
    assert!(texts[1].contains("No billing account"), "{texts:?}");
    assert_eq!(texts[2], "Billing is not configured on this server");
    assert_eq!(texts[3], "an upstream service is unavailable");
    assert_eq!(texts[4], "your role does not allow this");
    assert!(model.authed, "none of these is a session expiry");
}

// ── billing: return from Stripe Checkout ─────────────────────────────────────

#[test]
fn checkout_return_param_is_parsed_strictly() {
    assert_eq!(checkout_return_from_query("?checkout=success"), Some(CheckoutReturn::Success));
    assert_eq!(checkout_return_from_query("?x=1&checkout=cancelled"), Some(CheckoutReturn::Cancelled));
    for none in ["", "?checkout=", "?checkout=ok", "?checkouts=success", "?other=checkout=success"] {
        assert_eq!(checkout_return_from_query(none), None, "{none:?}");
    }
}

#[test]
fn returning_from_checkout_lands_on_billing_and_the_notice_can_be_dismissed() {
    let mut model = AppModel::default();
    update(&mut model, Msg::CheckoutReturned(CheckoutReturn::Success));
    let effects = update(&mut model, Msg::SessionEstablished(user(Role::Owner)));

    assert_eq!(model.nav.section, Section::Billing);
    assert_eq!(model.checkout_return, Some(CheckoutReturn::Success));
    assert!(effects.contains(&Effect::Http(ApiCall::LoadSubscription)));

    update(&mut model, Msg::DismissCheckoutNotice);
    assert_eq!(model.checkout_return, None);
}

#[test]
fn without_a_checkout_return_sign_in_lands_on_the_dashboard() {
    let model = signed_in(Role::Owner);
    assert_eq!(model.nav.section, Section::Dashboard);
}

// ── account deletion ─────────────────────────────────────────────────────────

fn delete(code: SignInCode) -> ApiCall {
    ApiCall::DeleteAccount { password: "correct horse battery".into(), code, confirm_account_name: "Acme".into() }
}

#[test]
fn delete_account_sends_the_confirmation_and_exactly_one_second_factor() {
    let totp = to_request(&delete(SignInCode::Totp("123456".into())));
    assert_eq!((totp.method, totp.path.as_str()), (Method::Delete, "/admin/v1/account"));
    let body = totp.body.unwrap();
    assert_eq!(body["password"], "correct horse battery");
    assert_eq!(body["confirm_account_name"], "Acme");
    assert_eq!(body["totp_code"], "123456");
    assert!(body.get("recovery_code").is_none());

    let recovery = to_request(&delete(SignInCode::Recovery("abcd-efgh".into()))).body.unwrap();
    assert_eq!(recovery["recovery_code"], "abcd-efgh");
    assert!(recovery.get("totp_code").is_none());
}

#[test]
fn delete_account_secrets_never_appear_in_debug_output() {
    let shown = format!("{:?}", Msg::Request(delete(SignInCode::Totp("123456".into()))));
    for secret in ["correct horse battery", "123456"] {
        assert!(!shown.contains(secret), "{secret} leaked: {shown}");
    }
}

#[test]
fn the_typed_confirmation_must_match_the_account_name_exactly() {
    assert!(confirms_account_delete("Acme", "Acme"));
    for wrong in ["acme", "Acme ", " Acme", "", "Acm"] {
        assert!(!confirms_account_delete(wrong, "Acme"), "{wrong:?}");
    }
    assert!(!confirms_account_delete("", ""), "an unknown account name never confirms");
}

#[test]
fn only_the_owner_can_open_the_delete_account_modal() {
    for (role, opens) in [(Role::Owner, true), (Role::Admin, false), (Role::Viewer, false)] {
        let mut model = signed_in(role.clone());
        update(&mut model, Msg::OpenDeleteAccount);
        assert_eq!(model.delete_account_modal_open, opens, "{role:?}");
        assert_eq!(model.can_delete_account(), opens, "{role:?}");
    }
}

#[test]
fn closing_the_modal_forgets_the_previous_error() {
    let mut model = signed_in(Role::Owner);
    update(&mut model, Msg::OpenDeleteAccount);
    apply(&mut model, &delete(SignInCode::Totp("1".into())), response(401, r#"{"message":"Invalid credentials"}"#));
    assert!(model.account_delete_error.is_some());

    update(&mut model, Msg::CloseDeleteAccount);
    assert!(!model.delete_account_modal_open && model.account_delete_error.is_none());
}

#[test]
fn accepted_deletion_returns_to_sign_in_with_the_purge_date() {
    let mut model = signed_in(Role::Owner);
    update(&mut model, Msg::OpenDeleteAccount);
    update(&mut model, Msg::Request(delete(SignInCode::Totp("123456".into()))));

    apply(
        &mut model,
        &delete(SignInCode::Totp("123456".into())),
        response(
            202,
            r#"{"deleted_at":"2026-11-17T10:00:00Z","purge_after":"2026-12-17T10:00:00Z","projects_deleted":2,"subscription":"cancelled"}"#,
        ),
    );

    assert!(!model.authed && model.current_user.is_none() && model.databases.is_empty());
    assert_eq!(model.auth_screen, AuthScreen::SignIn);
    assert!(!model.delete_account_modal_open && model.pending.is_empty());
    let notice = model.auth_notice.as_deref().expect("notice");
    assert!(notice.contains("scheduled for deletion") && notice.contains("2026-12-17"), "{notice}");
}

#[test]
fn a_wrong_password_or_code_keeps_the_modal_open_and_is_not_a_session_expiry() {
    let mut model = signed_in(Role::Owner);
    update(&mut model, Msg::OpenDeleteAccount);
    apply(&mut model, &delete(SignInCode::Totp("0".into())), response(401, r#"{"message":"Invalid credentials"}"#));

    assert!(model.authed && model.delete_account_modal_open);
    assert_eq!(model.account_delete_error.as_deref(), Some("Invalid credentials"));
    assert!(model.sign_in_error.is_none() && model.toasts.is_empty());
}

#[test]
fn deletion_failures_are_reported_inside_the_modal() {
    let mut model = signed_in(Role::Owner);
    let call = delete(SignInCode::Totp("1".into()));

    apply(&mut model, &call, response(400, r#"{"message":"Account name confirmation does not match"}"#));
    assert_eq!(model.account_delete_error.as_deref(), Some("Account name confirmation does not match"));

    apply(&mut model, &call, response(502, r#"{"message":"Could not cancel the subscription — nothing was deleted"}"#));
    assert!(model.account_delete_error.as_deref().is_some_and(|m| m.contains("nothing was deleted")));

    apply(&mut model, &call, response(403, ""));
    assert_eq!(model.account_delete_error.as_deref(), Some("your role does not allow this"));

    apply(&mut model, &call, HttpResponse { retry_after_ms: Some(2300), ..response(429, "") });
    assert_eq!(model.account_delete_error.as_deref(), Some("Too many attempts, retry in 3 s"));

    apply(&mut model, &call, response(429, ""));
    assert!(model.account_delete_error.as_deref().is_some_and(|m| m.contains("locked")));

    assert!(model.authed && model.toasts.is_empty());
}

#[test]
fn a_new_deletion_attempt_clears_the_previous_error() {
    let mut model = signed_in(Role::Owner);
    apply(&mut model, &delete(SignInCode::Totp("1".into())), response(401, r#"{"message":"nope"}"#));
    update(&mut model, Msg::Request(delete(SignInCode::Totp("2".into()))));
    assert!(model.account_delete_error.is_none());
}

// ── invitation accept ────────────────────────────────────────────────────────

const TOKEN: &str = "inv-token_ABC123";
const SECRET: &str = "JBSWY3DPEHPK3PXP";
const ACCEPTED: &str = r#"{"otpauth_uri":"otpauth://totp/embyr:ada%40example.com?secret=JBSWY3DPEHPK3PXP","totp_secret":"JBSWY3DPEHPK3PXP","enrolment_expires_at":"2026-11-17T10:30:00Z"}"#;

fn accept() -> ApiCall {
    ApiCall::AcceptInvitation { token: TOKEN.into(), display_name: "Ada".into(), password: "correct horse battery".into() }
}

fn confirm() -> ApiCall {
    ApiCall::ConfirmInvitation { token: TOKEN.into(), totp_code: "123456".into() }
}

#[test]
fn invite_token_is_read_from_the_query_string() {
    assert_eq!(invite_token_from_query("?invite=abc_-DEF123"), Some("abc_-DEF123".into()));
    assert_eq!(invite_token_from_query("?x=1&invite=tok&y=2"), Some("tok".into()));
    for none in ["", "?", "?invite=", "?invitex=tok", "?reset=tok"] {
        assert_eq!(invite_token_from_query(none), None, "{none:?}");
    }
}

#[test]
fn accept_and_confirm_post_to_the_documented_routes() {
    let req = to_request(&accept());
    assert_eq!((req.method, req.path.as_str()), (Method::Post, "/admin/v1/auth/invitations/accept"));
    assert_eq!(
        req.body.unwrap(),
        serde_json::json!({ "token": TOKEN, "display_name": "Ada", "password": "correct horse battery" })
    );

    let req = to_request(&confirm());
    assert_eq!((req.method, req.path.as_str()), (Method::Post, "/admin/v1/auth/invitations/confirm"));
    assert_eq!(req.body.unwrap(), serde_json::json!({ "token": TOKEN, "totp_code": "123456" }));
}

#[test]
fn token_password_and_code_never_appear_in_debug_output() {
    let shown = format!("{:?} {:?}", Msg::Request(accept()), Msg::Request(confirm()));
    for secret in [TOKEN, "correct horse battery", "123456"] {
        assert!(!shown.contains(secret), "{secret} leaked: {shown}");
    }
}

#[test]
fn the_enrolment_secret_and_recovery_codes_are_held_redacted() {
    let mut model = AppModel { auth_screen: AuthScreen::AcceptInvite, ..AppModel::default() };
    apply(&mut model, &accept(), response(200, ACCEPTED));
    assert!(matches!(model.invite_step, InviteStep::Enrol(ref e) if e.totp_secret == SECRET));

    apply(&mut model, &confirm(), response(200, r#"{"recovery_codes":["aaaa-1111","bbbb-2222"],"email":"ada@example.com"}"#));
    assert!(matches!(model.invite_step, InviteStep::Codes(ref c) if c.0.len() == 2));

    let shown = format!("{model:?}");
    for secret in [SECRET, "aaaa-1111", "bbbb-2222", "otpauth"] {
        assert!(!shown.contains(secret), "{secret} leaked: {shown}");
    }
}

#[test]
fn redeeming_walks_details_then_enrolment_then_codes_then_sign_in() {
    let mut model = AppModel { auth_screen: AuthScreen::AcceptInvite, ..AppModel::default() };
    assert_eq!(model.invite_step, InviteStep::Details);

    update(&mut model, Msg::Request(accept()));
    assert!(model.pending.contains(&CallKind::InviteAccept));
    apply(&mut model, &accept(), response(200, ACCEPTED));
    assert!(matches!(model.invite_step, InviteStep::Enrol(_)) && model.pending.is_empty());

    apply(&mut model, &confirm(), response(200, r#"{"recovery_codes":["aaaa-1111"],"email":"ada@example.com"}"#));
    assert!(matches!(model.invite_step, InviteStep::Codes(_)));

    update(&mut model, Msg::InviteFinished);
    assert_eq!(model.invite_step, InviteStep::Details);
    assert_eq!(model.auth_screen, AuthScreen::SignIn);
    assert!(model.auth_notice.as_deref().is_some_and(|n| n.contains("Sign in")));
    assert!(!model.authed);
}

#[test]
fn leaving_the_screen_drops_the_secret() {
    let mut model = AppModel { auth_screen: AuthScreen::AcceptInvite, ..AppModel::default() };
    apply(&mut model, &accept(), response(200, ACCEPTED));
    update(&mut model, Msg::ShowAuth(AuthScreen::SignIn));
    assert_eq!(model.invite_step, InviteStep::Details);
}

#[test]
fn going_back_from_enrolment_returns_to_the_details_form() {
    let mut model = AppModel { auth_screen: AuthScreen::AcceptInvite, ..AppModel::default() };
    apply(&mut model, &accept(), response(200, ACCEPTED));
    update(&mut model, Msg::ShowAuth(AuthScreen::AcceptInvite));
    assert_eq!((model.auth_screen, model.invite_step.clone()), (AuthScreen::AcceptInvite, InviteStep::Details));
}

#[test]
fn accept_errors_stay_on_the_form_with_specific_messages() {
    let mut model = AppModel { auth_screen: AuthScreen::AcceptInvite, ..AppModel::default() };

    apply(&mut model, &accept(), response(404, r#"{"message":"Invalid or expired invitation"}"#));
    assert!(model.reset_error.as_deref().is_some_and(|m| m.contains("invalid or has expired")));

    apply(&mut model, &accept(), response(422, ""));
    assert!(model.reset_error.as_deref().is_some_and(|m| m.contains("at least 12 characters")));

    apply(&mut model, &accept(), HttpResponse { retry_after_ms: Some(1000), ..response(429, "") });
    assert_eq!(model.reset_error.as_deref(), Some("Too many attempts, retry in 1 s"));

    assert_eq!(model.invite_step, InviteStep::Details);
    assert!(model.toasts.is_empty() && model.sign_in_error.is_none());
}

#[test]
fn a_wrong_authenticator_code_is_not_a_session_expiry() {
    let mut model = AppModel { auth_screen: AuthScreen::AcceptInvite, ..AppModel::default() };
    apply(&mut model, &accept(), response(200, ACCEPTED));
    apply(&mut model, &confirm(), response(401, r#"{"message":"Invalid or expired code"}"#));

    assert_eq!(model.reset_error.as_deref(), Some("Invalid or expired code"));
    assert!(matches!(model.invite_step, InviteStep::Enrol(_)), "stay on the enrolment step to retry");
    assert!(model.sign_in_error.is_none());
}

#[test]
fn an_expired_enrolment_and_a_taken_email_get_clear_messages() {
    let mut model = AppModel { auth_screen: AuthScreen::AcceptInvite, ..AppModel::default() };
    apply(&mut model, &confirm(), response(404, r#"{"message":"Invalid or expired invitation"}"#));
    assert!(model.reset_error.as_deref().is_some_and(|m| m.contains("Go back and set your password again")));

    apply(&mut model, &confirm(), response(409, r#"{"message":"Email already registered"}"#));
    assert_eq!(model.reset_error.as_deref(), Some("Email already registered"));

    update(&mut model, Msg::ApiFailed { call: CallKind::InviteConfirm, error: ApiError::Server(500) });
    assert_eq!(model.reset_error.as_deref(), Some("server error (500)"));
}

#[test]
fn a_new_invite_attempt_clears_the_previous_verdict() {
    let mut model = AppModel { auth_screen: AuthScreen::AcceptInvite, ..AppModel::default() };
    apply(&mut model, &accept(), response(422, ""));
    assert!(model.reset_error.is_some());
    update(&mut model, Msg::Request(accept()));
    assert!(model.reset_error.is_none());
}
