// SCAFFOLD: true
//! Auth handlers: signin, signout, oidc_callback.
//!
//! signin (POST /admin/v1/auth/signin):
//!   Validates email + Argon2id password + TOTP code.
//!   Creates session row (BLAKE3 token hash). Sets HttpOnly cookie.
//!   Returns 200 { account_id, display_name, role }.
//!
//! signout (POST /admin/v1/auth/signout):
//!   Deletes session row. Sets Max-Age=0 on cookie. Returns 204.
//!
//! oidc_callback (GET /admin/v1/auth/oidc/callback):
//!   Validates id_token (issuer, audience, expiry, JWKS signature).
//!   Verifies CSRF state nonce. Creates session. 302 → /admin/.
//!   On failure: 302 → /admin/?error=oidc_failed.

/// POST /admin/v1/auth/signin
///
/// # RED scaffold
pub async fn signin() {
    panic!("Not yet implemented -- RED scaffold: signin handler")
}

/// POST /admin/v1/auth/signout
///
/// # RED scaffold
pub async fn signout() {
    panic!("Not yet implemented -- RED scaffold: signout handler")
}

/// GET /admin/v1/auth/oidc/callback
///
/// # RED scaffold
pub async fn oidc_callback() {
    panic!("Not yet implemented -- RED scaffold: oidc_callback handler")
}
