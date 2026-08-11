// SCAFFOLD: true
//! Common test infrastructure for user-admin-ui acceptance tests.

// make_model_with_db and assert_owner_invariant are used across multiple slice
// files; each slice re-includes this module via #[path], so the compiler may
// flag helpers unused in slices that don't call every function.
#![allow(dead_code)]

pub mod arb;

use embyr_admin_ui::model::{AppModel, Database, DbId, DbStatus, Role};
use uuid::Uuid;

/// Construct a minimal authenticated AppModel with one active database
/// and two members (one Owner, one Viewer).
///
/// Used as the baseline "Given" state for focused scenario tests.
pub fn make_model_with_db() -> AppModel {
    use embyr_admin_ui::model::{DbBackendMode, Member, UserId};

    let db_id = DbId(Uuid::new_v4());
    let db = Database {
        id: db_id.clone(),
        name: "test-db".to_string(),
        status: DbStatus::Active,
        backend_mode: DbBackendMode::DirectPg,
        logging_enabled: false,
        log_retention: None,
        created_at: None,
        usage: Default::default(),
    };

    let owner = Member {
        id: UserId(Uuid::new_v4()),
        email: "owner@test.com".to_string(),
        display_name: None,
        role: Role::Owner,
        pending: false,
        mfa_enabled: true,
        last_login: None,
    };

    let viewer = Member {
        id: UserId(Uuid::new_v4()),
        email: "viewer@test.com".to_string(),
        display_name: None,
        role: Role::Viewer,
        pending: false,
        mfa_enabled: false,
        last_login: None,
    };

    let mut model = AppModel::default();
    model.authed = true;
    model.databases = vec![db];
    model.members = vec![owner, viewer];
    model.sdk_keys.insert(db_id, vec![]);
    model
}

/// Assert that `model.members` contains at least one Owner.
///
/// Used as the sole-Owner invariant assertion after RemoveMember / SetMemberRole.
pub fn assert_owner_invariant(model: &AppModel) {
    assert!(
        model.members.iter().any(|m| m.role == Role::Owner),
        "invariant violated: model.members has no Owner"
    );
}
