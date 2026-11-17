//! admin-ui-api-wiring slices 03-04 — real sign-in/out, boot session check, project list.
//! AC-AUW-07..18 (ADR-082). Pure host tests: `update` + `api::{to_request, decode}`; no browser, no wasm.
//!
//! bypass: example-based rather than property-based — each case pins one documented status/body
//! contract of the admin API (G-10), which a generator would only restate.

use embyr_admin_ui::api::{
    decode, decode_network_failure, to_request, ApiCall, ApiError, CallKind, HttpRequest,
    HttpResponse, Method, SignInCode,
};
use embyr_admin_ui::model::{
    AppModel, CurrentUser, DbBackendMode, DbId, DbStatus, Role, Section,
};
use embyr_admin_ui::msg::{Effect, Msg};
use embyr_admin_ui::update::update;

// ── helpers ──────────────────────────────────────────────────────────────────

/// What `GET /billing/subscription` answers once a session is established.
const SUBSCRIPTION_JSON: &str = r#"{"plan":"free","status":"active","stripe_customer_id":"cus_1","current_period_end":null,"card":null}"#;

fn response(status: u16, body: &str) -> HttpResponse {
    HttpResponse { status, retry_after_ms: None, body: body.to_string() }
}

fn sign_in(code: SignInCode) -> ApiCall {
    ApiCall::SignIn { email: "ada@example.com".into(), password: "hunter2".into(), code }
}

const SESSION_JSON: &str = r#"{"account_id":"a-1","account_name":"Acme","user_id":"u-1",
    "email":"ada@example.com","display_name":"Ada","role":"Admin"}"#;

const PROJECTS_JSON: &str = r#"[
    {"id":"shop","name":"Shop","status":"active","backend_mode":"direct_pg","logging_enabled":true,
     "created_at":"2026-01-02T03:04:05Z","account_id":"a-1"},
    {"id":"raw-id","name":null,"status":"suspended","backend_mode":"aws_secret","logging_enabled":false,
     "created_at":"2026-02-03T04:05:06.123456Z","account_id":"a-1"}]"#;

fn user() -> CurrentUser {
    CurrentUser {
        user_id: "u-1".into(),
        account_id: "a-1".into(),
        account_name: "Acme".into(),
        email: "ada@example.com".into(),
        display_name: "Ada".into(),
        role: Role::Admin,
    }
}

/// Minimal TEA runtime: feed `msg`, execute effects against `server`, feed decoded results back.
fn drive(model: &mut AppModel, msg: Msg, server: &dyn Fn(&HttpRequest) -> HttpResponse) {
    let mut queue = vec![msg];
    while let Some(msg) = queue.pop() {
        for effect in update(model, msg) {
            let Effect::Http(call) = effect else { continue };
            let mut results = decode(&call, server(&to_request(&call)));
            results.reverse();
            queue.extend(results);
        }
    }
}

fn signed_in_model() -> AppModel {
    let mut model = AppModel::default();
    drive(&mut model, Msg::SessionEstablished(user()), &|req| match req.path.as_str() {
        "/admin/v1/metrics/summary" => response(200, "[]"),
        "/admin/v1/billing/subscription" => response(200, SUBSCRIPTION_JSON),
        _ => response(200, PROJECTS_JSON),
    });
    model
}

fn failed(call: CallKind, error: ApiError) -> Msg {
    Msg::ApiFailed { call, error }
}

// ── request construction (AC-AUW-07, 10) ─────────────────────────────────────

#[test]
fn sign_in_sends_exactly_one_second_factor() {
    let totp = to_request(&sign_in(SignInCode::Totp("123456".into())));
    assert_eq!((totp.method, totp.path.as_str()), (Method::Post, "/admin/v1/auth/signin"));
    let body = totp.body.unwrap();
    assert_eq!(body["email"], "ada@example.com");
    assert_eq!(body["totp_code"], "123456");
    assert!(body.get("recovery_code").is_none());

    let recovery = to_request(&sign_in(SignInCode::Recovery("abcd-efgh".into()))).body.unwrap();
    assert_eq!(recovery["recovery_code"], "abcd-efgh");
    assert!(recovery.get("totp_code").is_none(), "AC-AUW-10: no totp_code with a recovery code");
}

#[test]
fn session_signout_and_projects_use_the_documented_routes() {
    let routes: Vec<_> = [ApiCall::LoadSession, ApiCall::SignOut, ApiCall::LoadProjects]
        .iter()
        .map(|c| to_request(c))
        .map(|r| (r.method, r.path))
        .collect();
    assert_eq!(
        routes,
        vec![
            (Method::Get, "/admin/v1/auth/session".to_string()),
            (Method::Post, "/admin/v1/auth/signout".to_string()),
            (Method::Get, "/admin/v1/projects".to_string()),
        ]
    );
}

#[test]
fn secrets_never_appear_in_debug_output() {
    let shown = format!("{:?}", Msg::Request(sign_in(SignInCode::Totp("123456".into()))));
    assert!(shown.contains("ada@example.com"));
    assert!(!shown.contains("hunter2") && !shown.contains("123456"), "{shown}");
}

// ── response decoding (AC-AUW-07, 08, 09, 15) ────────────────────────────────

#[test]
fn successful_sign_in_chains_to_the_session_check() {
    let msgs = decode(&sign_in(SignInCode::Totp("1".into())), response(200, "{}"));
    assert!(matches!(msgs.as_slice(), [Msg::Settled(CallKind::SignIn), Msg::Request(ApiCall::LoadSession)]));
}

#[test]
fn session_response_becomes_the_current_user() {
    let msgs = decode(&ApiCall::LoadSession, response(200, SESSION_JSON));
    match msgs.as_slice() {
        [Msg::Settled(CallKind::Session), Msg::SessionEstablished(u)] => assert_eq!(*u, user()),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn project_list_maps_names_modes_status_and_dates() {
    let msgs = decode(&ApiCall::LoadProjects, response(200, PROJECTS_JSON));
    let Some(Msg::SetDatabases(dbs)) = msgs.into_iter().nth(1) else { panic!("expected SetDatabases") };
    assert_eq!(dbs.len(), 2);
    assert_eq!((dbs[0].id.clone(), dbs[0].name.as_str()), (DbId("shop".into()), "Shop"));
    assert_eq!(dbs[0].created_at.unwrap().to_rfc3339(), "2026-01-02T03:04:05+00:00");
    assert!(dbs[0].logging_enabled && dbs[0].status == DbStatus::Active);
    assert_eq!(dbs[1].name, "raw-id", "G-07: null name falls back to the id");
    assert_eq!((dbs[1].backend_mode.clone(), dbs[1].status.clone()), (DbBackendMode::AwsSecret, DbStatus::Suspended));
}

#[test]
fn status_codes_map_to_the_documented_errors() {
    let cases: Vec<(HttpResponse, ApiError)> = vec![
        (response(401, r#"{"message":"Invalid credentials"}"#), ApiError::Unauthorized("Invalid credentials".into())),
        (response(401, ""), ApiError::Unauthorized("Unauthorized".into())),
        (response(403, ""), ApiError::Forbidden),
        (response(404, ""), ApiError::NotFound),
        (response(409, r#"{"message":"last Owner"}"#), ApiError::Conflict("last Owner".into())),
        (response(422, r#"{"error":"bad status","detail":"x"}"#), ApiError::Invalid("bad status".into())),
        (response(502, ""), ApiError::Upstream),
        (response(500, ""), ApiError::Server(500)),
        (response(429, ""), ApiError::Throttled { retry_after_ms: None }),
        (
            HttpResponse { retry_after_ms: Some(2300), ..response(429, "{}") },
            ApiError::Throttled { retry_after_ms: Some(2300) },
        ),
    ];
    for (resp, expected) in cases {
        let msgs = decode(&ApiCall::LoadProjects, resp.clone());
        assert!(
            matches!(msgs.as_slice(), [Msg::Settled(_), Msg::ApiFailed { error, .. }] if *error == expected),
            "{resp:?} -> {msgs:?}"
        );
    }
}

#[test]
fn malformed_success_bodies_and_network_failures_are_failures_not_panics() {
    for call in [ApiCall::LoadSession, ApiCall::LoadProjects] {
        let msgs = decode(&call, response(200, "<html>proxy error</html>"));
        assert!(matches!(msgs.as_slice(), [_, Msg::ApiFailed { error: ApiError::Decode(_), .. }]));
    }
    let msgs = decode_network_failure(&ApiCall::LoadSession, "connection refused".into());
    assert!(matches!(msgs.as_slice(), [_, Msg::ApiFailed { error: ApiError::Network(m), .. }] if m == "connection refused"));
}

// ── update: requests and loading state (AC-AUW-16) ───────────────────────────

#[test]
fn a_request_is_pending_until_settled_and_never_issued_twice() {
    let mut model = AppModel::default();
    let call = sign_in(SignInCode::Totp("1".into()));

    let first = update(&mut model, Msg::Request(call.clone()));
    assert_eq!(first, vec![Effect::Http(call.clone())]);
    assert!(model.pending.contains(&CallKind::SignIn));
    assert!(update(&mut model, Msg::Request(call.clone())).is_empty(), "double submit must not re-send");

    update(&mut model, Msg::Settled(CallKind::SignIn));
    assert!(model.pending.is_empty());
    assert_eq!(update(&mut model, Msg::Request(call)).len(), 1);
}

// ── update: sign-in outcomes (AC-AUW-07, 08, 09, 11) ─────────────────────────

#[test]
fn full_sign_in_reaches_the_shell_with_the_users_projects() {
    let server = |req: &HttpRequest| match req.path.as_str() {
        "/admin/v1/auth/signin" => response(200, r#"{"account_id":"a-1","display_name":"Ada","role":"Admin"}"#),
        "/admin/v1/auth/session" => response(200, SESSION_JSON),
        "/admin/v1/projects" => response(200, PROJECTS_JSON),
        "/admin/v1/metrics/summary" => response(200, "[]"),
        "/admin/v1/billing/subscription" => response(200, SUBSCRIPTION_JSON),
        other => panic!("unexpected call {other}"),
    };
    let mut model = AppModel { booting: false, ..AppModel::default() };
    drive(&mut model, Msg::Request(sign_in(SignInCode::Totp("123456".into()))), &server);

    assert!(model.authed);
    assert_eq!(model.current_user, Some(user()));
    assert_eq!(model.nav.section, Section::Dashboard);
    assert_eq!(model.databases.len(), 2);
    assert!(model.pending.is_empty());
}

#[test]
fn rejected_credentials_show_the_servers_message_and_stay_on_sign_in() {
    let mut model = AppModel::default();
    update(&mut model, failed(CallKind::SignIn, ApiError::Unauthorized("Invalid or expired code".into())));
    assert_eq!(model.sign_in_error.as_deref(), Some("Invalid or expired code"));
    assert!(!model.authed && !model.account_locked);
}

#[test]
fn throttling_distinguishes_per_ip_throttle_from_account_lock() {
    let mut model = AppModel::default();
    update(&mut model, failed(CallKind::SignIn, ApiError::Throttled { retry_after_ms: Some(2300) }));
    assert_eq!(model.sign_in_error.as_deref(), Some("Too many attempts, retry in 3 s"));
    assert!(!model.account_locked);

    update(&mut model, failed(CallKind::SignIn, ApiError::Throttled { retry_after_ms: None }));
    assert!(model.account_locked);
    assert!(model.sign_in_error.as_deref().is_some_and(|m| m.contains("locked")));

    update(&mut model, Msg::Request(sign_in(SignInCode::Totp("1".into()))));
    assert!(model.sign_in_error.is_none() && !model.account_locked, "a new attempt clears the old verdict");
}

#[test]
fn rejected_session_cookie_after_sign_in_explains_https_or_localhost() {
    let mut model = AppModel::default();
    update(&mut model, failed(CallKind::Session, ApiError::Unauthorized("Unauthorized".into())));
    assert!(model.sign_in_error.as_deref().is_some_and(|m| m.contains("HTTPS or localhost")));
}

// ── update: boot (AC-AUW-11, 14) ─────────────────────────────────────────────

#[test]
fn boot_restores_a_live_session_without_showing_sign_in() {
    let server = |req: &HttpRequest| match req.path.as_str() {
        "/admin/v1/auth/session" => response(200, SESSION_JSON),
        _ => response(200, "[]"),
    };
    let mut model = AppModel::default();
    let effects = update(&mut model, Msg::Boot);
    assert_eq!(effects, vec![Effect::Http(ApiCall::LoadSession)]);
    assert!(model.booting && !model.authed, "splash while the check is in flight");

    model = AppModel::default();
    drive(&mut model, Msg::Boot, &server);
    assert!(model.authed && !model.booting && !model.server_unreachable);
    assert_eq!(model.current_user.map(|u| u.role), Some(Role::Admin));
}

#[test]
fn boot_without_a_session_shows_sign_in_and_unreachable_server_offers_retry() {
    let mut model = AppModel::default();
    drive(&mut model, Msg::Boot, &|_| response(401, ""));
    assert!(!model.booting && !model.authed && !model.server_unreachable);
    assert!(model.sign_in_error.is_none(), "no 'cookie rejected' hint on a plain boot");

    let mut model = AppModel::default();
    update(&mut model, Msg::Boot);
    for msg in decode_network_failure(&ApiCall::LoadSession, "offline".into()) {
        update(&mut model, msg);
    }
    assert!(model.server_unreachable && !model.booting && !model.authed);

    let retry = update(&mut model, Msg::Boot);
    assert_eq!(retry, vec![Effect::Http(ApiCall::LoadSession)]);
    assert!(model.booting && !model.server_unreachable);
}

// ── update: after sign-in (AC-AUW-12, 13, 17) ────────────────────────────────

#[test]
fn unauthorized_on_any_other_call_drops_account_data_and_shows_session_expired() {
    let mut model = signed_in_model();
    assert!(!model.databases.is_empty());

    update(&mut model, failed(CallKind::Projects, ApiError::Unauthorized("Unauthorized".into())));

    assert!(!model.authed && model.databases.is_empty() && model.current_user.is_none());
    assert!(model.sign_in_error.as_deref().is_some_and(|m| m.contains("expired")));
}

#[test]
fn other_failures_toast_and_keep_loaded_data() {
    let mut model = signed_in_model();
    update(&mut model, failed(CallKind::Projects, ApiError::Server(500)));
    update(&mut model, failed(CallKind::Projects, ApiError::Forbidden));

    assert!(model.authed);
    assert_eq!(model.databases.len(), 2, "previously loaded data stays");
    assert_eq!(model.toasts.len(), 2);
    assert_eq!(model.toasts[0].message, "server error (500)");
    assert_eq!(model.toasts[1].message, "your role does not allow this");
    assert_ne!(model.toasts[0].id, model.toasts[1].id);
}

#[test]
fn sign_out_is_requested_then_confirmed_by_the_server_and_resets_everything() {
    let mut model = signed_in_model();
    assert_eq!(update(&mut model, Msg::Request(ApiCall::SignOut)), vec![Effect::Http(ApiCall::SignOut)]);
    assert!(model.authed, "nothing changes before the server confirms");

    for msg in decode(&ApiCall::SignOut, response(204, "")) {
        update(&mut model, msg);
    }
    assert!(!model.authed && model.databases.is_empty() && model.current_user.is_none());
    assert!(model.pending.is_empty() && model.toasts.is_empty() && model.nav.section == Section::Login);
}

#[test]
fn opening_a_section_loads_its_data() {
    let mut model = signed_in_model();
    assert_eq!(
        update(&mut model, Msg::NavigateTo(Section::Databases)),
        vec![Effect::Http(ApiCall::LoadProjects)]
    );
    assert_eq!(
        update(&mut model, Msg::NavigateTo(Section::Billing)),
        vec![Effect::Http(ApiCall::LoadSubscription), Effect::Http(ApiCall::LoadUsage)]
    );
}

// ── AC-AUW-18: shipped UI code never reaches for mock data ───────────────────

fn rust_sources(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn shipped_ui_code_has_no_mock_references() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![src.join("app.rs")];
    rust_sources(&src.join("views"), &mut files);
    rust_sources(&src.join("components"), &mut files);
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for needle in ["from_mock", "data::mock", "display_stats"] {
            assert!(!text.contains(needle), "{} must not reference {needle}", file.display());
        }
    }
}

/// The whole crate source, not just the views: the fixture constructor, the fixture module, the
/// unreachable billing modals and the OIDC mock are gone and must not come back.
#[test]
fn no_fixture_constructor_dead_modal_or_oidc_mock_remains_in_src() {
    let mut files = Vec::new();
    rust_sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for needle in ["from_mock", "pub mod mock", "CardModal", "UpgradeModal", "OidcProvider", "oidc_providers"] {
            assert!(!text.contains(needle), "{} must not reference {needle}", file.display());
        }
    }
}
