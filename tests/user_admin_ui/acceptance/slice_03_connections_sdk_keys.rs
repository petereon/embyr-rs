// SCAFFOLD: true
//! Slice 03 — Connections + SDK Keys acceptance scenarios.
//! Stories: US-005 (Backend Config), US-006 (SDK Keys)
//!
//! All tests are #[ignore] (RED). Enable one at a time in DELIVER.

use embyr_admin_ui::model::{DbId, DbPatch, KeyId, SdkKey};
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
    let phantom = DbId(Uuid::new_v4());

    update(&mut m, Msg::PatchDb(phantom, DbPatch::Dsn("postgres://x/y".to_string())));

    assert_eq!(m.databases.len(), count_before, "no-op: database count unchanged");
}

// ─────────────────────────────────────────────────────────────────────────────
// US-006 / AC-006-02: Create SDK key shows full key once.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-006-02: SdkKeyCreated appends the key to sdk_keys[db_id] with correct prefix.
#[test]
#[ignore = "RED — implement update(_, Msg::SdkKeyCreated)"]
fn create_sdk_key_appends_to_model() {
    // AC-006-02
    let mut m = make_model_with_db();
    let db_id = m.databases[0].id.clone();
    // Ensure entry exists.
    m.sdk_keys.entry(db_id.clone()).or_default();
    let count_before = m.sdk_keys[&db_id].len();

    let key = SdkKey {
        id: KeyId(Uuid::new_v4()),
        db_id: db_id.clone(),
        name: "prod-key".to_string(),
        prefix: "embyr_sd".to_string(),
        created_at: None,
    };

    update(&mut m, Msg::SdkKeyCreated { db_id: db_id.clone(), key: key.clone() });

    let keys = m.sdk_keys.get(&db_id).expect("sdk_keys entry must exist");
    assert_eq!(
        keys.len(), count_before + 1,
        "AC-006-02: SdkKeyCreated must append one key"
    );
    assert!(
        keys.iter().any(|k| k.name == "prod-key"),
        "AC-006-02: 'prod-key' must appear in sdk_keys"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// US-006 / AC-006-04: Revoke SDK key.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-006-04: RevokeSdkKey removes the matching key from sdk_keys[db_id].
#[test]
#[ignore = "RED — implement update(_, Msg::RevokeSdkKey)"]
fn revoke_sdk_key_removes_it_from_model() {
    // AC-006-04
    let mut m = make_model_with_db();
    let db_id = m.databases[0].id.clone();

    let key_id = KeyId(Uuid::new_v4());
    let key = SdkKey {
        id: key_id.clone(),
        db_id: db_id.clone(),
        name: "old-key".to_string(),
        prefix: "embyr_sd".to_string(),
        created_at: None,
    };
    m.sdk_keys.insert(db_id.clone(), vec![key]);

    update(&mut m, Msg::RevokeSdkKey(db_id.clone(), key_id.clone()));

    let keys = m.sdk_keys.get(&db_id).expect("sdk_keys entry must exist");
    assert!(
        !keys.iter().any(|k| k.id == key_id),
        "AC-006-04: revoked key must be absent from sdk_keys"
    );
}

/// AC-006-05: DeleteDatabase cascades to revoke all SDK keys.
#[test]
#[ignore = "RED — implement DeleteDatabase cascade to SDK keys"]
fn delete_database_cascades_revokes_all_sdk_keys() {
    // AC-006-05
    let mut m = make_model_with_db();
    let db_id = m.databases[0].id.clone();

    let keys: Vec<SdkKey> = (0..3).map(|i| SdkKey {
        id: KeyId(Uuid::new_v4()),
        db_id: db_id.clone(),
        name: format!("key-{}", i),
        prefix: "embyr_sd".to_string(),
        created_at: None,
    }).collect();
    m.sdk_keys.insert(db_id.clone(), keys);

    update(&mut m, Msg::DeleteDatabase(db_id.clone()));

    let remaining = m.sdk_keys.get(&db_id).map(|v| v.len()).unwrap_or(0);
    assert_eq!(remaining, 0, "AC-006-05: all SDK keys must be revoked when database is deleted");
}

/// Sad: RevokeSdkKey with non-existent key_id is a no-op (no panic).
#[test]
#[ignore = "RED — implement RevokeSdkKey no-op guard"]
fn revoke_sdk_key_nonexistent_is_noop() {
    let mut m = make_model_with_db();
    let db_id = m.databases[0].id.clone();
    m.sdk_keys.insert(db_id.clone(), vec![]);
    let phantom_key_id = KeyId(Uuid::new_v4());

    update(&mut m, Msg::RevokeSdkKey(db_id.clone(), phantom_key_id));

    assert_eq!(m.sdk_keys[&db_id].len(), 0, "no-op: key list unchanged");
}

/// Sad: SdkKeyCreated with unknown db_id is a no-op (no new entry created).
#[test]
#[ignore = "RED — implement SdkKeyCreated no-op guard on unknown db"]
fn create_sdk_key_for_unknown_db_is_noop() {
    let mut m = make_model_with_db();
    let phantom_db = DbId(Uuid::new_v4());
    let sdk_map_size_before = m.sdk_keys.len();

    let key = SdkKey {
        id: KeyId(Uuid::new_v4()),
        db_id: phantom_db.clone(),
        name: "ghost-key".to_string(),
        prefix: "embyr_sd".to_string(),
        created_at: None,
    };

    update(&mut m, Msg::SdkKeyCreated { db_id: phantom_db, key });

    assert_eq!(m.sdk_keys.len(), sdk_map_size_before, "no-op: sdk_keys map size unchanged");
}
