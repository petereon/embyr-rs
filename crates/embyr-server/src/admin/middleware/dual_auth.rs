// SCAFFOLD: true
//! Dual auth middleware (Tower).
//!
//! Used only for `GET /admin/v1/projects/:id` (AA-01).
//! Tries session auth first, then operator Bearer.
//! Sets `AuthPrincipal` request extension.
//! Returns 401 if neither principal validates.

/// Tower middleware function: accepts either session cookie or operator Bearer.
///
/// # RED scaffold
/// Panics until B-02 implementation wires the dual-auth sub-router.
pub async fn dual_auth_middleware() {
    panic!("Not yet implemented -- RED scaffold: dual_auth_middleware")
}
