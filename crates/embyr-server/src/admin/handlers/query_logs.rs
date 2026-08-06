// SCAFFOLD: true
//! Query log handler.
//!
//! list_query_logs (GET /admin/v1/projects/:id/query_logs):
//!   Session auth, any role. 404 if project not in account.
//!   Query params: op, prefix, range (1h/6h/24h/7d), status (ok/error), sort (ts_desc default).
//!   Returns: { total, entries: [...] }. Max 500 rows per page.
//!   Cursor pagination: ?after=<id>.
//!   When logging_enabled = false: 200 { total: 0, entries: [] }.

/// GET /admin/v1/projects/:id/query_logs
///
/// # RED scaffold
pub async fn list_query_logs() {
    panic!("Not yet implemented -- RED scaffold: list_query_logs handler")
}
