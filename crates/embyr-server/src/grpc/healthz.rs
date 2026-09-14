use std::sync::Arc;
use std::time::Duration;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

use crate::adapters::system_db::SystemDb;

/// Liveness — is this process itself alive and able to handle an HTTP
/// request at all. Zero I/O, zero external dependency (ADR-078): a Postgres
/// outage must never fail this handler, or an orchestrator would restart
/// every pod in the fleet during a downstream outage it cannot fix.
pub async fn livez_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({"status": "ok"})))
}

/// Readiness — can this pod usefully serve traffic right now (ADR-078).
// Doc-comment kept plain (no dependency-adapter type names) so the
// `livez_handler_body_has_zero_postgres_or_external_calls` structural test's
// coarse text scan can't mistake this handler's own documentation for text
// inside `livez_handler`'s body above.
//
// Calls the shared system Postgres reachability check fresh on every
// request (no caching, OQ-HDC-02), bounded by an explicit 3s timeout
// (OQ-HDC-03) so a hung dependency reads as unhealthy promptly. On failure
// or timeout, returns a fixed generic body — never the raw driver/schema
// error text (AC-HDC-11, a leak surface ADR-075's tonic-only sweep never
// covered).
pub async fn healthz_handler(State(system_db): State<Arc<SystemDb>>) -> impl IntoResponse {
    match tokio::time::timeout(Duration::from_secs(3), system_db.probe()).await {
        Ok(Ok(())) => (StatusCode::OK, Json(json!({"status": "ok"}))),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "unhealthy"})),
        ),
    }
}
