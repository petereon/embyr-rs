//! `stripe_signature_middleware` — verifies `Stripe-Signature` on
//! `POST /admin/v1/webhooks/stripe` (US-203, AC-203-02).
//!
//! `SCAFFOLD: true` — created by DISTILL (card-payments-backend). Panics —
//! DELIVER implements it via Outside-In TDD.
//!
//! Buffers the raw request body for HMAC verification (Stripe's signature is
//! computed over the exact raw bytes; any JSON re-serialization would break
//! verification) — mirrors the SHAPE (not the auth mechanism) of
//! `operator_auth_middleware`: reject before any handler logic runs, zero DB
//! writes on rejection (AC-203-02).

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::admin::state::WebhookState;

/// Rejects the request with 401 (no DB write) if `Stripe-Signature` is
/// missing or does not verify against `state.webhook_signing_secret`
/// (`StripeGateway::verify_webhook_signature`). On success, stashes the
/// verified `WebhookEvent` in request extensions and forwards to
/// `stripe_webhook_handler`.
///
/// Buffers the raw request body via axum's `Bytes`-shaped extraction
/// (`axum::body::to_bytes`) BEFORE any JSON parsing — Stripe's HMAC is
/// computed over the exact raw bytes, never a re-serialized payload. No DB
/// access happens anywhere in this function, so a rejection here is
/// guaranteed zero DB writes (AC-203-02).
pub async fn stripe_signature_middleware(
    State(state): State<WebhookState>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let signature = req
        .headers()
        .get("Stripe-Signature")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let event = state
        .stripe_gateway
        .verify_webhook_signature(&bytes, &signature, &state.webhook_signing_secret)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let mut req = Request::from_parts(parts, Body::from(bytes));
    req.extensions_mut().insert(event);

    Ok(next.run(req).await.into_response())
}
