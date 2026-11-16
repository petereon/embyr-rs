//! admin-ui-api-wiring slices 06-08 — database detail on real data: project detail, hourly metrics,
//! connection, API-key rotation (AC-AUW-15, 16, 19, 20, 21, 23, 29, 30; ADR-082/083/084/085).
//! Pure host tests: `update` + `api::{to_request, decode}`; no browser, no wasm.
//!
//! The JSON constants are the response shapes the server serialises (handlers `get_project`,
//! `metrics`, `connection`, `api_key`), so a field renamed on either side fails here.
//!
//! bypass: example-based rather than property-based — each case pins one documented status/body
//! contract of the admin API, which a generator would only restate.

use embyr_admin_ui::api::{
    decode, to_request, ApiCall, ApiError, CallKind, ConnectionConfig, HttpRequest, HttpResponse, Method,
};
use embyr_admin_ui::model::{
    latency_label, AppModel, Connection, ConnectionSummary, CurrentUser, DbBackendMode, DbId, DbStatus,
    LogRetention, MetricsWindow, Role, Section,
};
use embyr_admin_ui::msg::{Effect, Msg};
use embyr_admin_ui::update::update;

// ── helpers ──────────────────────────────────────────────────────────────────

fn response(status: u16, body: &str) -> HttpResponse {
    HttpResponse { status, retry_after_ms: None, body: body.to_string() }
}

fn shop() -> DbId {
    DbId("shop".into())
}

const LIST_JSON: &str = r#"[{"id":"shop","name":"Shop","status":"active","backend_mode":"direct_pg",
    "logging_enabled":false,"created_at":"2026-01-02T03:04:05Z","account_id":"a-1"}]"#;

const DETAIL_JSON: &str = r#"{"id":"shop","name":"Shop","status":"active","backend_mode":"direct_pg",
    "auth_mode":"key","logging_enabled":true,"log_retention_days":14,"created_at":"2026-01-02T03:04:05Z",
    "api_key_rotated_at":"2026-03-01T10:00:00Z","api_key_previous_expires_at":"2026-03-02T10:00:00Z"}"#;

const SUSPENDED_JSON: &str = r#"{"id":"shop","name":"Shop","status":"suspended","backend_mode":"direct_pg",
    "auth_mode":"key","logging_enabled":false,"log_retention_days":null,"created_at":"2026-01-02T03:04:05Z",
    "api_key_rotated_at":null,"api_key_previous_expires_at":null}"#;

const METRICS_JSON: &str = r#"{"reads_today":1200,"writes_today":300,"deletes_today":4,
    "latency":{"read":{"p50_ms":4.0,"p95_ms":11.5,"p99_ms":5000.0,"samples":900,"saturated":true},
               "write":{"p50_ms":null,"p95_ms":null,"p99_ms":null,"samples":0,"saturated":false}},
    "series":[{"hour":"2026-03-01T09:00:00Z","reads":5,"writes":2,"deletes":0,"errors":1},
              {"hour":"2026-03-01T10:00:00Z","reads":7,"writes":0,"deletes":1,"errors":0}],
    "p95_read_ms":11.5,"p95_write_ms":0.0,"sparkline":[{"hour":0,"reads":1,"writes":1}]}"#;

const SUMMARY_JSON: &str = r#"[
    {"project_id":"shop","name":"Shop","reads_today":10,"writes_today":2,"deletes_today":0,"p95_read_ms":8.0,"p95_write_ms":null},
    {"project_id":"blog","name":"blog","reads_today":5,"writes_today":1,"deletes_today":0,"p95_read_ms":12.0,"p95_write_ms":3.0},
    {"project_id":"new","name":"new","reads_today":0,"writes_today":0,"deletes_today":0,"p95_read_ms":null,"p95_write_ms":null}]"#;

const ROTATED_JSON: &str = r#"{"api_key":"embyr_live_SECRET_1234",
    "rotated_at":"2026-03-05T12:00:00Z","previous_key_expires_at":"2026-03-06T12:00:00Z"}"#;

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

/// Minimal TEA runtime: feed `msg`, execute effects against `server`, feed decoded results back.
fn drive(model: &mut AppModel, msg: Msg, server: &dyn Fn(&HttpRequest) -> HttpResponse) {
    let mut queue = vec![msg];
    while let Some(msg) = queue.pop() {
        for Effect::Http(call) in update(model, msg) {
            let mut results = decode(&call, server(&to_request(&call)));
            results.reverse();
            queue.extend(results);
        }
    }
}

/// A signed-in model that has loaded the project list and is looking at `shop`.
fn model_on_shop(role: Role) -> AppModel {
    let mut model = AppModel::default();
    drive(&mut model, Msg::SessionEstablished(user(role)), &|req| match req.path.as_str() {
        "/admin/v1/metrics/summary" => response(200, "[]"),
        _ => response(200, LIST_JSON),
    });
    model.nav.section = Section::DbDetail(shop());
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

// ── request construction ─────────────────────────────────────────────────────

#[test]
fn every_call_uses_its_documented_route_and_body() {
    let id = shop();
    let dsn = ConnectionConfig::DirectPg { dsn: "postgres://u:p@h/db".into() };
    let agent = ConnectionConfig::Agent {
        endpoint: "10.0.0.5:9191".into(),
        ca_pem: "CA".into(),
        client_cert_pem: "CERT".into(),
        client_key_pem: "KEY".into(),
    };
    let cases: Vec<(ApiCall, Method, &str, Option<serde_json::Value>)> = vec![
        (ApiCall::LoadProject(id.clone()), Method::Get, "/admin/v1/projects/shop", None),
        (ApiCall::LoadMetricsSummary, Method::Get, "/admin/v1/metrics/summary", None),
        (
            ApiCall::LoadDbMetrics { id: id.clone(), window: MetricsWindow::Week },
            Method::Get,
            "/admin/v1/projects/shop/metrics?window=7d",
            None,
        ),
        (ApiCall::LoadConnection(id.clone()), Method::Get, "/admin/v1/projects/shop/connection", None),
        (
            ApiCall::PutConnection { id: id.clone(), config: dsn },
            Method::Put,
            "/admin/v1/projects/shop/connection",
            Some(serde_json::json!({"backend_mode":"direct_pg","dsn":"postgres://u:p@h/db"})),
        ),
        (
            ApiCall::PutConnection { id: id.clone(), config: agent },
            Method::Put,
            "/admin/v1/projects/shop/connection",
            Some(serde_json::json!({"backend_mode":"agent","agent_endpoint":"10.0.0.5:9191",
                "agent_ca_pem":"CA","agent_client_cert_pem":"CERT","agent_client_key_pem":"KEY"})),
        ),
        (
            ApiCall::SetSuspended { id: id.clone(), suspended: true },
            Method::Patch,
            "/admin/v1/projects/shop",
            Some(serde_json::json!({"status":"suspended"})),
        ),
        (
            ApiCall::SetSuspended { id: id.clone(), suspended: false },
            Method::Patch,
            "/admin/v1/projects/shop",
            Some(serde_json::json!({"status":"active"})),
        ),
        (
            ApiCall::SetLogging { id: id.clone(), enabled: true, retention: Some(LogRetention::ThirtyDays) },
            Method::Patch,
            "/admin/v1/projects/shop",
            Some(serde_json::json!({"logging_enabled":true,"log_retention_days":30})),
        ),
        (
            ApiCall::SetLogging { id: id.clone(), enabled: false, retention: None },
            Method::Patch,
            "/admin/v1/projects/shop",
            Some(serde_json::json!({"logging_enabled":false})),
        ),
        (ApiCall::RotateApiKey(id.clone()), Method::Post, "/admin/v1/projects/shop/api_key/rotate", None),
        (ApiCall::RevokeApiKey(id), Method::Post, "/admin/v1/projects/shop/api_key/revoke", None),
    ];
    for (call, method, path, body) in cases {
        let request = to_request(&call);
        assert_eq!((request.method, request.path.as_str(), request.body), (method, path, body), "{call:?}");
    }
}

#[test]
fn connection_secrets_never_appear_in_debug_output() {
    let call = ApiCall::PutConnection {
        id: shop(),
        config: ConnectionConfig::DirectPg { dsn: "postgres://u:hunter2@h/db".into() },
    };
    let shown = format!("{:?}", Msg::Request(call));
    assert!(shown.contains("shop") && !shown.contains("hunter2"), "{shown}");
}

// ── response decoding (consumer contract) ────────────────────────────────────

#[test]
fn project_detail_carries_retention_and_key_rotation_metadata() {
    let mut model = model_on_shop(Role::Admin);
    apply(&mut model, &ApiCall::LoadProject(shop()), response(200, DETAIL_JSON));

    let db = &model.databases[0];
    assert!(db.logging_enabled);
    assert_eq!(db.log_retention, Some(LogRetention::Days(14)), "G-27: other values are kept, not coerced");
    assert_eq!(db.log_retention.as_ref().map(LogRetention::label).as_deref(), Some("14 days"));
    let info = &model.api_key_info[&shop()];
    assert_eq!(info.rotated_at.unwrap().to_rfc3339(), "2026-03-01T10:00:00+00:00");
    assert_eq!(info.previous_expires_at.unwrap().to_rfc3339(), "2026-03-02T10:00:00+00:00");
}

#[test]
fn a_list_refresh_keeps_the_retention_the_detail_loaded() {
    let mut model = model_on_shop(Role::Admin);
    apply(&mut model, &ApiCall::LoadProject(shop()), response(200, DETAIL_JSON));
    apply(&mut model, &ApiCall::LoadProjects, response(200, LIST_JSON));
    assert_eq!(model.databases[0].log_retention, Some(LogRetention::Days(14)));
}

#[test]
fn metrics_decode_with_null_percentiles_shown_as_a_dash() {
    let mut model = model_on_shop(Role::Admin);
    let call = ApiCall::LoadDbMetrics { id: shop(), window: MetricsWindow::Day };
    apply(&mut model, &call, response(200, METRICS_JSON));

    let m = &model.db_metrics[&shop()];
    assert_eq!((m.reads_today, m.writes_today, m.deletes_today), (1200, 300, 4));
    assert_eq!(m.series.len(), 2);
    assert_eq!((m.series[0].reads, m.series[0].errors), (5, 1));
    assert_eq!(m.window, MetricsWindow::Day);
    // AC-AUW-30: no samples -> "—", never "0".
    assert_eq!(latency_label(m.write_latency.p95_ms, m.write_latency.saturated), "—");
    assert_eq!(latency_label(m.read_latency.p95_ms, m.read_latency.saturated), "11.5");
    assert_eq!(latency_label(m.read_latency.p50_ms, m.read_latency.saturated), "4");
    assert_eq!(latency_label(m.read_latency.p99_ms, m.read_latency.saturated), ">5000", "overflow bucket is a lower bound");
}

#[test]
fn summary_decodes_per_project_and_averages_only_databases_with_samples() {
    let mut model = model_on_shop(Role::Admin);
    assert!(model.summary_totals().is_none(), "nothing shown as zero before the summary has loaded");
    apply(&mut model, &ApiCall::LoadMetricsSummary, response(200, SUMMARY_JSON));

    assert_eq!(model.metrics_summary.len(), 3);
    assert_eq!(model.metrics_summary[&DbId("new".into())].p95_read_ms, None);
    let totals = model.summary_totals().unwrap();
    assert_eq!((totals.reads_today, totals.writes_today), (15, 3));
    assert_eq!(totals.avg_p95_read_ms, Some(10.0), "(8 + 12) / 2: the empty database does not drag it to 0");
}

#[test]
fn connection_decodes_each_mode_without_credentials() {
    let cases = [
        (
            r#"{"backend_mode":"direct_pg","summary":{"host":"db.prod","port":6432,"database":"app","tls":true},
                "secret_stored":true,"converted":true}"#,
            DbBackendMode::DirectPg,
            ConnectionSummary::Dsn { host: "db.prod".into(), port: 6432, database: Some("app".into()), tls: true },
            "db.prod:6432/app",
        ),
        (
            r#"{"backend_mode":"agent","summary":{"endpoint":"10.0.0.5:9191","cert_expires_at":"2027-01-01T00:00:00+00:00"},
                "secret_stored":true,"converted":true}"#,
            DbBackendMode::AgentMode,
            ConnectionSummary::Agent {
                endpoint: "10.0.0.5:9191".into(),
                cert_expires_at: Some("2027-01-01T00:00:00Z".parse().unwrap()),
            },
            "10.0.0.5:9191 (client certificate expires 2027-01-01)",
        ),
        (
            r#"{"backend_mode":"aws_secret","summary":{"reference":"arn:aws:secretsmanager:x"},"secret_stored":true,"converted":true}"#,
            DbBackendMode::AwsSecret,
            ConnectionSummary::SecretRef("arn:aws:secretsmanager:x".into()),
            "arn:aws:secretsmanager:x",
        ),
        (
            r#"{"backend_mode":"direct_pg","summary":null,"secret_stored":true,"converted":false}"#,
            DbBackendMode::DirectPg,
            ConnectionSummary::NotReadable,
            "stored encrypted",
        ),
    ];
    for (body, mode, summary, text) in cases {
        let mut model = model_on_shop(Role::Admin);
        apply(&mut model, &ApiCall::LoadConnection(shop()), response(200, body));
        assert_eq!(model.connections[&shop()], Connection { mode, summary: summary.clone() });
        assert_eq!(summary.describe(), text);
    }
}

#[test]
fn malformed_bodies_fail_cleanly() {
    for call in [
        ApiCall::LoadProject(shop()),
        ApiCall::LoadMetricsSummary,
        ApiCall::LoadDbMetrics { id: shop(), window: MetricsWindow::Day },
        ApiCall::LoadConnection(shop()),
        ApiCall::RotateApiKey(shop()),
    ] {
        let msgs = decode(&call, response(200, "<html>proxy error</html>"));
        assert!(matches!(msgs.as_slice(), [_, Msg::ApiFailed { error: ApiError::Decode(_), .. }]), "{call:?}");
    }
}

// ── navigation loads (AC-AUW-15, 16) ─────────────────────────────────────────

#[test]
fn opening_the_dashboard_loads_projects_and_the_summary() {
    let mut model = model_on_shop(Role::Admin);
    assert_eq!(
        update(&mut model, Msg::NavigateTo(Section::Dashboard)),
        vec![Effect::Http(ApiCall::LoadProjects), Effect::Http(ApiCall::LoadMetricsSummary)]
    );
    assert!(model.pending.contains(&CallKind::MetricsSummary), "AC-AUW-16: loading state while in flight");
}

#[test]
fn opening_a_database_loads_detail_metrics_and_connection_even_while_another_is_loading() {
    let mut model = model_on_shop(Role::Admin);
    let open = |id: &str| Msg::NavigateTo(Section::DbDetail(DbId(id.into())));
    let expected = |id: &str| {
        vec![
            Effect::Http(ApiCall::LoadProject(DbId(id.into()))),
            Effect::Http(ApiCall::LoadDbMetrics { id: DbId(id.into()), window: MetricsWindow::Day }),
            Effect::Http(ApiCall::LoadConnection(DbId(id.into()))),
        ]
    };
    assert_eq!(update(&mut model, open("shop")), expected("shop"));
    assert_eq!(update(&mut model, open("blog")), expected("blog"), "database B must not wait on A's loads");
}

// ── mutations are confirmed by the server (AC-AUW-19, 21) ────────────────────

#[test]
fn suspend_changes_the_model_only_after_the_server_confirms() {
    let mut model = model_on_shop(Role::Admin);
    let call = ApiCall::SetSuspended { id: shop(), suspended: true };

    assert_eq!(update(&mut model, Msg::Request(call.clone())), vec![Effect::Http(call.clone())]);
    assert_eq!(model.databases[0].status, DbStatus::Active, "no optimistic update");
    assert!(update(&mut model, Msg::Request(call.clone())).is_empty(), "AC-AUW-16: no double submit");

    apply(&mut model, &call, response(200, SUSPENDED_JSON));
    assert_eq!(model.databases[0].status, DbStatus::Suspended);
    assert!(model.pending.is_empty());
}

#[test]
fn rejected_database_changes_leave_the_model_alone_and_explain() {
    let cases = [
        (ApiCall::SetSuspended { id: shop(), suspended: true }, 403, "", "your role does not allow this"),
        (
            ApiCall::SetSuspended { id: shop(), suspended: false },
            409,
            "",
            "cannot activate: the account is on a billing hold (past due or free-plan limit reached)",
        ),
        (ApiCall::SetLogging { id: shop(), enabled: true, retention: None }, 404, "", "not found"),
    ];
    for (call, status, body, message) in cases {
        let mut model = model_on_shop(Role::Admin);
        let before = model.databases.clone();
        apply(&mut model, &call, response(status, body));
        assert_eq!(model.databases, before, "{call:?}");
        assert_eq!(toast_texts(&model), vec![message], "{call:?}");
    }
}

#[test]
fn only_owners_and_admins_may_manage_databases() {
    for (role, allowed) in [(Role::Owner, true), (Role::Admin, true), (Role::Viewer, false)] {
        assert_eq!(model_on_shop(role.clone()).can_manage_databases(), allowed, "{role:?}");
    }
    assert!(!AppModel::default().can_manage_databases());
}

// ── connection edit (AC-AUW-29) ──────────────────────────────────────────────

#[test]
fn saving_a_connection_switches_the_mode_and_reloads_the_summary() {
    let mut model = model_on_shop(Role::Admin);
    let call = ApiCall::PutConnection {
        id: shop(),
        config: ConnectionConfig::Agent {
            endpoint: "10.0.0.5:9191".into(),
            ca_pem: "CA".into(),
            client_cert_pem: "CERT".into(),
            client_key_pem: "KEY".into(),
        },
    };
    let server = |req: &HttpRequest| match (req.method, req.path.as_str()) {
        (Method::Put, "/admin/v1/projects/shop/connection") => response(200, r#"{"backend_mode":"agent","probe":"ok"}"#),
        (Method::Get, "/admin/v1/projects/shop/connection") => response(
            200,
            r#"{"backend_mode":"agent","summary":{"endpoint":"10.0.0.5:9191","cert_expires_at":null},"secret_stored":true,"converted":true}"#,
        ),
        other => panic!("unexpected call {other:?}"),
    };
    drive(&mut model, Msg::Request(call), &server);

    assert_eq!(model.databases[0].backend_mode, DbBackendMode::AgentMode);
    assert_eq!(model.connection_saves, 1, "the view closes its edit form on this signal");
    assert_eq!(
        model.connections[&shop()].summary,
        ConnectionSummary::Agent { endpoint: "10.0.0.5:9191".into(), cert_expires_at: None }
    );
    assert!(model.pending.is_empty());
}

#[test]
fn a_probe_failure_is_shown_verbatim_and_the_summary_is_unchanged() {
    let mut model = model_on_shop(Role::Admin);
    let summary = Connection {
        mode: DbBackendMode::DirectPg,
        summary: ConnectionSummary::Dsn { host: "db.prod".into(), port: 5432, database: None, tls: true },
    };
    update(&mut model, Msg::SetConnection(shop(), summary.clone()));
    let call = ApiCall::PutConnection {
        id: shop(),
        config: ConnectionConfig::DirectPg { dsn: "postgres://u:p@typo/db".into() },
    };

    apply(&mut model, &call, response(400, r#"{"error":"backend_unavailable"}"#));

    assert_eq!(toast_texts(&model), vec!["backend_unavailable"]);
    assert_eq!(model.connections[&shop()], summary);
    assert_eq!(model.databases[0].backend_mode, DbBackendMode::DirectPg);
    assert_eq!(model.connection_saves, 0);
}

// ── reveal-once API key (AC-AUW-20) ──────────────────────────────────────────

#[test]
fn rotate_and_revoke_reveal_the_new_key_and_record_the_rotation() {
    for call in [ApiCall::RotateApiKey(shop()), ApiCall::RevokeApiKey(shop())] {
        let mut model = model_on_shop(Role::Owner);
        drive(&mut model, Msg::Request(call.clone()), &|_| response(200, ROTATED_JSON));

        let revealed = model.revealed_secret.as_ref().expect("the new key is shown");
        assert_eq!((revealed.db.clone(), revealed.secret.as_str()), (shop(), "embyr_live_SECRET_1234"));
        let info = &model.api_key_info[&shop()];
        assert_eq!(info.rotated_at.unwrap().to_rfc3339(), "2026-03-05T12:00:00+00:00");
        assert_eq!(info.previous_expires_at.unwrap().to_rfc3339(), "2026-03-06T12:00:00+00:00");
        assert!(model.pending.is_empty());
    }
}

#[test]
fn the_revealed_key_is_gone_once_the_modal_closes() {
    let mut model = model_on_shop(Role::Owner);
    drive(&mut model, Msg::Request(ApiCall::RotateApiKey(shop())), &|_| response(200, ROTATED_JSON));
    assert!(model.revealed_secret.is_some());

    update(&mut model, Msg::CloseSecret);

    assert!(model.revealed_secret.is_none());
    assert!(!format!("{model:?}").contains("SECRET_1234"), "the key is nowhere in the model");
}

#[test]
fn the_revealed_key_does_not_survive_navigation_or_sign_out_or_session_expiry() {
    let reveal = |model: &mut AppModel| {
        drive(model, Msg::Request(ApiCall::RotateApiKey(shop())), &|_| response(200, ROTATED_JSON));
        assert!(model.revealed_secret.is_some());
    };

    let mut model = model_on_shop(Role::Owner);
    reveal(&mut model);
    update(&mut model, Msg::NavigateTo(Section::Databases));
    assert!(model.revealed_secret.is_none(), "navigation");

    reveal(&mut model);
    update(&mut model, Msg::SignOut);
    assert!(model.revealed_secret.is_none(), "sign-out");

    let mut model = model_on_shop(Role::Owner);
    reveal(&mut model);
    update(&mut model, Msg::ApiFailed { call: CallKind::Projects, error: ApiError::Unauthorized("x".into()) });
    assert!(model.revealed_secret.is_none(), "session expiry");
}

#[test]
fn a_failed_rotation_reveals_nothing() {
    let mut model = model_on_shop(Role::Admin);
    let call = ApiCall::RotateApiKey(shop());
    update(&mut model, Msg::Request(call.clone()));
    assert!(update(&mut model, Msg::Request(ApiCall::RevokeApiKey(shop()))).is_empty(), "one key operation at a time");

    apply(&mut model, &call, response(409, r#"{"error":"connection_secrets_unconverted"}"#));

    assert!(model.revealed_secret.is_none());
    assert_eq!(toast_texts(&model), vec!["connection_secrets_unconverted"]);
    assert!(model.pending.is_empty());
}

#[test]
fn the_secret_is_redacted_wherever_a_message_is_printed() {
    let msgs = decode(&ApiCall::RotateApiKey(shop()), response(200, ROTATED_JSON));
    let shown = format!("{msgs:?}");
    assert!(shown.contains("SecretRevealed") && !shown.contains("SECRET_1234"), "{shown}");
}
