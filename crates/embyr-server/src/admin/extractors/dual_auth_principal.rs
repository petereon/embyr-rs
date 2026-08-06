//! DualAuthPrincipal extractor — FromRequestParts for routes behind dual_auth_middleware.
//!
//! Resolves caller identity from either:
//!   - A session cookie (User variant — account-scoped with role)
//!   - An operator Bearer EMBYR_ADMIN_KEY (Operator variant — unscoped)
//!
//! The middleware inserts the `AuthPrincipal` extension; this extractor retrieves it.

use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use embyr_core::admin::account::Role;
use uuid::Uuid;

/// Resolved caller identity for dual-auth routes.
#[derive(Debug, Clone)]
pub enum AuthPrincipal {
    /// A session-authenticated user with account scope and role.
    User {
        session_id: Uuid,
        user_id: Uuid,
        account_id: Uuid,
        role: Role,
    },
    /// An operator authenticated via Bearer EMBYR_ADMIN_KEY (unscoped).
    Operator,
}

#[async_trait::async_trait]
impl<S> FromRequestParts<S> for AuthPrincipal
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthPrincipal>()
            .cloned()
            .ok_or(StatusCode::UNAUTHORIZED)
    }
}
