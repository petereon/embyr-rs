//! Admin SSO links (ADR-086 decision 5): the only code that creates or removes an
//! `admin_sso_identities` row besides provider/member cleanup.
//!
//! - `complete_invite`: callback intent `invite` (slice 4). One transaction, owned by the caller,
//!   creates the SSO-only user, the member, the link and closes the invitation.
//! - `complete_link`: callback intent `link` (slice 5). Links `(provider, sub)` to the user that
//!   re-authenticated at `start`.
//! - `DELETE /admin/v1/auth/sso_identity`: a member removes their own link(s); refused for an
//!   SSO-only user (409) so nobody can lock themselves out.
//!
//! E-mail is compared only against the invitation the invitee already holds; it is never used to
//! find an existing user (a clash with another user's e-mail fails and creates nothing). Every
//! failure is a static reason label that goes to the audit row; the browser sees `sso_failed`.

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use embyr_core::admin::audit::{events, AuditEvent, AuditOutcome};
use embyr_core::admin::sso::claims::VerifiedIdToken;
use embyr_core::admin::sso::enroll::{check_invite_identity, default_display_name, sso_invite_role_allowed};

use crate::adapters::admin_audit::record;
use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;

fn db_err(context: &'static str) -> impl Fn(sqlx::Error) -> &'static str {
    move |e| {
        tracing::error!("oidc_sso_links: {context}: {e}");
        "db_error"
    }
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error().and_then(|d| d.code()).is_some_and(|c| c == "23505")
}

/// AC-SSO-11. Runs inside the caller's transaction (the caller then issues the session and
/// commits); on `Err` the caller drops the transaction and nothing was created.
pub(super) async fn complete_invite(
    tx: &mut PgConnection,
    provider_id: Uuid,
    provider_account_id: Uuid,
    invitation_id: Uuid,
    verified: &VerifiedIdToken,
) -> Result<Uuid, &'static str> {
    // The row lock serialises concurrent redemptions: exactly one user is created.
    let inv = sqlx::query(
        "SELECT account_id, email, role FROM invitations \
         WHERE id = $1 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at > now() \
           AND auth_method = 'sso' FOR UPDATE",
    )
    .bind(invitation_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db_err("lock invitation"))?
    .ok_or("invitation_unavailable")?;
    let account_id: Uuid = inv.try_get("account_id").map_err(db_err("read invitation"))?;
    let invitation_email: String = inv.try_get("email").map_err(db_err("read invitation"))?;
    let role: String = inv.try_get("role").map_err(db_err("read invitation"))?;

    if account_id != provider_account_id {
        return Err("account_mismatch");
    }
    if !sso_invite_role_allowed(&role) {
        return Err("owner_invitation");
    }
    check_invite_identity(&invitation_email, verified.email.as_deref(), verified.email_verified)
        .map_err(|r| r.reason())?;

    // The invitation email must not already belong to a user (any case): nothing is merged or taken over.
    let taken: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE lower(email) = lower($1))")
        .bind(&invitation_email)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err("email check"))?;
    if taken {
        return Err("email_taken");
    }

    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (account_id, email, display_name, password_hash, totp_secret_enc) \
         VALUES ($1, $2, $3, NULL, NULL) RETURNING id",
    )
    .bind(account_id)
    .bind(&invitation_email)
    .bind(default_display_name(&invitation_email))
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| if is_unique_violation(&e) { "email_taken" } else { db_err("insert user")(e) })?;
    sqlx::query("INSERT INTO account_members (user_id, account_id, role, joined_at) VALUES ($1, $2, $3, now())")
        .bind(user_id)
        .bind(account_id)
        .bind(&role)
        .execute(&mut *tx)
        .await
        .map_err(db_err("insert member"))?;
    insert_link(tx, provider_id, &verified.subject, account_id, user_id).await?;
    sqlx::query("UPDATE invitations SET accepted_at = now() WHERE id = $1")
        .bind(invitation_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err("close invitation"))?;
    Ok(user_id)
}

/// AC-SSO-12. A `sub` already linked to anyone (including another user) fails.
pub(super) async fn complete_link(
    tx: &mut PgConnection,
    provider_id: Uuid,
    provider_account_id: Uuid,
    user_id: Uuid,
    subject: &str,
) -> Result<(), &'static str> {
    let member: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM account_members WHERE user_id = $1 AND account_id = $2)")
            .bind(user_id)
            .bind(provider_account_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err("member check"))?;
    if !member {
        return Err("member_removed");
    }
    insert_link(tx, provider_id, subject, provider_account_id, user_id).await
}

/// `ON CONFLICT DO NOTHING` covers both unique keys: `(provider, sub)` taken by someone, or this
/// user already linked at this provider.
async fn insert_link(
    tx: &mut PgConnection,
    provider_id: Uuid,
    subject: &str,
    account_id: Uuid,
    user_id: Uuid,
) -> Result<(), &'static str> {
    let n = sqlx::query(
        "INSERT INTO admin_sso_identities (provider_id, subject, account_id, user_id, last_login_at) \
         VALUES ($1, $2, $3, $4, now()) ON CONFLICT DO NOTHING",
    )
    .bind(provider_id)
    .bind(subject)
    .bind(account_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(db_err("insert link"))?
    .rows_affected();
    if n == 0 {
        return Err("link_conflict");
    }
    Ok(())
}

// ── DELETE /admin/v1/auth/sso_identity ───────────────────────────────────────

#[derive(Deserialize)]
pub struct UnlinkQuery {
    /// Unlink only this provider; without it every link of the caller is removed.
    provider_id: Option<Uuid>,
}

/// Removes the caller's own link(s). 409 when the caller has no password (SSO-only: the link is
/// their only way in), 404 when there was nothing to remove, 204 otherwise. SSO sessions the user
/// minted through a removed link end with it, except the session making this request.
pub async fn unlink_sso_identity(
    State(state): State<UserAdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    session: SessionContext,
    Query(q): Query<UnlinkQuery>,
) -> Response {
    let pool = state.system_db.pool();
    let ip = state.signin_rate_limiter.source_ip(peer, &headers);
    let fail = |context: &'static str, e: sqlx::Error| {
        tracing::error!("oidc_sso_links: unlink: {context}: {e}");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    };
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => return fail("begin", e),
    };
    // Lock the user row so a concurrent unlink cannot race the SSO-only check.
    let has_password: Option<bool> =
        match sqlx::query_scalar("SELECT password_hash IS NOT NULL FROM users WHERE id = $1 FOR UPDATE")
            .bind(session.user_id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(v) => v,
            Err(e) => return fail("load user", e),
        };
    match has_password {
        None => return StatusCode::NOT_FOUND.into_response(), // service-account key: no user row
        Some(false) => return StatusCode::CONFLICT.into_response(),
        Some(true) => {}
    }
    let removed: Vec<Uuid> = match sqlx::query_scalar(
        "DELETE FROM admin_sso_identities \
         WHERE user_id = $1 AND account_id = $2 AND ($3::uuid IS NULL OR provider_id = $3) \
         RETURNING provider_id",
    )
    .bind(session.user_id)
    .bind(session.account_id)
    .bind(q.provider_id)
    .fetch_all(&mut *tx)
    .await
    {
        Ok(v) => v,
        Err(e) => return fail("delete links", e),
    };
    if removed.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(e) = sqlx::query(
        "DELETE FROM sessions WHERE user_id = $1 AND sso_provider_id = ANY($2) AND id <> $3",
    )
    .bind(session.user_id)
    .bind(&removed)
    .bind(session.session_id)
    .execute(&mut *tx)
    .await
    {
        return fail("end sso sessions", e);
    }
    if let Err(e) = tx.commit().await {
        return fail("commit", e);
    }
    for provider_id in removed {
        let mut ev = AuditEvent::new(events::SSO_LINK_REMOVED, AuditOutcome::Ok);
        ev.account_id = Some(session.account_id);
        ev.actor_user_id = Some(session.user_id);
        ev.provider_id = Some(provider_id);
        ev.source_ip = Some(ip);
        record(pool, &ev).await;
    }
    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    /// E-mail is never a lookup key for an existing user: the only `users` read is the clash check
    /// against the invitation address, and no query selects a user by the IdP's claim.
    #[test]
    fn idp_email_claim_is_only_compared_never_queried() {
        let src = include_str!("oidc_sso_links.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        assert_eq!(code.matches("verified.email.as_deref()").count(), 1, "one comparison against the invitation");
        assert_eq!(code.matches("FROM users").count(), 2, "clash check + SSO-only check, nothing else");
        assert!(!code.contains("UPDATE users"));
    }
}
