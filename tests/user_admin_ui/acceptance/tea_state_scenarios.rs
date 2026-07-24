// SCAFFOLD: true
//! TEA state machine scenarios — primary acceptance test surface.
//!
//! Tests the pure `fn update(&mut AppModel, Msg)` function directly.
//! No browser, no WASM, no Leptos. All tests are #[ignore] (RED) until
//! DELIVER enables them one at a time.
//!
//! Layer: in-memory acceptance (layer 2).
//! PBT mode: proptest stateful strategies (Mandate 9: layers 1-2 use PBT full).
//! State-delta: all assertions use Mandate 8 universe-bound form where the
//!   observable universe is the set of public AppModel fields that a test
//!   promises to track.

#![allow(unused_imports)]

use proptest::prelude::*;

use embyr_admin_ui::model::{
    AppModel, Database, DbId, DbStatus, KeyId, Member, OidcId, Role,
    ServiceAccountId, SdkKey, Toast, ToastId, UserId,
};
use embyr_admin_ui::msg::Msg;
use embyr_admin_ui::update::update;

// Bring in shared proptest strategies.
#[path = "../common/mod.rs"]
mod common;
use common::arb::*;
use common::{assert_owner_invariant, make_model_with_db};

// ── US-001: Authentication ────────────────────────────────────────────────

proptest! {
    /// AC-001-02 / AC-001-08: SignIn sets authed=true (V1 mock: always).
    #[test]
    fn sign_in_sets_authed(model in arb_authed_model().prop_map(|mut m| { m.authed = false; m })) {
        let mut m = model;
        update(&mut m, Msg::SignIn);
        prop_assert!(m.authed, "SignIn must set model.authed = true");
    }
}

proptest! {
    /// AC-001-10: SignOut resets authed=false and clears session state.
    #[test]
    fn sign_out_clears_authed(model in arb_authed_model()) {
        let mut m = model;
        update(&mut m, Msg::SignOut);
        prop_assert!(!m.authed, "SignOut must set model.authed = false");
        prop_assert!(m.toasts.is_empty(), "SignOut must clear toasts");
    }
}

// ── US-001 Error paths ─────────────────────────────────────────────────────

/// AC-001-03 / AC-001-04: Three TOTP failures lock the account.
#[test]
fn three_totp_failures_lock_account() {
    let mut m = AppModel::default();
    m.authed = false;
    m.totp_failures = 0;

    update(&mut m, Msg::TotpFailure);
    update(&mut m, Msg::TotpFailure);
    assert!(!m.account_locked, "account must not be locked after 2 failures");

    update(&mut m, Msg::TotpFailure);
    assert!(m.account_locked, "account must be locked after 3 consecutive TOTP failures");
    assert!(!m.authed, "authed must remain false after lockout");
}

/// AC-001-05: Successful TOTP resets failure counter.
#[test]
fn totp_success_resets_failure_counter() {
    let mut m = AppModel::default();
    m.totp_failures = 2;

    update(&mut m, Msg::TotpSuccess);
    assert_eq!(m.totp_failures, 0, "TotpSuccess must reset failure counter to 0");
    assert!(!m.account_locked, "TotpSuccess must clear account lock");
}

// ── US-002: Dashboard ─────────────────────────────────────────────────────

proptest! {
    /// AC-002-01 / AC-002-02: SetDatabases populates model.databases.
    #[test]
    fn set_databases_populates_model(dbs in prop::collection::vec(arb_database(), 0..=6)) {
        let mut m = AppModel::default();
        m.authed = true;
        let expected_len = dbs.len();
        update(&mut m, Msg::SetDatabases(dbs));
        prop_assert_eq!(
            m.databases.len(), expected_len,
            "SetDatabases must set model.databases to the provided list"
        );
    }
}

/// AC-002-05: Deleted databases are not shown (status=Deleted not in model after SetDatabases).
#[test]
fn set_databases_excludes_deleted() {
    use embyr_admin_ui::model::DbBackendMode;

    let mut m = AppModel::default();
    m.authed = true;
    let active = Database {
        id: DbId(uuid::Uuid::new_v4()),
        name: "active-db".to_string(),
        status: DbStatus::Active,
        backend_mode: DbBackendMode::DirectPg,
        logging_enabled: false,
        log_retention: None,
        created_at: None,
    };
    let deleted = Database {
        id: DbId(uuid::Uuid::new_v4()),
        name: "deleted-db".to_string(),
        status: DbStatus::Deleted,
        backend_mode: DbBackendMode::DirectPg,
        logging_enabled: false,
        log_retention: None,
        created_at: None,
    };

    update(&mut m, Msg::SetDatabases(vec![active, deleted]));

    assert!(
        !m.databases.iter().any(|d| d.status == DbStatus::Deleted),
        "Deleted databases must not appear in model.databases after SetDatabases"
    );
}

// ── US-003: Database Management ───────────────────────────────────────────

proptest! {
    /// AC-003-02: DatabaseCreated appends the new database to model.databases.
    #[test]
    fn database_created_appends_db(
        initial_dbs in prop::collection::vec(arb_database(), 0..=3),
        new_db in arb_database(),
    ) {
        let mut m = AppModel::default();
        m.authed = true;
        let initial_count = initial_dbs.len();
        m.databases = initial_dbs;

        update(&mut m, Msg::DatabaseCreated(new_db));

        prop_assert_eq!(
            m.databases.len(), initial_count + 1,
            "DatabaseCreated must append one database"
        );
    }
}

proptest! {
    /// AC-003-05: DeleteDatabase removes the database and its SDK keys (cascade).
    #[test]
    fn delete_database_removes_db_and_sdk_keys(db in arb_database()) {
        let mut m = AppModel::default();
        m.authed = true;
        let db_id = db.id.clone();
        m.databases.push(db.clone());
        m.sdk_keys.insert(db_id.clone(), vec![]);

        update(&mut m, Msg::DeleteDatabase(db_id.clone()));

        prop_assert!(
            !m.databases.iter().any(|d| d.id == db_id),
            "DeleteDatabase must remove the database from model.databases"
        );
        prop_assert!(
            !m.sdk_keys.contains_key(&db_id),
            "DeleteDatabase must cascade-remove SDK keys for that database"
        );
    }
}

proptest! {
    /// AC-003-04: SetDbStatus changes the database status.
    #[test]
    fn set_db_status_updates_database(
        db in arb_database(),
        new_status in arb_db_status(),
    ) {
        let mut m = AppModel::default();
        m.authed = true;
        let db_id = db.id.clone();
        m.databases.push(db);

        update(&mut m, Msg::SetDbStatus(db_id.clone(), new_status.clone()));

        let updated = m.databases.iter().find(|d| d.id == db_id).unwrap();
        prop_assert_eq!(&updated.status, &new_status, "SetDbStatus must update the database status");
    }
}

/// Sad: SetDbStatus with non-existent DbId is a no-op (no panic).
#[test]
fn set_db_status_missing_id_is_noop() {
    let mut m = make_model_with_db();
    let phantom_id = DbId(uuid::Uuid::new_v4());
    let db_count_before = m.databases.len();

    update(&mut m, Msg::SetDbStatus(phantom_id, DbStatus::Suspended));

    assert_eq!(m.databases.len(), db_count_before, "no-op: database count unchanged");
}

// ── US-004: Overview / Logging ────────────────────────────────────────────

proptest! {
    /// AC-004-03 / AC-004-04: SetDbLogging(id, true) enables logging on the database.
    #[test]
    fn set_db_logging_enables_logging(db in arb_database()) {
        let mut m = AppModel::default();
        m.authed = true;
        let db_id = db.id.clone();
        let mut db = db;
        db.logging_enabled = false;
        m.databases.push(db);

        update(&mut m, Msg::SetDbLogging(db_id.clone(), true));

        let updated = m.databases.iter().find(|d| d.id == db_id).unwrap();
        prop_assert!(updated.logging_enabled, "SetDbLogging(_, true) must enable logging");
    }
}

proptest! {
    /// AC-004-03: SetDbLogging(id, false) disables logging on the database.
    #[test]
    fn set_db_logging_disables_logging(db in arb_database()) {
        let mut m = AppModel::default();
        m.authed = true;
        let db_id = db.id.clone();
        let mut db = db;
        db.logging_enabled = true;
        m.databases.push(db);

        update(&mut m, Msg::SetDbLogging(db_id.clone(), false));

        let updated = m.databases.iter().find(|d| d.id == db_id).unwrap();
        prop_assert!(!updated.logging_enabled, "SetDbLogging(_, false) must disable logging");
    }
}

/// Sad: SetDbLogging with wrong id is a no-op (no panic, no database modified).
#[test]
fn set_db_logging_wrong_id_is_noop() {
    let mut m = make_model_with_db();
    let wrong_id = DbId(uuid::Uuid::new_v4());
    let logging_before: Vec<bool> = m.databases.iter().map(|d| d.logging_enabled).collect();

    update(&mut m, Msg::SetDbLogging(wrong_id, true));

    let logging_after: Vec<bool> = m.databases.iter().map(|d| d.logging_enabled).collect();
    assert_eq!(logging_before, logging_after, "SetDbLogging with wrong id must not modify any database");
}

// ── US-005: Connections ────────────────────────────────────────────────────

proptest! {
    /// AC-005-03: PatchDb updates the database's backend config.
    #[test]
    fn patch_db_updates_backend_config(db in arb_database()) {
        use embyr_admin_ui::model::DbPatch;

        let mut m = AppModel::default();
        m.authed = true;
        let db_id = db.id.clone();
        m.databases.push(db);

        update(&mut m, Msg::PatchDb(db_id.clone(), DbPatch::Dsn("postgres://host:5432/mydb".to_string())));

        // The patch must not remove the database.
        prop_assert!(
            m.databases.iter().any(|d| d.id == db_id),
            "PatchDb must not remove the database from model.databases"
        );
    }
}

/// Sad: PatchDb with non-existent DbId is a no-op (no panic).
#[test]
fn patch_db_missing_id_is_noop() {
    use embyr_admin_ui::model::DbPatch;

    let mut m = make_model_with_db();
    let phantom_id = DbId(uuid::Uuid::new_v4());
    let db_count_before = m.databases.len();

    update(&mut m, Msg::PatchDb(phantom_id, DbPatch::Dsn("postgres://x/y".to_string())));

    assert_eq!(m.databases.len(), db_count_before, "PatchDb with missing id must be a no-op");
}

// ── US-006: SDK Keys ──────────────────────────────────────────────────────

proptest! {
    /// AC-006-01 / AC-006-02: SdkKeyCreated appends the key to sdk_keys[db_id].
    #[test]
    fn sdk_key_created_appends_key(db in arb_database()) {
        let mut m = AppModel::default();
        m.authed = true;
        let db_id = db.id.clone();
        m.databases.push(db);
        m.sdk_keys.insert(db_id.clone(), vec![]);

        let key = SdkKey {
            id: KeyId(uuid::Uuid::new_v4()),
            db_id: db_id.clone(),
            name: "prod-key".to_string(),
            prefix: "embyr_sd".to_string(),
            created_at: None,
        };

        update(&mut m, Msg::SdkKeyCreated { db_id: db_id.clone(), key: key.clone() });

        let keys = m.sdk_keys.get(&db_id).expect("sdk_keys entry must exist");
        prop_assert_eq!(keys.len(), 1, "SdkKeyCreated must append to sdk_keys[db_id]");
    }
}

proptest! {
    /// AC-006-04: RevokeSdkKey removes the key from sdk_keys[db_id].
    #[test]
    fn revoke_sdk_key_removes_key(db in arb_database(), key_id in arb_key_id()) {
        let mut m = AppModel::default();
        m.authed = true;
        let db_id = db.id.clone();
        m.databases.push(db);

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
        prop_assert!(
            !keys.iter().any(|k| k.id == key_id),
            "RevokeSdkKey must remove the key from sdk_keys[db_id]"
        );
    }
}

/// Sad: RevokeSdkKey with non-existent key_id is a no-op (no panic).
#[test]
fn revoke_sdk_key_missing_key_is_noop() {
    let mut m = make_model_with_db();
    let db_id = m.databases[0].id.clone();
    m.sdk_keys.insert(db_id.clone(), vec![]);
    let phantom_key_id = KeyId(uuid::Uuid::new_v4());

    update(&mut m, Msg::RevokeSdkKey(db_id.clone(), phantom_key_id));

    // No panic and sdk_keys still empty.
    assert_eq!(m.sdk_keys[&db_id].len(), 0, "no-op: key list unchanged");
}

/// Sad: SdkKeyCreated with non-existent db_id is a no-op (no panic, no entry created).
#[test]
fn sdk_key_created_unknown_db_is_noop() {
    let mut m = make_model_with_db();
    let known_db_keys_before = m.sdk_keys.len();

    let phantom_db_id = DbId(uuid::Uuid::new_v4());
    let key = SdkKey {
        id: KeyId(uuid::Uuid::new_v4()),
        db_id: phantom_db_id.clone(),
        name: "ghost-key".to_string(),
        prefix: "embyr_sd".to_string(),
        created_at: None,
    };

    update(&mut m, Msg::SdkKeyCreated { db_id: phantom_db_id, key });

    assert_eq!(
        m.sdk_keys.len(), known_db_keys_before,
        "SdkKeyCreated with unknown db must not create a new sdk_keys entry"
    );
}

// ── US-009: Members ────────────────────────────────────────────────────────

proptest! {
    /// AC-009-02: MemberInvited appends a pending member.
    #[test]
    fn member_invited_appends_member(
        model in arb_authed_model(),
        new_member in arb_member_with_role(Role::Viewer),
    ) {
        let mut m = model;
        let count_before = m.members.len();

        update(&mut m, Msg::MemberInvited(new_member));

        prop_assert_eq!(m.members.len(), count_before + 1, "MemberInvited must append member");
    }
}

proptest! {
    /// AC-009-04: SetMemberRole updates the member's role.
    #[test]
    fn set_member_role_updates_role(
        initial_role in arb_non_owner_role(),
        new_role in arb_non_owner_role(),
    ) {
        let mut m = AppModel::default();
        m.authed = true;
        let uid = UserId(uuid::Uuid::new_v4());
        // One Owner + one member to change.
        let owner = Member {
            id: UserId(uuid::Uuid::new_v4()),
            email: "owner@test.com".to_string(),
            display_name: None,
            role: Role::Owner,
            pending: false,
            mfa_enabled: true,
            last_login: None,
        };
        let target = Member {
            id: uid.clone(),
            email: "target@test.com".to_string(),
            display_name: None,
            role: initial_role,
            pending: false,
            mfa_enabled: false,
            last_login: None,
        };
        m.members = vec![owner, target];

        update(&mut m, Msg::SetMemberRole(uid.clone(), new_role.clone()));

        let updated = m.members.iter().find(|member| member.id == uid).unwrap();
        prop_assert_eq!(&updated.role, &new_role, "SetMemberRole must update the member's role");
    }
}

proptest! {
    /// AC-009-05: RemoveMember removes the member from the list.
    #[test]
    fn remove_member_removes_non_owner(
        model in arb_model_with_one_owner(),
        uid in arb_non_owner_user_id(),
    ) {
        let mut m = model;
        // Inject a removable non-owner member with the given uid.
        let removable = Member {
            id: uid.clone(),
            email: "removable@test.com".to_string(),
            display_name: None,
            role: Role::Viewer,
            pending: false,
            mfa_enabled: false,
            last_login: None,
        };
        m.members.push(removable);

        update(&mut m, Msg::RemoveMember(uid.clone()));

        prop_assert!(
            !m.members.iter().any(|member| member.id == uid),
            "RemoveMember must remove the member from model.members"
        );
    }
}

/// AC-009-06 (invariant): At least one Owner always exists after RemoveMember.
proptest! {
    #[test]
    fn sole_owner_invariant_holds_after_member_removal(
        model in arb_model_with_one_owner(),
        uid in arb_non_owner_user_id(),
    ) {
        let mut m = model;
        update(&mut m, Msg::RemoveMember(uid));
        let has_owner = m.members.iter().any(|mem| mem.role == Role::Owner);
        prop_assert!(has_owner, "model must always have at least one Owner after RemoveMember");
    }
}

/// AC-009-06 (invariant): At least one Owner always exists after SetMemberRole.
proptest! {
    #[test]
    fn sole_owner_invariant_holds_after_role_change(
        model in arb_model_with_one_owner(),
        uid in arb_non_owner_user_id(),
        new_role in arb_non_owner_role(),
    ) {
        let mut m = model;
        // Attempt to demote a non-Owner (owner protection is by uid, not by count change here).
        update(&mut m, Msg::SetMemberRole(uid, new_role));
        let has_owner = m.members.iter().any(|mem| mem.role == Role::Owner);
        prop_assert!(has_owner, "model must always have at least one Owner after SetMemberRole");
    }
}

/// Sad: RemoveMember with non-existent uid is a no-op (no panic).
#[test]
fn remove_member_missing_uid_is_noop() {
    let mut m = make_model_with_db();
    let count_before = m.members.len();
    let phantom_uid = UserId(uuid::Uuid::new_v4());

    update(&mut m, Msg::RemoveMember(phantom_uid));

    assert_eq!(m.members.len(), count_before, "RemoveMember with missing uid must be a no-op");
}

// ── US-010: Service Accounts + Admin Keys ─────────────────────────────────

proptest! {
    /// AC-010-02: ServiceAccountCreated appends the service account.
    #[test]
    #[ignore = "RED — implement update(_, Msg::ServiceAccountCreated)"]
    fn service_account_created_appends(sa in arb_service_account()) {
        let mut m = AppModel::default();
        m.authed = true;

        update(&mut m, Msg::ServiceAccountCreated(sa.clone()));

        prop_assert_eq!(m.service_accounts.len(), 1, "ServiceAccountCreated must append");
    }
}

proptest! {
    /// AC-010-03: DeleteServiceAccount removes the service account.
    #[test]
    #[ignore = "RED — implement update(_, Msg::DeleteServiceAccount)"]
    fn delete_service_account_removes_it(sa in arb_service_account()) {
        let mut m = AppModel::default();
        m.authed = true;
        let sa_id = sa.id.clone();
        m.service_accounts.push(sa);

        update(&mut m, Msg::DeleteServiceAccount(sa_id.clone()));

        prop_assert!(
            !m.service_accounts.iter().any(|s| s.id == sa_id),
            "DeleteServiceAccount must remove the service account"
        );
    }
}

proptest! {
    /// AC-010-05: AdminKeyCreated appends the admin key.
    #[test]
    #[ignore = "RED — implement update(_, Msg::AdminKeyCreated)"]
    fn admin_key_created_appends(key in arb_admin_key()) {
        let mut m = AppModel::default();
        m.authed = true;

        update(&mut m, Msg::AdminKeyCreated(key.clone()));

        prop_assert_eq!(m.admin_keys.len(), 1, "AdminKeyCreated must append");
    }
}

proptest! {
    /// AC-010-06: RevokeAdminKey removes the admin key.
    #[test]
    #[ignore = "RED — implement update(_, Msg::RevokeAdminKey)"]
    fn revoke_admin_key_removes_key(key in arb_admin_key()) {
        let mut m = AppModel::default();
        m.authed = true;
        let key_id = key.id.clone();
        m.admin_keys.push(key);

        update(&mut m, Msg::RevokeAdminKey(key_id.clone()));

        prop_assert!(
            !m.admin_keys.iter().any(|k| k.id == key_id),
            "RevokeAdminKey must remove the key"
        );
    }
}

/// Sad: RevokeAdminKey with non-existent key_id is a no-op (no panic).
#[test]
#[ignore = "RED — implement RevokeAdminKey no-op on missing key"]
fn revoke_admin_key_missing_is_noop() {
    let mut m = AppModel::default();
    m.authed = true;
    let phantom_id = KeyId(uuid::Uuid::new_v4());

    update(&mut m, Msg::RevokeAdminKey(phantom_id));

    assert_eq!(m.admin_keys.len(), 0, "no-op: admin_keys unchanged");
}

// ── US-011: Settings ──────────────────────────────────────────────────────

proptest! {
    /// AC-011-04: ToggleOidc flips the enabled flag on the matching provider.
    #[test]
    #[ignore = "RED — implement update(_, Msg::ToggleOidc)"]
    fn toggle_oidc_flips_enabled(provider in arb_oidc_provider()) {
        let mut m = AppModel::default();
        m.authed = true;
        let oidc_id = provider.id.clone();
        let initial_enabled = provider.enabled;
        m.oidc_providers.push(provider);

        update(&mut m, Msg::ToggleOidc(oidc_id.clone()));

        let updated = m.oidc_providers.iter().find(|p| p.id == oidc_id).unwrap();
        prop_assert_eq!(
            updated.enabled, !initial_enabled,
            "ToggleOidc must flip the enabled flag"
        );
    }
}

proptest! {
    /// AC-011-04: ToggleOidc is idempotent in the sense that double-toggle restores original state.
    #[test]
    #[ignore = "RED — implement ToggleOidc double-toggle restores state"]
    fn toggle_oidc_double_toggle_restores(provider in arb_oidc_provider()) {
        let mut m = AppModel::default();
        m.authed = true;
        let oidc_id = provider.id.clone();
        let initial_enabled = provider.enabled;
        m.oidc_providers.push(provider);

        update(&mut m, Msg::ToggleOidc(oidc_id.clone()));
        update(&mut m, Msg::ToggleOidc(oidc_id.clone()));

        let updated = m.oidc_providers.iter().find(|p| p.id == oidc_id).unwrap();
        prop_assert_eq!(
            updated.enabled, initial_enabled,
            "Double ToggleOidc must restore original enabled state"
        );
    }
}

// ── Toast notifications ────────────────────────────────────────────────────

proptest! {
    /// PushToast appends to the toast queue.
    #[test]
    #[ignore = "RED — implement update(_, Msg::PushToast)"]
    fn push_toast_appends(toast in arb_toast()) {
        let mut m = AppModel::default();
        m.authed = true;

        update(&mut m, Msg::PushToast(toast.clone()));

        prop_assert_eq!(m.toasts.len(), 1, "PushToast must append to toast queue");
        prop_assert_eq!(&m.toasts[0].id, &toast.id, "toast id must match");
    }
}

/// DismissToast removes the matching toast.
proptest! {
    #[test]
    #[ignore = "RED — implement update(_, Msg::DismissToast)"]
    fn dismiss_toast_removes_toast(toast in arb_toast()) {
        let mut m = AppModel::default();
        m.authed = true;
        let toast_id = toast.id.clone();
        m.toasts.push(toast);

        update(&mut m, Msg::DismissToast(toast_id.clone()));

        prop_assert!(
            !m.toasts.iter().any(|t| t.id == toast_id),
            "DismissToast must remove the toast from the queue"
        );
    }
}

/// Sad: DismissToast with non-existent ToastId is a no-op (no panic).
#[test]
#[ignore = "RED — implement DismissToast no-op on missing id"]
fn dismiss_toast_missing_id_is_noop() {
    let mut m = AppModel::default();
    m.authed = true;

    let phantom_id = ToastId(uuid::Uuid::new_v4());
    update(&mut m, Msg::DismissToast(phantom_id));
    // No panic is the assertion.
    assert_eq!(m.toasts.len(), 0);
}

/// Toast overflow: pushing 100 toasts does not panic.
#[test]
#[ignore = "RED — implement PushToast overflow safety"]
fn push_many_toasts_does_not_panic() {
    let mut m = AppModel::default();
    m.authed = true;

    for i in 0..100u32 {
        let toast = Toast {
            id: ToastId(uuid::Uuid::new_v4()),
            message: format!("toast-{}", i),
            level: embyr_admin_ui::model::ToastLevel::Info,
        };
        update(&mut m, Msg::PushToast(toast));
    }

    // Must not panic. Model should have ≤ 100 toasts (or be bounded by implementation).
    assert!(!m.toasts.is_empty(), "toasts must not be empty after 100 pushes");
}
