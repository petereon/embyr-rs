//! `accounts:signUp()` hosted-identity signup endpoint (US-02, ADR-036
//! Decision 6).
//!
//! POST /v1/projects/{project_id}/accounts:signUp?key={api_key}
//!   Body:  { "email": "...", "password": "..." }
//!   201:   { "localId": "<end_user_id>", "email": "...", "idToken": "<token>", "expiresIn": "<seconds>" }
//!   400:   { "reason": "MISSING_FIELD" | "WEAK_PASSWORD" | "HOSTED_IDENTITY_NOT_ENABLED" }
//!   401:   { "reason": "INVALID_API_KEY" }
//!   409:   { "reason": "EMAIL_ALREADY_IN_USE" }
//!
//! Status code: ADR-036 Decision 6's own worked example table shows 200, but
//! separately confirms 201 is equally defensible for a resource-creation
//! POST. This handler uses 201 — matching this feature's own Slice 01
//! (`enable_hosted_identity` -> 201) and standard REST creation semantics.
//!
//! Response shape deviation from ADR-036 Decision 6's literal table: this
//! handler adds an `idToken` field the ADR's table omits. AC-18-05 requires
//! that "a SUBSEQUENT real Firestore call ... carrying THE RETURNED TOKEN"
//! succeeds — but embyr mints a brand-new token server-side here that the
//! caller has never seen (unlike `signInWithCustomToken`, where the token
//! IS the request body and returning it again would be redundant). Without
//! an `idToken` field in the response, the caller would have no way to
//! authenticate the very next call, making AC-18-05 unsatisfiable as
//! literally specified. This mirrors real Firebase Identity Toolkit's own
//! `accounts:signUp` response field name.
//!
//! `api_key` is REQUIRED via `?key=` (unlike `signInWithCustomToken`, which
//! has no auth header of its own) — BC-5 must resolve a Customer DB
//! connection to write `hosted_identity_accounts`, which requires
//! Argon2id-authenticating the caller (ADR-036 Decision 6).
//!
//! Transport shape (exact URL path, `?key=` placement) is provisional
//! pending `OQ-CHI-01`'s required DISTILL/DELIVER-wave empirical spike
//! against the real Firebase JS SDK (mirrors `rest/sign_in.rs`'s own
//! `OQ-CA-01` caveat) — NOT run for this slice, per this feature's own
//! established precedent (`client-auth`'s `OQ-CA-01`): the logical contract
//! below is implementation-ready regardless of the spike's outcome.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use embyr_core::{
    auth::{argon2, ecies},
    client_identity, hosted_identity,
};
use serde::{Deserialize, Serialize};

use crate::adapters::{
    aws_secret_fetcher::AwsSecretFetcher,
    credential_cache::CredentialCache,
    gcp_secret_fetcher::GcpSecretFetcher,
    project_auth::{resolve_customer_db_adapter, ProjectAuthError},
    system_db::SystemDb,
};

/// Hosted-identity-minted token lifetime (v1: fixed, no configurability —
/// no story in this feature's locked scope requires it). `pub(crate)`: Slice
/// 03's `sign_in_with_password` mints a token with the identical TTL — one
/// source of truth, not a second hand-copied constant.
pub(crate) const TOKEN_TTL_SECS: i64 = 3600;

/// State for hosted-identity's own data-plane REST routes (signup here;
/// signin/reset-request/reset-confirm are later slices' own additions to
/// this SAME struct — ADR-036 Decision 6's own minimality discipline,
/// mirrors `SignInState`). Wired alongside `sign_in_state`, merged into the
/// same `:8081` axum_app — no new listener, no new port.
#[derive(Clone)]
pub struct HostedIdentityState {
    pub system_db: Arc<SystemDb>,
    pub credential_cache: Arc<CredentialCache>,
    pub aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
    pub gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
    /// Slice 04: reset-request's own driven port for "if this account
    /// exists, a reset was sent" (ADR-036, ADR-011). V1: `NoopEmailSender`.
    pub email_sender: Arc<dyn embyr_core::admin::email::IEmailSender + Send + Sync>,
    /// pool-sizing-and-limits (ADR-079): threaded into
    /// `resolve_customer_db_adapter`'s `with_pool_config` call — same
    /// `EMBYR_TENANT_DB_MAX_CONNECTIONS`-sourced value the gRPC path uses.
    pub tenant_db_max_connections: u32,
    /// pool-sizing-and-limits (ADR-079): same as above, for `acquire_timeout`.
    pub tenant_db_acquire_timeout: std::time::Duration,
}

#[derive(Deserialize)]
pub struct SignUpQuery {
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Deserialize)]
pub struct SignUpBody {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Serialize)]
pub struct SignUpSuccessResponse {
    #[serde(rename = "localId")]
    pub local_id: String,
    pub email: String,
    #[serde(rename = "idToken")]
    pub id_token: String,
    #[serde(rename = "expiresIn")]
    pub expires_in: String,
}

#[derive(Serialize)]
pub struct SignUpFailureResponse {
    pub reason: &'static str,
}

fn failure(status: StatusCode, reason: &'static str) -> Response {
    (status, Json(SignUpFailureResponse { reason })).into_response()
}

/// `pub(crate)`: Slice 03's `sign_in_with_password` reuses this exact
/// response shape (same failure taxonomy, same route family) rather than
/// hand-rolling a second `{"reason": "INVALID_API_KEY"}` constructor.
pub(crate) fn invalid_api_key() -> Response {
    failure(StatusCode::UNAUTHORIZED, "INVALID_API_KEY")
}

/// `pub(crate)`: see `invalid_api_key` — reused by `sign_in_with_password`
/// for AC-18-12.
pub(crate) fn hosted_identity_not_enabled() -> Response {
    failure(StatusCode::BAD_REQUEST, "HOSTED_IDENTITY_NOT_ENABLED")
}

/// POST /v1/projects/{project_id}/accounts:signUp
///
/// The route pattern (`accounts:signUp`) contains a literal `:` inside the
/// last path segment — `matchit` treats it as a second named path
/// parameter, so `Path<HashMap<String, String>>` is required (identical
/// reasoning to `rest/sign_in.rs::sign_in_with_custom_token`'s own doc
/// comment).
pub async fn sign_up(
    Path(params): Path<HashMap<String, String>>,
    State(state): State<HostedIdentityState>,
    Query(query): Query<SignUpQuery>,
    Json(body): Json<SignUpBody>,
) -> Response {
    let project_id = params.get("project_id").cloned().unwrap_or_default();

    let Some(api_key) = query.key.filter(|k| !k.is_empty()) else {
        return invalid_api_key();
    };

    // Verifies api_key against api_key_hash_current AND resolves the
    // Customer DB connection needed to write hosted_identity_accounts
    // (ADR-036 Decision 7) — one call serves both purposes.
    let adapter = match resolve_customer_db_adapter(
        &state.system_db,
        &state.credential_cache,
        state.aws_secret_fetcher.as_deref(),
        state.gcp_secret_fetcher.as_deref(),
        &project_id,
        &api_key,
        state.tenant_db_max_connections,
        state.tenant_db_acquire_timeout,
    )
    .await
    {
        Ok(adapter) => adapter,
        Err(ProjectAuthError::ProjectNotFound | ProjectAuthError::InvalidApiKey) => {
            return invalid_api_key();
        }
        Err(ProjectAuthError::HostedIdentityUnavailable) => {
            return hosted_identity_not_enabled();
        }
        Err(ProjectAuthError::Internal(e)) => {
            tracing::error!("sign_up: resolve_customer_db_adapter failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // AC-18-08: hosted identity must be enabled for this project (US-01 must
    // have run) — distinguishable from an INVALID_API_KEY credential
    // failure, already ruled out above.
    let hosted_key_row = match state
        .system_db
        .get_hosted_identity_signing_key(&project_id)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return hosted_identity_not_enabled(),
        Err(e) => {
            tracing::error!("sign_up: get_hosted_identity_signing_key failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // AC-18-09: `password` (the raw plaintext) is used ONLY as bytes to hash
    // below — it is NEVER passed to a tracing/log macro and NEVER appears in
    // any response body on any branch of this handler.
    let (Some(email), Some(password)) = (
        body.email.filter(|e| !e.is_empty()),
        body.password.filter(|p| !p.is_empty()),
    ) else {
        return failure(StatusCode::BAD_REQUEST, "MISSING_FIELD");
    };

    if hosted_identity::validate_password_strength(&password).is_err() {
        return failure(StatusCode::BAD_REQUEST, "WEAK_PASSWORD");
    }

    let password_hash = match argon2::hash_password(password.as_bytes()) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("sign_up: hash_password failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // AC-18-06: PRIMARY KEY (project_id, email) makes a duplicate email a
    // database-enforced unique-violation (23505) -> 409, not an app-level
    // check that could drift from the schema.
    let end_user_id: uuid::Uuid = match sqlx::query_scalar(
        "INSERT INTO hosted_identity_accounts (project_id, email, password_hash) \
         VALUES ($1, $2, $3) RETURNING end_user_id",
    )
    .bind(&project_id)
    .bind(&email)
    .bind(&password_hash)
    .fetch_one(adapter.pool())
    .await
    {
        Ok(id) => id,
        Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
            return failure(StatusCode::CONFLICT, "EMAIL_ALREADY_IN_USE");
        }
        Err(e) => {
            tracing::error!("sign_up: insert hosted_identity_accounts failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Decrypt the project's embyr-owned signing key seed — System DB via
    // SystemDb, NOT the Customer DB adapter (ADR-036 Decision 2/7).
    let seed_bytes = match ecies::decrypt(api_key.as_bytes(), &hosted_key_row.private_key_enc) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("sign_up: ecies decrypt signing key failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let seed: [u8; 32] = match seed_bytes.as_slice().try_into() {
        Ok(s) => s,
        Err(_) => {
            tracing::error!("sign_up: decrypted signing key seed is not 32 bytes");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let end_user_id_str = end_user_id.to_string();
    let expires_at_unix = chrono::Utc::now().timestamp() + TOKEN_TTL_SECS;
    let token = client_identity::mint_client_identity_token(
        &seed,
        &end_user_id_str,
        &project_id,
        expires_at_unix,
    );

    (
        StatusCode::CREATED,
        Json(SignUpSuccessResponse {
            local_id: end_user_id_str,
            email,
            id_token: token,
            expires_in: TOKEN_TTL_SECS.to_string(),
        }),
    )
        .into_response()
}
