//! Operator auth middleware (Tower).
//!
//! Validates `Authorization: Bearer <EMBYR_ADMIN_KEY>`.
//! Returns 401 if missing or mismatched.
//! Used on all existing operator-only routes via `route_layer`.
//!
//! The bearer extraction logic is centralised here (moved from provision.rs per AA-01).

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::admin::state::OperatorState;

/// Tower middleware function: validates operator Bearer token.
///
/// Extracts `Authorization: Bearer <token>`, compares to `state.admin_key`.
/// Returns 401 Unauthorized if the token is absent or does not match.
/// Calls `next.run(request)` on success.
pub async fn operator_auth_middleware(
    State(state): State<OperatorState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let bearer = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match bearer {
        Some(token) if token == state.admin_key => next.run(request).await,
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}
