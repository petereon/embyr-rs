use axum::{routing::{get, post}, Router};
use std::sync::Arc;

use crate::adapters::system_db::SystemDb;

use super::handlers::get_project::get_project;
use super::handlers::provision::{provision, AdminState};

pub fn build(system_db: Arc<SystemDb>, admin_key: String) -> Router {
    let state = AdminState {
        system_db,
        admin_key,
    };

    Router::new()
        .route("/admin/v1/projects", post(provision))
        .route("/admin/v1/projects/:project_id", get(get_project))
        .with_state(state)
}
