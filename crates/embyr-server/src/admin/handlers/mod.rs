pub mod access_rules;
pub mod account;
pub mod admin_keys;
// ADR-085: POST /projects/:id/api_key/{rotate,revoke}.
pub mod api_key;
// anonymous-sessions (US-01, ADR-043): admin enablement action.
pub mod anonymous_identity;
pub mod auth;
pub mod billing;
pub mod billing_metering;
pub mod billing_subscription;
// client-auth (US-01/US-03/US-04, ADR-025): credential register/rotate/verify.
pub mod client_identity;
pub mod composite_indexes;
// ADR-083: GET/PUT /projects/:id/connection.
pub mod connection;
pub mod get_project;
// client-auth-hosted-identity (US-01, ADR-036): admin enablement action.
pub mod hosted_identity;
pub mod invitations;
pub mod lifecycle;
pub mod members;
pub mod metrics;
// oauth-providers (US-01, ADR-037): admin registration of a project's Google
// OAuth Client ID.
pub mod oauth_providers;
pub mod oidc_providers;
pub mod password_reset;
pub mod projects;
pub mod prometheus_metrics;
pub mod provision;
pub mod query_logs;
pub mod service_accounts;
pub(crate) mod shared;
pub mod webhooks_stripe;
