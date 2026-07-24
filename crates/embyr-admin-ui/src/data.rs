//! Mock data layer — `data::mock::*()` functions.
//!
//! V1: all data is in-process mock returning hardcoded collections.
//! Individual slices populate this data as they are implemented.
//! V2 plan: replace with `#[server]` function calls. Component code is unchanged (ADR-007).

use crate::model::{
    AdminKey, Database, DbBackendMode, DbId, DbStatus, KeyId, Member, OidcProvider, Role, SdkKey,
    ServiceAccount, ServiceAccountId,
};
use uuid::Uuid;

pub mod mock {
    use super::*;

    /// Return a mock list of databases for the account.
    ///
    /// Two hardcoded entries covering both backend modes (direct_pg and agent_mode).
    /// Deleted databases are intentionally excluded — they would be filtered by SetDatabases.
    pub fn databases() -> Vec<Database> {
        vec![
            Database {
                id: DbId(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()),
                name: "production".to_string(),
                status: DbStatus::Active,
                backend_mode: DbBackendMode::DirectPg,
                logging_enabled: true,
                log_retention: Some(crate::model::LogRetention::SevenDays),
                created_at: None,
            },
            Database {
                id: DbId(Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap()),
                name: "staging".to_string(),
                status: DbStatus::Active,
                backend_mode: DbBackendMode::AgentMode,
                logging_enabled: false,
                log_retention: None,
                created_at: None,
            },
        ]
    }

    /// Return a mock list of members for the account.
    pub fn members() -> Vec<Member> {
        Vec::new()
    }

    /// Return mock SDK keys for the given database.
    pub fn sdk_keys(_db_id: &DbId) -> Vec<SdkKey> {
        Vec::new()
    }

    /// Return mock account-level admin API keys.
    ///
    /// Two hardcoded entries with the "embyr_adm_" prefix covering Owner and Admin roles.
    pub fn admin_keys() -> Vec<AdminKey> {
        vec![
            AdminKey {
                id: KeyId(Uuid::parse_str("00000000-0000-0000-0000-000000000010").unwrap()),
                name: "ci-pipeline-key".to_string(),
                service_account_id: Some(ServiceAccountId(
                    Uuid::parse_str("00000000-0000-0000-0000-000000000020").unwrap(),
                )),
                member_id: None,
                role: Role::Admin,
                prefix: "embyr_adm_".to_string(),
                created_at: None,
            },
            AdminKey {
                id: KeyId(Uuid::parse_str("00000000-0000-0000-0000-000000000011").unwrap()),
                name: "read-only-key".to_string(),
                service_account_id: None,
                member_id: None,
                role: Role::Viewer,
                prefix: "embyr_adm_".to_string(),
                created_at: None,
            },
        ]
    }

    /// Return mock service accounts.
    ///
    /// One hardcoded entry for a CI service account.
    pub fn service_accounts() -> Vec<ServiceAccount> {
        vec![ServiceAccount {
            id: ServiceAccountId(
                Uuid::parse_str("00000000-0000-0000-0000-000000000020").unwrap(),
            ),
            name: "ci-service-account".to_string(),
            description: Some("Used by the CI pipeline".to_string()),
            role: Role::Admin,
            created_at: None,
        }]
    }

    /// Return mock OIDC providers.
    pub fn oidc_providers() -> Vec<OidcProvider> {
        Vec::new()
    }
}
