// SCAFFOLD: true
//! Metrics handler.
//!
//! get_project_metrics (GET /admin/v1/projects/:id/metrics):
//!   Session auth, any role. 404 if project not in account.
//!   Returns: { p95_read_ms, p95_write_ms, reads_today, writes_today, deletes_today,
//!              sparkline: [{ hour, reads, writes }] }.
//!   Sourced from daily_project_metrics. Sparkline: 24 equal buckets.

/// GET /admin/v1/projects/:id/metrics
///
/// # RED scaffold
pub async fn get_project_metrics() {
    panic!("Not yet implemented -- RED scaffold: get_project_metrics handler")
}
