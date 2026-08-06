// SCAFFOLD: true
//! Admin key handlers.
//!
//! list_admin_keys (GET /admin/v1/admin_keys):
//!   Session auth, any role. Includes revoked keys (audit). Plaintext key never returned.
//!
//! create_admin_key (POST /admin/v1/admin_keys):
//!   Session auth, Owner or Admin.
//!   Body: { name, member_id? | service_account_id?, role }.
//!   Role cap: Admins cannot create Owner keys (check_rbac).
//!   201 { id, key: "embyr_adm_<32chars>", prefix, role }. Stored as BLAKE3(key).
//!   key shown once only.
//!
//! revoke_admin_key (DELETE /admin/v1/admin_keys/:key_id):
//!   Session auth, Owner or Admin. Sets revoked_at = now(). 204. Immediate effect.

/// GET /admin/v1/admin_keys
///
/// # RED scaffold
pub async fn list_admin_keys() {
    panic!("Not yet implemented -- RED scaffold: list_admin_keys handler")
}

/// POST /admin/v1/admin_keys
///
/// # RED scaffold
pub async fn create_admin_key() {
    panic!("Not yet implemented -- RED scaffold: create_admin_key handler")
}

/// DELETE /admin/v1/admin_keys/:key_id
///
/// # RED scaffold
pub async fn revoke_admin_key() {
    panic!("Not yet implemented -- RED scaffold: revoke_admin_key handler")
}
