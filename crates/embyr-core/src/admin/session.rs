// SCAFFOLD: true
//! Session domain types: SessionToken, SessionContext.

use crate::admin::account::{AccountId, Role, UserId};

/// A cryptographically random 32-byte session token formatted as base64url.
/// Only the BLAKE3 hash is stored in the `sessions` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionToken(pub String);

/// The resolved identity for an authenticated request.
///
/// Injected into request extensions by `SessionContextExtractor`.
/// Handlers use `account_id` to scope all DB queries — never accept
/// `account_id` from the request body.
#[derive(Debug, Clone)]
pub struct SessionContext {
    pub user_id: UserId,
    pub account_id: AccountId,
    pub role: Role,
}
