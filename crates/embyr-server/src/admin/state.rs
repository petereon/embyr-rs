//! Admin router state types.
//!
//! Two state types for two sub-routers:
//!   - `OperatorState`: for existing operator-only routes (Bearer EMBYR_ADMIN_KEY).
//!     Renamed from `AdminState` to avoid ambiguity with UserAdminState.
//!   - `UserAdminState`: for new session-auth user routes.
//!     Carries email sender, encryption key, and system DB.

use std::sync::Arc;

use embyr_core::admin::email::IEmailSender;

use crate::adapters::{
    aws_secret_fetcher::AwsSecretFetcher,
    credential_cache::CredentialCache,
    gcp_secret_fetcher::GcpSecretFetcher,
    system_db::SystemDb,
};

/// State for operator-only routes (Bearer EMBYR_ADMIN_KEY).
/// Renamed from `AdminState` (AA-08, ADR-009).
#[derive(Clone)]
pub struct OperatorState {
    pub system_db: Arc<SystemDb>,
    pub admin_key: String,
    pub credential_cache: Arc<CredentialCache>,
    pub aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
    pub gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
}

/// State for session-auth user-admin routes.
/// Carries the email sender port and the 32-byte AES-256-GCM encryption key.
#[derive(Clone)]
pub struct UserAdminState {
    pub system_db: Arc<SystemDb>,
    /// EMBYR_ENCRYPTION_KEY — 32-byte AES-256-GCM key.
    pub encryption_key: [u8; 32],
    /// Email delivery port — V1 uses NoopEmailSender; V2 uses SmtpEmailSender.
    pub email_sender: Arc<dyn IEmailSender + Send + Sync>,
    pub credential_cache: Arc<CredentialCache>,
    /// EMBYR_ADMIN_KEY value — retained for dual-auth bearer check.
    pub admin_key_env: String,
}
