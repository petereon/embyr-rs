use axum::{
    routing::{delete, get, post},
    Router,
};
use std::sync::Arc;

use crate::adapters::{
    aws_secret_fetcher::AwsSecretFetcher,
    credential_cache::CredentialCache,
    gcp_secret_fetcher::GcpSecretFetcher,
    system_db::SystemDb,
};

use super::handlers::get_project::get_project;
use super::handlers::lifecycle::{activate_project, delete_project, suspend_project};
use super::handlers::provision::{provision, AdminState};

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
    let state = AdminState {
        system_db,
        admin_key,
        credential_cache,
        aws_secret_fetcher,
        gcp_secret_fetcher,
    };

    Router::new()
        .route("/admin/v1/projects", post(provision))
        .route("/admin/v1/projects/:project_id", get(get_project))
        .route(
            "/admin/v1/projects/:project_id/suspend",
            post(suspend_project),
        )
        .route(
            "/admin/v1/projects/:project_id/activate",
            post(activate_project),
        )
        .route(
            "/admin/v1/projects/:project_id",
            delete(delete_project),
        )
        .with_state(state)
}
