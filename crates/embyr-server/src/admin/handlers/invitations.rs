//! Account bootstrap and invitation redemption (account-bootstrap-and-invitations).
//!
//! create_account   POST   /admin/v1/accounts                       operator Bearer
//! revoke_invitation DELETE /admin/v1/members/invitations/:id        session Admin+
//! accept_invitation POST  /admin/v1/auth/invitations/accept         public, rate-limited
//! confirm_invitation POST /admin/v1/auth/invitations/confirm        public, rate-limited
//!
//! One enrolment path for the first Owner and every invitee: an invitation
//! token is redeemed in two steps — accept (password, TOTP secret issued) then
//! confirm (TOTP proven; user + membership + recovery codes created in one
//! transaction). Only the BLAKE3 hash of the token is stored.

use axum::{
    extract::{ConnectInfo, Json, Path, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgRow, PgConnection, Row};
use std::net::SocketAddr;
use totp_rs::{Algorithm as TotpAlgorithm, TOTP};
use uuid::Uuid;

use embyr_core::admin::account::Role;
use embyr_core::admin::email::EmailMessage;
use embyr_core::admin::invitation::{
    base32_encode, generate_recovery_codes, generate_token, hash_token, is_plausible_email,
    is_well_formed_token, otpauth_uri, parse_role, password_acceptable, wrong_code_outcome,
    WrongCodeOutcome,
};

use crate::adapters::encryption::{decrypt_with_rotation, encrypt};
use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;

// ── Shared helpers ────────────────────────────────────────────────────────────

fn message(status: StatusCode, text: &str) -> Response {
    (status, Json(serde_json::json!({ "message": text }))).into_response()
}

/// AC-ABI-07: unknown / expired / revoked / used / malformed tokens are indistinguishable.
fn invalid_invitation() -> Response {
    message(StatusCode::NOT_FOUND, "Invalid or expired invitation")
}

fn internal(context: &str, e: impl std::fmt::Display) -> Response {
    tracing::error!("invitations: {context}: {e}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c == "23505")
}

// `Response` is the handler's error type everywhere; same allowance as client_identity.rs.
#[allow(clippy::result_large_err)]
fn col<T>(row: &PgRow, name: &str) -> Result<T, Response>
where
    T: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    row.try_get(name)
        .map_err(|e| internal(&format!("read column {name}"), e))
}

/// AC-ABI-15: per-source-IP limit before any Argon2 / DB work. Shares the
/// signin limiter under a distinct `i:` key prefix (keeps within the
/// `signin_rate_limits.source_key` VARCHAR(45) bound for IPv6).
async fn throttle(state: &UserAdminState, peer: SocketAddr) -> Option<Response> {
    let key = format!("i:{}", peer.ip());
    let retry_after_ms = state.signin_rate_limiter.check(&key).await.err()?;
    let mut response = message(
        StatusCode::TOO_MANY_REQUESTS,
        "Too many attempts — try again later",
    );
    if let Ok(v) = HeaderValue::from_str(&retry_after_ms.to_string()) {
        response.headers_mut().insert("retry-after-ms", v);
    }
    Some(response)
}

pub(super) struct IssuedInvitation {
    pub id: Uuid,
    pub token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

pub(super) enum IssueError {
    /// The email already belongs to a user or has an open invitation.
    EmailTaken,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for IssueError {
    fn from(e: sqlx::Error) -> Self {
        if is_unique_violation(&e) {
            IssueError::EmailTaken
        } else {
            IssueError::Db(e)
        }
    }
}

/// Create a redeemable invitation inside the caller's transaction.
/// Expired-but-open invitations for the same email are revoked first so they
/// cannot block re-issue (the open-email unique index cannot reference now()).
pub(super) async fn issue_invitation(
    conn: &mut PgConnection,
    account_id: Uuid,
    email: &str,
    role: &str,
    invited_by: Option<Uuid>,
) -> Result<IssuedInvitation, IssueError> {
    sqlx::query(
        "UPDATE invitations SET revoked_at = now() \
         WHERE lower(email) = lower($1) AND accepted_at IS NULL \
           AND revoked_at IS NULL AND expires_at <= now()",
    )
    .bind(email)
    .execute(&mut *conn)
    .await?;

    let user_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE lower(email) = lower($1))")
            .bind(email)
            .fetch_one(&mut *conn)
            .await?;
    if user_exists {
        return Err(IssueError::EmailTaken);
    }

    let token = generate_token();
    let row = sqlx::query(
        "INSERT INTO invitations (account_id, email, role, expires_at, token_hash, invited_by) \
         VALUES ($1, $2, $3, now() + interval '7 days', $4, $5) \
         RETURNING id, expires_at",
    )
    .bind(account_id)
    .bind(email)
    .bind(role)
    .bind(hash_token(&token))
    .bind(invited_by)
    .fetch_one(&mut *conn)
    .await?;
    Ok(IssuedInvitation {
        id: row.try_get("id")?,
        token,
        expires_at: row.try_get("expires_at")?,
    })
}

/// Fire-and-forget: delivery failures are logged, never surfaced (the inviter
/// still receives the token). The link is relative to the admin host.
pub(super) async fn send_invite_email(
    state: &UserAdminState,
    to: &str,
    role: &str,
    issued: &IssuedInvitation,
) {
    let result = state
        .email_sender
        .send(EmailMessage {
            to: to.to_string(),
            subject: format!("You've been invited to join as {role}"),
            body_text: format!(
                "You have been invited to join with role '{role}'. \
                 Open /admin/?invite={} on your embyr admin host to set a password \
                 and enrol an authenticator. This invitation expires at {}.",
                issued.token, issued.expires_at
            ),
            body_html: None,
        })
        .await;
    if let Err(e) = result {
        tracing::warn!("invitations: invite email delivery failed: {e}");
    }
}

// ── POST /admin/v1/accounts (operator) ────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateAccountBody {
    pub account_name: String,
    pub owner_email: String,
}

#[derive(Serialize)]
struct CreateAccountResponse {
    account_id: String,
    invitation_id: String,
    invite_token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

/// Creates an account plus a single Owner invitation; no user exists until the
/// token is redeemed. 409 if the email already has a user or open invitation.
pub async fn create_account(
    State(state): State<UserAdminState>,
    Json(body): Json<CreateAccountBody>,
) -> Response {
    let name = body.account_name.trim();
    if name.is_empty() || name.chars().count() > 200 || !is_plausible_email(&body.owner_email) {
        return StatusCode::UNPROCESSABLE_ENTITY.into_response();
    }
    let pool = state.system_db.pool();
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => return internal("begin", e),
    };
    let account_id: Uuid =
        match sqlx::query_scalar("INSERT INTO accounts (name) VALUES ($1) RETURNING id")
            .bind(name)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(id) => id,
            Err(e) => return internal("insert account", e),
        };
    let issued =
        match issue_invitation(&mut tx, account_id, &body.owner_email, "Owner", None).await {
            Ok(i) => i,
            Err(IssueError::EmailTaken) => {
                return message(StatusCode::CONFLICT, "Email already registered or invited")
            }
            Err(IssueError::Db(e)) => return internal("issue owner invitation", e),
        };
    if let Err(e) = tx.commit().await {
        return internal("commit", e);
    }
    send_invite_email(&state, &body.owner_email, "Owner", &issued).await;
    (
        StatusCode::CREATED,
        Json(CreateAccountResponse {
            account_id: account_id.to_string(),
            invitation_id: issued.id.to_string(),
            invite_token: issued.token,
            expires_at: issued.expires_at,
        }),
    )
        .into_response()
}

// ── DELETE /admin/v1/members/invitations/:id ──────────────────────────────────

/// Pending invitation of the caller's account only; an Admin may not revoke an
/// Owner invitation. Foreign / unknown / already-closed id → 404.
pub async fn revoke_invitation(
    Path(id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Response {
    if session.role < Role::Admin {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(invitation_id) = Uuid::parse_str(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let pool = state.system_db.pool();
    let role: Option<String> = match sqlx::query_scalar(
        "SELECT role FROM invitations \
         WHERE id = $1 AND account_id = $2 AND accepted_at IS NULL AND revoked_at IS NULL",
    )
    .bind(invitation_id)
    .bind(session.account_id)
    .fetch_optional(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => return internal("select invitation", e),
    };
    let Some(role) = role else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if parse_role(&role).is_some_and(|r| r > session.role) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Err(e) = sqlx::query(
        "UPDATE invitations SET revoked_at = now() \
         WHERE id = $1 AND accepted_at IS NULL AND revoked_at IS NULL",
    )
    .bind(invitation_id)
    .execute(pool)
    .await
    {
        return internal("revoke invitation", e);
    }
    StatusCode::NO_CONTENT.into_response()
}

// ── POST /admin/v1/auth/invitations/accept ────────────────────────────────────

// No `Debug` derive: the request carries the password and the token.
#[derive(Deserialize)]
pub struct AcceptBody {
    pub token: String,
    pub display_name: String,
    pub password: String,
}

#[derive(Serialize)]
struct AcceptResponse {
    otpauth_uri: String,
    totp_secret: String,
    enrolment_expires_at: chrono::DateTime<chrono::Utc>,
}

const OPEN_INVITATION: &str = "token_hash = $1 AND accepted_at IS NULL \
                               AND revoked_at IS NULL AND expires_at > now()";

/// Step 1: validate the token, hash the password (under the process-wide
/// Argon2 gate inside `embyr_core::auth::argon2`), issue a TOTP secret and
/// store a pending enrolment. No user, membership or session is created.
pub async fn accept_invitation(
    State(state): State<UserAdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<AcceptBody>,
) -> Response {
    if let Some(throttled) = throttle(&state, peer).await {
        return throttled;
    }
    let display_name = body.display_name.trim().to_string();
    if display_name.is_empty()
        || display_name.chars().count() > 100
        || !password_acceptable(&body.password)
    {
        return StatusCode::UNPROCESSABLE_ENTITY.into_response();
    }
    if !is_well_formed_token(&body.token) {
        return invalid_invitation();
    }
    match accept_inner(&state, &body.token, display_name, body.password).await {
        Ok(r) | Err(r) => r,
    }
}

async fn accept_inner(
    state: &UserAdminState,
    token: &str,
    display_name: String,
    password: String,
) -> Result<Response, Response> {
    let pool = state.system_db.pool();
    let invitation = sqlx::query(&format!("SELECT id, email FROM invitations WHERE {OPEN_INVITATION}"))
        .bind(hash_token(token))
        .fetch_optional(pool)
        .await
        .map_err(|e| internal("lookup invitation", e))?
        .ok_or_else(invalid_invitation)?;
    let invitation_id: Uuid = col(&invitation, "id")?;
    let email: String = col(&invitation, "email")?;

    let password_hash = tokio::task::spawn_blocking(move || {
        embyr_core::auth::argon2::hash_password(password.as_bytes())
    })
    .await
    .map_err(|e| internal("spawn_blocking join (hash password)", e))?
    .map_err(|e| internal("hash password", e))?;

    let mut secret = [0u8; 20];
    OsRng.fill_bytes(&mut secret);
    let secret_enc =
        encrypt(&state.encryption_key, &secret).map_err(|e| internal("encrypt totp secret", e))?;

    let expires_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "INSERT INTO pending_enrolments \
         (invitation_id, display_name, password_hash, totp_secret_enc, failed_attempts, expires_at) \
         VALUES ($1, $2, $3, $4, 0, now() + interval '30 minutes') \
         ON CONFLICT (invitation_id) DO UPDATE SET \
           display_name = EXCLUDED.display_name, password_hash = EXCLUDED.password_hash, \
           totp_secret_enc = EXCLUDED.totp_secret_enc, failed_attempts = 0, \
           expires_at = EXCLUDED.expires_at \
         RETURNING expires_at",
    )
    .bind(invitation_id)
    .bind(&display_name)
    .bind(&password_hash)
    .bind(&secret_enc)
    .fetch_one(pool)
    .await
    .map_err(|e| internal("upsert pending enrolment", e))?;

    Ok((
        StatusCode::OK,
        Json(AcceptResponse {
            otpauth_uri: otpauth_uri(&secret, &email),
            totp_secret: base32_encode(&secret),
            enrolment_expires_at: expires_at,
        }),
    )
        .into_response())
}

// ── POST /admin/v1/auth/invitations/confirm ───────────────────────────────────

#[derive(Deserialize)]
pub struct ConfirmBody {
    pub token: String,
    pub totp_code: String,
}

#[derive(Serialize)]
struct ConfirmResponse {
    recovery_codes: Vec<String>,
    email: String,
}

/// Step 2: prove the TOTP secret. One transaction creates the user, the
/// membership and the recovery codes and closes the invitation; the invitation
/// row lock serialises concurrent redemptions (exactly one user is created) and
/// the wrong-code counter.
pub async fn confirm_invitation(
    State(state): State<UserAdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<ConfirmBody>,
) -> Response {
    if let Some(throttled) = throttle(&state, peer).await {
        return throttled;
    }
    if !is_well_formed_token(&body.token) {
        return invalid_invitation();
    }
    match confirm_inner(&state, &body).await {
        Ok(r) | Err(r) => r,
    }
}

async fn confirm_inner(state: &UserAdminState, body: &ConfirmBody) -> Result<Response, Response> {
    let mut tx = state
        .system_db
        .pool()
        .begin()
        .await
        .map_err(|e| internal("begin", e))?;

    let invitation = sqlx::query(&format!(
        "SELECT id, account_id, email, role FROM invitations WHERE {OPEN_INVITATION} FOR UPDATE"
    ))
    .bind(hash_token(&body.token))
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| internal("lock invitation", e))?
    .ok_or_else(invalid_invitation)?;
    let invitation_id: Uuid = col(&invitation, "id")?;
    let account_id: Uuid = col(&invitation, "account_id")?;
    let email: String = col(&invitation, "email")?;
    let role: String = col(&invitation, "role")?;

    // Confirm without a live (accepted, unexpired) enrolment looks like any invalid token.
    let enrolment = sqlx::query(
        "SELECT display_name, password_hash, totp_secret_enc, failed_attempts \
         FROM pending_enrolments WHERE invitation_id = $1 AND expires_at > now()",
    )
    .bind(invitation_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| internal("load enrolment", e))?
    .ok_or_else(invalid_invitation)?;
    let display_name: String = col(&enrolment, "display_name")?;
    let password_hash: String = col(&enrolment, "password_hash")?;
    let secret_enc: Vec<u8> = col(&enrolment, "totp_secret_enc")?;
    let failed_attempts: i32 = col(&enrolment, "failed_attempts")?;

    let secret = decrypt_with_rotation(
        &state.encryption_key,
        state.encryption_key_previous.as_ref(),
        &secret_enc,
    )
    .map_err(|e| internal("decrypt totp secret", e))?;
    let totp = TOTP::new(TotpAlgorithm::SHA1, 6, 1, 30, secret, None, String::new())
        .map_err(|e| internal("build TOTP", e))?;

    if !totp.check_current(&body.totp_code).unwrap_or(false) {
        let update = match wrong_code_outcome(failed_attempts) {
            WrongCodeOutcome::Invalidate => {
                sqlx::query("DELETE FROM pending_enrolments WHERE invitation_id = $1")
            }
            WrongCodeOutcome::Retry => sqlx::query(
                "UPDATE pending_enrolments SET failed_attempts = failed_attempts + 1 \
                 WHERE invitation_id = $1",
            ),
        };
        update
            .bind(invitation_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| internal("record wrong code", e))?;
        tx.commit().await.map_err(|e| internal("commit", e))?;
        return Ok(message(StatusCode::UNAUTHORIZED, "Invalid or expired code"));
    }

    let user_id: Uuid = match sqlx::query_scalar(
        "INSERT INTO users (account_id, email, display_name, password_hash, totp_secret_enc) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(account_id)
    .bind(&email)
    .bind(&display_name)
    .bind(&password_hash)
    .bind(&secret_enc)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(id) => id,
        Err(e) if is_unique_violation(&e) => {
            return Ok(message(StatusCode::CONFLICT, "Email already registered"))
        }
        Err(e) => return Err(internal("insert user", e)),
    };
    sqlx::query(
        "INSERT INTO account_members (user_id, account_id, role, joined_at) \
         VALUES ($1, $2, $3, now())",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(&role)
    .execute(&mut *tx)
    .await
    .map_err(|e| internal("insert membership", e))?;

    let recovery_codes = generate_recovery_codes();
    let code_hashes: Vec<Vec<u8>> = recovery_codes.iter().map(|c| hash_token(c)).collect();
    sqlx::query(
        "INSERT INTO mfa_recovery_codes (user_id, code_hash) SELECT $1, unnest($2::bytea[])",
    )
    .bind(user_id)
    .bind(&code_hashes)
    .execute(&mut *tx)
    .await
    .map_err(|e| internal("insert recovery codes", e))?;

    sqlx::query("UPDATE invitations SET accepted_at = now() WHERE id = $1")
        .bind(invitation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| internal("close invitation", e))?;
    sqlx::query("DELETE FROM pending_enrolments WHERE invitation_id = $1")
        .bind(invitation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| internal("delete enrolment", e))?;
    tx.commit().await.map_err(|e| internal("commit", e))?;

    Ok((
        StatusCode::OK,
        Json(ConfirmResponse {
            recovery_codes,
            email,
        }),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use axum::{
        routing::{delete, patch, post},
        Router,
    };

    /// matchit 0.7 aborts router construction on conflicting routes; the new
    /// invitation routes must coexist with the `:member_id` ones.
    #[test]
    fn invitation_routes_coexist_with_member_routes() {
        let _: Router = Router::new()
            .route("/admin/v1/members/invite", post(|| async {}))
            .route("/admin/v1/members/invitations/:id", delete(|| async {}))
            .route("/admin/v1/members/:member_id/role", patch(|| async {}))
            .route("/admin/v1/members/:member_id", delete(|| async {}));
    }
}
