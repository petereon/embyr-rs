use axum::{
    routing::{delete, get, patch, post},
    Router,
};
use std::sync::Arc;

use embyr_core::admin::email::IEmailSender;
use metrics_exporter_prometheus::PrometheusHandle;

use crate::adapters::{
    aws_secret_fetcher::AwsSecretFetcher, cap_status_cache::CapStatusCache,
    credential_cache::CredentialCache, gcp_secret_fetcher::GcpSecretFetcher,
    stripe_gateway::StripeGateway, system_db::SystemDb,
};

use super::handlers::admin_keys::{create_admin_key, list_admin_keys, revoke_admin_key};
use super::handlers::auth::{oidc_callback, signin, signout};
use super::handlers::billing::get_billing;
use super::handlers::billing_metering::run_metering;
use super::handlers::billing_subscription::{get_subscription, post_subscription};
use super::handlers::oidc_providers::{
    create_oidc_provider, delete_oidc_provider, list_oidc_providers, patch_oidc_provider,
};
// client-auth (US-01/US-03/US-04, ADR-025): credential register/rotate/verify.
use super::handlers::client_identity::{
    register_client_identity_credential, rotate_client_identity_credential,
    verify_client_identity_credential,
};
// client-auth-hosted-identity (US-01, ADR-036): admin enablement action.
use super::handlers::hosted_identity::enable_hosted_identity;
// oauth-providers (US-01, ADR-037): admin registration of a project's
// Google OAuth Client ID.
use super::handlers::oauth_providers::register_google_oauth_provider;
// anonymous-sessions (US-01, ADR-043): admin enablement action.
use super::handlers::anonymous_identity::enable_anonymous_identity;
// security-rules (US-01/US-05, ADR-029): access-rule define/redefine + simulate.
// security-rules-write-path (US-01, ADR-030): independent write-rule define/redefine.
// security-rules-query-path (US-07, ADR-031): simulate a candidate query
// shape before shipping client code — a NEW, DISTINCT sibling handler, not
// an extension of simulate_access_rule.
// security-rules-collection-group-rules (US-01, ADR-032): independent
// collection-group rule define/redefine — a NEW, distinct route/handler,
// mirroring define_write_access_rule's shape (ADR-032 § Decision — Admin
// Surface, rejected alternative: not a branch on any existing route).
// security-rules-collection-group-rules (US-07, LAST slice, ADR-032):
// simulate a candidate collection-group query before shipping — a NEW
// sibling handler/route to simulate_query_compliance, not an extension of
// it in place (ADR-032 § simulate_group_query_compliance).
// security-rules-operations (US-02, ADR-035): retrieve a collection's
// access-rule history — a NEW any-role, read-only route, mirroring
// simulate_access_rule's identical shape.
// security-rules-cel-parity (US-01, Slice 01, ADR-062): import + decompose
// a real .rules file into the existing per-collection admin API — a NEW,
// distinct route/handler (Owner/Admin, gated in-handler, mirrors
// define_access_rule's exact gate).
use super::handlers::access_rules::{
    define_access_rule, define_group_access_rule, define_write_access_rule,
    get_access_rule_history, get_group_access_rule_history, get_write_access_rule_history,
    import_rules_file, simulate_access_rule, simulate_group_query_compliance,
    simulate_query_compliance, simulate_routed_access_rule,
};
use super::handlers::composite_indexes::{
    create_composite_index, delete_composite_index, list_composite_indexes,
};
use super::handlers::get_project::get_project;
use super::handlers::lifecycle::{activate_project, delete_project, suspend_project};
use super::handlers::members::{change_member_role, invite_member, list_members, remove_member};
use super::handlers::metrics::get_project_metrics;
use super::handlers::projects::{list_projects, patch_project};
use super::handlers::prometheus_metrics::get_prometheus_metrics;
use super::handlers::provision::provision;
use super::handlers::query_logs::list_query_logs;
use super::handlers::sdk_keys::{create_sdk_key, list_sdk_keys, revoke_sdk_key};
use super::handlers::service_accounts::{
    create_service_account, delete_service_account, list_service_accounts,
};
use super::handlers::webhooks_stripe::stripe_webhook_handler;
use super::middleware::dual_auth::dual_auth_middleware;
use super::middleware::operator_auth::operator_auth_middleware;
use super::middleware::session_auth::session_auth_middleware;
use super::middleware::stripe_signature::stripe_signature_middleware;
use super::state::{OperatorState, UserAdminState, WebhookState};
use crate::middleware::signin_rate_limit::SigninRateLimiter;

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
    // stripe-webhook-secret-required (D1/D2): `None` = Stripe billing not
    // enabled for this deployment — the webhook sub-router is not mounted at
    // all (AC-WHS-01). `Some(secret)` = mount it, gated by
    // `stripe_signature_middleware` exactly as today.
    webhook_signing_secret: Option<String>,
    // card-payments-backend (ADR-020, step 03-01): shared with the
    // composition root's `CapUsageRefresher::spawn` call so the background
    // task writes into the SAME cache instance `get_subscription` reads from
    // (previously each call built its own disconnected `CapStatusCache`,
    // silently defeating AC-206-01/02/05 regardless of `run_cycle`'s own
    // correctness).
    cap_status_cache: Arc<CapStatusCache>,
    // admin-signin-hardening (ADR-076): per-source-IP token bucket gating
    // POST /admin/v1/auth/signin — threaded into UserAdminState.
    signin_rate_limiter: Arc<SigninRateLimiter>,
) -> Router {
    let operator_state = OperatorState {
        system_db: system_db.clone(),
        admin_key: admin_key.clone(),
        admin_key_previous: admin_key_previous.clone(),
        credential_cache: credential_cache.clone(),
        aws_secret_fetcher: aws_secret_fetcher.clone(),
        gcp_secret_fetcher: gcp_secret_fetcher.clone(),
        rate_limit_capacity,
        prometheus_handle,
        stripe_gateway: stripe_gateway.clone(),
    };

    // Webhook sub-router: no session/operator auth (AC-203-01) — gated by its
    // own `stripe_signature_middleware` instead (US-203). Built only when
    // Stripe billing is enabled (stripe-webhook-secret-required, D1/D2) —
    // `None` means the route must not exist at all (AC-WHS-01). Built here,
    // before `system_db`/`credential_cache`/`stripe_gateway` are moved into
    // `user_state` below.
    let webhook_router: Option<Router> = webhook_signing_secret.map(|secret| {
        let webhook_state = WebhookState {
            system_db: system_db.clone(),
            stripe_gateway: stripe_gateway.clone(),
            webhook_signing_secret: secret,
            credential_cache: credential_cache.clone(),
        };
        Router::<WebhookState>::new()
            .route("/admin/v1/webhooks/stripe", post(stripe_webhook_handler))
            .route_layer(axum::middleware::from_fn_with_state(
                webhook_state.clone(),
                stripe_signature_middleware,
            ))
            .with_state(webhook_state)
    });

    let user_state = UserAdminState {
        system_db,
        encryption_key,
        encryption_key_previous,
        email_sender,
        credential_cache,
        admin_key_env: admin_key,
        admin_key_previous_env: admin_key_previous,
        stripe_gateway,
        cap_status_cache,
        aws_secret_fetcher,
        gcp_secret_fetcher,
        signin_rate_limiter,
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
        // card-payments-backend (US-205, step 02-01): operator-triggered
        // nightly usage metering. Mounted on operator_router so
        // operator_auth_middleware enforces AC-205-05 (no in-handler auth
        // check needed).
        .route("/admin/v1/billing/run-metering", post(run_metering))
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
        // client-auth (US-01/US-03/US-04, ADR-025): credential lifecycle.
        // Owner/Admin gate for register/rotate enforced inside the handlers
        // (mirrors sdk_keys.rs's own in-handler role check); verify (debug
        // check) is any role, also gated in-handler.
        .route(
            "/admin/v1/projects/:project_id/client_identity_credential",
            post(register_client_identity_credential),
        )
        .route(
            "/admin/v1/projects/:project_id/client_identity_credential/rotate",
            post(rotate_client_identity_credential),
        )
        .route(
            "/admin/v1/projects/:project_id/client_identity_credential/verify",
            post(verify_client_identity_credential),
        )
        // client-auth-hosted-identity (US-01, ADR-036): enable embyr-hosted
        // email/password identity for a project (Owner/Admin, gated
        // in-handler) — mirrors client_identity_credential's identical
        // in-handler-gate shape.
        .route(
            "/admin/v1/projects/:project_id/hosted_identity/enable",
            post(enable_hosted_identity),
        )
        // oauth-providers (US-01, ADR-037): register/redefine a project's
        // Google OAuth Client ID (Owner/Admin, gated in-handler) — mirrors
        // hosted_identity/enable's identical in-handler-gate shape.
        .route(
            "/admin/v1/projects/:project_id/oauth_providers/google",
            post(register_google_oauth_provider),
        )
        // anonymous-sessions (US-01, ADR-043): enable anonymous end-user
        // identity for a project (Owner/Admin, gated in-handler) — mirrors
        // oauth_providers/google's identical in-handler-gate shape, no
        // backend_mode check (ADR-044).
        .route(
            "/admin/v1/projects/:project_id/anonymous_identity/enable",
            post(enable_anonymous_identity),
        )
        // security-rules (US-01/US-05, ADR-029): define/redefine an access
        // rule (Owner/Admin, gated in-handler) + simulate a candidate rule
        // (any role, read-only, gated in-handler) — mirrors
        // client_identity_credential's identical in-handler-gate shape.
        .route(
            "/admin/v1/projects/:project_id/access_rules",
            post(define_access_rule),
        )
        .route(
            "/admin/v1/projects/:project_id/access_rules/simulate",
            post(simulate_access_rule),
        )
        // security-rules-operations (US-02, ADR-035): retrieve a collection's
        // complete, correctly-ordered access-rule history (any role,
        // read-only, gated in-handler) — a NEW, DISTINCT sibling route, not
        // a branch on the define/simulate routes above.
        .route(
            "/admin/v1/projects/:project_id/access_rules/:collection_path/history",
            get(get_access_rule_history),
        )
        // security-rules-query-path (US-07, LAST slice, ADR-031): simulate
        // a candidate query filter shape against a candidate/published rule
        // (any role, read-only, gated in-handler) — a NEW, DISTINCT
        // sibling route, not a branch on the read-rule simulate route above.
        .route(
            "/admin/v1/projects/:project_id/access_rules/simulate_query",
            post(simulate_query_compliance),
        )
        // security-rules-write-path (US-01, ADR-030): independent
        // write-rule define/redefine (Owner/Admin, gated in-handler) — a
        // NEW, distinct route/handler, not a branch on the read-rule route
        // above (ADR-030 § Decision — Composition, rejected alternative).
        .route(
            "/admin/v1/projects/:project_id/write_access_rules",
            post(define_write_access_rule),
        )
        // security-rules-operations (Slice 04, ADR-035): retrieve a
        // collection's complete, correctly-ordered WRITE-rule history (any
        // role, read-only, gated in-handler) — a NEW, DISTINCT sibling
        // route, mirroring get_access_rule_history's identical shape,
        // structurally independent of it (AC-17-169).
        .route(
            "/admin/v1/projects/:project_id/write_access_rules/:collection_path/history",
            get(get_write_access_rule_history),
        )
        // security-rules-collection-group-rules (US-01, ADR-032):
        // independent collection-group rule define/redefine (Owner/Admin,
        // gated in-handler) — a NEW, distinct route/handler, not a branch
        // on the exact-path routes above.
        .route(
            "/admin/v1/projects/:project_id/group_access_rules",
            post(define_group_access_rule),
        )
        // security-rules-operations (Slice 05, ADR-035, LAST slice of this
        // feature): retrieve a collection-group rule's complete,
        // correctly-ordered history (any role, read-only, gated in-handler)
        // — a NEW, DISTINCT sibling route, mirroring
        // get_write_access_rule_history's identical shape, structurally
        // independent of it (AC-17-172).
        .route(
            "/admin/v1/projects/:project_id/group_access_rules/:collection_id/history",
            get(get_group_access_rule_history),
        )
        // security-rules-collection-group-rules (US-07, LAST slice, ADR-032):
        // simulate a candidate collection-group query before shipping (any
        // role, read-only, gated in-handler) — a NEW, DISTINCT sibling
        // route to simulate_query, not a branch on it.
        .route(
            "/admin/v1/projects/:project_id/access_rules/simulate_group_query",
            post(simulate_group_query_compliance),
        )
        // security-rules-cel-parity (US-01, Slice 01, ADR-062): import a
        // real .rules file (Owner/Admin, gated in-handler) — a NEW,
        // DISTINCT sibling route, not a branch on the define/simulate
        // routes above.
        .route(
            "/admin/v1/projects/:project_id/access_rules/import",
            post(import_rules_file),
        )
        // security-rules-cel-path-matching (US-06, LAST slice, ADR-063):
        // simulate a candidate multi-segment pattern's own ROUTING (any
        // role, read-only, gated in-handler) — a NEW, DISTINCT sibling
        // route to simulate_access_rule, not a branch on it (DDD-PM-9).
        .route(
            "/admin/v1/projects/:project_id/access_rules/simulate_route",
            post(simulate_routed_access_rule),
        )
        // firestore-composite-indexes-admin-api (Slice 01, US-01, ADR-068):
        // Create/List a project's own composite indexes — closes the
        // last-mile gap in the query path's own existing, unmodified
        // `IndexManager::is_index_ready` gate (Owner/Admin for create, any
        // role read-only for list, mirrors `define_access_rule`'s/
        // `get_access_rule_history`'s identical gate shapes).
        .route(
            "/admin/v1/projects/:project_id/indexes",
            get(list_composite_indexes).post(create_composite_index),
        )
        .route(
            "/admin/v1/projects/:project_id/indexes/:index_id",
            delete(delete_composite_index),
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
        .route(
            "/admin/v1/members/:member_id/role",
            patch(change_member_role),
        )
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

    let mut router = Router::new()
        .merge(operator_router)
        .merge(dual_auth_router)
        .merge(public_router)
        .merge(session_router);
    if let Some(webhook_router) = webhook_router {
        router = router.merge(webhook_router);
    }
    router
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
    build_with_secret_fetchers(
        system_db,
        admin_key,
        credential_cache,
        aws_secret_fetcher,
        None,
    )
}

pub fn build_with_gcp(
    system_db: Arc<SystemDb>,
    admin_key: String,
    credential_cache: Arc<CredentialCache>,
    gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
) -> Router {
    build_with_secret_fetchers(
        system_db,
        admin_key,
        credential_cache,
        None,
        gcp_secret_fetcher,
    )
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
    // In-process only (mirrors RateLimiter::new(_, _, None)'s existing
    // test-safety precedent) — no acceptance test run under a shared
    // testcontainers Postgres instance can pollute another test's throttle
    // counters via a shared table row.
    let signin_rate_limiter = SigninRateLimiter::new(150.0, 10.0 / 60.0);
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
        None,
        Arc::new(CapStatusCache::new()),
        signin_rate_limiter,
    )
}
