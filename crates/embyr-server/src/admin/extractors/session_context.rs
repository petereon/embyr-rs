//! SessionContext extractor — `FromRequestParts` impl (ADR-010).
//!
//! Reads the `SessionContext` injected by `session_auth_middleware` from request extensions.
//! Handlers declare `session: SessionContext` in their parameter list — the compiler enforces
//! that only session-auth routes can access `session.account_id`.
//!
//! Token lookup (performed by the middleware, not here):
//!   `embyr_session` cookie → BLAKE3 → sessions table lookup → SessionContext.
//!   Bearer admin_api_key → BLAKE3 → admin_api_keys table lookup → SessionContext.

use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use embyr_core::admin::account::Role;
use uuid::Uuid;

/// Resolved identity for an authenticated request, injected by `session_auth_middleware`.
///
/// Handlers declare `session: SessionContext` to guarantee the route is session-protected.
/// `account_id` is the ONLY authoritative source of account scope for DB queries.
#[derive(Debug, Clone)]
pub struct SessionContext {
    /// The sessions row UUID (used for signout — DELETE FROM sessions WHERE id = $1).
    pub session_id: Uuid,
    /// The authenticated user UUID (or Uuid::nil() for service-account API keys).
    pub user_id: Uuid,
    /// The account this session is scoped to.
    pub account_id: Uuid,
    /// Role of the member in this account (Owner, Admin, Viewer).
    pub role: Role,
}

#[async_trait::async_trait]
impl<S> FromRequestParts<S> for SessionContext
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<SessionContext>()
            .cloned()
            .ok_or(StatusCode::UNAUTHORIZED)
    }
}
