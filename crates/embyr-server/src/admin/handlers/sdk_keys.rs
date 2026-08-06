// SCAFFOLD: true
//! SDK key handlers.
//!
//! list_sdk_keys (GET /admin/v1/projects/:id/sdk_keys):
//!   Session auth, any role. Includes revoked keys (for audit).
//!
//! create_sdk_key (POST /admin/v1/projects/:id/sdk_keys):
//!   Session auth, Owner or Admin.
//!   Calls new_sdk_key_material on blocking thread. Evicts credential cache.
//!   201 { id, name, key, prefix, created_at }. key shown once only.
//!
//! revoke_sdk_key (DELETE /admin/v1/projects/:id/sdk_keys/:key_id):
//!   Session auth, Owner or Admin. Sets revoked_at. 204.

/// GET /admin/v1/projects/:id/sdk_keys
///
/// # RED scaffold
pub async fn list_sdk_keys() {
    panic!("Not yet implemented -- RED scaffold: list_sdk_keys handler")
}

/// POST /admin/v1/projects/:id/sdk_keys
///
/// # RED scaffold
pub async fn create_sdk_key() {
    panic!("Not yet implemented -- RED scaffold: create_sdk_key handler")
}

/// DELETE /admin/v1/projects/:id/sdk_keys/:key_id
///
/// # RED scaffold
pub async fn revoke_sdk_key() {
    panic!("Not yet implemented -- RED scaffold: revoke_sdk_key handler")
}
