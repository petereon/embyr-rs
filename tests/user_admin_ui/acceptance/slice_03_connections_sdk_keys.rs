// SCAFFOLD: true
//! Slice 03 — Connections + project API key panel acceptance scenarios.
//! Stories: US-005 (Backend Config), US-006 (project API key, read-only; AC-AKR-17 pending slice 4)
//!
//! All tests are enabled (un-ignored 2026-11-13) except any carrying an explicit
//! `#[ignore = "..."]` reason string.

use chrono::{TimeZone, Utc};
use embyr_admin_ui::model::{DbId, DbPatch, ProjectApiKeyInfo};
use embyr_admin_ui::msg::Msg;
use embyr_admin_ui::update::update;
use uuid::Uuid;

#[path = "../common/mod.rs"]
mod common;
use common::make_model_with_db;

// ─────────────────────────────────────────────────────────────────────────────
// US-005 / AC-005-03: PatchDb updates backend config fields.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-005-03: PatchDb(id, Dsn) updates the DSN without removing the database.
#[test]
fn patch_db_updates_dsn_field() {
    // AC-005-03
    let mut m = make_model_with_db();
    let db_id = m.databases[0].id.clone();

    update(&mut m, Msg::PatchDb(db_id.clone(), DbPatch::Dsn("postgres://newhost:5432/prod".to_string())));

    let db = m.databases.iter().find(|d| d.id == db_id);
    assert!(db.is_some(), "AC-005-03: database must still exist after PatchDb");
}

/// AC-005-03: PatchDb(id, AgentEndpoint) updates the agent endpoint.
#[test]
fn patch_db_updates_agent_endpoint() {
    // AC-005-03
    let mut m = make_model_with_db();
    let db_id = m.databases[0].id.clone();

    update(
        &mut m,
        Msg::PatchDb(db_id.clone(), DbPatch::AgentEndpoint("10.0.0.5:9191".to_string())),
    );

    let db = m.databases.iter().find(|d| d.id == db_id);
    assert!(db.is_some(), "AC-005-03: database must still exist after agent endpoint patch");
}

/// Sad: PatchDb with non-existent DbId is a no-op (no panic, no database modified).
#[test]
fn patch_db_unknown_id_is_noop() {
    let mut m = make_model_with_db();
    let count_before = m.databases.len();
    let phantom = DbId(Uuid::new_v4().to_string());

    update(&mut m, Msg::PatchDb(phantom, DbPatch::Dsn("postgres://x/y".to_string())));

    assert_eq!(m.databases.len(), count_before, "no-op: database count unchanged");
}

// ─────────────────────────────────────────────────────────────────────────────
// US-006: read-only project API key panel state (api-key-rotation slice 3).
// ─────────────────────────────────────────────────────────────────────────────

fn rotated_info() -> ProjectApiKeyInfo {
    ProjectApiKeyInfo {
        rotated_at: Some(Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap()),
        previous_expires_at: Some(Utc.with_ymd_and_hms(2026, 11, 2, 9, 0, 0).unwrap()),
    }
}

/// ApiKeyInfoLoaded stores rotation metadata for the database; absent entry = never rotated.
#[test]
fn api_key_info_loaded_is_stored_for_db() {
    let mut m = make_model_with_db();
    let db_id = m.databases[0].id.clone();
    assert!(m.api_key_info.get(&db_id).is_none(), "never rotated: no entry");

    update(&mut m, Msg::ApiKeyInfoLoaded(db_id.clone(), rotated_info()));

    assert_eq!(m.api_key_info.get(&db_id), Some(&rotated_info()));
}

/// DeleteDatabase drops the database's API key metadata.
#[test]
fn delete_database_drops_api_key_info() {
    let mut m = make_model_with_db();
    let db_id = m.databases[0].id.clone();
    m.api_key_info.insert(db_id.clone(), rotated_info());

    update(&mut m, Msg::DeleteDatabase(db_id.clone()));

    assert!(m.api_key_info.get(&db_id).is_none(), "metadata must go with the database");
}

/// Sad: ApiKeyInfoLoaded for an unknown db_id is a no-op (no entry created).
#[test]
fn api_key_info_for_unknown_db_is_noop() {
    let mut m = make_model_with_db();
    let phantom_db = DbId(Uuid::new_v4().to_string());

    update(&mut m, Msg::ApiKeyInfoLoaded(phantom_db.clone(), rotated_info()));

    assert!(m.api_key_info.get(&phantom_db).is_none());
}
