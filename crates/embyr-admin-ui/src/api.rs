//! api — the single anti-corruption layer between the admin API and the TEA core (ADR-082).
//!
//! Host-compiled, no browser types. This is the only module that knows routes and JSON field names:
//! `to_request` turns an intent (`ApiCall`) into plain request data, and `decode` turns a response
//! into the `Msg`s `update` applies. The `csr` runtime in `app.rs` only moves bytes in between.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::model::{CurrentUser, Database, DbBackendMode, DbId, DbStatus, Role};
use crate::msg::Msg;

/// Which call is in flight (the `model.pending` key).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CallKind {
    SignIn,
    Session,
    SignOut,
    Projects,
}

/// How the second factor was supplied. Exactly one reaches the server.
#[derive(Clone, PartialEq, Eq)]
pub enum SignInCode {
    Totp(String),
    Recovery(String),
}

/// A user intent that needs the server. Secrets live only inside this value; `Debug` redacts them.
#[derive(Clone, PartialEq, Eq)]
pub enum ApiCall {
    SignIn { email: String, password: String, code: SignInCode },
    LoadSession,
    SignOut,
    LoadProjects,
}

impl ApiCall {
    pub fn kind(&self) -> CallKind {
        match self {
            ApiCall::SignIn { .. } => CallKind::SignIn,
            ApiCall::LoadSession => CallKind::Session,
            ApiCall::SignOut => CallKind::SignOut,
            ApiCall::LoadProjects => CallKind::Projects,
        }
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
            ApiCall::LoadSession => f.write_str("LoadSession"),
            ApiCall::SignOut => f.write_str("SignOut"),
            ApiCall::LoadProjects => f.write_str("LoadProjects"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
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
    match call {
        ApiCall::SignIn { email, password, code } => {
            let mut body = serde_json::json!({ "email": email, "password": password });
            match code {
                SignInCode::Totp(c) => body["totp_code"] = c.as_str().into(),
                SignInCode::Recovery(c) => body["recovery_code"] = c.as_str().into(),
            }
            HttpRequest { method: Method::Post, path: "/admin/v1/auth/signin".into(), body: Some(body) }
        }
        ApiCall::LoadSession => get("/admin/v1/auth/session"),
        ApiCall::SignOut => HttpRequest {
            method: Method::Post,
            path: "/admin/v1/auth/signout".into(),
            body: None,
        },
        ApiCall::LoadProjects => get("/admin/v1/projects"),
    }
}

/// Response -> `Msg`s. The first message always settles the call (clears `pending`); failures follow
/// as `ApiFailed`, successes as the result messages `update` applies.
pub fn decode(call: &ApiCall, response: HttpResponse) -> Vec<Msg> {
    let result = if (200..300).contains(&response.status) {
        decode_success(call, &response.body)
    } else {
        Err(error_for(&response))
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
        ApiCall::LoadSession => Ok(vec![Msg::SessionEstablished(parse::<SessionDto>(body)?.try_into()?)]),
        ApiCall::SignOut => Ok(vec![Msg::SignOut]),
        ApiCall::LoadProjects => {
            let projects: Vec<ProjectDto> = parse(body)?;
            let databases = projects.into_iter().map(Database::try_from).collect::<Result<_, _>>()?;
            Ok(vec![Msg::SetDatabases(databases)])
        }
    }
}

fn parse<T: for<'de> Deserialize<'de>>(body: &str) -> Result<T, ApiError> {
    serde_json::from_str(body).map_err(|e| ApiError::Decode(e.to_string()))
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

#[derive(Deserialize)]
struct ProjectDto {
    id: String,
    name: Option<String>,
    status: String,
    backend_mode: String,
    logging_enabled: bool,
    created_at: String,
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
        let backend_mode = match dto.backend_mode.as_str() {
            "direct_pg" => DbBackendMode::DirectPg,
            "agent" => DbBackendMode::AgentMode,
            "aws_secret" => DbBackendMode::AwsSecret,
            "gcp_secret" => DbBackendMode::GcpSecret,
            other => return Err(ApiError::Decode(format!("unknown backend mode '{other}'"))),
        };
        let created_at = DateTime::parse_from_rfc3339(&dto.created_at)
            .map_err(|e| ApiError::Decode(format!("created_at: {e}")))?
            .with_timezone(&Utc);
        Ok(Database {
            // G-07: `name` is nullable; fall back to the id.
            name: dto.name.filter(|n| !n.is_empty()).unwrap_or_else(|| dto.id.clone()),
            id: DbId(dto.id),
            status,
            backend_mode,
            logging_enabled: dto.logging_enabled,
            log_retention: None,
            created_at: Some(created_at),
            usage: Default::default(),
        })
    }
}
