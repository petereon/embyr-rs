//! admin-ui-api-wiring slices 09-11 + hosted identity (UI) — query logs, members and invitations,
//! service accounts, admin keys, hosted-identity enablement.
//! Pure host tests: `update` + `api::{to_request, decode}`; no browser, no wasm.
//!
//! The JSON constants are the shapes `handlers/{query_logs,members,invitations,service_accounts,
//! admin_keys,hosted_identity}.rs` answer with, so a renamed field on either side fails here.
//!
//! bypass: example-based rather than property-based — each case pins one documented status/body
//! contract of the admin API, which a generator would only restate.

use embyr_admin_ui::api::{
    decode, invite_link, to_request, ApiCall, ApiError, CallKind, HttpRequest, HttpResponse, Method,
};
use embyr_admin_ui::model::{
    AppModel, CurrentUser, DbId, KeyId, Member, QueryLogs, Role, SecretKind, Section, ServiceAccountId, UserId,
};
use embyr_admin_ui::msg::{Effect, Msg};
use embyr_admin_ui::update::update;
use uuid::Uuid;

// ── helpers ──────────────────────────────────────────────────────────────────

fn response(status: u16, body: &str) -> HttpResponse {
    HttpResponse { status, retry_after_ms: None, body: body.to_string() }
}

fn uid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn shop() -> DbId {
    DbId("shop".into())
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

/// A signed-in model sitting on the given section, with no loads performed.
fn model_as(role: Role, section: Section) -> AppModel {
    let mut model = AppModel::default();
    model.authed = true;
    model.current_user = Some(user(role));
    model.nav.section = section;
    model
}

fn apply(model: &mut AppModel, call: &ApiCall, resp: HttpResponse) {
    for msg in decode(call, resp) {
        update(model, msg);
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

fn toast_texts(model: &AppModel) -> Vec<&str> {
    model.toasts.iter().map(|t| t.message.as_str()).collect()
}

fn calls(effects: Vec<Effect>) -> Vec<ApiCall> {
    effects
        .into_iter()
        .filter_map(|e| match e {
            Effect::Http(call) => Some(call),
            Effect::Redirect(_) => None,
        })
        .collect()
}

fn member(n: u128, email: &str, role: Role, pending: bool) -> Member {
    Member {
        id: UserId(uid(n)),
        email: email.into(),
        display_name: None,
        role,
        pending,
        mfa_enabled: false,
        last_login: None,
    }
}

// ── request shapes ───────────────────────────────────────────────────────────

#[test]
fn every_call_uses_its_documented_route_and_body() {
    let sa = ServiceAccountId(uid(7));
    let cases: Vec<(ApiCall, Method, String, Option<serde_json::Value>)> = vec![
        (
            ApiCall::LoadQueryLogs { id: shop(), op: None, after: None },
            Method::Get,
            "/admin/v1/projects/shop/query_logs".into(),
            None,
        ),
        (
            ApiCall::LoadQueryLogs { id: shop(), op: Some("write".into()), after: None },
            Method::Get,
            "/admin/v1/projects/shop/query_logs?op=write".into(),
            None,
        ),
        (
            ApiCall::LoadQueryLogs { id: shop(), op: Some("read".into()), after: Some("cursor-1".into()) },
            Method::Get,
            "/admin/v1/projects/shop/query_logs?op=read&after=cursor-1".into(),
            None,
        ),
        (
            ApiCall::EnableHostedIdentity(shop()),
            Method::Post,
            "/admin/v1/projects/shop/hosted_identity/enable".into(),
            None,
        ),
        (ApiCall::LoadMembers, Method::Get, "/admin/v1/members".into(), None),
        (
            ApiCall::InviteMember { email: " new@example.com ".into(), role: Role::Admin },
            Method::Post,
            "/admin/v1/members/invite".into(),
            Some(serde_json::json!({"email":"new@example.com","role":"Admin"})),
        ),
        (
            ApiCall::ChangeMemberRole { id: UserId(uid(3)), role: Role::Viewer },
            Method::Patch,
            format!("/admin/v1/members/{}/role", uid(3)),
            Some(serde_json::json!({"role":"Viewer"})),
        ),
        (ApiCall::RemoveMember(UserId(uid(3))), Method::Delete, format!("/admin/v1/members/{}", uid(3)), None),
        (
            ApiCall::RevokeInvitation(UserId(uid(4))),
            Method::Delete,
            format!("/admin/v1/members/invitations/{}", uid(4)),
            None,
        ),
        (ApiCall::LoadServiceAccounts, Method::Get, "/admin/v1/service_accounts".into(), None),
        (
            ApiCall::CreateServiceAccount { name: " ci ".into(), description: Some("  ".into()), role: Role::Viewer },
            Method::Post,
            "/admin/v1/service_accounts".into(),
            Some(serde_json::json!({"name":"ci","role":"Viewer"})),
        ),
        (
            ApiCall::CreateServiceAccount {
                name: "ci".into(),
                description: Some("deploys".into()),
                role: Role::Admin,
            },
            Method::Post,
            "/admin/v1/service_accounts".into(),
            Some(serde_json::json!({"name":"ci","description":"deploys","role":"Admin"})),
        ),
        (
            ApiCall::DeleteServiceAccount(sa.clone()),
            Method::Delete,
            format!("/admin/v1/service_accounts/{}", uid(7)),
            None,
        ),
        (ApiCall::LoadAdminKeys, Method::Get, "/admin/v1/admin_keys".into(), None),
        (
            ApiCall::CreateAdminKey { name: "tf".into(), role: Role::Viewer, service_account_id: None },
            Method::Post,
            "/admin/v1/admin_keys".into(),
            Some(serde_json::json!({"name":"tf","role":"Viewer"})),
        ),
        (
            ApiCall::CreateAdminKey { name: "tf".into(), role: Role::Admin, service_account_id: Some(sa) },
            Method::Post,
            "/admin/v1/admin_keys".into(),
            Some(serde_json::json!({"name":"tf","role":"Admin","service_account_id":uid(7).to_string()})),
        ),
        (ApiCall::RevokeAdminKey(KeyId(uid(9))), Method::Delete, format!("/admin/v1/admin_keys/{}", uid(9)), None),
    ];
    for (call, method, path, body) in cases {
        let request = to_request(&call);
        assert_eq!((request.method, request.path, request.body), (method, path, body), "{call:?}");
    }
}

#[test]
fn the_invite_link_points_at_the_console_origin() {
    assert_eq!(invite_link("https://emb.example", "TOK"), "https://emb.example/admin/?invite=TOK");
    assert_eq!(invite_link("https://emb.example/", "TOK"), "https://emb.example/admin/?invite=TOK");
}

// ── section loads ────────────────────────────────────────────────────────────

#[test]
fn opening_identities_loads_members_and_service_accounts() {
    let mut model = model_as(Role::Owner, Section::Dashboard);
    let loads = calls(update(&mut model, Msg::NavigateTo(Section::Identities)));
    assert_eq!(loads, vec![ApiCall::LoadMembers, ApiCall::LoadServiceAccounts]);
}

#[test]
fn opening_api_keys_loads_admin_keys_and_the_service_accounts_to_bind_them_to() {
    let mut model = model_as(Role::Owner, Section::Dashboard);
    let loads = calls(update(&mut model, Msg::NavigateTo(Section::ApiKeys)));
    assert_eq!(loads, vec![ApiCall::LoadAdminKeys, ApiCall::LoadServiceAccounts]);
}

// ── query logs ───────────────────────────────────────────────────────────────

const LOGS_PAGE_1: &str = r#"{"total":3,"entries":[
  {"id":"e3","op":"read","collection_path":"users/u1","latency_ms":12,"status":"ok","error_code":null,
   "created_at":"2026-03-05T12:00:03Z"},
  {"id":"e2","op":"write","collection_path":null,"latency_ms":null,"status":"error","error_code":"ABORTED",
   "created_at":"2026-03-05T12:00:02Z"}]}"#;

const LOGS_PAGE_2: &str = r#"{"total":3,"entries":[
  {"id":"e1","op":"delete","collection_path":"users/u2","latency_ms":3,"status":"ok","error_code":null,
   "created_at":"2026-03-05T12:00:01Z"}]}"#;

#[test]
fn query_logs_decode_and_a_later_page_appends_with_a_cursor() {
    let mut model = model_as(Role::Viewer, Section::DbDetail(shop()));
    let first = ApiCall::LoadQueryLogs { id: shop(), op: None, after: None };
    update(&mut model, Msg::Request(first.clone()));
    assert!(model.pending.contains(&CallKind::QueryLogs));
    apply(&mut model, &first, response(200, LOGS_PAGE_1));

    let logs = &model.query_logs[&shop()];
    assert_eq!((logs.total, logs.entries.len()), (3, 2));
    assert_eq!(logs.entries[1].error_code.as_deref(), Some("ABORTED"));
    assert_eq!((logs.entries[1].collection_path.clone(), logs.entries[1].latency_ms), (None, None));
    assert_eq!(logs.next_cursor(), Some("e2"), "more rows exist: the cursor is the last loaded id");
    assert!(model.pending.is_empty());

    let more = ApiCall::LoadQueryLogs { id: shop(), op: None, after: Some("e2".into()) };
    apply(&mut model, &more, response(200, LOGS_PAGE_2));
    let logs = &model.query_logs[&shop()];
    assert_eq!(logs.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["e3", "e2", "e1"]);
    assert_eq!(logs.next_cursor(), None, "everything is loaded");
}

#[test]
fn a_fresh_first_page_replaces_what_was_loaded() {
    let mut model = model_as(Role::Viewer, Section::DbDetail(shop()));
    let call = ApiCall::LoadQueryLogs { id: shop(), op: None, after: None };
    apply(&mut model, &call, response(200, LOGS_PAGE_1));
    apply(&mut model, &call, response(200, LOGS_PAGE_2));
    assert_eq!(model.query_logs[&shop()].entries.len(), 1);
}

#[test]
fn logging_off_answers_an_empty_page_without_a_cursor() {
    assert_eq!(QueryLogs::default().next_cursor(), None);
    let mut model = model_as(Role::Viewer, Section::DbDetail(shop()));
    let call = ApiCall::LoadQueryLogs { id: shop(), op: None, after: None };
    apply(&mut model, &call, response(200, r#"{"total":0,"entries":[]}"#));
    assert!(model.query_logs[&shop()].entries.is_empty());
}

#[test]
fn a_failed_log_load_keeps_the_loaded_rows_and_explains() {
    let mut model = model_as(Role::Viewer, Section::DbDetail(shop()));
    let call = ApiCall::LoadQueryLogs { id: shop(), op: None, after: None };
    apply(&mut model, &call, response(200, LOGS_PAGE_1));
    update(&mut model, Msg::Request(call.clone()));
    apply(&mut model, &call, response(500, ""));
    assert_eq!(model.query_logs[&shop()].entries.len(), 2);
    assert_eq!(toast_texts(&model), vec!["server error (500)"]);
    assert!(model.pending.is_empty());

    apply(&mut model, &call, response(404, ""));
    assert_eq!(toast_texts(&model).last(), Some(&"not found"));
}

#[test]
fn a_malformed_log_page_fails_cleanly() {
    let mut model = model_as(Role::Viewer, Section::DbDetail(shop()));
    let call = ApiCall::LoadQueryLogs { id: shop(), op: None, after: None };
    apply(&mut model, &call, response(200, r#"{"total":1,"entries":[{"id":"e1"}]}"#));
    assert!(model.query_logs.is_empty());
    assert_eq!(model.toasts.len(), 1);
}

#[test]
fn deleting_a_database_forgets_its_logs() {
    let mut model = model_as(Role::Owner, Section::Databases);
    let call = ApiCall::LoadQueryLogs { id: shop(), op: None, after: None };
    apply(&mut model, &call, response(200, LOGS_PAGE_1));
    update(&mut model, Msg::DeleteDatabase(shop()));
    assert!(model.query_logs.is_empty());
}

// ── members ──────────────────────────────────────────────────────────────────

const MEMBERS_JSON: &str = r#"[
  {"id":"00000000-0000-0000-0000-000000000001","email":"owner@example.com","display_name":"Olive",
   "role":"Owner","auth_method":"password","mfa_enabled":true,
   "last_login_at":"2026-03-01T10:00:00Z","joined_at":"2026-01-01T00:00:00Z"},
  {"id":"00000000-0000-0000-0000-000000000002","email":"wait@example.com","display_name":null,
   "role":"Viewer","auth_method":"pending","mfa_enabled":false,"last_login_at":null,"joined_at":null}]"#;

#[test]
fn members_decode_with_pending_invitations_marked() {
    let mut model = model_as(Role::Owner, Section::Identities);
    apply(&mut model, &ApiCall::LoadMembers, response(200, MEMBERS_JSON));

    assert_eq!(model.members.len(), 2);
    let owner = &model.members[0];
    assert_eq!((owner.role.clone(), owner.pending, owner.mfa_enabled), (Role::Owner, false, true));
    assert!(owner.last_login.is_some());
    let waiting = &model.members[1];
    assert_eq!((waiting.pending, waiting.display_name.clone(), waiting.last_login), (true, None, None));
}

#[test]
fn an_unknown_role_or_id_in_the_member_list_fails_cleanly() {
    for body in [
        r#"[{"id":"not-a-uuid","email":"a@b.c","display_name":null,"role":"Owner","auth_method":"password","mfa_enabled":false}]"#,
        r#"[{"id":"00000000-0000-0000-0000-000000000001","email":"a@b.c","display_name":null,"role":"God","auth_method":"password","mfa_enabled":false}]"#,
    ] {
        let mut model = model_as(Role::Owner, Section::Identities);
        apply(&mut model, &ApiCall::LoadMembers, response(200, body));
        assert!(model.members.is_empty());
        assert_eq!(model.toasts.len(), 1);
    }
}

const INVITED_JSON: &str = r#"{"invitation_id":"00000000-0000-0000-0000-0000000000aa","email":"new@example.com",
  "role":"Admin","expires_at":"2026-03-12T00:00:00Z","invite_token":"TOKEN_ONCE_xyz"}"#;

#[test]
fn an_invitation_adds_a_pending_row_and_reveals_the_token_once() {
    let mut model = model_as(Role::Owner, Section::Identities);
    let call = ApiCall::InviteMember { email: "new@example.com".into(), role: Role::Admin };
    update(&mut model, Msg::Request(call.clone()));
    assert!(model.pending.contains(&CallKind::MemberInvite));
    assert!(model.members.is_empty(), "nothing before the server confirms");

    apply(&mut model, &call, response(202, INVITED_JSON));

    assert_eq!(model.members.len(), 1);
    let row = &model.members[0];
    assert_eq!((row.id.clone(), row.pending, row.role.clone()), (UserId(uid(0xaa)), true, Role::Admin));
    let revealed = model.revealed_secret.as_ref().expect("the link is shown");
    assert_eq!(
        (revealed.kind, revealed.label.as_str(), revealed.secret.as_str()),
        (SecretKind::InviteToken, "new@example.com", "TOKEN_ONCE_xyz")
    );
    assert!(!format!("{revealed:?}").contains("TOKEN_ONCE_xyz"), "redacted in Debug");

    update(&mut model, Msg::CloseSecret);
    assert!(model.revealed_secret.is_none());
}

#[test]
fn a_rejected_invitation_adds_nothing_and_explains() {
    let cases: [(u16, &str, &str); 4] = [
        (409, "", "That email already belongs to a member or has an open invitation"),
        (422, "", "Enter a valid email address and choose a role"),
        (403, "", "You cannot invite someone to a role above your own"),
        (500, "", "server error (500)"),
    ];
    for (status, body, shown) in cases {
        let mut model = model_as(Role::Admin, Section::Identities);
        let call = ApiCall::InviteMember { email: "x@example.com".into(), role: Role::Owner };
        update(&mut model, Msg::Request(call.clone()));
        apply(&mut model, &call, response(status, body));
        assert!(model.members.is_empty() && model.revealed_secret.is_none(), "{status}");
        assert_eq!(toast_texts(&model), vec![shown], "{status}");
        assert!(model.pending.is_empty());
    }
}

#[test]
fn a_role_change_applies_only_after_the_server_confirms() {
    let mut model = model_as(Role::Owner, Section::Identities);
    model.members = vec![member(1, "o@x.io", Role::Owner, false), member(2, "v@x.io", Role::Viewer, false)];
    let call = ApiCall::ChangeMemberRole { id: UserId(uid(2)), role: Role::Admin };
    update(&mut model, Msg::Request(call.clone()));
    assert_eq!(model.members[1].role, Role::Viewer);

    apply(&mut model, &call, response(200, r#"{"ok":true}"#));
    assert_eq!(model.members[1].role, Role::Admin);
    assert!(model.pending.is_empty());
}

#[test]
fn rejected_member_changes_leave_the_list_alone_and_explain() {
    let change = ApiCall::ChangeMemberRole { id: UserId(uid(2)), role: Role::Admin };
    let remove = ApiCall::RemoveMember(UserId(uid(2)));
    let revoke = ApiCall::RevokeInvitation(UserId(uid(2)));
    let cases: Vec<(ApiCall, u16, &str, &str)> = vec![
        (
            change.clone(),
            403,
            "",
            "That change is not allowed: an Admin cannot change an Owner, and the last Owner cannot be demoted",
        ),
        (change, 404, "", "That member no longer exists. Refresh the list."),
        (remove.clone(), 409, r#"{"message":"cannot remove the last Owner"}"#, "cannot remove the last Owner"),
        (remove.clone(), 403, "", "That removal is not allowed: an Admin cannot remove an Owner"),
        (remove, 404, "", "That member no longer exists. Refresh the list."),
        (revoke.clone(), 403, "", "That removal is not allowed: an Admin cannot remove an Owner"),
        (revoke, 404, "", "That member no longer exists. Refresh the list."),
    ];
    for (call, status, body, shown) in cases {
        let mut model = model_as(Role::Admin, Section::Identities);
        model.members = vec![member(1, "o@x.io", Role::Owner, false), member(2, "v@x.io", Role::Viewer, false)];
        let before = model.members.clone();
        update(&mut model, Msg::Request(call.clone()));
        apply(&mut model, &call, response(status, body));
        assert_eq!(model.members, before, "{call:?} {status}");
        assert_eq!(toast_texts(&model), vec![shown], "{call:?} {status}");
    }
}

#[test]
fn removing_a_member_and_revoking_an_invitation_drop_the_right_row() {
    let mut model = model_as(Role::Owner, Section::Identities);
    model.members = vec![
        member(1, "o@x.io", Role::Owner, false),
        member(2, "v@x.io", Role::Viewer, false),
        member(3, "p@x.io", Role::Admin, true),
    ];
    apply(&mut model, &ApiCall::RemoveMember(UserId(uid(2))), response(204, ""));
    assert_eq!(model.members.iter().map(|m| m.email.as_str()).collect::<Vec<_>>(), ["o@x.io", "p@x.io"]);

    apply(&mut model, &ApiCall::RevokeInvitation(UserId(uid(3))), response(204, ""));
    assert_eq!(model.members.iter().map(|m| m.email.as_str()).collect::<Vec<_>>(), ["o@x.io"]);
}

#[test]
fn a_pending_owner_invitation_does_not_make_the_real_owner_removable() {
    let mut model = model_as(Role::Owner, Section::Identities);
    model.members = vec![member(1, "o@x.io", Role::Owner, false), member(2, "p@x.io", Role::Owner, true)];
    update(&mut model, Msg::RemoveMember(UserId(uid(1))));
    assert_eq!(model.members.len(), 2, "the sole real Owner stays");
}

#[test]
fn only_owners_and_admins_manage_identities_and_roles_never_exceed_the_users_own() {
    let owner_row = member(1, "o@x.io", Role::Owner, false);
    let viewer_row = member(2, "v@x.io", Role::Viewer, false);
    let expectations = [
        (Role::Owner, true, vec![Role::Owner, Role::Admin, Role::Viewer], true, true),
        (Role::Admin, true, vec![Role::Admin, Role::Viewer], false, true),
        (Role::Viewer, false, vec![], false, false),
    ];
    for (role, manages, assignable, can_edit_owner, can_edit_viewer) in expectations {
        let model = model_as(role.clone(), Section::Identities);
        assert_eq!(model.can_manage_identities(), manages, "{role:?}");
        assert_eq!(model.assignable_roles(), assignable, "{role:?}");
        assert_eq!(model.can_edit_member(&owner_row), can_edit_owner, "{role:?} editing an Owner");
        assert_eq!(model.can_edit_member(&viewer_row), can_edit_viewer, "{role:?} editing a Viewer");
    }
    assert!(!AppModel::default().can_manage_identities(), "signed out");
}

// ── service accounts ─────────────────────────────────────────────────────────

const SERVICE_ACCOUNTS_JSON: &str = r#"[
  {"id":"00000000-0000-0000-0000-000000000007","name":"ci","description":"deploys",
   "role":"Admin","created_at":"2026-02-01T00:00:00Z"},
  {"id":"00000000-0000-0000-0000-000000000008","name":"bot","description":null,
   "role":"Viewer","created_at":"2026-02-02T00:00:00Z"}]"#;

#[test]
fn service_accounts_decode_and_create_appends_the_servers_row() {
    let mut model = model_as(Role::Owner, Section::Identities);
    apply(&mut model, &ApiCall::LoadServiceAccounts, response(200, SERVICE_ACCOUNTS_JSON));
    assert_eq!(model.service_accounts.len(), 2);
    assert_eq!(model.service_accounts[1].description, None);

    let create = ApiCall::CreateServiceAccount { name: "new".into(), description: None, role: Role::Viewer };
    update(&mut model, Msg::Request(create.clone()));
    assert!(model.pending.contains(&CallKind::ServiceAccountWrite));
    apply(
        &mut model,
        &create,
        response(
            201,
            r#"{"id":"00000000-0000-0000-0000-000000000009","name":"new","description":null,
                "role":"Viewer","created_at":"2026-02-03T00:00:00Z"}"#,
        ),
    );
    assert_eq!(model.service_accounts.len(), 3);
    assert_eq!(model.service_accounts[2].id, ServiceAccountId(uid(9)));
    assert!(model.pending.is_empty());
}

#[test]
fn deleting_a_service_account_removes_it_and_rereads_the_keys_it_revoked() {
    let mut model = model_as(Role::Owner, Section::Identities);
    apply(&mut model, &ApiCall::LoadServiceAccounts, response(200, SERVICE_ACCOUNTS_JSON));

    drive(&mut model, Msg::Request(ApiCall::DeleteServiceAccount(ServiceAccountId(uid(7)))), &|req| {
        match (req.method, req.path.as_str()) {
            (Method::Delete, _) => response(204, ""),
            (Method::Get, "/admin/v1/admin_keys") => response(200, "[]"),
            other => panic!("unexpected request {other:?}"),
        }
    });

    assert_eq!(model.service_accounts.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["bot"]);
    assert!(model.pending.is_empty(), "the admin-key reload settled too");
}

#[test]
fn rejected_service_account_writes_change_nothing_and_explain() {
    let create = ApiCall::CreateServiceAccount { name: "x".into(), description: None, role: Role::Viewer };
    let delete = ApiCall::DeleteServiceAccount(ServiceAccountId(uid(7)));
    let cases: Vec<(ApiCall, u16, &str)> = vec![
        (create.clone(), 403, "your role does not allow this"),
        (create, 422, "Enter a name and choose a role"),
        (delete.clone(), 404, "That service account no longer exists. Refresh the list."),
        (delete, 403, "your role does not allow this"),
    ];
    for (call, status, shown) in cases {
        let mut model = model_as(Role::Viewer, Section::Identities);
        apply(&mut model, &ApiCall::LoadServiceAccounts, response(200, SERVICE_ACCOUNTS_JSON));
        update(&mut model, Msg::Request(call.clone()));
        apply(&mut model, &call, response(status, ""));
        assert_eq!(model.service_accounts.len(), 2, "{call:?} {status}");
        assert_eq!(toast_texts(&model), vec![shown], "{call:?} {status}");
    }
}

// ── admin keys ───────────────────────────────────────────────────────────────

const ADMIN_KEYS_JSON: &str = r#"[
  {"id":"00000000-0000-0000-0000-000000000011","name":"tf","role":"Admin","prefix":"embyr_adm_ab",
   "created_at":"2026-02-01T00:00:00Z","last_used_at":"2026-02-05T00:00:00Z","revoked_at":null},
  {"id":"00000000-0000-0000-0000-000000000012","name":"old","role":"Viewer","prefix":"embyr_adm_cd",
   "created_at":"2026-01-01T00:00:00Z","last_used_at":null,"revoked_at":"2026-01-09T00:00:00Z"}]"#;

#[test]
fn the_admin_key_list_shows_live_keys_only_and_never_a_secret() {
    let mut model = model_as(Role::Owner, Section::ApiKeys);
    apply(&mut model, &ApiCall::LoadAdminKeys, response(200, ADMIN_KEYS_JSON));
    assert_eq!(model.admin_keys.len(), 1, "the revoked key is audit-only");
    assert_eq!((model.admin_keys[0].name.as_str(), model.admin_keys[0].role.clone()), ("tf", Role::Admin));
    assert_eq!(model.admin_keys[0].prefix, "embyr_adm_ab");
}

const CREATED_KEY_JSON: &str = r#"{"id":"00000000-0000-0000-0000-000000000013","name":"ci-key",
  "key":"embyr_adm_SECRET_ONCE","prefix":"embyr_adm_ef","created_at":"2026-03-05T12:00:00Z"}"#;

#[test]
fn creating_an_admin_key_reveals_the_plaintext_once() {
    let mut model = model_as(Role::Owner, Section::ApiKeys);
    let sa = ServiceAccountId(uid(7));
    let call = ApiCall::CreateAdminKey { name: "ci-key".into(), role: Role::Viewer, service_account_id: Some(sa.clone()) };
    update(&mut model, Msg::Request(call.clone()));
    assert!(model.admin_keys.is_empty());

    apply(&mut model, &call, response(201, CREATED_KEY_JSON));

    assert_eq!(model.admin_keys.len(), 1);
    let key = &model.admin_keys[0];
    assert_eq!((key.id.clone(), key.role.clone(), key.service_account_id.clone()), (KeyId(uid(0x13)), Role::Viewer, Some(sa)));
    let revealed = model.revealed_secret.as_ref().expect("the key is shown");
    assert_eq!(
        (revealed.kind, revealed.label.as_str(), revealed.secret.as_str()),
        (SecretKind::AdminKey, "ci-key", "embyr_adm_SECRET_ONCE")
    );
    assert!(!format!("{:?}", model.revealed_secret).contains("SECRET_ONCE"));
    assert!(model.admin_keys.iter().all(|k| !format!("{k:?}").contains("SECRET_ONCE")), "not kept in the list");

    // Closing, navigating, signing out and session expiry all drop it.
    update(&mut model, Msg::CloseSecret);
    assert!(model.revealed_secret.is_none());
    apply(&mut model, &call, response(201, CREATED_KEY_JSON));
    update(&mut model, Msg::NavigateTo(Section::Billing));
    assert!(model.revealed_secret.is_none(), "navigation");
}

#[test]
fn revoking_an_admin_key_removes_it_after_the_204() {
    let mut model = model_as(Role::Owner, Section::ApiKeys);
    apply(&mut model, &ApiCall::LoadAdminKeys, response(200, ADMIN_KEYS_JSON));
    apply(&mut model, &ApiCall::RevokeAdminKey(KeyId(uid(0x11))), response(204, ""));
    assert!(model.admin_keys.is_empty());
}

#[test]
fn rejected_admin_key_writes_change_nothing_and_explain() {
    let create = ApiCall::CreateAdminKey { name: "k".into(), role: Role::Owner, service_account_id: None };
    let revoke = ApiCall::RevokeAdminKey(KeyId(uid(0x11)));
    let cases: Vec<(ApiCall, u16, &str)> = vec![
        (create.clone(), 403, "You cannot create a key with a role above your own"),
        (create, 422, "Enter a name, choose a role, and pick an existing service account"),
        (revoke.clone(), 404, "That key no longer exists. Refresh the list."),
        (revoke, 403, "your role does not allow this"),
    ];
    for (call, status, shown) in cases {
        let mut model = model_as(Role::Admin, Section::ApiKeys);
        apply(&mut model, &ApiCall::LoadAdminKeys, response(200, ADMIN_KEYS_JSON));
        update(&mut model, Msg::Request(call.clone()));
        apply(&mut model, &call, response(status, ""));
        assert_eq!(model.admin_keys.len(), 1, "{call:?} {status}");
        assert!(model.revealed_secret.is_none());
        assert_eq!(toast_texts(&model), vec![shown], "{call:?} {status}");
    }
}

#[test]
fn an_expired_session_during_any_identity_call_drops_to_sign_in() {
    for call in [
        ApiCall::LoadMembers,
        ApiCall::LoadServiceAccounts,
        ApiCall::LoadAdminKeys,
        ApiCall::InviteMember { email: "a@b.c".into(), role: Role::Viewer },
        ApiCall::LoadQueryLogs { id: shop(), op: None, after: None },
    ] {
        let mut model = model_as(Role::Owner, Section::Identities);
        model.members = vec![member(1, "o@x.io", Role::Owner, false)];
        update(&mut model, Msg::Request(call.clone()));
        apply(&mut model, &call, response(401, r#"{"message":"expired"}"#));
        assert!(!model.authed && model.members.is_empty(), "{call:?}");
        assert_eq!(model.sign_in_error.as_deref(), Some("Session expired - please sign in again"));
    }
}

#[test]
fn a_second_write_of_the_same_kind_is_not_sent_while_one_is_in_flight() {
    let mut model = model_as(Role::Owner, Section::Identities);
    let first = ApiCall::RemoveMember(UserId(uid(2)));
    assert_eq!(update(&mut model, Msg::Request(first)).len(), 1);
    assert!(update(&mut model, Msg::Request(ApiCall::RevokeInvitation(UserId(uid(3))))).is_empty());
}

// ── hosted identity ──────────────────────────────────────────────────────────

#[test]
fn enabling_hosted_identity_records_it_and_says_so() {
    let mut model = model_as(Role::Admin, Section::DbDetail(shop()));
    let call = ApiCall::EnableHostedIdentity(shop());
    update(&mut model, Msg::Request(call.clone()));
    assert!(model.pending.contains(&CallKind::HostedIdentity));

    apply(
        &mut model,
        &call,
        response(201, r#"{"project_id":"shop","algorithm":"EdDSA","created_at":"2026-03-05T12:00:00Z"}"#),
    );

    assert!(model.hosted_identity.contains(&shop()));
    assert_eq!(toast_texts(&model), vec!["Hosted identity is enabled for this database"]);
    assert!(model.pending.is_empty());
}

#[test]
fn hosted_identity_refusals_leave_it_off_and_explain() {
    let call = ApiCall::EnableHostedIdentity(shop());
    let cases: [(u16, &str, &str); 3] = [
        (
            403,
            r#"{"reason":"HOSTED_IDENTITY_UNAVAILABLE_FOR_BACKEND_MODE"}"#,
            "Hosted identity is not available for agent-mode databases",
        ),
        (403, "", "your role does not allow this"),
        (404, "", "not found"),
    ];
    for (status, body, shown) in cases {
        let mut model = model_as(Role::Admin, Section::DbDetail(shop()));
        update(&mut model, Msg::Request(call.clone()));
        apply(&mut model, &call, response(status, body));
        assert!(model.hosted_identity.is_empty(), "{status}");
        assert_eq!(toast_texts(&model), vec![shown], "{status} {body}");
    }
}

#[test]
fn the_error_type_distinguishes_forbidden_from_conflict_on_invites() {
    let call = ApiCall::InviteMember { email: "a@b.c".into(), role: Role::Viewer };
    let forbidden = decode(&call, response(403, ""));
    assert!(matches!(forbidden.last(), Some(Msg::ApiFailed { error: ApiError::Invalid(_), .. })));
    let conflict = decode(&call, response(409, ""));
    assert!(matches!(conflict.last(), Some(Msg::ApiFailed { error: ApiError::Conflict(_), .. })));
}
