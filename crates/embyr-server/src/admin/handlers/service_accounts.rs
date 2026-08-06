// SCAFFOLD: true
//! Service account handlers.
//!
//! list_service_accounts (GET /admin/v1/service_accounts):
//!   Session auth, any role.
//!
//! create_service_account (POST /admin/v1/service_accounts):
//!   Session auth, Owner or Admin.
//!   Body: { name, description?, role }. 201 { id, name, role, created_at }.
//!
//! delete_service_account (DELETE /admin/v1/service_accounts/:id):
//!   Session auth, Owner or Admin. 204.
//!   Cascade: revoke all admin_api_keys linked to this service account.

/// GET /admin/v1/service_accounts
///
/// # RED scaffold
pub async fn list_service_accounts() {
    panic!("Not yet implemented -- RED scaffold: list_service_accounts handler")
}

/// POST /admin/v1/service_accounts
///
/// # RED scaffold
pub async fn create_service_account() {
    panic!("Not yet implemented -- RED scaffold: create_service_account handler")
}

/// DELETE /admin/v1/service_accounts/:id
///
/// # RED scaffold
pub async fn delete_service_account() {
    panic!("Not yet implemented -- RED scaffold: delete_service_account handler")
}
