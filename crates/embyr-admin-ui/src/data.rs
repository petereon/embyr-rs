//! Mock data layer — `data::mock::*()` functions.
//!
//! V1: all data is in-process mock returning hardcoded collections.
//! Individual slices populate this data as they are implemented.
//! V2 plan: replace with `#[server]` function calls. Component code is unchanged (ADR-007).

use crate::model::{
    AdminKey, Database, DbBackendMode, DbId, DbStatus, Member, OidcProvider, SdkKey,
    ServiceAccount,
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
    pub fn admin_keys() -> Vec<AdminKey> {
        Vec::new()
    }

    /// Return mock service accounts.
    pub fn service_accounts() -> Vec<ServiceAccount> {
        Vec::new()
    }

    /// Return mock OIDC providers.
    pub fn oidc_providers() -> Vec<OidcProvider> {
        Vec::new()
    }
}
