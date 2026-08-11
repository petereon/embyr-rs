// SCAFFOLD: true
//! Slice 01 — Auth + Dashboard acceptance scenarios.
//! Stories: US-001 (Authentication Gate), US-002 (Dashboard Health Overview)
//!
//! All tests are #[ignore] (RED). Enable one at a time in DELIVER.
//! Layer: in-memory acceptance (layer 2) — direct update() call.

use embyr_admin_ui::model::{AppModel, Database, DbStatus};
use embyr_admin_ui::msg::Msg;
use embyr_admin_ui::update::update;

#[path = "../common/mod.rs"]
mod common;
use common::make_model_with_db;

// ─────────────────────────────────────────────────────────────────────────────
// US-001 / AC-001-02: SignIn sets authed = true (V1 mock flow).
// ─────────────────────────────────────────────────────────────────────────────

/// AC-001-02, AC-001-08: Mock sign-in with any credentials grants session.
#[test]
fn sign_in_with_mock_credentials_sets_authed() {
    // AC-001-02
    let mut m = AppModel::default();
    m.authed = false;

    update(&mut m, Msg::SignIn);

    assert!(m.authed, "AC-001-02: SignIn must set model.authed = true (V1 mock)");
}

// ─────────────────────────────────────────────────────────────────────────────
// US-001 / AC-001-10: SignOut clears session.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-001-10: SignOut resets authed=false and navigates to login.
#[test]
fn sign_out_clears_session() {
    // AC-001-10
    let mut m = make_model_with_db();
    assert!(m.authed);

    update(&mut m, Msg::SignOut);

    assert!(!m.authed, "AC-001-10: SignOut must set authed = false");
}

// ─────────────────────────────────────────────────────────────────────────────
// US-001 / AC-001-04: TOTP lockout after 3 consecutive failures.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-001-04: Three consecutive TOTP failures lock the account for 15 minutes.
#[test]
fn three_totp_failures_lock_account_for_fifteen_minutes() {
    // AC-001-04
    let mut m = AppModel::default();
    m.authed = false;

    update(&mut m, Msg::TotpFailure);
    update(&mut m, Msg::TotpFailure);
    assert!(!m.account_locked, "must not lock before 3rd failure");

    update(&mut m, Msg::TotpFailure);

    assert!(m.account_locked, "AC-001-04: account must be locked after 3 TOTP failures");
    assert!(!m.authed, "AC-001-04: authed must remain false on lockout");
}

// ─────────────────────────────────────────────────────────────────────────────
// US-002 / AC-002-01: Dashboard shows card per database.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-002-01: SetDatabases populates model with the provided database list.
#[test]
fn dashboard_shows_all_databases_after_load() {
    // AC-002-01
    use embyr_admin_ui::model::{Database, DbBackendMode, DbId};
    use uuid::Uuid;

    let mut m = AppModel::default();
    m.authed = true;

    let dbs = vec![
        Database {
            id: DbId(Uuid::new_v4()),
            name: "prod-db".to_string(),
            status: DbStatus::Active,
            backend_mode: DbBackendMode::DirectPg,
            logging_enabled: false,
            log_retention: None,
            created_at: None,
            usage: Default::default(),
        },
        Database {
            id: DbId(Uuid::new_v4()),
            name: "staging-db".to_string(),
            status: DbStatus::Active,
            backend_mode: DbBackendMode::AgentMode,
            logging_enabled: false,
            log_retention: None,
            created_at: None,
            usage: Default::default(),
        },
    ];

    update(&mut m, Msg::SetDatabases(dbs));

    assert_eq!(m.databases.len(), 2, "AC-002-01: model.databases must reflect the loaded list");
    assert!(
        m.databases.iter().any(|d| d.name == "prod-db"),
        "AC-002-01: 'prod-db' must appear in model.databases"
    );
    assert!(
        m.databases.iter().any(|d| d.name == "staging-db"),
        "AC-002-01: 'staging-db' must appear in model.databases"
    );
}

/// AC-002-01 (empty state): Zero databases after SetDatabases([]).
#[test]
fn dashboard_empty_state_when_no_databases() {
    // AC-002-01 empty path
    let mut m = AppModel::default();
    m.authed = true;
    m.databases = vec![
        Database {
            id: embyr_admin_ui::model::DbId(uuid::Uuid::new_v4()),
            name: "to-be-cleared".to_string(),
            status: DbStatus::Active,
            backend_mode: embyr_admin_ui::model::DbBackendMode::DirectPg,
            logging_enabled: false,
            log_retention: None,
            created_at: None,
            usage: Default::default(),
        }
    ];

    update(&mut m, Msg::SetDatabases(vec![]));

    assert!(
        m.databases.is_empty(),
        "AC-002-01: empty state — SetDatabases([]) must result in no databases"
    );
}

/// AC-002-05: Suspended databases remain in model but Deleted databases are excluded.
#[test]
fn dashboard_suspended_visible_deleted_hidden() {
    // AC-002-05
    use embyr_admin_ui::model::{DbBackendMode, DbId};
    use uuid::Uuid;

    let mut m = AppModel::default();
    m.authed = true;

    let suspended = Database {
        id: DbId(Uuid::new_v4()),
        name: "suspended-db".to_string(),
        status: DbStatus::Suspended,
        backend_mode: DbBackendMode::DirectPg,
        logging_enabled: false,
        log_retention: None,
        created_at: None,
        usage: Default::default(),
    };
    let deleted = Database {
        id: DbId(Uuid::new_v4()),
        name: "deleted-db".to_string(),
        status: DbStatus::Deleted,
        backend_mode: DbBackendMode::DirectPg,
        logging_enabled: false,
        log_retention: None,
        created_at: None,
        usage: Default::default(),
    };

    update(&mut m, Msg::SetDatabases(vec![suspended, deleted]));

    assert!(
        m.databases.iter().any(|d| d.name == "suspended-db"),
        "AC-002-05: suspended databases must appear in model"
    );
    assert!(
        !m.databases.iter().any(|d| d.name == "deleted-db"),
        "AC-002-05: deleted databases must not appear in model"
    );
}
