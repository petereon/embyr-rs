// SCAFFOLD: true
//! Project handlers for session-auth routes.
//!
//! list_projects (GET /admin/v1/projects):
//!   Session auth, any role. WHERE account_id = $session_account AND status != 'deleted'.
//!
//! patch_project (PATCH /admin/v1/projects/:id):
//!   Session auth, Owner or Admin.
//!   Partial update: name, backend_mode, backend_pg_dsn, logging_enabled, log_retention_days, status.
//!   backend_pg_dsn → ECIES-encrypt before storage. Evict credential cache.
//!   status 'deleted' not patchable (422).

/// GET /admin/v1/projects — account-scoped project list.
///
/// # RED scaffold
pub async fn list_projects() {
    panic!("Not yet implemented -- RED scaffold: list_projects handler")
}

/// PATCH /admin/v1/projects/:id — partial update.
///
/// # RED scaffold
pub async fn patch_project() {
    panic!("Not yet implemented -- RED scaffold: patch_project handler")
}
