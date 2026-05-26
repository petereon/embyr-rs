use axum::{routing::post, Router};
use std::sync::Arc;

use crate::adapters::system_db::SystemDb;

use super::handlers::provision::{provision, AdminState};

pub fn build(system_db: Arc<SystemDb>, admin_key: String) -> Router {
    let state = AdminState {
        system_db,
        admin_key,
    };

    Router::new()
        .route("/admin/v1/projects", post(provision))
        .with_state(state)
}
