//! admin-ui-api-wiring slice 05 (UI) — create / delete database (AC-AUW-19, 20, 21, 25, 26).
//! Pure host tests: `update` + `api::{to_request, decode}`; no browser, no wasm.
//!
//! The JSON constants are the shapes `handlers/provision.rs` and `handlers/lifecycle.rs` answer
//! with, so a renamed field or error code on either side fails here.
//!
//! bypass: example-based rather than property-based — each case pins one documented status/body
//! contract of the admin API, which a generator would only restate.

use embyr_admin_ui::api::{
    decode, is_valid_project_id, to_request, ApiCall, ApiError, CallKind, ConnectionConfig, HttpRequest,
    HttpResponse, Method, NewProject,
};
use embyr_admin_ui::model::{confirms_delete, AppModel, CurrentUser, DbId, Role, Section};
use embyr_admin_ui::msg::{Effect, Msg};
use embyr_admin_ui::update::update;

// ── helpers ──────────────────────────────────────────────────────────────────

/// What `GET /billing/subscription` answers once a session is established.
const SUBSCRIPTION_JSON: &str = r#"{"plan":"free","status":"active","stripe_customer_id":"cus_1","current_period_end":null,"card":null}"#;

fn response(status: u16, body: &str) -> HttpResponse {
    HttpResponse { status, retry_after_ms: None, body: body.to_string() }
}

fn shop() -> DbId {
    DbId("shop".into())
}

const LIST_JSON: &str = r#"[{"id":"shop","name":"Shop","status":"active","backend_mode":"direct_pg",
    "logging_enabled":false,"created_at":"2026-01-02T03:04:05Z","account_id":"a-1"}]"#;

const LIST_WITH_NEW_JSON: &str = r#"[
    {"id":"shop","name":"Shop","status":"active","backend_mode":"direct_pg",
     "logging_enabled":false,"created_at":"2026-01-02T03:04:05Z"},
    {"id":"blog","name":null,"status":"active","backend_mode":"agent",
     "logging_enabled":false,"created_at":"2026-04-01T00:00:00Z"}]"#;

const PROVISIONED_JSON: &str = r#"{"project_id":"blog","api_key":"KEY_ONCE_abc123"}"#;

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

fn signed_in(role: Role) -> AppModel {
    let mut model = AppModel::default();
    drive(&mut model, Msg::SessionEstablished(user(role)), &|req| match req.path.as_str() {
        "/admin/v1/metrics/summary" => response(200, "[]"),
        "/admin/v1/billing/subscription" => response(200, SUBSCRIPTION_JSON),
        _ => response(200, LIST_JSON),
    });
    model
}

fn apply(model: &mut AppModel, call: &ApiCall, resp: HttpResponse) {
    for msg in decode(call, resp) {
        update(model, msg);
    }
}

fn toast_texts(model: &AppModel) -> Vec<&str> {
    model.toasts.iter().map(|t| t.message.as_str()).collect()
}

fn dsn_project(id: &str, name: &str) -> NewProject {
    NewProject {
        project_id: id.into(),
        name: name.into(),
        config: ConnectionConfig::DirectPg { dsn: "postgres://u:hunter2@h/db".into() },
    }
}

fn agent_project() -> NewProject {
    NewProject {
        project_id: "blog".into(),
        name: String::new(),
        config: ConnectionConfig::Agent {
            endpoint: "10.0.0.5:9191".into(),
            ca_pem: "CA".into(),
            client_cert_pem: "CERT".into(),
            client_key_pem: "KEY".into(),
        },
    }
}

// ── request shapes ───────────────────────────────────────────────────────────

#[test]
fn create_posts_the_provision_body_for_each_mode() {
    let direct = to_request(&ApiCall::CreateProject(dsn_project("shop", " Shop ")));
    assert_eq!(
        (direct.method, direct.path.as_str(), direct.body),
        (
            Method::Post,
            "/admin/v1/projects",
            Some(serde_json::json!({"project_id":"shop","name":"Shop","backend_mode":"direct_pg",
                "dsn":"postgres://u:hunter2@h/db"})),
        )
    );

    let agent = to_request(&ApiCall::CreateProject(agent_project()));
    assert_eq!(
        agent.body,
        Some(serde_json::json!({"project_id":"blog","backend_mode":"agent","agent_endpoint":"10.0.0.5:9191",
            "agent_ca_pem":"CA","agent_client_cert_pem":"CERT","agent_client_key_pem":"KEY"})),
        "a blank name is not sent; the agent bundle uses the provision field names"
    );
}

#[test]
fn delete_is_a_bodyless_delete_on_the_project() {
    let request = to_request(&ApiCall::DeleteProject(shop()));
    assert_eq!((request.method, request.path.as_str(), request.body), (Method::Delete, "/admin/v1/projects/shop", None));
}

#[test]
fn create_secrets_never_appear_in_debug_output() {
    let shown = format!("{:?}", Msg::Request(ApiCall::CreateProject(dsn_project("shop", ""))));
    assert!(shown.contains("shop") && !shown.contains("hunter2"), "{shown}");
}

// ── client-side gates ────────────────────────────────────────────────────────

#[test]
fn project_id_rule_matches_the_servers() {
    for ok in ["a", "shop", "my-shop-2", &"a".repeat(63)] {
        assert!(is_valid_project_id(ok), "{ok}");
    }
    for bad in ["", "Shop", "1shop", "-shop", "my_shop", "shop!", &"a".repeat(64)] {
        assert!(!is_valid_project_id(bad), "{bad}");
    }
}

#[test]
fn create_is_ready_only_with_a_valid_id_and_a_complete_config() {
    assert!(dsn_project("shop", "").is_ready());
    assert!(agent_project().is_ready());
    assert!(!dsn_project("Shop", "").is_ready(), "bad id");
    let blank_dsn = NewProject { config: ConnectionConfig::DirectPg { dsn: "  ".into() }, ..dsn_project("shop", "") };
    assert!(!blank_dsn.is_ready(), "blank DSN");
    let no_key = NewProject {
        config: ConnectionConfig::Agent {
            endpoint: "e".into(),
            ca_pem: "c".into(),
            client_cert_pem: "c".into(),
            client_key_pem: String::new(),
        },
        ..agent_project()
    };
    assert!(!no_key.is_ready(), "agent bundle missing a part");
}

#[test]
fn delete_needs_the_exact_id_typed() {
    assert!(confirms_delete("shop", &shop()));
    assert!(confirms_delete("  shop ", &shop()));
    assert!(!confirms_delete("", &shop()));
    assert!(!confirms_delete("Shop", &shop()));
    assert!(!confirms_delete("sho", &shop()));
}

// ── role gating ──────────────────────────────────────────────────────────────

#[test]
fn owner_and_admin_may_create_but_only_owner_may_delete() {
    for (role, create, delete) in [(Role::Owner, true, true), (Role::Admin, true, false), (Role::Viewer, false, false)] {
        let model = signed_in(role.clone());
        assert_eq!(model.can_manage_databases(), create, "create as {role:?}");
        assert_eq!(model.can_delete_databases(), delete, "delete as {role:?}");
    }
    assert!(!AppModel::default().can_delete_databases(), "signed out");
}

#[test]
fn the_create_modal_opens_for_admins_and_never_for_a_viewer() {
    for (role, opens) in [(Role::Owner, true), (Role::Admin, true), (Role::Viewer, false)] {
        let mut model = signed_in(role.clone());
        update(&mut model, Msg::OpenCreateDb);
        assert_eq!(model.create_db_modal_open, opens, "{role:?}");
    }
    let mut model = signed_in(Role::Admin);
    update(&mut model, Msg::OpenCreateDb);
    update(&mut model, Msg::CloseCreateDb);
    assert!(!model.create_db_modal_open);
}

// ── create outcomes ──────────────────────────────────────────────────────────

#[test]
fn a_successful_create_reveals_the_key_once_closes_the_modal_and_refreshes_the_list() {
    let mut model = signed_in(Role::Admin);
    update(&mut model, Msg::OpenCreateDb);
    let call = ApiCall::CreateProject(agent_project());

    drive(&mut model, Msg::Request(call.clone()), &|req| {
        match (req.method, req.path.as_str()) {
            (Method::Post, "/admin/v1/projects") => response(201, PROVISIONED_JSON),
            _ => response(200, LIST_WITH_NEW_JSON),
        }
    });

    assert!(!model.create_db_modal_open, "the modal closes on success");
    let revealed = model.revealed_secret.as_ref().expect("the key is shown");
    assert_eq!((revealed.db.clone(), revealed.secret.as_str()), (DbId("blog".into()), "KEY_ONCE_abc123"));
    let ids: Vec<_> = model.databases.iter().map(|d| d.id.0.as_str()).collect();
    assert_eq!(ids, ["shop", "blog"], "the list is re-read from the server");
    assert_eq!(model.databases[1].name, "blog", "null name falls back to the id");
    assert!(model.pending.is_empty());

    update(&mut model, Msg::CloseSecret);
    assert!(!format!("{model:?}").contains("KEY_ONCE"), "the key is nowhere in the model after Done");
}

#[test]
fn the_new_key_is_redacted_wherever_a_message_is_printed() {
    let shown = format!("{:?}", decode(&ApiCall::CreateProject(agent_project()), response(201, PROVISIONED_JSON)));
    assert!(shown.contains("SecretRevealed") && !shown.contains("KEY_ONCE"), "{shown}");
}

#[test]
fn every_create_failure_keeps_the_modal_open_reveals_nothing_and_explains() {
    let cases: [(u16, &str, &str); 11] = [
        (403, r#"{"error":"forbidden"}"#, "your role does not allow this"),
        (
            409,
            r#"{"error":"account_billing_hold"}"#,
            "This account is on a billing hold (past due or free-plan limit reached). Resolve it under Billing, then try again.",
        ),
        (409, r#"{"error":"project_already_exists"}"#, "A database with this id already exists"),
        (
            400,
            r#"{"error":"invalid_project_id_format"}"#,
            "Database id must start with a lowercase letter and use only lowercase letters, digits and hyphens (max 63)",
        ),
        (400, r#"{"error":"invalid_name"}"#, "Name must be 1-100 characters"),
        (
            400,
            r#"{"error":"invalid_dsn","detail":"host resolves to a private address"}"#,
            "The DSN was rejected: host resolves to a private address",
        ),
        (
            400,
            r#"{"error":"backend_unavailable"}"#,
            "Could not connect to that Postgres. Check the DSN, network access and TLS.",
        ),
        (
            400,
            r#"{"error":"customer_db_not_prepped","detail":"table 'documents' not found"}"#,
            "The target database is not prepared for Embyr: table 'documents' not found",
        ),
        (
            400,
            r#"{"error":"agent_client_key_pem_required"}"#,
            "Agent endpoint, CA certificate, client certificate and client key are all required",
        ),
        (422, "Failed to deserialize the JSON body", "Invalid request"),
        (429, "", "Too many requests - wait a moment and try again"),
    ];
    for (status, body, shown) in cases {
        let mut model = signed_in(Role::Admin);
        update(&mut model, Msg::OpenCreateDb);
        let call = ApiCall::CreateProject(dsn_project("shop", ""));
        update(&mut model, Msg::Request(call.clone()));
        assert!(model.pending.contains(&CallKind::CreateProject));

        apply(&mut model, &call, response(status, body));

        assert!(model.create_db_modal_open, "{status} {body}: the form stays for a retry");
        assert!(model.revealed_secret.is_none(), "{status}: nothing revealed");
        assert_eq!(toast_texts(&model), vec![shown], "{status} {body}");
        assert!(model.pending.is_empty(), "{status}: the call settled");
        assert_eq!(model.databases.len(), 1, "{status}: the list is unchanged");
    }
}

#[test]
fn an_expired_session_during_create_drops_to_sign_in() {
    let mut model = signed_in(Role::Admin);
    let call = ApiCall::CreateProject(dsn_project("shop", ""));
    update(&mut model, Msg::Request(call.clone()));
    apply(&mut model, &call, response(401, r#"{"message":"expired"}"#));
    assert!(!model.authed && model.databases.is_empty());
}

#[test]
fn a_second_create_is_not_sent_while_one_is_in_flight() {
    let mut model = signed_in(Role::Admin);
    let call = ApiCall::CreateProject(dsn_project("shop", ""));
    assert_eq!(update(&mut model, Msg::Request(call.clone())).len(), 1);
    assert!(update(&mut model, Msg::Request(call)).is_empty());
}

// ── delete outcomes ──────────────────────────────────────────────────────────

#[test]
fn a_successful_delete_removes_the_database_and_leaves_its_detail_page() {
    let mut model = signed_in(Role::Owner);
    model.nav.section = Section::DbDetail(shop());

    drive(&mut model, Msg::Request(ApiCall::DeleteProject(shop())), &|req| match req.method {
        Method::Delete => response(200, ""),
        _ => response(200, "[]"),
    });

    assert!(model.databases.is_empty(), "gone from the model after the 2xx");
    assert_eq!(model.nav.section, Section::Databases, "back on the list");
    assert!(model.pending.is_empty());
    assert!(model.toasts.is_empty());
}

#[test]
fn a_delete_elsewhere_does_not_move_the_user() {
    let mut model = signed_in(Role::Owner);
    model.nav.section = Section::Billing;
    apply(&mut model, &ApiCall::DeleteProject(shop()), response(200, ""));
    assert!(model.databases.is_empty());
    assert_eq!(model.nav.section, Section::Billing);
}

#[test]
fn a_failed_delete_changes_nothing_and_explains() {
    let cases: [(u16, &str, &str); 3] = [
        (403, "", "your role does not allow this"),
        (404, "", "That database no longer exists. Refresh the list."),
        (500, "", "server error (500)"),
    ];
    for (status, body, shown) in cases {
        let mut model = signed_in(Role::Owner);
        model.nav.section = Section::DbDetail(shop());
        let call = ApiCall::DeleteProject(shop());
        update(&mut model, Msg::Request(call.clone()));

        apply(&mut model, &call, response(status, body));

        assert_eq!(model.databases.len(), 1, "{status}: still listed");
        assert_eq!(model.nav.section, Section::DbDetail(shop()), "{status}: still on the page");
        assert_eq!(toast_texts(&model), vec![shown], "{status}");
        assert!(model.pending.is_empty());
    }
}

#[test]
fn the_error_type_distinguishes_forbidden_from_a_missing_database() {
    let forbidden = decode(&ApiCall::DeleteProject(shop()), response(403, ""));
    assert!(matches!(forbidden.last(), Some(Msg::ApiFailed { error: ApiError::Forbidden, .. })));
}
