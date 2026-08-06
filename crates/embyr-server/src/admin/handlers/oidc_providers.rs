// SCAFFOLD: true
//! OIDC provider handlers.
//!
//! list_oidc_providers (GET /admin/v1/oidc_providers):
//!   Session auth, Owner only. client_secret_enc never returned.
//!
//! create_oidc_provider (POST /admin/v1/oidc_providers):
//!   Session auth, Owner only.
//!   client_secret → AES-256-GCM under EMBYR_ENCRYPTION_KEY before storage.
//!   201 { id, issuer, client_id, enabled: true }. client_secret absent.
//!
//! patch_oidc_provider (PATCH /admin/v1/oidc_providers/:id):
//!   Session auth, Owner only. Partial update. Disabling does not invalidate sessions.
//!
//! delete_oidc_provider (DELETE /admin/v1/oidc_providers/:id):
//!   Session auth, Owner only. 204. Does not invalidate sessions.

/// GET /admin/v1/oidc_providers
///
/// # RED scaffold
pub async fn list_oidc_providers() {
    panic!("Not yet implemented -- RED scaffold: list_oidc_providers handler")
}

/// POST /admin/v1/oidc_providers
///
/// # RED scaffold
pub async fn create_oidc_provider() {
    panic!("Not yet implemented -- RED scaffold: create_oidc_provider handler")
}

/// PATCH /admin/v1/oidc_providers/:id
///
/// # RED scaffold
pub async fn patch_oidc_provider() {
    panic!("Not yet implemented -- RED scaffold: patch_oidc_provider handler")
}

/// DELETE /admin/v1/oidc_providers/:id
///
/// # RED scaffold
pub async fn delete_oidc_provider() {
    panic!("Not yet implemented -- RED scaffold: delete_oidc_provider handler")
}
