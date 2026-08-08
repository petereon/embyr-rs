//! GET /metrics — Prometheus scrape endpoint (OBS-01).
//!
//! Named `prometheus_metrics` to avoid collision with the existing
//! `handlers/metrics.rs` module (project-level operation metrics).
//!
//! Auth: `operator_auth_middleware` (Bearer EMBYR_ADMIN_KEY) is applied at
//! the router layer — this handler is reached only with a valid operator key.
//!
//! D-OBS-8: Gauges are updated immediately before `handle.render()` to ensure
//! the scrape response reflects the current pool state, even if the 15-second
//! background task has not ticked since the last significant connection change.

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
};

use crate::admin::state::OperatorState;

/// Render the current Prometheus metrics text and return it as a scrape response.
///
/// Before rendering, updates the system-DB pool gauges so the response reflects
/// the pool state at the moment of the HTTP request (AC-OBS-05-04 dual-update).
pub async fn get_prometheus_metrics(State(state): State<OperatorState>) -> impl IntoResponse {
    // Scrape-time pool gauge update (D-OBS-8 dual-update strategy).
    let pool = state.system_db.pool();
    metrics::gauge!("embyr_pg_pool_size", "pool" => "system").set(pool.size() as f64);
    metrics::gauge!("embyr_pg_pool_idle", "pool" => "system").set(pool.num_idle() as f64);

    let body = state.prometheus_handle.render();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        body,
    )
}
