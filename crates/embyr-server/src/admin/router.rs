use axum::{
    routing::{delete, get, patch, post},
    Router,
};
use std::sync::Arc;

use embyr_core::admin::email::IEmailSender;
use metrics_exporter_prometheus::PrometheusHandle;

use crate::adapters::{
    aws_secret_fetcher::AwsSecretFetcher,
    cap_status_cache::CapStatusCache,
    credential_cache::CredentialCache,
    gcp_secret_fetcher::GcpSecretFetcher,
    stripe_gateway::StripeGateway,
    system_db::SystemDb,
};

use super::handlers::admin_keys::{create_admin_key, list_admin_keys, revoke_admin_key};
use super::handlers::billing::get_billing;
use super::handlers::billing_subscription::{get_subscription, post_subscription};
use super::handlers::oidc_providers::{
    create_oidc_provider, delete_oidc_provider, list_oidc_providers, patch_oidc_provider,
};
use super::handlers::auth::{oidc_callback, signin, signout};
use super::handlers::members::{change_member_role, invite_member, list_members, remove_member};
use super::handlers::projects::{list_projects, patch_project};
use super::handlers::sdk_keys::{create_sdk_key, list_sdk_keys, revoke_sdk_key};
use super::handlers::get_project::get_project;
use super::handlers::metrics::get_project_metrics;
use super::handlers::lifecycle::{activate_project, delete_project, suspend_project};
use super::handlers::prometheus_metrics::get_prometheus_metrics;
use super::handlers::provision::provision;
use super::handlers::query_logs::list_query_logs;
use super::handlers::service_accounts::{
    create_service_account, delete_service_account, list_service_accounts,
};
use super::handlers::webhooks_stripe::stripe_webhook_handler;
use super::middleware::dual_auth::dual_auth_middleware;
use super::middleware::operator_auth::operator_auth_middleware;
use super::middleware::session_auth::session_auth_middleware;
use super::middleware::stripe_signature::stripe_signature_middleware;
use super::state::{OperatorState, UserAdminState, WebhookState};

/// Build the admin router with all five sub-routers merged under /admin/v1.
///
/// Sub-router breakdown (ADR-009, AA-01):
///   - `operator_router`:   Bearer EMBYR_ADMIN_KEY; operator routes + GET /metrics.
///   - `dual_auth_router`:  GET /projects/:id; session cookie OR operator Bearer (step 02-02).
///   - `public_router`:     No auth; signin/signout placeholders replaced in step 01-04.
///   - `session_router`:    Session cookie / admin_api_key Bearer; populated in steps 01-04 through 06-03.
///   - `webhook_router`:    No session/operator auth; own `stripe_signature_middleware` gate instead (US-203, step 01-03).
// Composition-root wiring function — each parameter is a distinct required
// dependency for one of the four sub-routers; splitting into a config struct
// wouldn't reduce the actual coupling, just relocate it.
#[allow(clippy::too_many_arguments)]
pub fn build_admin_router(
    system_db: Arc<SystemDb>,
    admin_key: String,
    admin_key_previous: Option<String>,
    credential_cache: Arc<CredentialCache>,
    encryption_key: [u8; 32],
    encryption_key_previous: Option<[u8; 32]>,
    email_sender: Arc<dyn IEmailSender + Send + Sync>,
    aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
    gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
    rate_limit_capacity: f64,
    prometheus_handle: PrometheusHandle,
    stripe_gateway: Arc<StripeGateway>,
    // card-payments-backend (US-203): the 5th sub-router's own signing
    // secret, checked by `stripe_signature_middleware` — no session/operator
    // auth guards this sub-router.
    webhook_signing_secret: String,
) -> Router {
    let operator_state = OperatorState {
        system_db: system_db.clone(),
        admin_key: admin_key.clone(),
        admin_key_previous: admin_key_previous.clone(),
        credential_cache: credential_cache.clone(),
        aws_secret_fetcher,
        gcp_secret_fetcher,
        rate_limit_capacity,
        prometheus_handle,
    };
    let webhook_state = WebhookState {
        system_db: system_db.clone(),
        stripe_gateway: stripe_gateway.clone(),
        webhook_signing_secret,
    };
    let user_state = UserAdminState {
        system_db,
        encryption_key,
        encryption_key_previous,
        email_sender,
        credential_cache,
        admin_key_env: admin_key,
        admin_key_previous_env: admin_key_previous,
        stripe_gateway,
        cap_status_cache: Arc::new(CapStatusCache::new()),
    };

    // Operator sub-router: mutating operator routes + GET /metrics, all guarded by
    // operator Bearer middleware (ADR-016 D-OBS-4).
    let operator_router = Router::new()
        .route("/admin/v1/projects", post(provision))
        .route("/admin/v1/projects/:project_id", delete(delete_project))
        .route(
            "/admin/v1/projects/:project_id/suspend",
            post(suspend_project),
        )
        .route(
            "/admin/v1/projects/:project_id/activate",
            post(activate_project),
        )
        .route("/metrics", get(get_prometheus_metrics))
        .route_layer(axum::middleware::from_fn_with_state(
            operator_state.clone(),
            operator_auth_middleware,
        ))
        .with_state(operator_state.clone());

    // Dual-auth sub-router: GET /projects/:id.
    // Accepts session cookie (account-scoped) OR operator Bearer EMBYR_ADMIN_KEY (unscoped).
    let dual_auth_router = Router::<UserAdminState>::new()
        .route("/admin/v1/projects/:project_id", get(get_project))
        .route_layer(axum::middleware::from_fn_with_state(
            user_state.clone(),
            dual_auth_middleware,
        ))
        .with_state(user_state.clone());

    // Public sub-router: no auth. Wired to UserAdminState (step 01-04).
    let public_router = Router::<UserAdminState>::new()
        .route("/admin/v1/auth/signin", post(signin))
        .route("/admin/v1/auth/signout", post(signout))
        .route("/admin/v1/auth/oidc/callback", get(oidc_callback))
        .with_state(user_state.clone());

    // Session sub-router: session_auth_middleware guards all routes added in steps 01-04+.
    // Routes MUST be added BEFORE route_layer so the middleware applies to them.
    let session_router = Router::<UserAdminState>::new()
        .route("/admin/v1/projects", get(list_projects))
        .route("/admin/v1/projects/:project_id", patch(patch_project))
        .route(
            "/admin/v1/projects/:project_id/sdk_keys",
            get(list_sdk_keys).post(create_sdk_key),
        )
        .route(
            "/admin/v1/projects/:project_id/sdk_keys/:key_id",
            delete(revoke_sdk_key),
        )
        .route(
            "/admin/v1/projects/:project_id/metrics",
            get(get_project_metrics),
        )
        .route(
            "/admin/v1/projects/:project_id/query_logs",
            get(list_query_logs),
        )
        // Members routes (step 05-02).
        .route("/admin/v1/members", get(list_members))
        .route("/admin/v1/members/invite", post(invite_member))
        .route("/admin/v1/members/:member_id/role", patch(change_member_role))
        .route("/admin/v1/members/:member_id", delete(remove_member))
        // Service account routes (step 05-03).
        .route(
            "/admin/v1/service_accounts",
            get(list_service_accounts).post(create_service_account),
        )
        .route(
            "/admin/v1/service_accounts/:sa_id",
            delete(delete_service_account),
        )
        // Admin key routes (step 05-03).
        .route(
            "/admin/v1/admin_keys",
            get(list_admin_keys).post(create_admin_key),
        )
        .route("/admin/v1/admin_keys/:key_id", delete(revoke_admin_key))
        // OIDC provider routes (step 06-01).
        .route(
            "/admin/v1/oidc_providers",
            get(list_oidc_providers).post(create_oidc_provider),
        )
        .route(
            "/admin/v1/oidc_providers/:provider_id",
            patch(patch_oidc_provider).delete(delete_oidc_provider),
        )
        // Billing route (step 06-03).
        .route("/admin/v1/billing", get(get_billing))
        // Subscription routes (card-payments-backend, US-201/US-202).
        .route(
            "/admin/v1/billing/subscription",
            get(get_subscription).post(post_subscription),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            user_state.clone(),
            session_auth_middleware,
        ))
        .with_state(user_state);

    // Webhook sub-router: no session/operator auth (AC-203-01) — gated by
    // its own `stripe_signature_middleware` instead (US-203).
    let webhook_router = Router::<WebhookState>::new()
        .route("/admin/v1/webhooks/stripe", post(stripe_webhook_handler))
        .route_layer(axum::middleware::from_fn_with_state(
            webhook_state.clone(),
            stripe_signature_middleware,
        ))
        .with_state(webhook_state);

    Router::new()
        .merge(operator_router)
        .merge(dual_auth_router)
        .merge(public_router)
        .merge(webhook_router)
        .merge(session_router)
}

// ---------------------------------------------------------------------------
// Backward-compatible wrappers (called by lib.rs test server constructors).
// Use NoopEmailSender and a zeroed encryption key — test servers do not exercise
// UserAdminState routes at this stage.
//
// Each wrapper calls `get_or_install_prometheus_handle()` internally so no
// call-site changes are required (ADR-016).
// ---------------------------------------------------------------------------

pub fn build(
    system_db: Arc<SystemDb>,
    admin_key: String,
    credential_cache: Arc<CredentialCache>,
) -> Router {
    build_with_aws(system_db, admin_key, credential_cache, None)
}

pub fn build_with_aws(
    system_db: Arc<SystemDb>,
    admin_key: String,
    credential_cache: Arc<CredentialCache>,
    aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
) -> Router {
    build_with_secret_fetchers(system_db, admin_key, credential_cache, aws_secret_fetcher, None)
}

pub fn build_with_gcp(
    system_db: Arc<SystemDb>,
    admin_key: String,
    credential_cache: Arc<CredentialCache>,
    gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
) -> Router {
    build_with_secret_fetchers(system_db, admin_key, credential_cache, None, gcp_secret_fetcher)
}

pub fn build_with_secret_fetchers(
    system_db: Arc<SystemDb>,
    admin_key: String,
    credential_cache: Arc<CredentialCache>,
    aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
    gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
) -> Router {
    use crate::adapters::email::NoopEmailSender;
    let prometheus_handle = crate::observability::get_or_install_prometheus_handle();
    // These test-server wrappers (used by suites that predate card-payments-backend
    // and never exercise billing routes) get a no-I/O placeholder StripeGateway —
    // mirrors main.rs's own "billing unwired" placeholder-key fallback.
    let stripe_gateway = Arc::new(StripeGateway::new("stripe-secret-key-not-configured"));
    build_admin_router(
        system_db,
        admin_key,
        None,
        credential_cache,
        [0u8; 32],
        None,
        Arc::new(NoopEmailSender),
        aws_secret_fetcher,
        gcp_secret_fetcher,
        1000.0,
        prometheus_handle,
        stripe_gateway,
        String::new(),
    )
}
