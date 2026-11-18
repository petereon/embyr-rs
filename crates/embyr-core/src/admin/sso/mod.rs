//! Admin-console OIDC SSO, pure domain half (ADR-086, `docs/feature/admin-oidc-sso`).
//!
//! No IO: PKCE/state/nonce generation, ID-token claim validation against a
//! caller-supplied JWKS, and the outbound URL/IP policy. Fetching discovery
//! documents and JWKS, resolving DNS and talking to the token endpoint live in
//! `embyr-server`.

pub mod claims;
pub mod discovery;
pub mod enroll;
pub mod pkce;
pub mod url_policy;

#[cfg(test)]
pub(crate) mod test_keys;
