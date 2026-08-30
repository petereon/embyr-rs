pub mod access_rules;
pub mod admin_keys;
pub mod auth;
pub mod billing;
pub mod billing_metering;
pub mod billing_subscription;
// client-auth (US-01/US-03/US-04, ADR-025): credential register/rotate/verify.
pub mod client_identity;
pub mod get_project;
// client-auth-hosted-identity (US-01, ADR-036): admin enablement action.
pub mod hosted_identity;
pub mod lifecycle;
pub mod members;
pub mod metrics;
// oauth-providers (US-01, ADR-037): admin registration of a project's Google
// OAuth Client ID.
pub mod oauth_providers;
pub mod oidc_providers;
pub mod projects;
pub mod prometheus_metrics;
pub mod provision;
pub mod query_logs;
pub mod sdk_keys;
pub mod service_accounts;
pub(crate) mod shared;
pub mod webhooks_stripe;
