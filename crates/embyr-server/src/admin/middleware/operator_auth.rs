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
use subtle::ConstantTimeEq;

use crate::admin::state::OperatorState;

/// Constant-time dual-token comparison (ADR-018 §6): `presented` is checked
/// against `current` and, when set, `previous`. Both comparisons always run
/// — the result is OR-ed together rather than short-circuited — so timing
/// does not leak which key (if either) matched. `pub(crate)` so the
/// dual-auth middleware can reuse it (B-SM-07 consistency fix).
pub(crate) fn bearer_matches(presented: &str, current: &str, previous: Option<&str>) -> bool {
    let matches_current: bool = presented.as_bytes().ct_eq(current.as_bytes()).into();
    let matches_previous: bool = match previous {
        Some(prev) => presented.as_bytes().ct_eq(prev.as_bytes()).into(),
        None => false,
    };
    matches_current | matches_previous
}

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
        Some(token) if bearer_matches(token, &state.admin_key, state.admin_key_previous.as_deref()) => {
            next.run(request).await
        }
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_matches_current_or_previous_key() {
        let cases: Vec<(&str, &str, Option<&str>, bool)> = vec![
            // current-key match
            ("new-token-2026", "new-token-2026", Some("old-token-2025"), true),
            // previous-key match
            ("old-token-2025", "new-token-2026", Some("old-token-2025"), true),
            // neither matches (previous configured)
            ("wrong-token", "new-token-2026", Some("old-token-2025"), false),
            // neither matches (previous absent)
            ("wrong-token", "new-token-2026", None, false),
            // current matches, previous absent
            ("new-token-2026", "new-token-2026", None, true),
        ];
        for (presented, current, previous, expected) in cases {
            assert_eq!(
                bearer_matches(presented, current, previous),
                expected,
                "presented={presented:?} current={current:?} previous={previous:?}"
            );
        }
    }
}
