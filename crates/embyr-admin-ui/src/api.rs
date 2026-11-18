//! api — the single anti-corruption layer between the admin API and the TEA core (ADR-082).
//!
//! Host-compiled, no browser types. This is the only module that knows routes and JSON field names:
//! `to_request` turns an intent (`ApiCall`) into plain request data, and `decode` turns a response
//! into the `Msg`s `update` applies. The `csr` runtime in `app.rs` only moves bytes in between.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::model::{
    AdminKey, CapUsage, CheckoutReturn, Connection, ConnectionSummary, CurrentUser, Database, DbBackendMode, DbId,
    DbMetrics, DbStatus, Enrolment, InviteKind, KeyId, LatencyStats, LogRetention, Member, MetricsSummaryRow,
    MetricsWindow, OidcProvider, Plan, ProjectApiKeyInfo, ProviderIds, SsoReturn, QueryLogEntry, RecoveryCodes, RevealedSecret, Role, SecretKind, SeriesPoint, ServiceAccount,
    ServiceAccountId, Subscription, SubscriptionStatus, UsageStats, UsageTableRow, UserId,
};
use crate::msg::Msg;

/// Which call is in flight (the `model.pending` key).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CallKind {
    SignIn,
    ResetRequest,
    ResetConfirm,
    Session,
    SignOut,
    Projects,
    Project,
    MetricsSummary,
    DbMetrics,
    Connection,
    PutConnection,
    PatchProject,
    CreateProject,
    DeleteProject,
    /// Rotate and revoke share a kind: one key operation at a time.
    ApiKey,
    Subscription,
    Usage,
    /// Checkout and portal share a kind: one hosted-billing redirect at a time.
    BillingRedirect,
    DeleteAccount,
    InviteAccept,
    InviteConfirm,
    QueryLogs,
    HostedIdentity,
    Members,
    MemberInvite,
    /// Role change, removal and invitation revoke share a kind: one member operation at a time.
    MemberChange,
    ServiceAccounts,
    /// Create and delete share a kind: one service-account operation at a time.
    ServiceAccountWrite,
    AdminKeys,
    /// Create and revoke share a kind: one admin-key operation at a time.
    AdminKeyWrite,
    /// `POST /auth/oidc/start`, whatever the intent: one SSO redirect at a time.
    SsoStart,
    InviteLookup,
    SsoUnlink,
    OidcProviders,
    /// Create, enable/disable, rotate and delete share a kind: one provider operation at a time.
    OidcProviderWrite,
}

/// How the second factor was supplied. Exactly one reaches the server.
#[derive(Clone, PartialEq, Eq)]
pub enum SignInCode {
    Totp(String),
    Recovery(String),
}

/// What an SSO round trip is for. Secrets live only inside this value; `Debug` redacts them.
#[derive(Clone, PartialEq, Eq)]
pub enum SsoIntent {
    SignIn,
    /// Redeem an SSO invitation (the emailed token).
    Invite { token: String },
    /// Link the signed-in user's account: re-authenticates with password + authenticator code.
    Link { password: String, totp_code: String },
}

impl SsoIntent {
    fn label(&self) -> &'static str {
        match self {
            SsoIntent::SignIn => "signin",
            SsoIntent::Invite { .. } => "invite",
            SsoIntent::Link { .. } => "link",
        }
    }
}

/// New connection secrets for `PUT /projects/:id/connection`. Only ever inside an `ApiCall`.
#[derive(Clone, PartialEq, Eq)]
pub enum ConnectionConfig {
    DirectPg { dsn: String },
    Agent { endpoint: String, ca_pem: String, client_cert_pem: String, client_key_pem: String },
}

impl ConnectionConfig {
    pub fn mode(&self) -> DbBackendMode {
        match self {
            ConnectionConfig::DirectPg { .. } => DbBackendMode::DirectPg,
            ConnectionConfig::Agent { .. } => DbBackendMode::AgentMode,
        }
    }

    /// Every field the mode needs is non-blank (the server validates the content).
    pub fn is_complete(&self) -> bool {
        match self {
            ConnectionConfig::DirectPg { dsn } => !dsn.trim().is_empty(),
            ConnectionConfig::Agent { endpoint, ca_pem, client_cert_pem, client_key_pem } => {
                [endpoint, ca_pem, client_cert_pem, client_key_pem].iter().all(|v| !v.trim().is_empty())
            }
        }
    }

    /// The mode + secret fields shared by `PUT /connection` and `POST /projects`.
    fn to_json(&self) -> serde_json::Value {
        match self {
            ConnectionConfig::DirectPg { dsn } => serde_json::json!({ "backend_mode": "direct_pg", "dsn": dsn }),
            ConnectionConfig::Agent { endpoint, ca_pem, client_cert_pem, client_key_pem } => serde_json::json!({
                "backend_mode": "agent",
                "agent_endpoint": endpoint,
                "agent_ca_pem": ca_pem,
                "agent_client_cert_pem": client_cert_pem,
                "agent_client_key_pem": client_key_pem,
            }),
        }
    }
}

/// A new database for `POST /projects` (Owner/Admin). The secrets in `config` live only in the
/// `ApiCall`; the one-time API key comes back in the response.
#[derive(Clone, PartialEq, Eq)]
pub struct NewProject {
    pub project_id: String,
    /// Optional display name; blank is not sent.
    pub name: String,
    pub config: ConnectionConfig,
}

impl NewProject {
    /// Client-side gate only (the server re-validates): a well-formed id and a complete config.
    pub fn is_ready(&self) -> bool {
        is_valid_project_id(&self.project_id) && self.config.is_complete()
    }
}

/// Mirrors the server's `ProjectId` rule `^[a-z][a-z0-9-]{0,62}$`.
pub fn is_valid_project_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && id.len() <= 63
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A user intent that needs the server. Secrets live only inside this value; `Debug` redacts them.
#[derive(Clone, PartialEq, Eq)]
pub enum ApiCall {
    SignIn { email: String, password: String, code: SignInCode },
    /// `POST /auth/password_reset/request`: always answered 202, known email or not.
    RequestPasswordReset { email: String },
    /// `POST /auth/password_reset/confirm`: emailed token + new password + second factor.
    ConfirmPasswordReset { token: String, new_password: String, code: SignInCode },
    LoadSession,
    SignOut,
    LoadProjects,
    LoadProject(DbId),
    LoadMetricsSummary,
    LoadDbMetrics { id: DbId, window: MetricsWindow },
    LoadConnection(DbId),
    PutConnection { id: DbId, config: ConnectionConfig },
    SetSuspended { id: DbId, suspended: bool },
    /// `POST /projects` (Owner/Admin).
    CreateProject(NewProject),
    /// `DELETE /projects/:id` (Owner): soft delete.
    DeleteProject(DbId),
    SetLogging { id: DbId, enabled: bool, retention: Option<LogRetention> },
    RotateApiKey(DbId),
    RevokeApiKey(DbId),
    /// `GET /billing/subscription`: plan, server status, Free-plan caps.
    LoadSubscription,
    /// `GET /billing?range=month`: month-to-date usage per database.
    LoadUsage,
    /// `POST /billing/checkout_session` (Owner): answers the hosted Stripe Checkout URL for Pro.
    StartCheckout,
    /// `POST /billing/portal_session` (Owner): answers the hosted Stripe billing portal URL.
    OpenPortal,
    /// `DELETE /account` (Owner): re-authenticates with password + second factor and the typed account name.
    DeleteAccount { password: String, code: SignInCode, confirm_account_name: String },
    /// `POST /auth/invitations/accept`: step 1 of redeeming an emailed invitation.
    AcceptInvitation { token: String, display_name: String, password: String },
    /// `POST /auth/invitations/confirm`: step 2, proves the authenticator is enrolled.
    ConfirmInvitation { token: String, totp_code: String },
    /// `GET /projects/:id/query_logs`: `op` filters exactly, `after` is the cursor of a "load more".
    LoadQueryLogs { id: DbId, op: Option<String>, after: Option<String> },
    /// `POST /projects/:id/hosted_identity/enable` (Owner/Admin): idempotent, no body.
    EnableHostedIdentity(DbId),
    LoadMembers,
    /// `POST /members/invite` (Owner/Admin): the role may not exceed the inviter's own.
    InviteMember { email: String, role: Role },
    /// `PATCH /members/:id/role`.
    ChangeMemberRole { id: UserId, role: Role },
    /// `DELETE /members/:id`.
    RemoveMember(UserId),
    /// `DELETE /members/invitations/:id`.
    RevokeInvitation(UserId),
    LoadServiceAccounts,
    CreateServiceAccount { name: String, description: Option<String>, role: Role },
    DeleteServiceAccount(ServiceAccountId),
    LoadAdminKeys,
    /// `POST /admin_keys`: the plaintext comes back once.
    CreateAdminKey { name: String, role: Role, service_account_id: Option<ServiceAccountId> },
    /// `DELETE /admin_keys/:id`.
    RevokeAdminKey(KeyId),
    /// `POST /members/invite` with `auth_method: "sso"` (non-Owner role; needs an enabled provider).
    InviteSsoMember { email: String, role: Role },
    /// `POST /auth/oidc/start`: answers the IdP's authorization URL, which the browser then opens.
    StartSso { provider_id: String, intent: SsoIntent },
    /// `POST /auth/invitations/lookup`: is this emailed token a password or an SSO invitation?
    LookupInvitation { token: String },
    /// `DELETE /auth/sso_identity`: unlink the caller's own SSO identity.
    UnlinkSso,
    /// `GET /oidc_providers` (Owner).
    LoadOidcProviders,
    /// `POST /oidc_providers` (Owner). The secret is write-only.
    CreateOidcProvider { issuer: String, client_id: String, client_secret: String },
    /// `PATCH /oidc_providers/:id` (Owner).
    SetOidcProviderEnabled { id: String, enabled: bool },
    /// `PUT /oidc_providers/:id/secret` (Owner): rotate the client secret.
    RotateOidcSecret { id: String, client_secret: String },
    /// `DELETE /oidc_providers/:id` (Owner): also ends the sessions minted through it.
    DeleteOidcProvider(String),
}

impl ApiCall {
    pub fn kind(&self) -> CallKind {
        match self {
            ApiCall::SignIn { .. } => CallKind::SignIn,
            ApiCall::RequestPasswordReset { .. } => CallKind::ResetRequest,
            ApiCall::ConfirmPasswordReset { .. } => CallKind::ResetConfirm,
            ApiCall::LoadSession => CallKind::Session,
            ApiCall::SignOut => CallKind::SignOut,
            ApiCall::LoadProjects => CallKind::Projects,
            ApiCall::LoadProject(_) => CallKind::Project,
            ApiCall::LoadMetricsSummary => CallKind::MetricsSummary,
            ApiCall::LoadDbMetrics { .. } => CallKind::DbMetrics,
            ApiCall::LoadConnection(_) => CallKind::Connection,
            ApiCall::PutConnection { .. } => CallKind::PutConnection,
            ApiCall::SetSuspended { .. } | ApiCall::SetLogging { .. } => CallKind::PatchProject,
            ApiCall::CreateProject(_) => CallKind::CreateProject,
            ApiCall::DeleteProject(_) => CallKind::DeleteProject,
            ApiCall::RotateApiKey(_) | ApiCall::RevokeApiKey(_) => CallKind::ApiKey,
            ApiCall::LoadSubscription => CallKind::Subscription,
            ApiCall::LoadUsage => CallKind::Usage,
            ApiCall::StartCheckout | ApiCall::OpenPortal => CallKind::BillingRedirect,
            ApiCall::DeleteAccount { .. } => CallKind::DeleteAccount,
            ApiCall::AcceptInvitation { .. } => CallKind::InviteAccept,
            ApiCall::ConfirmInvitation { .. } => CallKind::InviteConfirm,
            ApiCall::LoadQueryLogs { .. } => CallKind::QueryLogs,
            ApiCall::EnableHostedIdentity(_) => CallKind::HostedIdentity,
            ApiCall::LoadMembers => CallKind::Members,
            ApiCall::InviteMember { .. } => CallKind::MemberInvite,
            ApiCall::ChangeMemberRole { .. } | ApiCall::RemoveMember(_) | ApiCall::RevokeInvitation(_) => {
                CallKind::MemberChange
            }
            ApiCall::LoadServiceAccounts => CallKind::ServiceAccounts,
            ApiCall::CreateServiceAccount { .. } | ApiCall::DeleteServiceAccount(_) => CallKind::ServiceAccountWrite,
            ApiCall::LoadAdminKeys => CallKind::AdminKeys,
            ApiCall::CreateAdminKey { .. } | ApiCall::RevokeAdminKey(_) => CallKind::AdminKeyWrite,
            ApiCall::InviteSsoMember { .. } => CallKind::MemberInvite,
            ApiCall::StartSso { .. } => CallKind::SsoStart,
            ApiCall::LookupInvitation { .. } => CallKind::InviteLookup,
            ApiCall::UnlinkSso => CallKind::SsoUnlink,
            ApiCall::LoadOidcProviders => CallKind::OidcProviders,
            ApiCall::CreateOidcProvider { .. }
            | ApiCall::SetOidcProviderEnabled { .. }
            | ApiCall::RotateOidcSecret { .. }
            | ApiCall::DeleteOidcProvider(_) => CallKind::OidcProviderWrite,
        }
    }

    /// Per-database reads are re-issued even while one of the same kind is in flight, so opening
    /// database B never waits on database A's load.
    pub fn repeatable(&self) -> bool {
        matches!(
            self,
            ApiCall::LoadProject(_) | ApiCall::LoadDbMetrics { .. } | ApiCall::LoadConnection(_)
        )
    }
}

impl std::fmt::Debug for ApiCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiCall::SignIn { email, .. } => f
                .debug_struct("SignIn")
                .field("email", email)
                .field("password", &"<redacted>")
                .field("code", &"<redacted>")
                .finish(),
            ApiCall::RequestPasswordReset { email } => {
                f.debug_struct("RequestPasswordReset").field("email", email).finish()
            }
            ApiCall::ConfirmPasswordReset { .. } => f
                .debug_struct("ConfirmPasswordReset")
                .field("token", &"<redacted>")
                .field("new_password", &"<redacted>")
                .field("code", &"<redacted>")
                .finish(),
            ApiCall::LoadSession => f.write_str("LoadSession"),
            ApiCall::SignOut => f.write_str("SignOut"),
            ApiCall::LoadProjects => f.write_str("LoadProjects"),
            ApiCall::LoadProject(id) => write!(f, "LoadProject({})", id.0),
            ApiCall::LoadMetricsSummary => f.write_str("LoadMetricsSummary"),
            ApiCall::LoadDbMetrics { id, window } => write!(f, "LoadDbMetrics({}, {})", id.0, window.api_value()),
            ApiCall::LoadConnection(id) => write!(f, "LoadConnection({})", id.0),
            ApiCall::PutConnection { id, config } => f
                .debug_struct("PutConnection")
                .field("id", &id.0)
                .field("mode", &config.mode().api_value())
                .field("config", &"<redacted>")
                .finish(),
            ApiCall::SetSuspended { id, suspended } => write!(f, "SetSuspended({}, {suspended})", id.0),
            ApiCall::CreateProject(new) => f
                .debug_struct("CreateProject")
                .field("project_id", &new.project_id)
                .field("mode", &new.config.mode().api_value())
                .field("config", &"<redacted>")
                .finish(),
            ApiCall::DeleteProject(id) => write!(f, "DeleteProject({})", id.0),
            ApiCall::SetLogging { id, enabled, retention } => {
                write!(f, "SetLogging({}, {enabled}, {retention:?})", id.0)
            }
            ApiCall::RotateApiKey(id) => write!(f, "RotateApiKey({})", id.0),
            ApiCall::RevokeApiKey(id) => write!(f, "RevokeApiKey({})", id.0),
            ApiCall::LoadSubscription => f.write_str("LoadSubscription"),
            ApiCall::LoadUsage => f.write_str("LoadUsage"),
            ApiCall::StartCheckout => f.write_str("StartCheckout"),
            ApiCall::OpenPortal => f.write_str("OpenPortal"),
            ApiCall::DeleteAccount { confirm_account_name, .. } => f
                .debug_struct("DeleteAccount")
                .field("confirm_account_name", confirm_account_name)
                .field("password", &"<redacted>")
                .field("code", &"<redacted>")
                .finish(),
            ApiCall::AcceptInvitation { display_name, .. } => f
                .debug_struct("AcceptInvitation")
                .field("token", &"<redacted>")
                .field("display_name", display_name)
                .field("password", &"<redacted>")
                .finish(),
            ApiCall::ConfirmInvitation { .. } => f
                .debug_struct("ConfirmInvitation")
                .field("token", &"<redacted>")
                .field("totp_code", &"<redacted>")
                .finish(),
            ApiCall::LoadQueryLogs { id, op, after } => write!(f, "LoadQueryLogs({}, {op:?}, {after:?})", id.0),
            ApiCall::EnableHostedIdentity(id) => write!(f, "EnableHostedIdentity({})", id.0),
            ApiCall::LoadMembers => f.write_str("LoadMembers"),
            ApiCall::InviteMember { email, role } => write!(f, "InviteMember({email}, {role:?})"),
            ApiCall::ChangeMemberRole { id, role } => write!(f, "ChangeMemberRole({}, {role:?})", id.0),
            ApiCall::RemoveMember(id) => write!(f, "RemoveMember({})", id.0),
            ApiCall::RevokeInvitation(id) => write!(f, "RevokeInvitation({})", id.0),
            ApiCall::LoadServiceAccounts => f.write_str("LoadServiceAccounts"),
            ApiCall::CreateServiceAccount { name, role, .. } => write!(f, "CreateServiceAccount({name}, {role:?})"),
            ApiCall::DeleteServiceAccount(id) => write!(f, "DeleteServiceAccount({})", id.0),
            ApiCall::LoadAdminKeys => f.write_str("LoadAdminKeys"),
            ApiCall::CreateAdminKey { name, role, .. } => write!(f, "CreateAdminKey({name}, {role:?})"),
            ApiCall::RevokeAdminKey(id) => write!(f, "RevokeAdminKey({})", id.0),
            ApiCall::InviteSsoMember { email, role } => write!(f, "InviteSsoMember({email}, {role:?})"),
            ApiCall::StartSso { intent, .. } => f
                .debug_struct("StartSso")
                .field("provider_id", &"<redacted>")
                .field("intent", &intent.label())
                .finish(),
            ApiCall::LookupInvitation { .. } => {
                f.debug_struct("LookupInvitation").field("token", &"<redacted>").finish()
            }
            ApiCall::UnlinkSso => f.write_str("UnlinkSso"),
            ApiCall::LoadOidcProviders => f.write_str("LoadOidcProviders"),
            ApiCall::CreateOidcProvider { issuer, client_id, .. } => f
                .debug_struct("CreateOidcProvider")
                .field("issuer", issuer)
                .field("client_id", client_id)
                .field("client_secret", &"<redacted>")
                .finish(),
            ApiCall::SetOidcProviderEnabled { id, enabled } => write!(f, "SetOidcProviderEnabled({id}, {enabled})"),
            ApiCall::RotateOidcSecret { id, .. } => f
                .debug_struct("RotateOidcSecret")
                .field("id", id)
                .field("client_secret", &"<redacted>")
                .finish(),
            ApiCall::DeleteOidcProvider(id) => write!(f, "DeleteOidcProvider({id})"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

/// Plain request data; the runtime adds same-origin credentials and the JSON content type.
#[derive(Clone, Debug, PartialEq)]
pub struct HttpRequest {
    pub method: Method,
    pub path: String,
    pub body: Option<serde_json::Value>,
}

/// What the runtime hands back after a completed fetch.
#[derive(Clone, Debug, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    /// The `retry-after-ms` response header, when present.
    pub retry_after_ms: Option<u64>,
    pub body: String,
}

/// Why a call failed. Messages are user-facing (G-10: one status -> message table).
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ApiError {
    /// 401 — carries the server's message ("Invalid credentials", ...).
    #[error("{0}")]
    Unauthorized(String),
    #[error("your role does not allow this")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    /// 400 / 422 — carries the server's message.
    #[error("{0}")]
    Invalid(String),
    /// 429. `None` means no `retry-after-ms`: the account is locked.
    #[error("too many attempts")]
    Throttled { retry_after_ms: Option<u64> },
    #[error("an upstream service is unavailable")]
    Upstream,
    #[error("server error ({0})")]
    Server(u16),
    #[error("cannot reach server: {0}")]
    Network(String),
    #[error("unexpected response: {0}")]
    Decode(String),
}

pub fn to_request(call: &ApiCall) -> HttpRequest {
    let get = |path: &str| HttpRequest { method: Method::Get, path: path.to_string(), body: None };
    // Project ids are validated server-side (`ProjectId::new`), so they need no URL escaping.
    let project = |id: &DbId, suffix: &str| format!("/admin/v1/projects/{}{suffix}", id.0);
    match call {
        ApiCall::SignIn { email, password, code } => {
            let mut body = serde_json::json!({ "email": email, "password": password });
            match code {
                SignInCode::Totp(c) => body["totp_code"] = c.as_str().into(),
                SignInCode::Recovery(c) => body["recovery_code"] = c.as_str().into(),
            }
            HttpRequest { method: Method::Post, path: "/admin/v1/auth/signin".into(), body: Some(body) }
        }
        ApiCall::RequestPasswordReset { email } => HttpRequest {
            method: Method::Post,
            path: "/admin/v1/auth/password_reset/request".into(),
            body: Some(serde_json::json!({ "email": email })),
        },
        ApiCall::ConfirmPasswordReset { token, new_password, code } => {
            let mut body = serde_json::json!({ "token": token, "new_password": new_password });
            match code {
                SignInCode::Totp(c) => body["totp_code"] = c.as_str().into(),
                SignInCode::Recovery(c) => body["recovery_code"] = c.as_str().into(),
            }
            HttpRequest { method: Method::Post, path: "/admin/v1/auth/password_reset/confirm".into(), body: Some(body) }
        }
        ApiCall::LoadSession => get("/admin/v1/auth/session"),
        ApiCall::SignOut => post("/admin/v1/auth/signout".into()),
        ApiCall::LoadProjects => get("/admin/v1/projects"),
        ApiCall::LoadProject(id) => get(&project(id, "")),
        ApiCall::LoadMetricsSummary => get("/admin/v1/metrics/summary"),
        ApiCall::LoadDbMetrics { id, window } => {
            get(&project(id, &format!("/metrics?window={}", window.api_value())))
        }
        ApiCall::LoadConnection(id) => get(&project(id, "/connection")),
        ApiCall::PutConnection { id, config } => HttpRequest {
            method: Method::Put,
            path: project(id, "/connection"),
            body: Some(config.to_json()),
        },
        ApiCall::CreateProject(new) => {
            let mut body = new.config.to_json();
            body["project_id"] = new.project_id.as_str().into();
            if !new.name.trim().is_empty() {
                body["name"] = new.name.trim().into();
            }
            HttpRequest { method: Method::Post, path: "/admin/v1/projects".into(), body: Some(body) }
        }
        ApiCall::DeleteProject(id) => {
            HttpRequest { method: Method::Delete, path: project(id, ""), body: None }
        }
        ApiCall::SetSuspended { id, suspended } => patch(
            project(id, ""),
            serde_json::json!({ "status": if *suspended { "suspended" } else { "active" } }),
        ),
        ApiCall::SetLogging { id, enabled, retention } => {
            let mut body = serde_json::json!({ "logging_enabled": enabled });
            if let Some(r) = retention {
                body["log_retention_days"] = r.days().into();
            }
            patch(project(id, ""), body)
        }
        ApiCall::RotateApiKey(id) => post(project(id, "/api_key/rotate")),
        ApiCall::RevokeApiKey(id) => post(project(id, "/api_key/revoke")),
        ApiCall::LoadSubscription => get("/admin/v1/billing/subscription"),
        ApiCall::LoadUsage => get("/admin/v1/billing?range=month"),
        ApiCall::StartCheckout => HttpRequest {
            method: Method::Post,
            path: "/admin/v1/billing/checkout_session".into(),
            body: Some(serde_json::json!({ "plan": "pro" })),
        },
        ApiCall::OpenPortal => post("/admin/v1/billing/portal_session".into()),
        ApiCall::DeleteAccount { password, code, confirm_account_name } => {
            let mut body = serde_json::json!({ "password": password, "confirm_account_name": confirm_account_name });
            match code {
                SignInCode::Totp(c) => body["totp_code"] = c.as_str().into(),
                SignInCode::Recovery(c) => body["recovery_code"] = c.as_str().into(),
            }
            HttpRequest { method: Method::Delete, path: "/admin/v1/account".into(), body: Some(body) }
        }
        ApiCall::AcceptInvitation { token, display_name, password } => HttpRequest {
            method: Method::Post,
            path: "/admin/v1/auth/invitations/accept".into(),
            body: Some(serde_json::json!({ "token": token, "display_name": display_name, "password": password })),
        },
        ApiCall::ConfirmInvitation { token, totp_code } => HttpRequest {
            method: Method::Post,
            path: "/admin/v1/auth/invitations/confirm".into(),
            body: Some(serde_json::json!({ "token": token, "totp_code": totp_code })),
        },
        ApiCall::LoadQueryLogs { id, op, after } => {
            // Op values and entry ids are API-controlled tokens (`[a-z_]`, UUID): no escaping needed.
            let query: Vec<String> = [("op", op), ("after", after)]
                .into_iter()
                .filter_map(|(key, value)| value.as_ref().map(|v| format!("{key}={v}")))
                .collect();
            let suffix = if query.is_empty() { String::new() } else { format!("?{}", query.join("&")) };
            get(&project(id, &format!("/query_logs{suffix}")))
        }
        ApiCall::EnableHostedIdentity(id) => post(project(id, "/hosted_identity/enable")),
        ApiCall::LoadMembers => get("/admin/v1/members"),
        ApiCall::InviteMember { email, role } => HttpRequest {
            method: Method::Post,
            path: "/admin/v1/members/invite".into(),
            body: Some(serde_json::json!({ "email": email.trim(), "role": role_value(role) })),
        },
        ApiCall::ChangeMemberRole { id, role } => {
            patch(format!("/admin/v1/members/{}/role", id.0), serde_json::json!({ "role": role_value(role) }))
        }
        ApiCall::RemoveMember(id) => {
            HttpRequest { method: Method::Delete, path: format!("/admin/v1/members/{}", id.0), body: None }
        }
        ApiCall::RevokeInvitation(id) => HttpRequest {
            method: Method::Delete,
            path: format!("/admin/v1/members/invitations/{}", id.0),
            body: None,
        },
        ApiCall::LoadServiceAccounts => get("/admin/v1/service_accounts"),
        ApiCall::CreateServiceAccount { name, description, role } => {
            let mut body = serde_json::json!({ "name": name.trim(), "role": role_value(role) });
            if let Some(d) = description.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
                body["description"] = d.into();
            }
            HttpRequest { method: Method::Post, path: "/admin/v1/service_accounts".into(), body: Some(body) }
        }
        ApiCall::DeleteServiceAccount(id) => HttpRequest {
            method: Method::Delete,
            path: format!("/admin/v1/service_accounts/{}", id.0),
            body: None,
        },
        ApiCall::LoadAdminKeys => get("/admin/v1/admin_keys"),
        ApiCall::CreateAdminKey { name, role, service_account_id } => {
            let mut body = serde_json::json!({ "name": name.trim(), "role": role_value(role) });
            if let Some(sa) = service_account_id {
                body["service_account_id"] = sa.0.to_string().into();
            }
            HttpRequest { method: Method::Post, path: "/admin/v1/admin_keys".into(), body: Some(body) }
        }
        ApiCall::RevokeAdminKey(id) => {
            HttpRequest { method: Method::Delete, path: format!("/admin/v1/admin_keys/{}", id.0), body: None }
        }
        ApiCall::InviteSsoMember { email, role } => HttpRequest {
            method: Method::Post,
            path: "/admin/v1/members/invite".into(),
            body: Some(serde_json::json!({ "email": email.trim(), "role": role_value(role), "auth_method": "sso" })),
        },
        ApiCall::StartSso { provider_id, intent } => {
            let mut body = serde_json::json!({ "provider_id": provider_id, "intent": intent.label() });
            match intent {
                SsoIntent::SignIn => {}
                SsoIntent::Invite { token } => body["invite_token"] = token.as_str().into(),
                SsoIntent::Link { password, totp_code } => {
                    body["password"] = password.as_str().into();
                    body["totp_code"] = totp_code.as_str().into();
                }
            }
            HttpRequest { method: Method::Post, path: "/admin/v1/auth/oidc/start".into(), body: Some(body) }
        }
        ApiCall::LookupInvitation { token } => HttpRequest {
            method: Method::Post,
            path: "/admin/v1/auth/invitations/lookup".into(),
            body: Some(serde_json::json!({ "token": token })),
        },
        ApiCall::UnlinkSso => {
            HttpRequest { method: Method::Delete, path: "/admin/v1/auth/sso_identity".into(), body: None }
        }
        ApiCall::LoadOidcProviders => get("/admin/v1/oidc_providers"),
        ApiCall::CreateOidcProvider { issuer, client_id, client_secret } => HttpRequest {
            method: Method::Post,
            path: "/admin/v1/oidc_providers".into(),
            body: Some(serde_json::json!({
                "issuer": issuer.trim(),
                "client_id": client_id.trim(),
                "client_secret": client_secret,
            })),
        },
        ApiCall::SetOidcProviderEnabled { id, enabled } => {
            patch(format!("/admin/v1/oidc_providers/{id}"), serde_json::json!({ "enabled": enabled }))
        }
        ApiCall::RotateOidcSecret { id, client_secret } => HttpRequest {
            method: Method::Put,
            path: format!("/admin/v1/oidc_providers/{id}/secret"),
            body: Some(serde_json::json!({ "client_secret": client_secret })),
        },
        ApiCall::DeleteOidcProvider(id) => {
            HttpRequest { method: Method::Delete, path: format!("/admin/v1/oidc_providers/{id}"), body: None }
        }
    }
}

/// G-08: the API's role strings are exactly `Owner | Admin | Viewer`.
fn role_value(role: &Role) -> &'static str {
    match role {
        Role::Owner => "Owner",
        Role::Admin => "Admin",
        Role::Viewer => "Viewer",
    }
}

/// The link an invitee opens to redeem `token`. `origin` is the console's own origin
/// (`window.location.origin`), so the link points at the deployment the admin is using.
pub fn invite_link(origin: &str, token: &str) -> String {
    format!("{}/admin/?invite={token}", origin.trim_end_matches('/'))
}

/// The link the Owner shares for SSO sign-in: the SPA turns `?sso=<provider id>` into a start call.
pub fn sso_sign_in_link(origin: &str, provider_id: &str) -> String {
    format!("{}/admin/?sso={provider_id}", origin.trim_end_matches('/'))
}

/// The redirect URI to register at the IdP (the server's fixed callback on the console origin).
pub fn sso_redirect_uri(origin: &str) -> String {
    format!("{}/admin/v1/auth/oidc/callback", origin.trim_end_matches('/'))
}

fn post(path: String) -> HttpRequest {
    HttpRequest { method: Method::Post, path, body: None }
}

fn patch(path: String, body: serde_json::Value) -> HttpRequest {
    HttpRequest { method: Method::Patch, path, body: Some(body) }
}

/// Response -> `Msg`s. The first message always settles the call (clears `pending`); failures follow
/// as `ApiFailed`, successes as the result messages `update` applies.
pub fn decode(call: &ApiCall, response: HttpResponse) -> Vec<Msg> {
    let result = if (200..300).contains(&response.status) {
        decode_success(call, &response.body)
    } else {
        Err(refine(call, error_for(&response), &response.body))
    };
    settle(call, result)
}

/// Transport failure (fetch rejected): same shape as any other failed call.
pub fn decode_network_failure(call: &ApiCall, reason: String) -> Vec<Msg> {
    settle(call, Err(ApiError::Network(reason)))
}

fn settle(call: &ApiCall, result: Result<Vec<Msg>, ApiError>) -> Vec<Msg> {
    let kind = call.kind();
    let mut msgs = vec![Msg::Settled(kind)];
    match result {
        Ok(more) => msgs.extend(more),
        Err(error) => msgs.push(Msg::ApiFailed { call: kind, error }),
    }
    msgs
}

fn decode_success(call: &ApiCall, body: &str) -> Result<Vec<Msg>, ApiError> {
    match call {
        // Signing in only mints the cookie; the session call both proves the cookie was accepted
        // (G-11) and fetches who we are.
        ApiCall::SignIn { .. } => Ok(vec![Msg::Request(ApiCall::LoadSession)]),
        ApiCall::RequestPasswordReset { .. } => Ok(vec![Msg::PasswordResetRequested]),
        ApiCall::ConfirmPasswordReset { .. } => Ok(vec![Msg::PasswordResetDone]),
        ApiCall::LoadSession => Ok(vec![Msg::SessionEstablished(parse::<SessionDto>(body)?.try_into()?)]),
        ApiCall::SignOut => Ok(vec![Msg::SignOut]),
        ApiCall::LoadProjects => {
            let projects: Vec<ProjectDto> = parse(body)?;
            let databases = projects.into_iter().map(Database::try_from).collect::<Result<_, _>>()?;
            Ok(vec![Msg::SetDatabases(databases)])
        }
        // A PATCH answers with the same detail shape as GET, so both refresh the model from the
        // server's truth rather than from what the client asked for.
        ApiCall::LoadProject(_) | ApiCall::SetSuspended { .. } | ApiCall::SetLogging { .. } => {
            let dto: ProjectDto = parse(body)?;
            let info = dto.api_key_info()?;
            let db = Database::try_from(dto)?;
            // The database goes first: key info for an unknown database is dropped.
            let id = db.id.clone();
            Ok(vec![Msg::DatabaseLoaded(db), Msg::ApiKeyInfoLoaded(id, info)])
        }
        ApiCall::LoadMetricsSummary => {
            let rows: Vec<SummaryDto> = parse(body)?;
            Ok(vec![Msg::SetMetricsSummary(rows.into_iter().map(SummaryDto::into_row).collect())])
        }
        ApiCall::LoadDbMetrics { id, window } => {
            Ok(vec![Msg::SetDbMetrics(id.clone(), parse::<MetricsDto>(body)?.into_model(*window)?)])
        }
        ApiCall::LoadConnection(id) => {
            Ok(vec![Msg::SetConnection(id.clone(), parse::<ConnectionDto>(body)?.into_model()?)])
        }
        ApiCall::PutConnection { id, config } => Ok(vec![
            Msg::ConnectionSaved(id.clone(), config.mode()),
            Msg::Request(ApiCall::LoadConnection(id.clone())),
        ]),
        // The key is shown once (AC-AUW-20); the list is refreshed from the server rather than
        // patched locally, so the new row carries the server's own id/status/created_at.
        ApiCall::CreateProject(_) => {
            let dto: ProvisionedDto = parse(body)?;
            Ok(vec![
                Msg::CloseCreateDb,
                Msg::SecretRevealed(RevealedSecret::api_key(DbId(dto.project_id), dto.api_key)),
                Msg::Request(ApiCall::LoadProjects),
            ])
        }
        ApiCall::DeleteProject(id) => Ok(vec![Msg::DeleteDatabase(id.clone())]),
        // Revoke is rotate with a zero window: both return the replacement key, shown once.
        ApiCall::RotateApiKey(id) | ApiCall::RevokeApiKey(id) => {
            let dto: RotatedKeyDto = parse(body)?;
            let info = ProjectApiKeyInfo {
                rotated_at: Some(parse_ts("rotated_at", &dto.rotated_at)?),
                previous_expires_at: parse_opt_ts("previous_key_expires_at", dto.previous_key_expires_at.as_deref())?,
            };
            Ok(vec![
                Msg::ApiKeyInfoLoaded(id.clone(), info),
                Msg::SecretRevealed(RevealedSecret::api_key(id.clone(), dto.api_key)),
            ])
        }
        ApiCall::LoadSubscription => {
            let dto: SubscriptionDto = parse(body)?;
            let status = parse_subscription_status(&dto.status)?;
            let subscription = Subscription {
                plan: parse_plan(&dto.plan)?,
                stripe_customer_id: dto.stripe_customer_id,
                current_period_end: parse_opt_ts("current_period_end", dto.current_period_end.as_deref())?,
                card: None,
                payment_failure: status == SubscriptionStatus::PastDue,
            };
            let cap_status = dto.cap_status.map(|caps| {
                caps.into_iter()
                    .map(|c| CapUsage { dimension: c.dimension, used: c.used, cap: c.cap, pct: c.pct })
                    .collect()
            });
            Ok(vec![Msg::SetSubscription(subscription), Msg::SetBilling { status, cap_status }])
        }
        ApiCall::LoadUsage => {
            let dto: UsageDto = parse(body)?;
            let rows = dto
                .databases
                .into_iter()
                .map(|d| UsageTableRow {
                    name: d.name.filter(|n| !n.is_empty()).unwrap_or_else(|| d.id.clone()),
                    db_id: DbId(d.id),
                    usage: UsageStats { reads: d.reads, writes: d.writes, deletes: d.deletes, storage_gb: 0.0 },
                })
                .collect();
            Ok(vec![Msg::SetUsage(rows)])
        }
        // Only follow a Stripe-hosted https link: never navigate to whatever the body says.
        ApiCall::StartCheckout | ApiCall::OpenPortal => {
            let dto: SessionUrlDto = parse(body)?;
            if !dto.url.starts_with("https://") {
                return Err(ApiError::Decode("billing link is not an https URL".into()));
            }
            Ok(vec![Msg::RedirectToHosted(dto.url)])
        }
        ApiCall::DeleteAccount { .. } => {
            let dto: DeletedAccountDto = parse(body)?;
            Ok(vec![Msg::AccountDeleted(parse_ts("purge_after", &dto.purge_after)?)])
        }
        ApiCall::AcceptInvitation { .. } => {
            let dto: AcceptedInvitationDto = parse(body)?;
            Ok(vec![Msg::InviteAccepted(Enrolment { totp_secret: dto.totp_secret, otpauth_uri: dto.otpauth_uri })])
        }
        ApiCall::ConfirmInvitation { .. } => {
            let dto: ConfirmedInvitationDto = parse(body)?;
            Ok(vec![Msg::InviteConfirmed(RecoveryCodes(dto.recovery_codes))])
        }
        ApiCall::LoadQueryLogs { id, after, .. } => {
            let dto: QueryLogsDto = parse(body)?;
            let entries = dto
                .entries
                .into_iter()
                .map(|e| {
                    Ok(QueryLogEntry {
                        created_at: parse_ts("created_at", &e.created_at)?,
                        id: e.id,
                        op: e.op,
                        collection_path: e.collection_path,
                        latency_ms: e.latency_ms,
                        status: e.status,
                        error_code: e.error_code,
                    })
                })
                .collect::<Result<_, ApiError>>()?;
            Ok(vec![Msg::SetQueryLogs { db: id.clone(), total: dto.total, entries, append: after.is_some() }])
        }
        ApiCall::EnableHostedIdentity(id) => Ok(vec![Msg::HostedIdentityEnabled(id.clone())]),
        ApiCall::LoadMembers => {
            let rows: Vec<MemberDto> = parse(body)?;
            let sso = rows
                .iter()
                .filter(|r| r.auth_method == "sso")
                .map(|r| parse_uuid("id", &r.id).map(UserId))
                .collect::<Result<_, _>>()?;
            let members = rows.into_iter().map(Member::try_from).collect::<Result<_, _>>()?;
            Ok(vec![Msg::SetMembers(members), Msg::SetSsoMembers(sso)])
        }
        // The token comes back once: reveal it so the admin can share the link when mail is not
        // delivered. The pending row is built from the server's own invitation id.
        ApiCall::InviteMember { .. } | ApiCall::InviteSsoMember { .. } => {
            let dto: InvitationDto = parse(body)?;
            let pending = Member {
                id: UserId(parse_uuid("invitation_id", &dto.invitation_id)?),
                email: dto.email.clone(),
                display_name: None,
                role: parse_role(&dto.role)?,
                pending: true,
                mfa_enabled: false,
                last_login: None,
            };
            Ok(vec![
                Msg::MemberInvited(pending),
                Msg::SecretRevealed(RevealedSecret::other(SecretKind::InviteToken, dto.email, dto.invite_token)),
            ])
        }
        ApiCall::ChangeMemberRole { id, role } => Ok(vec![Msg::SetMemberRole(id.clone(), role.clone())]),
        ApiCall::RemoveMember(id) => Ok(vec![Msg::RemoveMember(id.clone())]),
        ApiCall::RevokeInvitation(id) => Ok(vec![Msg::InvitationRevoked(id.clone())]),
        ApiCall::LoadServiceAccounts => {
            let rows: Vec<ServiceAccountDto> = parse(body)?;
            Ok(vec![Msg::SetServiceAccounts(
                rows.into_iter().map(ServiceAccount::try_from).collect::<Result<_, _>>()?,
            )])
        }
        ApiCall::CreateServiceAccount { .. } => {
            let dto: ServiceAccountDto = parse(body)?;
            Ok(vec![Msg::ServiceAccountCreated(dto.try_into()?)])
        }
        // Deleting a service account revokes its admin keys server-side: re-read them.
        ApiCall::DeleteServiceAccount(id) => {
            Ok(vec![Msg::DeleteServiceAccount(id.clone()), Msg::Request(ApiCall::LoadAdminKeys)])
        }
        ApiCall::LoadAdminKeys => {
            let rows: Vec<AdminKeyDto> = parse(body)?;
            // Revoked keys stay in the API list for audit; the console lists live keys only.
            let keys = rows
                .into_iter()
                .filter(|k| k.revoked_at.is_none())
                .map(AdminKey::try_from)
                .collect::<Result<_, _>>()?;
            Ok(vec![Msg::SetAdminKeys(keys)])
        }
        ApiCall::CreateAdminKey { role, service_account_id, .. } => {
            let dto: CreatedAdminKeyDto = parse(body)?;
            let key = AdminKey {
                id: KeyId(parse_uuid("id", &dto.id)?),
                name: dto.name.clone(),
                service_account_id: service_account_id.clone(),
                member_id: None,
                role: role.clone(),
                prefix: dto.prefix,
                created_at: Some(parse_ts("created_at", &dto.created_at)?),
            };
            Ok(vec![Msg::AdminKeyCreated(key), Msg::SecretRevealed(RevealedSecret::other(SecretKind::AdminKey, dto.name, dto.key))])
        }
        ApiCall::RevokeAdminKey(id) => Ok(vec![Msg::RevokeAdminKey(id.clone())]),
        ApiCall::StartSso { .. } => {
            let dto: SsoStartDto = parse(body)?;
            // `{error:"sso_unavailable"}` is a 200 on purpose (no oracle); anything without a URL is the same.
            let Some(url) = dto.authorization_url else { return Err(ApiError::Invalid(SSO_UNAVAILABLE.into())) };
            if !is_idp_url(&url) {
                return Err(ApiError::Decode("authorization URL is not https".into()));
            }
            Ok(vec![Msg::RedirectToHosted(url)])
        }
        ApiCall::LookupInvitation { .. } => {
            let dto: LookupDto = parse(body)?;
            let kind = match dto.auth_method.as_str() {
                "password" => InviteKind::Password,
                "sso" => InviteKind::Sso(ProviderIds(
                    dto.sso_provider_ids.into_iter().filter(|id| uuid::Uuid::parse_str(id).is_ok()).collect(),
                )),
                other => return Err(ApiError::Decode(format!("unknown auth method '{other}'"))),
            };
            Ok(vec![Msg::InviteLooked(kind)])
        }
        ApiCall::UnlinkSso => Ok(vec![Msg::SsoUnlinked]),
        ApiCall::LoadOidcProviders => {
            let rows: Vec<OidcProviderDto> = parse(body)?;
            Ok(vec![Msg::SetOidcProviders(
                rows.into_iter()
                    .map(|p| OidcProvider { id: p.id, issuer: p.issuer, client_id: p.client_id, enabled: p.enabled })
                    .collect(),
            )])
        }
        ApiCall::CreateOidcProvider { .. } => {
            Ok(vec![Msg::OidcProviderSaved, Msg::Request(ApiCall::LoadOidcProviders)])
        }
        ApiCall::RotateOidcSecret { .. } => Ok(vec![Msg::OidcProviderSaved]),
        ApiCall::SetOidcProviderEnabled { .. } | ApiCall::DeleteOidcProvider(_) => {
            Ok(vec![Msg::Request(ApiCall::LoadOidcProviders)])
        }
    }
}

/// Shown for `sso_unavailable`: unknown, disabled or foreign provider, or SSO not configured on the server.
const SSO_UNAVAILABLE: &str =
    "Single sign-on is not available for this link. Ask your account Owner for the current link.";

/// Only follow an IdP link that is https (plain http only to a local mock IdP in development).
fn is_idp_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://localhost") || url.starts_with("http://127.0.0.1")
}

/// Call-specific wording for statuses that carry no body of their own.
fn refine(call: &ApiCall, error: ApiError, body: &str) -> ApiError {
    match (call, &error) {
        (ApiCall::CreateProject(_), ApiError::Invalid(code)) => {
            let detail = serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|v| v.get("detail").and_then(|d| d.as_str().map(str::to_string)));
            ApiError::Invalid(create_message(code, detail.as_deref()))
        }
        (ApiCall::CreateProject(_), ApiError::Conflict(code)) => ApiError::Conflict(create_message(code, None)),
        (ApiCall::CreateProject(_), ApiError::Throttled { .. }) => {
            ApiError::Invalid("Too many requests - wait a moment and try again".into())
        }
        (ApiCall::DeleteProject(_), ApiError::NotFound) => {
            ApiError::Invalid("That database no longer exists. Refresh the list.".into())
        }
        (ApiCall::SetSuspended { suspended: false, .. }, ApiError::Conflict(_)) => ApiError::Conflict(
            "cannot activate: the account is on a billing hold (past due or free-plan limit reached)".into(),
        ),
        // The confirm endpoint answers 404 for every bad token and a bare 422 for a short password.
        (ApiCall::ConfirmPasswordReset { .. }, ApiError::NotFound) => {
            ApiError::Invalid("This reset link is invalid or has expired. Request a new one.".into())
        }
        // 12 = embyr_core::admin::invitation::MIN_PASSWORD_CHARS (this crate does not depend on core).
        (ApiCall::ConfirmPasswordReset { .. }, ApiError::Invalid(_)) => {
            ApiError::Invalid("Password must be at least 12 characters".into())
        }
        (ApiCall::StartCheckout | ApiCall::OpenPortal, ApiError::Server(503)) => {
            ApiError::Invalid("Billing is not configured on this server".into())
        }
        (ApiCall::StartCheckout, ApiError::Conflict(_)) => ApiError::Conflict(
            "This account already has an active subscription. Use Manage billing to change it.".into(),
        ),
        (ApiCall::OpenPortal, ApiError::Conflict(_)) => {
            ApiError::Conflict("No billing account exists yet. Reload the page and try again.".into())
        }
        (ApiCall::DeleteAccount { .. }, ApiError::Upstream) => ApiError::Invalid(
            "Could not cancel the subscription with Stripe, so nothing was deleted. Try again.".into(),
        ),
        (ApiCall::AcceptInvitation { .. }, ApiError::NotFound) => ApiError::Invalid(
            "This invitation link is invalid or has expired. Ask an admin to send a new one.".into(),
        ),
        // 12 = embyr_core::admin::invitation::MIN_PASSWORD_CHARS; 100 = the server's display-name limit.
        (ApiCall::AcceptInvitation { .. }, ApiError::Invalid(_)) => ApiError::Invalid(
            "Enter a display name (up to 100 characters) and a password of at least 12 characters".into(),
        ),
        (ApiCall::ConfirmInvitation { .. }, ApiError::NotFound) => ApiError::Invalid(
            "Enrolment expired or was reset after too many wrong codes. Go back and set your password again.".into(),
        ),
        // Hosted identity is refused for agent-mode databases with a `{reason}` body.
        (ApiCall::EnableHostedIdentity(_), ApiError::Forbidden) if body.contains("HOSTED_IDENTITY_UNAVAILABLE") => {
            ApiError::Invalid("Hosted identity is not available for agent-mode databases".into())
        }
        (ApiCall::InviteSsoMember { .. }, ApiError::Invalid(_)) => ApiError::Invalid(
            "SSO invitations need an enabled SSO provider and a role below Owner. Check Settings, or invite with a password.".into(),
        ),
        (ApiCall::StartSso { .. }, ApiError::Conflict(_)) => {
            ApiError::Conflict("That SSO provider is already linked to your account".into())
        }
        // An SSO-only member has no password or authenticator to re-enter, so cannot self-link.
        (ApiCall::StartSso { intent: SsoIntent::Link { .. }, .. }, ApiError::Forbidden) => ApiError::Invalid(
            "Linking needs a password and authenticator code, which your account does not have".into(),
        ),
        (ApiCall::LookupInvitation { .. }, ApiError::NotFound) => ApiError::Invalid(
            "This invitation link is invalid or has expired. Ask an admin to send a new one.".into(),
        ),
        (ApiCall::UnlinkSso, ApiError::Conflict(_)) => ApiError::Conflict(
            "You sign in only with SSO, so the link cannot be removed. Ask an Owner to reset your access.".into(),
        ),
        (ApiCall::UnlinkSso, ApiError::NotFound) => {
            ApiError::Invalid("No SSO identity is linked to your account".into())
        }
        (ApiCall::CreateOidcProvider { .. }, ApiError::Conflict(_)) => {
            ApiError::Conflict("A provider with that issuer and client ID already exists".into())
        }
        (
            ApiCall::CreateOidcProvider { .. } | ApiCall::SetOidcProviderEnabled { .. } | ApiCall::RotateOidcSecret { .. },
            ApiError::Invalid(_),
        ) => ApiError::Invalid(provider_message(body)),
        (
            ApiCall::SetOidcProviderEnabled { .. } | ApiCall::RotateOidcSecret { .. } | ApiCall::DeleteOidcProvider(_),
            ApiError::NotFound,
        ) => ApiError::Invalid("That provider no longer exists. Refresh the list.".into()),
        (ApiCall::InviteSsoMember { .. }, ApiError::Conflict(_)) => {
            ApiError::Conflict("That email already belongs to a member or has an open invitation".into())
        }
        (ApiCall::InviteMember { .. }, ApiError::Conflict(_)) => {
            ApiError::Conflict("That email already belongs to a member or has an open invitation".into())
        }
        (ApiCall::InviteMember { .. }, ApiError::Invalid(_)) => {
            ApiError::Invalid("Enter a valid email address and choose a role".into())
        }
        (ApiCall::InviteMember { .. } | ApiCall::InviteSsoMember { .. }, ApiError::Forbidden) => {
            ApiError::Invalid("You cannot invite someone to a role above your own".into())
        }
        (ApiCall::ChangeMemberRole { .. }, ApiError::Forbidden) => ApiError::Invalid(
            "That change is not allowed: an Admin cannot change an Owner, and the last Owner cannot be demoted".into(),
        ),
        (ApiCall::RemoveMember(_) | ApiCall::RevokeInvitation(_), ApiError::Forbidden) => ApiError::Invalid(
            "That removal is not allowed: an Admin cannot remove an Owner".into(),
        ),
        (ApiCall::ChangeMemberRole { .. } | ApiCall::RemoveMember(_) | ApiCall::RevokeInvitation(_), ApiError::NotFound) => {
            ApiError::Invalid("That member no longer exists. Refresh the list.".into())
        }
        (ApiCall::CreateServiceAccount { .. }, ApiError::Invalid(_)) => {
            ApiError::Invalid("Enter a name and choose a role".into())
        }
        (ApiCall::DeleteServiceAccount(_), ApiError::NotFound) => {
            ApiError::Invalid("That service account no longer exists. Refresh the list.".into())
        }
        (ApiCall::CreateAdminKey { .. }, ApiError::Forbidden) => {
            ApiError::Invalid("You cannot create a key with a role above your own".into())
        }
        (ApiCall::CreateAdminKey { .. }, ApiError::Invalid(_)) => {
            ApiError::Invalid("Enter a name, choose a role, and pick an existing service account".into())
        }
        (ApiCall::RevokeAdminKey(_), ApiError::NotFound) => {
            ApiError::Invalid("That key no longer exists. Refresh the list.".into())
        }
        _ => error,
    }
}

/// User-facing text for a provider 422: `{error:"issuer_invalid", reason}` or `{error:"invalid_request", reason}`.
fn provider_message(body: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let reason = value.get("reason").and_then(|r| r.as_str()).unwrap_or("");
    match (value.get("error").and_then(|e| e.as_str()), reason) {
        (Some("issuer_invalid"), reason) => format!(
            "The issuer was rejected ({reason}). It must be an https OIDC issuer whose discovery document names itself."
        ),
        (_, "client_id_invalid") => "Enter a client ID (1 to 1024 bytes)".into(),
        (_, "client_secret_invalid") => "Enter a client secret (1 to 1024 bytes)".into(),
        _ => "The provider details were rejected".into(),
    }
}

/// User-facing text for the error codes `POST /projects` answers with (400 and 409).
fn create_message(code: &str, detail: Option<&str>) -> String {
    let with_detail = |text: &str| match detail {
        Some(d) => format!("{text}: {d}"),
        None => text.to_string(),
    };
    match code {
        "invalid_project_id_format" => {
            "Database id must start with a lowercase letter and use only lowercase letters, digits and hyphens (max 63)".into()
        }
        "invalid_name" => "Name must be 1-100 characters".into(),
        "dsn_required" => "A Postgres DSN is required".into(),
        "invalid_dsn" => with_detail("The DSN was rejected"),
        "backend_unavailable" => {
            "Could not connect to that Postgres. Check the DSN, network access and TLS.".into()
        }
        "customer_db_not_prepped" | "customer_db_schema_stale" => {
            with_detail("The target database is not prepared for Embyr")
        }
        "agent_endpoint_required"
        | "agent_ca_pem_required"
        | "agent_client_cert_pem_required"
        | "agent_client_key_pem_required" => {
            "Agent endpoint, CA certificate, client certificate and client key are all required".into()
        }
        "project_already_exists" => "A database with this id already exists".into(),
        "account_billing_hold" => {
            "This account is on a billing hold (past due or free-plan limit reached). Resolve it under Billing, then try again."
                .into()
        }
        other => other.to_string(),
    }
}

/// The `reset` token of a `?reset=<token>` query string, if present and non-empty.
/// The token is base64url, so no percent-decoding is needed.
pub fn reset_token_from_query(search: &str) -> Option<String> {
    query_param(search, "reset")
}

/// The `invite` token of a `?invite=<token>` query string, if present and non-empty (base64url).
pub fn invite_token_from_query(search: &str) -> Option<String> {
    query_param(search, "invite")
}

/// Stripe's return from Checkout: `?checkout=success` or `?checkout=cancelled`; anything else is ignored.
pub fn checkout_return_from_query(search: &str) -> Option<CheckoutReturn> {
    match query_param(search, "checkout")?.as_str() {
        "success" => Some(CheckoutReturn::Success),
        "cancelled" => Some(CheckoutReturn::Cancelled),
        _ => None,
    }
}

/// The provider id of a `?sso=<provider id>` sign-in link, if it is a well-formed UUID.
pub fn sso_provider_from_query(search: &str) -> Option<String> {
    query_param(search, "sso").and_then(|id| uuid_text(&id))
}

/// A provider id from what a person pasted: the bare id or a whole `/admin/?sso=<id>` link.
pub fn sso_provider_from_input(input: &str) -> Option<String> {
    let input = input.trim();
    uuid_text(input).or_else(|| {
        let query = input.split_once('?').map_or("", |(_, q)| q);
        sso_provider_from_query(query.split('#').next().unwrap_or(""))
    })
}

fn uuid_text(value: &str) -> Option<String> {
    uuid::Uuid::parse_str(value).ok().map(|u| u.to_string())
}

/// The SSO round trip's return markers: `?error=sso_failed` (callback failed) or `?sso_linked=1`.
pub fn sso_return_from_query(search: &str) -> Option<SsoReturn> {
    if query_param(search, "error").as_deref() == Some("sso_failed") {
        Some(SsoReturn::Failed)
    } else if query_param(search, "sso_linked").as_deref() == Some("1") {
        Some(SsoReturn::Linked)
    } else {
        None
    }
}

fn query_param(search: &str, key: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix(key).and_then(|rest| rest.strip_prefix('=')))
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn parse<T: for<'de> Deserialize<'de>>(body: &str) -> Result<T, ApiError> {
    serde_json::from_str(body).map_err(|e| ApiError::Decode(e.to_string()))
}

fn parse_ts(field: &str, value: &str) -> Result<DateTime<Utc>, ApiError> {
    DateTime::parse_from_rfc3339(value)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| ApiError::Decode(format!("{field}: {e}")))
}

fn parse_opt_ts(field: &str, value: Option<&str>) -> Result<Option<DateTime<Utc>>, ApiError> {
    value.map(|v| parse_ts(field, v)).transpose()
}

/// Server text from the three body shapes in use: `{message}`, `{error, detail}`, or empty.
fn server_message(body: &str, fallback: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    ["message", "error"]
        .iter()
        .find_map(|key| value.get(key).and_then(|v| v.as_str()))
        .filter(|m| !m.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn error_for(response: &HttpResponse) -> ApiError {
    let message = |fallback: &str| server_message(&response.body, fallback);
    match response.status {
        401 => ApiError::Unauthorized(message("Unauthorized")),
        403 => ApiError::Forbidden,
        404 => ApiError::NotFound,
        409 => ApiError::Conflict(message("Conflict")),
        400 | 422 => ApiError::Invalid(message("Invalid request")),
        429 => ApiError::Throttled { retry_after_ms: response.retry_after_ms },
        502 => ApiError::Upstream,
        status => ApiError::Server(status),
    }
}

// ── DTOs (private: only this module knows the API's field names) ────────────────

#[derive(Deserialize)]
struct SsoStartDto {
    #[serde(default)]
    authorization_url: Option<String>,
}

#[derive(Deserialize)]
struct LookupDto {
    auth_method: String,
    #[serde(default)]
    sso_provider_ids: Vec<String>,
}

#[derive(Deserialize)]
struct OidcProviderDto {
    id: String,
    issuer: String,
    client_id: String,
    enabled: bool,
}

#[derive(Deserialize)]
struct SessionDto {
    user_id: String,
    account_id: String,
    account_name: String,
    email: String,
    display_name: String,
    role: String,
}

impl TryFrom<SessionDto> for CurrentUser {
    type Error = ApiError;
    fn try_from(dto: SessionDto) -> Result<Self, ApiError> {
        Ok(CurrentUser {
            user_id: dto.user_id,
            account_id: dto.account_id,
            account_name: dto.account_name,
            email: dto.email,
            display_name: dto.display_name,
            role: parse_role(&dto.role)?,
        })
    }
}

/// G-08: the API's role strings are exactly `Owner | Admin | Viewer`.
fn parse_role(value: &str) -> Result<Role, ApiError> {
    match value {
        "Owner" => Ok(Role::Owner),
        "Admin" => Ok(Role::Admin),
        "Viewer" => Ok(Role::Viewer),
        other => Err(ApiError::Decode(format!("unknown role '{other}'"))),
    }
}

fn parse_plan(value: &str) -> Result<Plan, ApiError> {
    match value {
        "free" => Ok(Plan::Free),
        "pro" => Ok(Plan::Pro),
        other => Err(ApiError::Decode(format!("unknown plan '{other}'"))),
    }
}

fn parse_subscription_status(value: &str) -> Result<SubscriptionStatus, ApiError> {
    match value {
        "active" => Ok(SubscriptionStatus::Active),
        "past_due" => Ok(SubscriptionStatus::PastDue),
        "free_cap_exceeded" => Ok(SubscriptionStatus::FreeCapExceeded),
        "canceled" => Ok(SubscriptionStatus::Canceled),
        other => Err(ApiError::Decode(format!("unknown subscription status '{other}'"))),
    }
}

fn parse_backend_mode(value: &str) -> Result<DbBackendMode, ApiError> {
    match value {
        "direct_pg" => Ok(DbBackendMode::DirectPg),
        "agent" => Ok(DbBackendMode::AgentMode),
        "aws_secret" => Ok(DbBackendMode::AwsSecret),
        "gcp_secret" => Ok(DbBackendMode::GcpSecret),
        other => Err(ApiError::Decode(format!("unknown backend mode '{other}'"))),
    }
}

/// One DTO for the list entry and the detail: the detail-only fields default to absent.
#[derive(Deserialize)]
struct ProjectDto {
    id: String,
    name: Option<String>,
    status: String,
    backend_mode: String,
    logging_enabled: bool,
    created_at: String,
    #[serde(default)]
    log_retention_days: Option<u32>,
    #[serde(default)]
    api_key_rotated_at: Option<String>,
    #[serde(default)]
    api_key_previous_expires_at: Option<String>,
}

impl ProjectDto {
    fn api_key_info(&self) -> Result<ProjectApiKeyInfo, ApiError> {
        Ok(ProjectApiKeyInfo {
            rotated_at: parse_opt_ts("api_key_rotated_at", self.api_key_rotated_at.as_deref())?,
            previous_expires_at: parse_opt_ts(
                "api_key_previous_expires_at",
                self.api_key_previous_expires_at.as_deref(),
            )?,
        })
    }
}

impl TryFrom<ProjectDto> for Database {
    type Error = ApiError;
    fn try_from(dto: ProjectDto) -> Result<Self, ApiError> {
        let status = match dto.status.as_str() {
            "active" => DbStatus::Active,
            "suspended" => DbStatus::Suspended,
            "deleted" => DbStatus::Deleted,
            other => return Err(ApiError::Decode(format!("unknown project status '{other}'"))),
        };
        Ok(Database {
            backend_mode: parse_backend_mode(&dto.backend_mode)?,
            created_at: Some(parse_ts("created_at", &dto.created_at)?),
            // G-07: `name` is nullable; fall back to the id.
            name: dto.name.filter(|n| !n.is_empty()).unwrap_or_else(|| dto.id.clone()),
            id: DbId(dto.id),
            status,
            logging_enabled: dto.logging_enabled,
            log_retention: dto.log_retention_days.map(LogRetention::from_days),
            usage: Default::default(),
        })
    }
}

#[derive(Deserialize)]
struct LatencyStatsDto {
    p50_ms: Option<f64>,
    p95_ms: Option<f64>,
    p99_ms: Option<f64>,
    samples: u64,
    #[serde(default)]
    saturated: bool,
}

impl From<LatencyStatsDto> for LatencyStats {
    fn from(dto: LatencyStatsDto) -> Self {
        LatencyStats {
            p50_ms: dto.p50_ms,
            p95_ms: dto.p95_ms,
            p99_ms: dto.p99_ms,
            samples: dto.samples,
            saturated: dto.saturated,
        }
    }
}

#[derive(Deserialize)]
struct LatencyDto {
    read: LatencyStatsDto,
    write: LatencyStatsDto,
}

#[derive(Deserialize)]
struct SeriesPointDto {
    hour: String,
    reads: u64,
    writes: u64,
    deletes: u64,
    errors: u64,
}

#[derive(Deserialize)]
struct MetricsDto {
    reads_today: u64,
    writes_today: u64,
    deletes_today: u64,
    latency: LatencyDto,
    series: Vec<SeriesPointDto>,
}

impl MetricsDto {
    fn into_model(self, window: MetricsWindow) -> Result<DbMetrics, ApiError> {
        let series = self
            .series
            .into_iter()
            .map(|p| {
                Ok(SeriesPoint {
                    hour: parse_ts("series.hour", &p.hour)?,
                    reads: p.reads,
                    writes: p.writes,
                    deletes: p.deletes,
                    errors: p.errors,
                })
            })
            .collect::<Result<_, ApiError>>()?;
        Ok(DbMetrics {
            window,
            reads_today: self.reads_today,
            writes_today: self.writes_today,
            deletes_today: self.deletes_today,
            read_latency: self.latency.read.into(),
            write_latency: self.latency.write.into(),
            series,
        })
    }
}

#[derive(Deserialize)]
struct SummaryDto {
    project_id: String,
    reads_today: u64,
    writes_today: u64,
    deletes_today: u64,
    p95_read_ms: Option<f64>,
    p95_write_ms: Option<f64>,
}

impl SummaryDto {
    fn into_row(self) -> (DbId, MetricsSummaryRow) {
        (
            DbId(self.project_id),
            MetricsSummaryRow {
                reads_today: self.reads_today,
                writes_today: self.writes_today,
                deletes_today: self.deletes_today,
                p95_read_ms: self.p95_read_ms,
                p95_write_ms: self.p95_write_ms,
            },
        )
    }
}

/// The three summary shapes of `GET /connection`, told apart by their fields.
#[derive(Deserialize)]
#[serde(untagged)]
enum SummaryShapeDto {
    Dsn { host: String, port: u16, database: Option<String>, tls: bool },
    Agent { endpoint: String, cert_expires_at: Option<String> },
    Reference { reference: String },
}

#[derive(Deserialize)]
struct ConnectionDto {
    backend_mode: String,
    summary: Option<SummaryShapeDto>,
}

impl ConnectionDto {
    fn into_model(self) -> Result<Connection, ApiError> {
        let mode = parse_backend_mode(&self.backend_mode)?;
        let summary = match self.summary {
            None => ConnectionSummary::NotReadable,
            Some(SummaryShapeDto::Dsn { host, port, database, tls }) => {
                ConnectionSummary::Dsn { host, port, database, tls }
            }
            Some(SummaryShapeDto::Agent { endpoint, cert_expires_at }) => ConnectionSummary::Agent {
                endpoint,
                cert_expires_at: parse_opt_ts("cert_expires_at", cert_expires_at.as_deref())?,
            },
            Some(SummaryShapeDto::Reference { reference }) => ConnectionSummary::SecretRef(reference),
        };
        Ok(Connection { mode, summary })
    }
}

#[derive(Deserialize)]
struct ProvisionedDto {
    project_id: String,
    api_key: String,
}

#[derive(Deserialize)]
struct CapStatusDto {
    dimension: String,
    used: u64,
    cap: u64,
    pct: u64,
}

#[derive(Deserialize)]
struct SubscriptionDto {
    plan: String,
    status: String,
    stripe_customer_id: String,
    current_period_end: Option<String>,
    /// Present only for Free-plan accounts.
    #[serde(default)]
    cap_status: Option<Vec<CapStatusDto>>,
}

#[derive(Deserialize)]
struct UsageEntryDto {
    id: String,
    name: Option<String>,
    reads: u64,
    writes: u64,
    deletes: u64,
}

#[derive(Deserialize)]
struct UsageDto {
    databases: Vec<UsageEntryDto>,
}

#[derive(Deserialize)]
struct SessionUrlDto {
    url: String,
}

#[derive(Deserialize)]
struct DeletedAccountDto {
    purge_after: String,
}

#[derive(Deserialize)]
struct AcceptedInvitationDto {
    otpauth_uri: String,
    totp_secret: String,
}

#[derive(Deserialize)]
struct ConfirmedInvitationDto {
    recovery_codes: Vec<String>,
}

#[derive(Deserialize)]
struct RotatedKeyDto {
    api_key: String,
    rotated_at: String,
    previous_key_expires_at: Option<String>,
}

#[derive(Deserialize)]
struct QueryLogEntryDto {
    id: String,
    op: String,
    collection_path: Option<String>,
    latency_ms: Option<i64>,
    status: String,
    error_code: Option<String>,
    created_at: String,
}

#[derive(Deserialize)]
struct QueryLogsDto {
    total: u64,
    entries: Vec<QueryLogEntryDto>,
}

fn parse_uuid(field: &str, value: &str) -> Result<uuid::Uuid, ApiError> {
    uuid::Uuid::parse_str(value).map_err(|e| ApiError::Decode(format!("{field}: {e}")))
}

/// `auth_method == "pending"` marks an invitation row, whose id is the invitation's.
#[derive(Deserialize)]
struct MemberDto {
    id: String,
    email: String,
    display_name: Option<String>,
    role: String,
    auth_method: String,
    mfa_enabled: bool,
    #[serde(default)]
    last_login_at: Option<String>,
}

impl TryFrom<MemberDto> for Member {
    type Error = ApiError;
    fn try_from(dto: MemberDto) -> Result<Self, ApiError> {
        Ok(Member {
            id: UserId(parse_uuid("id", &dto.id)?),
            email: dto.email,
            display_name: dto.display_name,
            role: parse_role(&dto.role)?,
            pending: dto.auth_method == "pending",
            mfa_enabled: dto.mfa_enabled,
            last_login: parse_opt_ts("last_login_at", dto.last_login_at.as_deref())?,
        })
    }
}

#[derive(Deserialize)]
struct InvitationDto {
    invitation_id: String,
    email: String,
    role: String,
    invite_token: String,
}

#[derive(Deserialize)]
struct ServiceAccountDto {
    id: String,
    name: String,
    description: Option<String>,
    role: String,
    created_at: String,
}

impl TryFrom<ServiceAccountDto> for ServiceAccount {
    type Error = ApiError;
    fn try_from(dto: ServiceAccountDto) -> Result<Self, ApiError> {
        Ok(ServiceAccount {
            id: ServiceAccountId(parse_uuid("id", &dto.id)?),
            name: dto.name,
            description: dto.description.filter(|d| !d.is_empty()),
            role: parse_role(&dto.role)?,
            created_at: Some(parse_ts("created_at", &dto.created_at)?),
        })
    }
}

#[derive(Deserialize)]
struct AdminKeyDto {
    id: String,
    name: String,
    role: String,
    prefix: String,
    created_at: String,
    #[serde(default)]
    revoked_at: Option<String>,
}

impl TryFrom<AdminKeyDto> for AdminKey {
    type Error = ApiError;
    fn try_from(dto: AdminKeyDto) -> Result<Self, ApiError> {
        Ok(AdminKey {
            id: KeyId(parse_uuid("id", &dto.id)?),
            name: dto.name,
            service_account_id: None,
            member_id: None,
            role: parse_role(&dto.role)?,
            prefix: dto.prefix,
            created_at: Some(parse_ts("created_at", &dto.created_at)?),
        })
    }
}

#[derive(Deserialize)]
struct CreatedAdminKeyDto {
    id: String,
    name: String,
    key: String,
    prefix: String,
    created_at: String,
}
