use axum::{
    http::StatusCode,
    routing::{delete, get, post},
    Router,
};
use std::sync::Arc;

use embyr_core::admin::email::IEmailSender;

use crate::adapters::{
    aws_secret_fetcher::AwsSecretFetcher,
    credential_cache::CredentialCache,
    gcp_secret_fetcher::GcpSecretFetcher,
    system_db::SystemDb,
};

use super::handlers::get_project::get_project;
use super::handlers::lifecycle::{activate_project, delete_project, suspend_project};
use super::handlers::provision::provision;
use super::middleware::operator_auth::operator_auth_middleware;
use super::state::{OperatorState, UserAdminState};

/// Placeholder handler for routes added in subsequent steps (01-04 through 06-03).
async fn placeholder_handler() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

/// Build the admin router with all four sub-routers merged under /admin/v1.
///
/// Sub-router breakdown (ADR-009, AA-01):
///   - `operator_router`:   Bearer EMBYR_ADMIN_KEY; 4 mutating operator routes.
///   - `dual_auth_router`:  GET /projects/:id; operator bearer for now, real dual-auth in step 02-02.
///   - `public_router`:     No auth; signin/signout placeholders replaced in step 01-04.
///   - `session_router`:    Session cookie / admin_api_key Bearer; populated in steps 01-04 through 06-03.
pub fn build_admin_router(
    system_db: Arc<SystemDb>,
    admin_key: String,
    credential_cache: Arc<CredentialCache>,
    encryption_key: [u8; 32],
    email_sender: Arc<dyn IEmailSender + Send + Sync>,
    aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
    gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
) -> Router {
    let operator_state = OperatorState {
        system_db: system_db.clone(),
        admin_key: admin_key.clone(),
        credential_cache: credential_cache.clone(),
        aws_secret_fetcher,
        gcp_secret_fetcher,
    };
    let user_state = UserAdminState {
        system_db,
        encryption_key,
        email_sender,
        credential_cache,
        admin_key_env: admin_key,
    };

    // Operator sub-router: 4 mutating routes guarded by operator Bearer middleware.
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
        .route_layer(axum::middleware::from_fn_with_state(
            operator_state.clone(),
            operator_auth_middleware,
        ))
        .with_state(operator_state.clone());

    // Dual-auth sub-router: GET /projects/:id.
    // Uses OperatorState + operator bearer for now; real dual-auth middleware added in step 02-02.
    let dual_auth_router = Router::new()
        .route("/admin/v1/projects/:project_id", get(get_project))
        .with_state(operator_state);

    // Public sub-router: no auth.
    // Placeholder handlers replaced by real signin/signout handlers in step 01-04.
    let public_router = Router::new()
        .route("/admin/v1/auth/signin", post(placeholder_handler))
        .route("/admin/v1/auth/signout", post(placeholder_handler));

    // Session sub-router: empty until step 01-04.
    // UserAdminState is reserved for handlers added in subsequent steps.
    let session_router: Router = {
        // Reference user_state so it is available to session handlers added in later steps.
        // The let binding keeps it in scope here for future use without a Rust unused-variable warning.
        let _ = user_state;
        Router::new()
    };

    Router::new()
        .merge(operator_router)
        .merge(dual_auth_router)
        .merge(public_router)
        .merge(session_router)
}

// ---------------------------------------------------------------------------
// Backward-compatible wrappers (called by lib.rs test server constructors).
// Use NoopEmailSender and a zeroed encryption key — test servers do not exercise
// UserAdminState routes at this stage.
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
    build_admin_router(
        system_db,
        admin_key,
        credential_cache,
        [0u8; 32],
        Arc::new(NoopEmailSender),
        aws_secret_fetcher,
        gcp_secret_fetcher,
    )
}
