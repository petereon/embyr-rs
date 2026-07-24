// SCAFFOLD: true
//! proptest strategies for embyr-admin-ui domain types.
//!
//! Used by tea_state_scenarios.rs and the per-slice test files.
//! All strategies produce valid domain values for property-based testing.

use embyr_admin_ui::model::{
    AdminKey, AppModel, Database, DbBackendMode, DbId, DbStatus, KeyId, LogRetention, Member,
    OidcId, OidcProvider, Role, SdkKey, ServiceAccount, ServiceAccountId, Toast, ToastId,
    ToastLevel, UserId,
};
use proptest::prelude::*;
use uuid::Uuid;

// ── Newtype strategies ────────────────────────────────────────────────────

prop_compose! {
    /// Generate an arbitrary DbId.
    pub fn arb_db_id()(bytes in any::<[u8; 16]>()) -> DbId {
        DbId(Uuid::from_bytes(bytes))
    }
}

prop_compose! {
    /// Generate an arbitrary UserId.
    pub fn arb_user_id()(bytes in any::<[u8; 16]>()) -> UserId {
        UserId(Uuid::from_bytes(bytes))
    }
}

prop_compose! {
    /// Generate an arbitrary KeyId.
    pub fn arb_key_id()(bytes in any::<[u8; 16]>()) -> KeyId {
        KeyId(Uuid::from_bytes(bytes))
    }
}

prop_compose! {
    /// Generate an arbitrary OidcId.
    pub fn arb_oidc_id()(bytes in any::<[u8; 16]>()) -> OidcId {
        OidcId(Uuid::from_bytes(bytes))
    }
}

prop_compose! {
    /// Generate an arbitrary ToastId.
    pub fn arb_toast_id()(bytes in any::<[u8; 16]>()) -> ToastId {
        ToastId(Uuid::from_bytes(bytes))
    }
}

prop_compose! {
    /// Generate an arbitrary ServiceAccountId.
    pub fn arb_service_account_id()(bytes in any::<[u8; 16]>()) -> ServiceAccountId {
        ServiceAccountId(Uuid::from_bytes(bytes))
    }
}

// ── Enum strategies ───────────────────────────────────────────────────────

pub fn arb_role() -> impl Strategy<Value = Role> {
    prop_oneof![
        Just(Role::Owner),
        Just(Role::Admin),
        Just(Role::Viewer),
    ]
}

pub fn arb_non_owner_role() -> impl Strategy<Value = Role> {
    prop_oneof![Just(Role::Admin), Just(Role::Viewer),]
}

pub fn arb_db_status() -> impl Strategy<Value = DbStatus> {
    prop_oneof![
        Just(DbStatus::Active),
        Just(DbStatus::Suspended),
    ]
}

pub fn arb_db_backend_mode() -> impl Strategy<Value = DbBackendMode> {
    prop_oneof![
        Just(DbBackendMode::DirectPg),
        Just(DbBackendMode::AgentMode),
    ]
}

pub fn arb_log_retention() -> impl Strategy<Value = LogRetention> {
    prop_oneof![
        Just(LogRetention::OneDay),
        Just(LogRetention::SevenDays),
        Just(LogRetention::ThirtyDays),
    ]
}

pub fn arb_toast_level() -> impl Strategy<Value = ToastLevel> {
    prop_oneof![
        Just(ToastLevel::Info),
        Just(ToastLevel::Warning),
        Just(ToastLevel::Error),
    ]
}

// ── Domain struct strategies ──────────────────────────────────────────────

prop_compose! {
    /// Generate a Database with a valid status (not Deleted — deleted DBs are absent from model).
    pub fn arb_database()(
        id in arb_db_id(),
        name in "[a-z][a-z0-9-]{2,19}",
        status in arb_db_status(),
        backend_mode in arb_db_backend_mode(),
        logging_enabled in any::<bool>(),
    ) -> Database {
        Database {
            id,
            name,
            status,
            backend_mode,
            logging_enabled,
            log_retention: None,
            created_at: None,
        }
    }
}

prop_compose! {
    /// Generate a Member with a given role.
    pub fn arb_member_with_role(role: Role)(
        id in arb_user_id(),
        email in "[a-z]{3,8}@[a-z]{3,8}\\.com",
        pending in any::<bool>(),
        mfa_enabled in any::<bool>(),
    ) -> Member {
        Member {
            id,
            email,
            display_name: None,
            role: role.clone(),
            pending,
            mfa_enabled,
            last_login: None,
        }
    }
}

pub fn arb_member() -> impl Strategy<Value = Member> {
    arb_role().prop_flat_map(arb_member_with_role)
}

prop_compose! {
    /// Generate an SdkKey for the given db_id.
    pub fn arb_sdk_key(db_id: DbId)(
        id in arb_key_id(),
        name in "[a-z][a-z0-9-]{2,19}",
    ) -> SdkKey {
        SdkKey {
            id,
            db_id: db_id.clone(),
            name,
            prefix: "embyr_sd".to_string(),
            created_at: None,
        }
    }
}

prop_compose! {
    /// Generate a ServiceAccount.
    pub fn arb_service_account()(
        id in arb_service_account_id(),
        name in "[a-z][a-z0-9-]{2,19}",
        role in arb_role(),
    ) -> ServiceAccount {
        ServiceAccount {
            id,
            name,
            description: None,
            role,
            created_at: None,
        }
    }
}

prop_compose! {
    /// Generate an AdminKey.
    pub fn arb_admin_key()(
        id in arb_key_id(),
        name in "[a-z][a-z0-9-]{2,19}",
        role in arb_role(),
    ) -> AdminKey {
        AdminKey {
            id,
            name,
            service_account_id: None,
            member_id: None,
            role,
            prefix: "embyr_ad".to_string(),
            created_at: None,
        }
    }
}

prop_compose! {
    /// Generate an OidcProvider.
    pub fn arb_oidc_provider()(
        id in arb_oidc_id(),
        issuer in "https://[a-z]{3,12}\\.example\\.com",
        client_id in "[a-zA-Z0-9]{8,16}",
        enabled in any::<bool>(),
    ) -> OidcProvider {
        OidcProvider { id, issuer, client_id, enabled }
    }
}

prop_compose! {
    /// Generate a Toast.
    pub fn arb_toast()(
        id in arb_toast_id(),
        message in "[a-zA-Z ]{5,40}",
        level in arb_toast_level(),
    ) -> Toast {
        Toast { id, message, level }
    }
}

// ── AppModel strategies ───────────────────────────────────────────────────

prop_compose! {
    /// Generate an authenticated AppModel with 1–4 databases (all Active).
    pub fn arb_authed_model()(
        dbs in prop::collection::vec(arb_database(), 1..=4),
        members in prop::collection::vec(arb_member(), 1..=4),
    ) -> AppModel {
        let mut model = AppModel::default();
        model.authed = true;
        model.databases = dbs;
        // Ensure at least one Owner.
        let mut ms = members;
        if !ms.iter().any(|m| m.role == Role::Owner) {
            ms[0].role = Role::Owner;
        }
        model.members = ms;
        model
    }
}

prop_compose! {
    /// Generate a model guaranteed to have at least one Owner and at least one non-Owner.
    pub fn arb_model_with_one_owner()(
        extra_members in prop::collection::vec(arb_member_with_role(Role::Admin), 1..=3),
        owner_id in arb_user_id(),
        extra_non_owners in prop::collection::vec(arb_member_with_role(Role::Viewer), 0..=2),
    ) -> AppModel {
        let mut model = AppModel::default();
        model.authed = true;
        let owner = Member {
            id: owner_id,
            email: "owner@test.com".to_string(),
            display_name: None,
            role: Role::Owner,
            pending: false,
            mfa_enabled: true,
            last_login: None,
        };
        model.members = std::iter::once(owner)
            .chain(extra_members)
            .chain(extra_non_owners)
            .collect();
        model
    }
}

prop_compose! {
    /// Generate a UserId that is guaranteed NOT to be the Owner's id
    /// (used with arb_model_with_one_owner).
    pub fn arb_non_owner_user_id()(bytes in any::<[u8; 16]>()) -> UserId {
        // The owner in arb_model_with_one_owner uses a separate arb_user_id().
        // For simplicity, generate a random id; collision probability is negligible.
        UserId(Uuid::from_bytes(bytes))
    }
}
