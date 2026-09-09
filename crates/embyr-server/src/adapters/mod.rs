pub mod agent_backend;
pub mod aws_secret_fetcher;
pub mod cap_status_cache;
// composite-index-real-creation (ADR-072): pure DDL-string builders +
// one-shot build-task orchestration for real composite-index provisioning.
pub mod composite_index_builder;
pub mod composite_index_ddl;
pub mod credential_cache;
// composite-index-real-creation (ADR-072 Decision C): shared DSN-resolution
// for reaching a customer DB without a live api_key, extracted from
// sweepers/transaction_sweeper.rs.
pub mod customer_db_connect;
pub mod email;
pub mod encryption;
pub mod gcp_secret_fetcher;
// oauth-providers (US-02, ADR-037 Decision 7): Google JWKS fetch/cache.
pub mod google_jwks_cache;
pub mod index_manager;
pub mod metrics_adapter;
pub mod postgres_backend;
pub mod postgres_notify_listener;
// client-auth-hosted-identity (ADR-036 Decision 7): resolve_customer_db_adapter.
pub mod project_auth;
pub mod query_log;
pub mod stripe_gateway;
pub mod system_db;
// firestore-tls-support: shared accept_maybe_tls() handshake helper.
pub mod tls;
