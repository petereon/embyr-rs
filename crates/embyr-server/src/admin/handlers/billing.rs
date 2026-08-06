// SCAFFOLD: true
//! Billing handler.
//!
//! get_billing (GET /admin/v1/billing?range=<7d|30d|month|last_month>):
//!   Session auth, any role.
//!   Returns: { range, databases: [ { id, name, reads, writes, deletes,
//!              peak_connections (null V1), log_storage_bytes } ], totals: {...} }.
//!   Sourced from daily_project_metrics GROUP BY project WHERE account_id = session.account_id.
//!   Projects with no metric rows included with all counters = 0 (AC-B06-08).

/// GET /admin/v1/billing
///
/// # RED scaffold
pub async fn get_billing() {
    panic!("Not yet implemented -- RED scaffold: get_billing handler")
}
