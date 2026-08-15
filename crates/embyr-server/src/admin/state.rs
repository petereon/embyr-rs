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
    cap_status_cache::CapStatusCache,
    credential_cache::CredentialCache,
    gcp_secret_fetcher::GcpSecretFetcher,
    stripe_gateway::StripeGateway,
    system_db::SystemDb,
};

/// State for operator-only routes (Bearer EMBYR_ADMIN_KEY).
/// Renamed from `AdminState` (AA-08, ADR-009).
#[derive(Clone)]
pub struct OperatorState {
    pub system_db: Arc<SystemDb>,
    pub admin_key: String,
    /// `EMBYR_ADMIN_KEY_PREVIOUS` — optional Bearer token that opens an
    /// auth-rotation window (ADR-018 §6); `None` means no rotation window —
    /// `operator_auth_middleware` degrades to today's single-key behavior.
    pub admin_key_previous: Option<String>,
    pub credential_cache: Arc<CredentialCache>,
    pub aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
    pub gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
    /// Configured rate-limit capacity (tokens/s).  Written into `rate_buckets`
    /// at project provisioning time so each new project starts at the right
    /// token count.
    pub rate_limit_capacity: f64,
    /// Prometheus metrics handle for rendering the `/metrics` scrape response.
    /// Installed once per process via `observability::get_or_install_prometheus_handle()`.
    pub prometheus_handle: metrics_exporter_prometheus::PrometheusHandle,
}

/// State for session-auth user-admin routes.
/// Carries the email sender port and the 32-byte AES-256-GCM encryption key.
#[derive(Clone)]
pub struct UserAdminState {
    pub system_db: Arc<SystemDb>,
    /// EMBYR_ENCRYPTION_KEY — 32-byte AES-256-GCM key.
    pub encryption_key: [u8; 32],
    /// EMBYR_ENCRYPTION_KEY_PREVIOUS — optional 32-byte AES-256-GCM key that
    /// opens a decrypt-rotation window (ADR-018 §5); `None` outside a
    /// rotation window.
    pub encryption_key_previous: Option<[u8; 32]>,
    /// Email delivery port — V1 uses NoopEmailSender; V2 uses SmtpEmailSender.
    pub email_sender: Arc<dyn IEmailSender + Send + Sync>,
    pub credential_cache: Arc<CredentialCache>,
    /// EMBYR_ADMIN_KEY value — retained for dual-auth bearer check.
    pub admin_key_env: String,
    /// EMBYR_ADMIN_KEY_PREVIOUS value — mirrors `admin_key_env`, keeps
    /// `dual_auth_middleware`'s Bearer arm rotation-aware like its sibling
    /// `operator_auth_middleware` (ADR-018 §6, B-SM-07 consistency fix).
    pub admin_key_previous_env: Option<String>,
    /// Sole Stripe-calling adapter (ADR-021, D-13) — used by
    /// `billing_subscription::get_subscription`/`post_subscription` to
    /// lazily provision/update the real Stripe Customer/Subscription.
    pub stripe_gateway: Arc<StripeGateway>,
    /// In-process cache of the latest `CapUsageRefresher`-computed
    /// `CapStatus` per account (ADR-020) — read (fail-open on a miss,
    /// AC-206-04) by `billing_subscription::get_subscription`.
    pub cap_status_cache: Arc<CapStatusCache>,
}

/// State for the Stripe webhook sub-router (US-203). No session/operator
/// auth — `stripe_signature_middleware` is this sub-router's sole gate.
#[derive(Clone)]
pub struct WebhookState {
    pub system_db: Arc<SystemDb>,
    /// Sole Stripe-calling adapter — used here for
    /// `verify_webhook_signature` (HMAC verification, no network call).
    pub stripe_gateway: Arc<StripeGateway>,
    /// `STRIPE_WEBHOOK_SIGNING_SECRET` this server instance was configured
    /// with — passed to `StripeGateway::verify_webhook_signature`.
    pub webhook_signing_secret: String,
    /// Same shared credential cache instance as `OperatorState`/`UserAdminState`
    /// (card-payments-backend US-204) — required so the dunning
    /// `invoice.payment_failed`/`.payment_succeeded` arms can build a
    /// `lifecycle::LifecycleDeps` and evict the SAME cache
    /// `suspend_project`/`activate_project` evict, not a throwaway instance.
    pub credential_cache: Arc<CredentialCache>,
}
