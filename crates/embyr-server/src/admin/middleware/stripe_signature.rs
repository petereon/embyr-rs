//! `stripe_signature_middleware` — verifies `Stripe-Signature` on
//! `POST /admin/v1/webhooks/stripe` (US-203, AC-203-02).
//!
//! Buffers the raw request body for HMAC verification (Stripe's signature is
//! computed over the exact raw bytes; any JSON re-serialization would break
//! verification) — mirrors the SHAPE (not the auth mechanism) of
//! `operator_auth_middleware`: reject before any handler logic runs, zero DB
//! writes on rejection (AC-203-02).

use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use http_body_util::{BodyExt, Limited};

use crate::admin::state::WebhookState;

/// Stripe's own real-world webhook event payloads are typically a few KB,
/// rarely approaching the low hundreds of KB even for large `invoice.*`
/// events with many line items (Stripe publishes no single hard maximum).
/// 5 MiB is a deliberately generous ceiling — 10-50x any realistic real
/// payload — chosen to make AC-WBL-02 regression risk effectively zero
/// while still bounding an attacker's forced per-request allocation to a
/// small, fixed number instead of `usize::MAX` (ADR-070).
const MAX_STRIPE_WEBHOOK_BODY_BYTES: usize = 5 * 1024 * 1024; // 5 MiB

/// Rejects the request with 401 (no DB write) if `Stripe-Signature` is
/// missing or does not verify against `state.webhook_signing_secret`
/// (`StripeGateway::verify_webhook_signature`). On success, stashes the
/// verified `WebhookEvent` in request extensions and forwards to
/// `stripe_webhook_handler`.
///
/// Buffers the raw request body BEFORE any JSON parsing — Stripe's HMAC is
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
    let bytes = read_body_bounded(body, MAX_STRIPE_WEBHOOK_BODY_BYTES).await?;

    let event = state
        .stripe_gateway
        .verify_webhook_signature(&bytes, &signature, &state.webhook_signing_secret)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let mut req = Request::from_parts(parts, Body::from(bytes));
    req.extensions_mut().insert(event);

    Ok(next.run(req).await.into_response())
}

/// Reads `body` into memory, never buffering past `limit` bytes
/// (`http_body_util::Limited`, ADR-070). Unlike `axum::body::to_bytes`,
/// which stops polling the instant the limit is exceeded, this keeps
/// draining (and discarding) the remaining frames so the client's TCP
/// connection is fully consumed instead of reset mid-write — required
/// because AC-WBL-01's oversized-body case sends an attacker-scale body far
/// larger than the OS socket buffer, and a server that stops reading before
/// the client finishes writing causes the peer to see a connection reset
/// rather than the intended 413 response. Memory stays bounded either way:
/// bytes past `limit` are polled and dropped, never appended to `buf`.
async fn read_body_bounded(body: Body, limit: usize) -> Result<Bytes, StatusCode> {
    let mut limited = Limited::new(body, limit);
    let mut buf = Vec::new();
    let mut too_large = false;

    while let Some(frame) = limited.frame().await {
        match frame {
            Ok(frame) => {
                if let Some(data) = frame.data_ref() {
                    if !too_large {
                        buf.extend_from_slice(data);
                    }
                }
            }
            Err(err) => {
                if err
                    .downcast_ref::<http_body_util::LengthLimitError>()
                    .is_some()
                {
                    too_large = true;
                    buf.clear();
                } else {
                    return Err(StatusCode::UNAUTHORIZED);
                }
            }
        }
    }

    if too_large {
        Err(StatusCode::PAYLOAD_TOO_LARGE)
    } else {
        Ok(Bytes::from(buf))
    }
}
