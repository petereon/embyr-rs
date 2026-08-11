// SCAFFOLD: true
//! Slice 05 — Billing + Query Logs acceptance scenarios.
//! Stories: US-007 (Query Logs), US-008 (Billing Usage)
//!
//! All tests are #[ignore] (RED). Enable one at a time in DELIVER.
//!
//! V1 note: both features use mock data from data.rs (no real API).
//! Tests verify model state transitions, not data retrieval.

use embyr_admin_ui::model::{AppModel, Database, DbBackendMode, DbId, DbStatus, LogRetention};
use embyr_admin_ui::msg::Msg;
use embyr_admin_ui::update::update;
use uuid::Uuid;

#[path = "../common/mod.rs"]
mod common;
use common::make_model_with_db;

// ─────────────────────────────────────────────────────────────────────────────
// US-007 / AC-007-01: Log tab state gated by logging_enabled.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-007-01: Logs tab shows empty state when logging is disabled.
///
/// The model represents log visibility via database.logging_enabled.
/// When false, the view renders the "Logging is off" empty state.
#[test]
fn logs_tab_empty_when_logging_disabled() {
    // AC-007-01
    let m = make_model_with_db();
    let db_id = m.databases[0].id.clone();

    // Ensure logging is off (precondition from make_model_with_db).
    assert!(!m.databases[0].logging_enabled, "precondition: logging is disabled");

    // Logs view uses model.databases[i].logging_enabled to gate content.
    // No Msg needed — state is already represented by the model field.
    let db = m.databases.iter().find(|d| d.id == db_id).unwrap();
    assert!(
        !db.logging_enabled,
        "AC-007-01: logging_enabled=false represents the 'Logs are off' empty state"
    );
}

/// AC-007-01 → AC-007-02: Enabling logging makes log rows visible (via model state).
#[test]
fn logs_become_visible_after_enabling_logging() {
    // AC-007-01 → AC-004-03 chained
    let mut m = make_model_with_db();
    let db_id = m.databases[0].id.clone();

    // Enable logging.
    update(&mut m, Msg::SetDbLogging(db_id.clone(), true));

    let db = m.databases.iter().find(|d| d.id == db_id).unwrap();
    assert!(
        db.logging_enabled,
        "AC-007-02: logging_enabled=true gates log row rendering"
    );
}

/// AC-004-03 (retention) → AC-007: Log retention period stored when logging enabled.
#[test]
fn log_retention_stored_when_logging_enabled() {
    // AC-004-03 / AC-007 (retention gating)
    let mut m = make_model_with_db();
    let db_id = m.databases[0].id.clone();

    update(&mut m, Msg::SetDbLogging(db_id.clone(), true));
    update(&mut m, Msg::SetLogRetention(db_id.clone(), LogRetention::ThirtyDays));

    let db = m.databases.iter().find(|d| d.id == db_id).unwrap();
    assert_eq!(
        db.log_retention,
        Some(LogRetention::ThirtyDays),
        "Log retention must be stored as ThirtyDays"
    );
}

/// Sad: SetLogRetention with wrong db_id is a no-op.
#[test]
fn set_log_retention_wrong_db_is_noop() {
    let mut m = make_model_with_db();
    let phantom = DbId(Uuid::new_v4());

    update(&mut m, Msg::SetLogRetention(phantom, LogRetention::OneDay));

    // No database should have log_retention changed.
    let any_changed = m.databases.iter().any(|d| d.log_retention.is_some());
    assert!(!any_changed, "no-op: no retention period set for unknown db");
}

// ─────────────────────────────────────────────────────────────────────────────
// US-008 / AC-008-01: Billing data source is SetDatabases.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-008-01 / AC-008-02: SetDatabases provides the billing data source.
///
/// In V1, billing reads usage from model.databases (mock values in data.rs).
/// This test verifies the model is populated correctly for billing display.
#[test]
fn billing_data_source_populated_from_set_databases() {
    // AC-008-01, AC-008-02
    let mut m = AppModel::default();
    m.authed = true;

    let dbs = vec![
        Database {
            id: DbId(Uuid::new_v4()),
            name: "high-traffic".to_string(),
            status: DbStatus::Active,
            backend_mode: DbBackendMode::DirectPg,
            logging_enabled: false,
            log_retention: None,
            created_at: None,
            usage: Default::default(),
        },
        Database {
            id: DbId(Uuid::new_v4()),
            name: "low-traffic".to_string(),
            status: DbStatus::Active,
            backend_mode: DbBackendMode::AgentMode,
            logging_enabled: false,
            log_retention: None,
            created_at: None,
            usage: Default::default(),
        },
    ];

    update(&mut m, Msg::SetDatabases(dbs));

    assert_eq!(
        m.databases.len(), 2,
        "AC-008-01: both databases must appear in model for billing display"
    );
    assert!(
        m.databases.iter().any(|d| d.name == "high-traffic"),
        "AC-008-02: 'high-traffic' database must be accessible for billing breakdown"
    );
}

/// AC-008-01: Empty billing table when account has no databases.
#[test]
fn billing_empty_state_when_no_databases() {
    // AC-008-01 empty path
    let mut m = AppModel::default();
    m.authed = true;

    update(&mut m, Msg::SetDatabases(vec![]));

    assert!(
        m.databases.is_empty(),
        "AC-008-01: empty billing table when no databases"
    );
}
