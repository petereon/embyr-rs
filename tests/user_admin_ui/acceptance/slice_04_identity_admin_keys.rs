// SCAFFOLD: true
//! Slice 04 — Identity (Members) + Admin API Keys acceptance scenarios.
//! Stories: US-009 (Members), US-010 (Service Accounts + Admin Keys)
//!
//! All tests are #[ignore] (RED). Enable one at a time in DELIVER.
//! Includes the sole-Owner invariant (AC-009-06) as a named invariant test.

use embyr_admin_ui::model::{
    AdminKey, AppModel, KeyId, Member, Role, ServiceAccount, ServiceAccountId, UserId,
};
use embyr_admin_ui::msg::Msg;
use embyr_admin_ui::update::update;
use uuid::Uuid;

#[path = "../common/mod.rs"]
mod common;
use common::{assert_owner_invariant, make_model_with_db};

// ─────────────────────────────────────────────────────────────────────────────
// US-009 / AC-009-02: Invite member.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-009-02: MemberInvited appends a member with pending = true.
#[test]
#[ignore = "RED — implement update(_, Msg::MemberInvited)"]
fn invite_member_appears_with_pending_status() {
    // AC-009-02
    let mut m = make_model_with_db();
    let count_before = m.members.len();

    let new_member = Member {
        id: UserId(Uuid::new_v4()),
        email: "dan@acme.com".to_string(),
        display_name: None,
        role: Role::Viewer,
        pending: true,
        mfa_enabled: false,
        last_login: None,
    };

    update(&mut m, Msg::MemberInvited(new_member));

    assert_eq!(m.members.len(), count_before + 1, "AC-009-02: member count must increase");
    assert!(
        m.members.iter().any(|mem| mem.email == "dan@acme.com" && mem.pending),
        "AC-009-02: invited member must be pending"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// US-009 / AC-009-04: Role change.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-009-04: SetMemberRole upgrades Viewer to Admin.
#[test]
#[ignore = "RED — implement update(_, Msg::SetMemberRole) promote"]
fn promote_viewer_to_admin() {
    // AC-009-04
    let mut m = make_model_with_db();
    let viewer_id = m.members.iter().find(|mem| mem.role == Role::Viewer)
        .map(|mem| mem.id.clone())
        .expect("test fixture must have a Viewer");

    update(&mut m, Msg::SetMemberRole(viewer_id.clone(), Role::Admin));

    let updated = m.members.iter().find(|mem| mem.id == viewer_id).unwrap();
    assert_eq!(updated.role, Role::Admin, "AC-009-04: Viewer must be promoted to Admin");
    assert_owner_invariant(&m);
}

// ─────────────────────────────────────────────────────────────────────────────
// US-009 / AC-009-05: Remove member.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-009-05: RemoveMember removes a non-Owner member.
#[test]
#[ignore = "RED — implement update(_, Msg::RemoveMember)"]
fn remove_viewer_member() {
    // AC-009-05
    let mut m = make_model_with_db();
    let viewer_id = m.members.iter().find(|mem| mem.role == Role::Viewer)
        .map(|mem| mem.id.clone())
        .expect("fixture must have a Viewer");

    update(&mut m, Msg::RemoveMember(viewer_id.clone()));

    assert!(
        !m.members.iter().any(|mem| mem.id == viewer_id),
        "AC-009-05: removed member must be absent from model.members"
    );
    assert_owner_invariant(&m);
}

// ─────────────────────────────────────────────────────────────────────────────
// US-009 / AC-009-06: Sole-Owner invariant.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-009-06: Attempting to remove the sole Owner is rejected — model unchanged.
#[test]
#[ignore = "RED — implement sole-Owner protection in RemoveMember"]
fn removing_sole_owner_is_rejected() {
    // AC-009-06
    let mut m = AppModel::default();
    m.authed = true;
    let sole_owner_id = UserId(Uuid::new_v4());
    m.members = vec![Member {
        id: sole_owner_id.clone(),
        email: "owner@acme.com".to_string(),
        display_name: None,
        role: Role::Owner,
        pending: false,
        mfa_enabled: true,
        last_login: None,
    }];

    update(&mut m, Msg::RemoveMember(sole_owner_id.clone()));

    // Sole Owner must still be present.
    assert!(
        m.members.iter().any(|mem| mem.id == sole_owner_id),
        "AC-009-06: sole Owner must not be removable"
    );
    assert_owner_invariant(&m);
}

/// AC-009-06: Sole Owner cannot be demoted via SetMemberRole.
#[test]
#[ignore = "RED — implement sole-Owner protection in SetMemberRole"]
fn demoting_sole_owner_is_rejected() {
    // AC-009-06
    let mut m = AppModel::default();
    m.authed = true;
    let sole_owner_id = UserId(Uuid::new_v4());
    m.members = vec![Member {
        id: sole_owner_id.clone(),
        email: "owner@acme.com".to_string(),
        display_name: None,
        role: Role::Owner,
        pending: false,
        mfa_enabled: true,
        last_login: None,
    }];

    update(&mut m, Msg::SetMemberRole(sole_owner_id.clone(), Role::Admin));

    let updated = m.members.iter().find(|mem| mem.id == sole_owner_id).unwrap();
    assert_eq!(
        updated.role, Role::Owner,
        "AC-009-06: sole Owner must not be demoted via SetMemberRole"
    );
    assert_owner_invariant(&m);
}

/// Sad: RemoveMember with non-existent uid is a no-op (no panic).
#[test]
#[ignore = "RED — implement RemoveMember no-op guard"]
fn remove_member_phantom_uid_is_noop() {
    let mut m = make_model_with_db();
    let count_before = m.members.len();
    let phantom = UserId(Uuid::new_v4());

    update(&mut m, Msg::RemoveMember(phantom));

    assert_eq!(m.members.len(), count_before, "no-op: member count unchanged");
}

// ─────────────────────────────────────────────────────────────────────────────
// US-010 / AC-010-02: Create service account.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-010-02: ServiceAccountCreated appends the new service account.
#[test]
#[ignore = "RED — implement update(_, Msg::ServiceAccountCreated)"]
fn create_service_account_appends_to_model() {
    // AC-010-02
    let mut m = make_model_with_db();

    let sa = ServiceAccount {
        id: ServiceAccountId(Uuid::new_v4()),
        name: "ci-deploy".to_string(),
        description: Some("CI/CD pipeline account".to_string()),
        role: Role::Viewer,
        created_at: None,
    };

    update(&mut m, Msg::ServiceAccountCreated(sa.clone()));

    assert_eq!(m.service_accounts.len(), 1, "AC-010-02: service account must appear");
    assert!(
        m.service_accounts.iter().any(|s| s.name == "ci-deploy"),
        "AC-010-02: 'ci-deploy' must be in service_accounts"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// US-010 / AC-010-03: Delete service account cascades admin keys.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-010-03: DeleteServiceAccount removes the SA and its associated admin keys.
#[test]
#[ignore = "RED — implement update(_, Msg::DeleteServiceAccount) with cascade"]
fn delete_service_account_cascades_admin_keys() {
    // AC-010-03
    let mut m = make_model_with_db();
    let sa_id = ServiceAccountId(Uuid::new_v4());
    let sa = ServiceAccount {
        id: sa_id.clone(),
        name: "ci-deploy".to_string(),
        description: None,
        role: Role::Viewer,
        created_at: None,
    };
    m.service_accounts.push(sa);

    // Plant an admin key linked to this service account.
    let key = AdminKey {
        id: KeyId(Uuid::new_v4()),
        name: "ci-key".to_string(),
        service_account_id: Some(sa_id.clone()),
        member_id: None,
        role: Role::Viewer,
        prefix: "embyr_ad".to_string(),
        created_at: None,
    };
    m.admin_keys.push(key);

    update(&mut m, Msg::DeleteServiceAccount(sa_id.clone()));

    assert!(
        !m.service_accounts.iter().any(|s| s.id == sa_id),
        "AC-010-03: service account must be removed"
    );
    assert!(
        !m.admin_keys.iter().any(|k| k.service_account_id == Some(sa_id.clone())),
        "AC-010-03: admin keys for deleted SA must be cascaded-removed"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// US-010 / AC-010-05: Create + revoke admin key.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-010-05: AdminKeyCreated appends the key with prefix.
#[test]
#[ignore = "RED — implement update(_, Msg::AdminKeyCreated)"]
fn create_admin_key_appends_with_prefix() {
    // AC-010-05
    let mut m = make_model_with_db();

    let key = AdminKey {
        id: KeyId(Uuid::new_v4()),
        name: "deploy-key".to_string(),
        service_account_id: None,
        member_id: None,
        role: Role::Admin,
        prefix: "embyr_ad".to_string(),
        created_at: None,
    };

    update(&mut m, Msg::AdminKeyCreated(key.clone()));

    assert_eq!(m.admin_keys.len(), 1, "AC-010-05: admin key must appear in model");
    assert_eq!(m.admin_keys[0].prefix, "embyr_ad", "AC-010-05: prefix must be set");
}

/// AC-010-06: RevokeAdminKey removes the key immediately.
#[test]
#[ignore = "RED — implement update(_, Msg::RevokeAdminKey)"]
fn revoke_admin_key_removes_from_model() {
    // AC-010-06
    let mut m = make_model_with_db();
    let key_id = KeyId(Uuid::new_v4());
    m.admin_keys.push(AdminKey {
        id: key_id.clone(),
        name: "to-revoke".to_string(),
        service_account_id: None,
        member_id: None,
        role: Role::Viewer,
        prefix: "embyr_ad".to_string(),
        created_at: None,
    });

    update(&mut m, Msg::RevokeAdminKey(key_id.clone()));

    assert!(
        !m.admin_keys.iter().any(|k| k.id == key_id),
        "AC-010-06: revoked admin key must be absent from model"
    );
}

/// Sad: RevokeAdminKey with non-existent key_id is a no-op (no panic).
#[test]
#[ignore = "RED — implement RevokeAdminKey no-op guard"]
fn revoke_admin_key_phantom_is_noop() {
    let mut m = make_model_with_db();
    let phantom = KeyId(Uuid::new_v4());

    update(&mut m, Msg::RevokeAdminKey(phantom));

    assert!(m.admin_keys.is_empty(), "no-op: admin_keys unchanged");
}
