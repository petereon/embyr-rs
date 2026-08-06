// SCAFFOLD: true
//! Account domain types: AccountId, UserId, User, Role, AccountMember.

use uuid::Uuid;

/// Opaque account identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountId(pub Uuid);

/// Opaque user identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserId(pub Uuid);

/// Role hierarchy: Owner (3) > Admin (2) > Viewer (1).
/// Comparable — higher numeric value = higher privilege.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer = 1,
    Admin = 2,
    Owner = 3,
}

/// A user in the system.
///
/// # RED scaffold
/// Argon2id password hash, AES-GCM TOTP secret, and lockout fields
/// are placeholders until the B-01 implementation is wired in.
#[derive(Debug, Clone)]
pub struct User {
    pub id: UserId,
    pub account_id: AccountId,
    pub email: String,
    pub display_name: String,
    /// Argon2id hash of the password (stored; never the plaintext).
    pub password_hash: String,
    /// AES-GCM encrypted TOTP shared secret.
    pub totp_secret_enc: Option<Vec<u8>>,
    pub failed_totp_attempts: u32,
    pub locked_until: Option<std::time::SystemTime>,
}

/// Junction between a User and an Account with an assigned Role.
///
/// Invariant: at least one AccountMember with Role::Owner must exist per account.
#[derive(Debug, Clone)]
pub struct AccountMember {
    pub id: Uuid,
    pub user_id: UserId,
    pub account_id: AccountId,
    pub role: Role,
    pub joined_at: Option<std::time::SystemTime>,
}
