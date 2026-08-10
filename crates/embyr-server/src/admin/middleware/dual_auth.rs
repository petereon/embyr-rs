//! Dual auth middleware (Tower) — GET /admin/v1/projects/:id only.
//!
//! Accepts:
//!   - Session cookie `embyr_session`: BLAKE3 lookup → sessions JOIN account_members.
//!   - Bearer EMBYR_ADMIN_KEY (or EMBYR_ADMIN_KEY_PREVIOUS during a rotation
//!     window, ADR-018 §6): constant-time match via `bearer_matches`.
//!
//! Inserts `AuthPrincipal` into request extensions on success.
//! Returns 401 if neither credential is valid.
//! Cookie path always takes priority over Bearer path.

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sqlx::Row;
use uuid::Uuid;

use embyr_core::admin::account::Role;

use crate::admin::extractors::dual_auth_principal::AuthPrincipal;
use crate::admin::middleware::operator_auth::bearer_matches;
use crate::admin::state::UserAdminState;

/// Extract a named cookie value from the `Cookie` request header.
fn extract_cookie_value(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get("cookie")?.to_str().ok()?;
    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if let Some((k, v)) = pair.split_once('=') {
            if k.trim() == name {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// Map TEXT role column value to the domain `Role` enum.
fn parse_role(s: &str) -> Option<Role> {
    match s {
        "Owner" => Some(Role::Owner),
        "Admin" => Some(Role::Admin),
        "Viewer" => Some(Role::Viewer),
        _ => None,
    }
}

/// Tower middleware: accepts session cookie OR operator Bearer EMBYR_ADMIN_KEY.
///
/// On success, inserts `AuthPrincipal` into request extensions and calls `next`.
/// Returns 401 if neither credential is present or valid.
pub async fn dual_auth_middleware(
    State(state): State<UserAdminState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let pool = state.system_db.pool();

    // ── Session cookie path ───────────────────────────────────────────────────
    if let Some(cookie_val) = extract_cookie_value(request.headers(), "embyr_session") {
        let hash = blake3::hash(cookie_val.as_bytes()).as_bytes().to_vec();

        let result = sqlx::query(
            "SELECT s.id AS session_id, s.user_id, s.account_id, am.role \
             FROM sessions s \
             JOIN account_members am \
               ON am.user_id = s.user_id AND am.account_id = s.account_id \
             WHERE s.token_hash = $1 AND s.expires_at > now()",
        )
        .bind(&hash)
        .fetch_optional(pool)
        .await;

        match result {
            Ok(Some(row)) => {
                let role_str: String = row.try_get("role").unwrap_or_default();
                let Some(role) = parse_role(&role_str) else {
                    tracing::warn!("dual_auth: unrecognised role value: {role_str:?}");
                    return StatusCode::UNAUTHORIZED.into_response();
                };
                let session_id: Uuid = row.try_get("session_id").unwrap_or_else(|_| Uuid::nil());
                let user_id: Uuid = row.try_get("user_id").unwrap_or_else(|_| Uuid::nil());
                let account_id: Uuid = row.try_get("account_id").unwrap_or_else(|_| Uuid::nil());

                // Touch last_active_at — fire-and-forget; never fail the request on update error.
                let _ = sqlx::query("UPDATE sessions SET last_active_at = now() WHERE id = $1")
                    .bind(session_id)
                    .execute(pool)
                    .await;

                request
                    .extensions_mut()
                    .insert(AuthPrincipal::User { session_id, user_id, account_id, role });
                return next.run(request).await;
            }
            Ok(None) => {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            Err(e) => {
                tracing::error!("dual_auth: DB error on cookie lookup: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }

    // ── Operator Bearer path ──────────────────────────────────────────────────
    let bearer = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.to_string());

    if let Some(key) = bearer {
        if bearer_matches(&key, &state.admin_key_env, state.admin_key_previous_env.as_deref()) {
            request.extensions_mut().insert(AuthPrincipal::Operator);
            return next.run(request).await;
        }
        // Non-empty Bearer that doesn't match operator key (current or previous): 401.
        return StatusCode::UNAUTHORIZED.into_response();
    }

    StatusCode::UNAUTHORIZED.into_response()
}
