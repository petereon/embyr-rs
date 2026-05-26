use axum::{
    routing::{delete, get, post},
    Router,
};
use std::sync::Arc;

use crate::adapters::{credential_cache::CredentialCache, system_db::SystemDb};

use super::handlers::get_project::get_project;
use super::handlers::lifecycle::{activate_project, delete_project, suspend_project};
use super::handlers::provision::{provision, AdminState};

pub fn build(
    system_db: Arc<SystemDb>,
    admin_key: String,
    credential_cache: Arc<CredentialCache>,
) -> Router {
    let state = AdminState {
        system_db,
        admin_key,
        credential_cache,
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
