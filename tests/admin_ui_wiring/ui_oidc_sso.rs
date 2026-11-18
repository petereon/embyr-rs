//! admin-oidc-sso slice 7 (UI) — SSO sign-in start, invitation lookup, provider management, link/unlink.
//! Pure host tests: `update` + `api::{to_request, decode, *_from_query}`; no browser, no wasm.
//!
//! The JSON constants are the shapes `handlers/{oidc_sso,oidc_sso_links,oidc_providers,invitations,members}.rs`
//! answer with, so a renamed field on either side fails here.
//!
//! bypass: example-based rather than property-based — each case pins one documented status/body
//! contract of the SSO endpoints, which a generator would only restate.

use embyr_admin_ui::api::{
    decode, sso_provider_from_input, sso_provider_from_query, sso_redirect_uri, sso_return_from_query,
    sso_sign_in_link, to_request, ApiCall, CallKind, HttpResponse, Method, SsoIntent,
};
use embyr_admin_ui::model::{
    AppModel, AuthMethod, AuthScreen, CurrentUser, InviteStep, LinkControl, OidcProvider, Role, Section, SsoNotice,
    SsoReturn, ToastLevel, UserId,
};
use embyr_admin_ui::msg::{Effect, Msg};
use embyr_admin_ui::update::update;
use uuid::Uuid;

const PROVIDER: &str = "6f1c1f0e-1d4e-4a53-9a41-0d9a3b7c2e11";
const OTHER_PROVIDER: &str = "0b8a7c1d-52a4-4e0a-8f4e-6c2f9a1d3b55";
const AUTH_URL: &str = "https://idp.example.com/authorize?response_type=code&state=abc";

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
        ..Default::default()
    }
}

fn model_as(role: Role) -> AppModel {
    let mut model = AppModel::default();
    model.authed = true;
    model.current_user = Some(user(role));
    model
}

/// Feed the decoded response of `call` through `update`, returning every effect it asked for.
fn apply(model: &mut AppModel, call: &ApiCall, resp: HttpResponse) -> Vec<Effect> {
    decode(call, resp).into_iter().flat_map(|msg| update(model, msg)).collect()
}

fn start(intent: SsoIntent) -> ApiCall {
    ApiCall::StartSso { provider_id: PROVIDER.into(), intent }
}

fn link_intent() -> SsoIntent {
    SsoIntent::Link { password: "hunter2-hunter2".into(), totp_code: "123456".into() }
}

fn provider(id: &str, enabled: bool) -> OidcProvider {
    OidcProvider { id: id.into(), issuer: "https://idp.example.com".into(), client_id: "client-1".into(), enabled }
}

// ── request construction ─────────────────────────────────────────────────────

#[test]
fn start_posts_provider_and_intent_only_for_sign_in() {
    let req = to_request(&start(SsoIntent::SignIn));
    assert_eq!((req.method, req.path.as_str()), (Method::Post, "/admin/v1/auth/oidc/start"));
    assert_eq!(req.body.unwrap(), serde_json::json!({ "provider_id": PROVIDER, "intent": "signin" }));
}

#[test]
fn start_for_an_invitation_carries_the_token_and_nothing_else() {
    let body = to_request(&start(SsoIntent::Invite { token: "tok-abc".into() })).body.unwrap();
    assert_eq!(
        body,
        serde_json::json!({ "provider_id": PROVIDER, "intent": "invite", "invite_token": "tok-abc" })
    );
}

#[test]
fn start_for_a_link_re_authenticates_with_password_and_totp() {
    let body = to_request(&start(link_intent())).body.unwrap();
    assert_eq!(body["intent"], "link");
    assert_eq!(body["password"], "hunter2-hunter2");
    assert_eq!(body["totp_code"], "123456");
    assert!(body.get("recovery_code").is_none() && body.get("invite_token").is_none());
}

#[test]
fn lookup_posts_the_token_in_the_body_not_the_url() {
    let req = to_request(&ApiCall::LookupInvitation { token: "tok-abc".into() });
    assert_eq!((req.method, req.path.as_str()), (Method::Post, "/admin/v1/auth/invitations/lookup"));
    assert_eq!(req.body.unwrap(), serde_json::json!({ "token": "tok-abc" }));
}

#[test]
fn unlink_is_a_bodyless_delete() {
    let req = to_request(&ApiCall::UnlinkSso);
    assert_eq!((req.method, req.path.as_str(), req.body), (Method::Delete, "/admin/v1/auth/sso_identity", None));
}

#[test]
fn provider_requests_match_the_routes() {
    let create = to_request(&ApiCall::CreateOidcProvider {
        issuer: " https://idp.example.com ".into(),
        client_id: " client-1 ".into(),
        client_secret: "s3cret".into(),
    });
    assert_eq!((create.method, create.path.as_str()), (Method::Post, "/admin/v1/oidc_providers"));
    assert_eq!(
        create.body.unwrap(),
        serde_json::json!({ "issuer": "https://idp.example.com", "client_id": "client-1", "client_secret": "s3cret" })
    );

    let toggle = to_request(&ApiCall::SetOidcProviderEnabled { id: PROVIDER.into(), enabled: false });
    assert_eq!((toggle.method, toggle.path), (Method::Patch, format!("/admin/v1/oidc_providers/{PROVIDER}")));
    assert_eq!(toggle.body.unwrap(), serde_json::json!({ "enabled": false }));

    let rotate = to_request(&ApiCall::RotateOidcSecret { id: PROVIDER.into(), client_secret: "n3w".into() });
    assert_eq!((rotate.method, rotate.path), (Method::Put, format!("/admin/v1/oidc_providers/{PROVIDER}/secret")));
    assert_eq!(rotate.body.unwrap(), serde_json::json!({ "client_secret": "n3w" }));

    let delete = to_request(&ApiCall::DeleteOidcProvider(PROVIDER.into()));
    assert_eq!((delete.method, delete.path), (Method::Delete, format!("/admin/v1/oidc_providers/{PROVIDER}")));
    assert_eq!(to_request(&ApiCall::LoadOidcProviders).path, "/admin/v1/oidc_providers");
}

#[test]
fn only_sso_invitations_send_an_auth_method() {
    let sso = to_request(&ApiCall::InviteSsoMember { email: " new@example.com ".into(), role: Role::Admin });
    assert_eq!(
        sso.body.unwrap(),
        serde_json::json!({ "email": "new@example.com", "role": "Admin", "auth_method": "sso" })
    );
    let password = to_request(&ApiCall::InviteMember { email: "new@example.com".into(), role: Role::Admin });
    assert!(password.body.unwrap().get("auth_method").is_none(), "password stays the server default");
}

#[test]
fn shareable_link_and_redirect_uri_use_the_console_origin() {
    assert_eq!(sso_sign_in_link("https://console.example.com/", PROVIDER), format!("https://console.example.com/admin/?sso={PROVIDER}"));
    assert_eq!(sso_redirect_uri("https://console.example.com"), "https://console.example.com/admin/v1/auth/oidc/callback");
}

// ── secrets stay out of Debug output ─────────────────────────────────────────

#[test]
fn secrets_and_ids_never_appear_in_debug_output() {
    let calls = [
        start(SsoIntent::Invite { token: "tok-abc".into() }),
        start(link_intent()),
        ApiCall::LookupInvitation { token: "tok-abc".into() },
        ApiCall::CreateOidcProvider { issuer: "https://i".into(), client_id: "c".into(), client_secret: "s3cret".into() },
        ApiCall::RotateOidcSecret { id: PROVIDER.into(), client_secret: "s3cret".into() },
    ];
    for call in calls {
        let shown = format!("{:?}", Msg::Request(call));
        for secret in ["tok-abc", "hunter2-hunter2", "123456", "s3cret"] {
            assert!(!shown.contains(secret), "{secret} leaked: {shown}");
        }
    }
    // The `?sso=` provider id is consumed straight into the start call, which does not print it either.
    let shown = format!("{:?}", Msg::Request(start(SsoIntent::SignIn)));
    assert!(!shown.contains(PROVIDER), "{shown}");
}

#[test]
fn an_sso_invitation_keeps_no_token_or_provider_id_in_the_model() {
    let mut model = AppModel { auth_screen: AuthScreen::AcceptInvite, ..AppModel::default() };
    let lookup = ApiCall::LookupInvitation { token: "tok-abc".into() };
    update(&mut model, Msg::Request(lookup.clone()));
    apply(&mut model, &lookup, response(200, &format!(r#"{{"auth_method":"sso","sso_provider_ids":["{PROVIDER}"]}}"#)));
    let shown = format!("{model:?}");
    assert!(!shown.contains(PROVIDER) && !shown.contains("tok-abc"), "{shown}");
}

#[test]
fn creating_a_provider_never_leaves_the_secret_in_the_model() {
    let mut model = model_as(Role::Owner);
    let call = ApiCall::CreateOidcProvider {
        issuer: "https://idp.example.com".into(),
        client_id: "client-1".into(),
        client_secret: "s3cret-value".into(),
    };
    update(&mut model, Msg::Request(call.clone()));
    apply(&mut model, &call, response(201, &format!(r#"{{"id":"{PROVIDER}","issuer":"https://idp.example.com","client_id":"client-1","enabled":false,"created_at":"2026-11-18T10:00:00Z"}}"#)));
    assert!(!format!("{model:?}").contains("s3cret-value"));
}

// ── URL parameters ───────────────────────────────────────────────────────────

#[test]
fn sso_provider_is_read_from_the_query_only_when_it_is_a_uuid() {
    assert_eq!(sso_provider_from_query(&format!("?sso={PROVIDER}")), Some(PROVIDER.into()));
    assert_eq!(sso_provider_from_query(&format!("?x=1&sso={PROVIDER}&y=2")), Some(PROVIDER.into()));
    for none in ["", "?", "?sso=", "?sso=not-a-uuid", "?sso=../../admin", "?ssox=1", "?sso_linked=1", "?other=sso="] {
        assert_eq!(sso_provider_from_query(none), None, "{none:?}");
    }
}

#[test]
fn return_markers_are_recognised_exactly() {
    assert_eq!(sso_return_from_query("?error=sso_failed"), Some(SsoReturn::Failed));
    assert_eq!(sso_return_from_query("?sso_linked=1"), Some(SsoReturn::Linked));
    for none in ["", "?error=oidc_failed", "?error=", "?sso_linked=0", "?sso_linked=", "?sso=x"] {
        assert_eq!(sso_return_from_query(none), None, "{none:?}");
    }
}

#[test]
fn a_pasted_link_or_bare_id_names_the_provider() {
    let link = format!("https://console.example.com/admin/?sso={PROVIDER}");
    assert_eq!(sso_provider_from_input(&link), Some(PROVIDER.into()));
    assert_eq!(sso_provider_from_input(&format!("  {PROVIDER}  ")), Some(PROVIDER.into()));
    assert_eq!(sso_provider_from_input(&format!("{link}#frag")), Some(PROVIDER.into()));
    for none in ["", "hello", "https://console.example.com/admin/", "https://x/?sso=nope"] {
        assert_eq!(sso_provider_from_input(none), None, "{none:?}");
    }
}

// ── sign-in start ────────────────────────────────────────────────────────────

#[test]
fn a_start_answer_redirects_to_the_idp() {
    let mut model = AppModel::default();
    let call = start(SsoIntent::SignIn);
    assert_eq!(update(&mut model, Msg::Request(call.clone())), vec![Effect::Http(call.clone())]);
    assert!(model.pending.contains(&CallKind::SsoStart));

    let effects = apply(&mut model, &call, response(200, &format!(r#"{{"authorization_url":"{AUTH_URL}"}}"#)));

    assert_eq!(effects, vec![Effect::Redirect(AUTH_URL.into())]);
    assert!(model.pending.is_empty() && model.sign_in_error.is_none());
}

#[test]
fn sso_unavailable_is_one_neutral_sentence_on_the_sign_in_screen() {
    let mut model = AppModel::default();
    let call = start(SsoIntent::SignIn);
    let effects = apply(&mut model, &call, response(200, r#"{"error":"sso_unavailable"}"#));
    assert!(effects.is_empty());
    assert!(model.sign_in_error.as_deref().is_some_and(|m| m.contains("not available")), "{:?}", model.sign_in_error);
    assert!(model.toasts.is_empty());
}

#[test]
fn only_https_idp_urls_are_followed() {
    let call = start(SsoIntent::SignIn);
    for url in ["http://evil.example.com/authorize", "javascript:alert(1)", "//evil.example.com"] {
        let mut model = AppModel::default();
        let effects = apply(&mut model, &call, response(200, &format!(r#"{{"authorization_url":"{url}"}}"#)));
        assert!(effects.is_empty(), "{url} must not be followed");
        assert!(model.sign_in_error.is_some());
    }
    let mut model = AppModel::default();
    let local = "http://localhost:9999/authorize?state=x";
    assert_eq!(
        apply(&mut model, &call, response(200, &format!(r#"{{"authorization_url":"{local}"}}"#))),
        vec![Effect::Redirect(local.into())],
        "a local mock IdP is followed for development"
    );
}

#[test]
fn a_throttled_start_shows_the_retry_time() {
    let mut model = AppModel::default();
    let call = start(SsoIntent::SignIn);
    apply(&mut model, &call, HttpResponse { retry_after_ms: Some(60_000), ..response(429, "") });
    assert_eq!(model.sign_in_error.as_deref(), Some("Too many attempts, retry in 60 s"));
}

#[test]
fn a_new_start_clears_the_previous_verdict() {
    let mut model = AppModel::default();
    model.sign_in_error = Some("old".into());
    update(&mut model, Msg::Request(start(SsoIntent::SignIn)));
    assert!(model.sign_in_error.is_none());
}

#[test]
fn the_callbacks_failure_marker_is_a_neutral_message_and_success_toasts() {
    let mut model = AppModel::default();
    update(&mut model, Msg::SsoReturned(SsoReturn::Failed));
    let shown = model.sign_in_error.clone().unwrap();
    assert!(shown.contains("did not complete") && !shown.contains("sso_failed"), "{shown}");

    let mut model = model_as(Role::Viewer);
    update(&mut model, Msg::SsoReturned(SsoReturn::Linked));
    assert_eq!(model.toasts.len(), 1);
    assert_eq!(model.toasts[0].level, ToastLevel::Info);
    assert!(model.toasts[0].message.contains("linked"));
}

// ── invitations ──────────────────────────────────────────────────────────────

fn invite_screen() -> (AppModel, ApiCall) {
    let mut model = AppModel { auth_screen: AuthScreen::AcceptInvite, ..AppModel::default() };
    let lookup = ApiCall::LookupInvitation { token: "tok-abc".into() };
    update(&mut model, Msg::Request(lookup.clone()));
    (model, lookup)
}

#[test]
fn lookup_holds_the_screen_until_it_answers() {
    let (model, _) = invite_screen();
    assert_eq!(model.invite_step, InviteStep::Checking);
    assert!(model.pending.contains(&CallKind::InviteLookup));
}

#[test]
fn a_password_invitation_shows_the_usual_form() {
    let (mut model, lookup) = invite_screen();
    apply(&mut model, &lookup, response(200, r#"{"auth_method":"password"}"#));
    assert_eq!(model.invite_step, InviteStep::Details);
    assert!(model.pending.is_empty());
}

#[test]
fn an_sso_invitation_skips_password_and_authenticator_steps() {
    let (mut model, lookup) = invite_screen();
    let body = format!(r#"{{"auth_method":"sso","sso_provider_ids":["{PROVIDER}","{OTHER_PROVIDER}","not-a-uuid"]}}"#);
    apply(&mut model, &lookup, response(200, &body));
    match &model.invite_step {
        InviteStep::Sso(ids) => assert_eq!(ids.0, vec![PROVIDER.to_string(), OTHER_PROVIDER.to_string()]),
        other => panic!("expected the SSO step, got {other:?}"),
    }
}

#[test]
fn a_late_lookup_answer_does_not_flip_a_screen_the_person_left() {
    let (mut model, lookup) = invite_screen();
    update(&mut model, Msg::ShowAuth(AuthScreen::SignIn));
    apply(&mut model, &lookup, response(200, &format!(r#"{{"auth_method":"sso","sso_provider_ids":["{PROVIDER}"]}}"#)));
    assert_eq!(model.invite_step, InviteStep::Details);
}

#[test]
fn a_dead_invitation_falls_back_to_the_form_with_the_error() {
    let (mut model, lookup) = invite_screen();
    apply(&mut model, &lookup, response(404, ""));
    assert_eq!(model.invite_step, InviteStep::Details);
    assert!(model.reset_error.as_deref().is_some_and(|m| m.contains("invalid or has expired")));
    assert!(model.sign_in_error.is_none());
}

#[test]
fn a_failed_invite_start_shows_on_the_invitation_screen() {
    let (mut model, _) = invite_screen();
    let call = start(SsoIntent::Invite { token: "tok-abc".into() });
    apply(&mut model, &call, response(200, r#"{"error":"sso_unavailable"}"#));
    assert!(model.reset_error.is_some());
    assert!(model.sign_in_error.is_none());
}

// ── provider management ──────────────────────────────────────────────────────

const LIST: &str = r#"[
    {"id":"6f1c1f0e-1d4e-4a53-9a41-0d9a3b7c2e11","issuer":"https://idp.example.com","client_id":"client-1","enabled":true,"created_at":"2026-11-18T10:00:00Z"}
]"#;

#[test]
fn the_provider_list_is_decoded_without_any_secret_field() {
    let mut model = model_as(Role::Owner);
    apply(&mut model, &ApiCall::LoadOidcProviders, response(200, LIST));
    assert_eq!(model.oidc_providers, vec![provider(PROVIDER, true)]);
}

#[test]
fn only_the_owner_loads_and_manages_providers() {
    for (role, expected) in [(Role::Owner, true), (Role::Admin, false), (Role::Viewer, false)] {
        assert_eq!(model_as(role.clone()).can_manage_sso(), expected, "{role:?}");
        for section in [Section::Settings, Section::Identities] {
            let mut model = model_as(role.clone()); // fresh: an in-flight load is not repeated
            let effects = update(&mut model, Msg::NavigateTo(section.clone()));
            let loads = effects.contains(&Effect::Http(ApiCall::LoadOidcProviders));
            assert_eq!(loads, expected, "{role:?} on {section:?}");
        }
    }
}

#[test]
fn a_confirmed_create_bumps_the_save_counter_and_reloads_the_list() {
    let mut model = model_as(Role::Owner);
    let call = ApiCall::CreateOidcProvider { issuer: "https://i".into(), client_id: "c".into(), client_secret: "s".into() };
    let effects = apply(&mut model, &call, response(201, "{}"));
    assert_eq!(model.oidc_provider_saves, 1);
    assert_eq!(effects, vec![Effect::Http(ApiCall::LoadOidcProviders)]);
}

#[test]
fn a_rejected_issuer_names_the_reason() {
    let mut model = model_as(Role::Owner);
    let call = ApiCall::CreateOidcProvider { issuer: "https://i".into(), client_id: "c".into(), client_secret: "s".into() };
    apply(&mut model, &call, response(422, r#"{"error":"issuer_invalid","reason":"issuer_mismatch"}"#));
    assert_eq!(model.oidc_provider_saves, 0, "a failed create must not clear the form");
    let toast = &model.toasts[0];
    assert_eq!(toast.level, ToastLevel::Error);
    assert!(toast.message.contains("issuer_mismatch") && toast.message.contains("rejected"), "{}", toast.message);
}

#[test]
fn credential_and_duplicate_errors_have_their_own_messages() {
    let mut model = model_as(Role::Owner);
    let call = ApiCall::CreateOidcProvider { issuer: "https://i".into(), client_id: "c".into(), client_secret: "s".into() };
    apply(&mut model, &call, response(422, r#"{"error":"invalid_request","reason":"client_secret_invalid"}"#));
    apply(&mut model, &call, response(409, ""));
    apply(&mut model, &call, response(403, ""));
    let messages: Vec<&str> = model.toasts.iter().map(|t| t.message.as_str()).collect();
    assert!(messages[0].contains("client secret"), "{messages:?}");
    assert!(messages[1].contains("already exists"), "{messages:?}");
    assert_eq!(messages[2], "your role does not allow this");
}

#[test]
fn rotate_enable_and_delete_map_a_vanished_provider() {
    for call in [
        ApiCall::RotateOidcSecret { id: PROVIDER.into(), client_secret: "s".into() },
        ApiCall::SetOidcProviderEnabled { id: PROVIDER.into(), enabled: true },
        ApiCall::DeleteOidcProvider(PROVIDER.into()),
    ] {
        let mut model = model_as(Role::Owner);
        apply(&mut model, &call, response(404, ""));
        assert!(model.toasts[0].message.contains("no longer exists"), "{call:?}");
    }
}

#[test]
fn enabling_with_an_unreachable_issuer_explains_why() {
    let mut model = model_as(Role::Owner);
    let call = ApiCall::SetOidcProviderEnabled { id: PROVIDER.into(), enabled: true };
    apply(&mut model, &call, response(422, r#"{"error":"issuer_invalid","reason":"discovery_unreachable"}"#));
    assert!(model.toasts[0].message.contains("discovery_unreachable"));
}

#[test]
fn a_confirmed_rotation_clears_the_form_without_reloading() {
    let mut model = model_as(Role::Owner);
    let call = ApiCall::RotateOidcSecret { id: PROVIDER.into(), client_secret: "s".into() };
    let effects = apply(&mut model, &call, response(204, ""));
    assert!(effects.is_empty());
    assert_eq!(model.oidc_provider_saves, 1);
}

// ── invite dialog ────────────────────────────────────────────────────────────

#[test]
fn sso_invitations_need_an_enabled_provider_for_the_owner() {
    let mut owner = model_as(Role::Owner);
    assert!(!owner.can_invite_sso(), "no provider yet");
    owner.oidc_providers = vec![provider(PROVIDER, false)];
    assert!(!owner.can_invite_sso(), "a disabled provider does not count");
    owner.oidc_providers = vec![provider(PROVIDER, false), provider(OTHER_PROVIDER, true)];
    assert!(owner.can_invite_sso());
}

#[test]
fn admins_leave_the_provider_check_to_the_server_and_viewers_cannot_invite() {
    assert!(model_as(Role::Admin).can_invite_sso());
    assert!(!model_as(Role::Viewer).can_invite_sso());
    assert!(!AppModel::default().can_invite_sso());
}

#[test]
fn an_sso_invitation_reveals_its_link_like_a_password_one() {
    let mut model = model_as(Role::Owner);
    let call = ApiCall::InviteSsoMember { email: "new@example.com".into(), role: Role::Viewer };
    let body = r#"{"invitation_id":"00000000-0000-0000-0000-000000000009","email":"new@example.com","role":"Viewer","auth_method":"sso","expires_at":"2026-11-25T10:00:00Z","invite_token":"tok-xyz"}"#;
    apply(&mut model, &call, response(202, body));
    assert_eq!(model.members.len(), 1);
    assert!(model.members[0].pending);
    assert!(model.revealed_secret.is_some());
}

#[test]
fn a_refused_sso_invitation_says_what_it_needs() {
    let mut model = model_as(Role::Admin);
    let call = ApiCall::InviteSsoMember { email: "new@example.com".into(), role: Role::Viewer };
    apply(&mut model, &call, response(422, ""));
    assert!(model.toasts[0].message.contains("enabled SSO provider"), "{}", model.toasts[0].message);
}

#[test]
fn members_who_sign_in_through_sso_are_flagged() {
    let mut model = model_as(Role::Owner);
    let body = r#"[
        {"id":"00000000-0000-0000-0000-000000000001","email":"owner@example.com","display_name":"O","role":"Owner","auth_method":"password","mfa_enabled":true,"last_login_at":null,"joined_at":null},
        {"id":"00000000-0000-0000-0000-000000000002","email":"sso@example.com","display_name":"S","role":"Viewer","auth_method":"sso","mfa_enabled":false,"last_login_at":null,"joined_at":null},
        {"id":"00000000-0000-0000-0000-000000000003","email":"p@example.com","display_name":null,"role":"Viewer","auth_method":"pending","mfa_enabled":false,"last_login_at":null,"joined_at":null}
    ]"#;
    apply(&mut model, &ApiCall::LoadMembers, response(200, body));
    assert_eq!(model.members.len(), 3);
    assert_eq!(model.sso_members.len(), 1);
    assert!(model.sso_members.contains(&UserId(Uuid::from_u128(2))));
    assert!(model.members.iter().find(|m| m.email == "p@example.com").unwrap().pending);
}

// ── link and unlink ──────────────────────────────────────────────────────────

#[test]
fn a_wrong_password_while_linking_stays_on_the_form_and_keeps_the_session() {
    let mut model = model_as(Role::Viewer);
    let call = start(link_intent());
    update(&mut model, Msg::Request(call.clone()));
    apply(&mut model, &call, response(401, r#"{"message":"Invalid credentials"}"#));
    assert!(model.authed, "must not masquerade as 'session expired'");
    assert_eq!(model.sso_link_error.as_deref(), Some("Invalid credentials"));
    assert!(model.sign_in_error.is_none() && model.toasts.is_empty());

    update(&mut model, Msg::Request(call));
    assert!(model.sso_link_error.is_none(), "a new attempt clears the verdict");
}

#[test]
fn linking_twice_and_sso_only_members_get_specific_messages() {
    let mut model = model_as(Role::Viewer);
    let call = start(link_intent());
    apply(&mut model, &call, response(409, r#"{"error":"already_linked"}"#));
    assert!(model.sso_link_error.as_deref().is_some_and(|m| m.contains("already linked")));
    apply(&mut model, &call, response(403, ""));
    assert!(model.sso_link_error.as_deref().is_some_and(|m| m.contains("password and authenticator")));
}

#[test]
fn unlinking_confirms_with_a_toast_and_explains_the_sso_only_refusal() {
    let mut model = model_as(Role::Viewer);
    apply(&mut model, &ApiCall::UnlinkSso, response(204, ""));
    assert!(model.toasts[0].message.contains("unlinked"));

    apply(&mut model, &ApiCall::UnlinkSso, response(409, ""));
    assert!(model.toasts[1].message.contains("only with SSO"), "{}", model.toasts[1].message);

    apply(&mut model, &ApiCall::UnlinkSso, response(404, ""));
    assert!(model.toasts[2].message.contains("No SSO identity"), "{}", model.toasts[2].message);
    assert!(model.authed);
}

// ── sso_required (slice 7b) ──────────────────────────────────────────────────

fn model_with(role: Role, auth_method: AuthMethod, sso_linked: bool, sso_required: bool) -> AppModel {
    let mut model = model_as(role);
    let u = model.current_user.as_mut().unwrap();
    (u.auth_method, u.sso_linked, u.sso_required) = (auth_method, sso_linked, sso_required);
    model
}

const SESSION_BASE: &str = r#""account_id":"a-1","account_name":"Acme","user_id":"u-1","email":"ada@example.com","display_name":"Ada","role":"Admin""#;

fn session_user(extra: &str) -> CurrentUser {
    let body = format!("{{{SESSION_BASE}{extra}}}");
    match decode(&ApiCall::LoadSession, response(200, &body)).as_slice() {
        [Msg::Settled(CallKind::Session), Msg::SessionEstablished(u)] => u.clone(),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn the_session_probe_reports_how_the_session_started_and_the_sso_policy() {
    let u = session_user(r#","auth_method":"sso","sso_linked":true,"sso_required":true"#);
    assert_eq!((u.auth_method, u.sso_linked, u.sso_required), (AuthMethod::Sso, true, true));
    let u = session_user(r#","auth_method":"password","sso_linked":false,"sso_required":false"#);
    assert_eq!((u.auth_method, u.sso_linked, u.sso_required), (AuthMethod::Password, false, false));
}

#[test]
fn an_older_server_without_the_new_fields_decodes_as_password_unlinked_not_required() {
    let u = session_user("");
    assert_eq!((u.auth_method, u.sso_linked, u.sso_required), (AuthMethod::Password, false, false));
    // An unknown method is never treated as SSO (which would hide the link controls).
    assert_eq!(session_user(r#","auth_method":"saml""#).auth_method, AuthMethod::Password);
}

#[test]
fn requiring_sso_patches_the_account_with_the_flag_only() {
    for on in [true, false] {
        let req = to_request(&ApiCall::SetSsoRequired(on));
        assert_eq!((req.method, req.path.as_str()), (Method::Patch, "/admin/v1/account"));
        assert_eq!(req.body.unwrap(), serde_json::json!({ "sso_required": on }));
    }
    assert_eq!(ApiCall::SetSsoRequired(true).kind(), CallKind::SsoRequired);
}

#[test]
fn a_confirmed_toggle_updates_the_session_flag_and_toasts() {
    let mut model = model_with(Role::Owner, AuthMethod::Password, true, false);
    let on = ApiCall::SetSsoRequired(true);
    update(&mut model, Msg::Request(on.clone()));
    assert!(model.pending.contains(&CallKind::SsoRequired));
    apply(&mut model, &on, response(200, r#"{"sso_required":true}"#));
    assert!(model.sso_required() && model.pending.is_empty());
    assert!(model.toasts[0].message.contains("now required"), "{}", model.toasts[0].message);

    apply(&mut model, &ApiCall::SetSsoRequired(false), response(200, r#"{"sso_required":false}"#));
    assert!(!model.sso_required());
    assert!(model.toasts[1].message.contains("no longer required"), "{}", model.toasts[1].message);
}

#[test]
fn unmet_prerequisites_tell_the_owner_to_link_first_and_change_nothing() {
    let mut model = model_with(Role::Owner, AuthMethod::Password, false, false);
    let call = ApiCall::SetSsoRequired(true);
    apply(&mut model, &call, response(422, r#"{"error":"sso_required_prerequisites"}"#));
    assert!(model.authed && !model.sso_required(), "not a sign-out, flag unchanged");
    let toast = &model.toasts[0];
    assert_eq!(toast.level, ToastLevel::Error);
    assert!(toast.message.contains("link your own SSO identity"), "{}", toast.message);
    assert!(!toast.message.contains("sso_required_prerequisites"));
}

#[test]
fn a_non_owner_is_told_only_the_owner_can_change_it_without_losing_the_session() {
    let mut model = model_with(Role::Admin, AuthMethod::Password, true, false);
    apply(&mut model, &ApiCall::SetSsoRequired(true), response(403, ""));
    assert!(model.authed);
    assert!(model.toasts[0].message.contains("Only the account Owner"), "{}", model.toasts[0].message);
}

#[test]
fn unlinking_clears_the_linked_flag() {
    let mut model = model_with(Role::Viewer, AuthMethod::Password, true, false);
    apply(&mut model, &ApiCall::UnlinkSso, response(204, ""));
    assert!(!model.current_user.unwrap().sso_linked);
}

#[test]
fn link_controls_follow_the_session_fields() {
    use LinkControl::*;
    let cases = [
        // (role, auth_method, linked, required) -> control
        (Role::Viewer, AuthMethod::Password, false, false, Link),
        (Role::Viewer, AuthMethod::Password, true, false, Unlink),
        (Role::Viewer, AuthMethod::Sso, true, false, SsoSession),
        // SSO-only users cannot self-link (O4), even if the link flag lags behind.
        (Role::Viewer, AuthMethod::Sso, false, false, SsoSession),
        (Role::Admin, AuthMethod::Password, true, true, RequiredByAccount),
        (Role::Admin, AuthMethod::Password, false, true, Link),
        // Owners keep both controls while SSO is required.
        (Role::Owner, AuthMethod::Password, true, true, Unlink),
        (Role::Owner, AuthMethod::Password, false, true, Link),
        (Role::Owner, AuthMethod::Sso, true, true, SsoSession),
    ];
    for (role, method, linked, required, expected) in cases {
        let model = model_with(role.clone(), method, linked, required);
        assert_eq!(model.link_control(), Some(expected), "{role:?} {method:?} linked={linked} required={required}");
    }
    assert_eq!(AppModel::default().link_control(), None);
}

#[test]
fn the_banner_depends_on_the_policy_role_and_link() {
    use SsoNotice::*;
    let cases = [
        (Role::Viewer, true, true, Some(MembersUseSso)),
        (Role::Admin, true, true, Some(MembersUseSso)),
        (Role::Admin, false, true, Some(LinkNow)),
        (Role::Owner, true, true, Some(OwnerExempt)),
        (Role::Owner, false, true, Some(OwnerExempt)),
        (Role::Viewer, true, false, None),
        (Role::Owner, true, false, None),
    ];
    for (role, linked, required, expected) in cases {
        let model = model_with(role.clone(), AuthMethod::Password, linked, required);
        assert_eq!(model.sso_notice(), expected, "{role:?} linked={linked} required={required}");
    }
    assert_eq!(AppModel::default().sso_notice(), None);
    assert!(OwnerExempt.message().contains("exempt") && LinkNow.message().contains("Link"));
}

#[test]
fn password_invitations_are_blocked_below_owner_only_while_required() {
    let required = model_with(Role::Owner, AuthMethod::Password, true, true);
    assert!(required.password_invite_allowed(&Role::Owner));
    assert!(!required.password_invite_allowed(&Role::Admin));
    assert!(!required.password_invite_allowed(&Role::Viewer));
    let open = model_with(Role::Owner, AuthMethod::Password, true, false);
    assert!(open.password_invite_allowed(&Role::Admin) && open.password_invite_allowed(&Role::Viewer));
    // Admins see the flag too, so their dialog blocks the same way.
    let admin = model_with(Role::Admin, AuthMethod::Password, true, true);
    assert!(!admin.password_invite_allowed(&Role::Viewer));
}
