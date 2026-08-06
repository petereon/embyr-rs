//! Session auth middleware (Tower).
//!
//! Tries session cookie BLAKE3 lookup, then admin_api_key BLAKE3 lookup.
//! Inserts [`SessionContext`] into request extensions on success.
//! Returns 401 on failure (neither credential present or valid).
//!
//! Token lookup path (ADR-010):
//!   1. Extract `embyr_session` cookie → BLAKE3(cookie_value) → query sessions JOIN account_members.
//!   2. If no cookie: extract `Authorization: Bearer <key>` → BLAKE3(key) → query admin_api_keys.
//!   3. Both absent or both invalid → 401.
//!
//! Updates `sessions.last_active_at` or `admin_api_keys.last_used_at` on success.

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sqlx::Row;
use uuid::Uuid;

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;
use embyr_core::admin::account::Role;

/// Extract a named cookie value from the `Cookie` request header (manual parse, no axum-extra dep).
///
/// HTTP/1.1 clients combine all cookies into one header (`Cookie: a=1; b=2`).
/// Returns the first matching value.
fn extract_cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get("cookie")?.to_str().ok()?;
    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if let Some((key, value)) = pair.split_once('=') {
            if key.trim() == name {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// Map TEXT role column value to the domain `Role` enum.
fn parse_role(role_str: &str) -> Option<Role> {
    match role_str {
        "Owner" => Some(Role::Owner),
        "Admin" => Some(Role::Admin),
        "Viewer" => Some(Role::Viewer),
        _ => None,
    }
}

/// Tower middleware: validates `embyr_session` cookie or `Authorization: Bearer` admin API key.
///
/// On success, inserts [`SessionContext`] into request extensions and calls `next`.
/// Returns 401 Unauthorized if neither credential is present or valid.
pub async fn session_auth_middleware(
    State(state): State<UserAdminState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let pool = state.system_db.pool();

    // ── Cookie path ──────────────────────────────────────────────────────────
    if let Some(cookie_value) = extract_cookie(request.headers(), "embyr_session") {
        let hash_bytes = blake3::hash(cookie_value.as_bytes()).as_bytes().to_vec();

        let result = sqlx::query(
            "SELECT s.id AS session_id, s.user_id, s.account_id, am.role \
             FROM sessions s \
             JOIN account_members am \
               ON am.user_id = s.user_id AND am.account_id = s.account_id \
             WHERE s.token_hash = $1 AND s.expires_at > now()",
        )
        .bind(&hash_bytes)
        .fetch_optional(pool)
        .await;

        match result {
            Ok(Some(row)) => {
                let role_str: String = match row.try_get("role") {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!("session_auth: failed to read role column: {e}");
                        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                };
                let Some(role) = parse_role(&role_str) else {
                    tracing::warn!("session_auth: unrecognised role value: {role_str:?}");
                    return StatusCode::UNAUTHORIZED.into_response();
                };
                let session_id: Uuid = match row.try_get("session_id") {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!("session_auth: failed to read session_id: {e}");
                        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                };
                let user_id: Uuid = match row.try_get("user_id") {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!("session_auth: failed to read user_id: {e}");
                        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                };
                let account_id: Uuid = match row.try_get("account_id") {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!("session_auth: failed to read account_id: {e}");
                        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                };

                // Touch last_active_at — fire-and-forget; never fail the request on update error.
                let _ = sqlx::query(
                    "UPDATE sessions SET last_active_at = now() WHERE id = $1",
                )
                .bind(session_id)
                .execute(pool)
                .await;

                let ctx = SessionContext {
                    session_id,
                    user_id,
                    account_id,
                    role,
                };
                request.extensions_mut().insert(ctx);
                return next.run(request).await;
            }
            Ok(None) => {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            Err(e) => {
                tracing::error!("session_auth: DB error on cookie lookup: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }

    // ── Bearer admin API key path ─────────────────────────────────────────────
    let bearer = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.to_string());

    if let Some(key) = bearer {
        let hash_bytes = blake3::hash(key.as_bytes()).as_bytes().to_vec();

        let result = sqlx::query(
            "SELECT id, account_id, role, member_id \
             FROM admin_api_keys \
             WHERE key_hash = $1 AND revoked_at IS NULL",
        )
        .bind(&hash_bytes)
        .fetch_optional(pool)
        .await;

        match result {
            Ok(Some(row)) => {
                let role_str: String = match row.try_get("role") {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!("session_auth: failed to read role from admin_api_keys: {e}");
                        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                };
                let Some(role) = parse_role(&role_str) else {
                    tracing::warn!("session_auth: unrecognised role in admin_api_keys: {role_str:?}");
                    return StatusCode::UNAUTHORIZED.into_response();
                };
                let key_id: Uuid = match row.try_get("id") {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!("session_auth: failed to read id from admin_api_keys: {e}");
                        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                };
                let account_id: Uuid = match row.try_get("account_id") {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!("session_auth: failed to read account_id from admin_api_keys: {e}");
                        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                };
                // member_id is NULL for service-account keys; use Uuid::nil() as sentinel.
                let member_id: Option<Uuid> = row.try_get("member_id").unwrap_or(None);
                let user_id = member_id.unwrap_or_else(Uuid::nil);

                // Touch last_used_at — fire-and-forget.
                let _ = sqlx::query(
                    "UPDATE admin_api_keys SET last_used_at = now() WHERE id = $1",
                )
                .bind(key_id)
                .execute(pool)
                .await;

                let ctx = SessionContext {
                    session_id: key_id,
                    user_id,
                    account_id,
                    role,
                };
                request.extensions_mut().insert(ctx);
                return next.run(request).await;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::error!("session_auth: DB error on admin_api_key lookup: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }

    StatusCode::UNAUTHORIZED.into_response()
}
