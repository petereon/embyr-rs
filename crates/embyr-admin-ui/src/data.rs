//! Mock data layer — `data::mock::*()` functions.
//!
//! V1: all data is in-process mock returning empty collections.
//! Individual slices populate this data as they are implemented.
//! V2 plan: replace with `#[server]` function calls. Component code is unchanged (ADR-007).

use crate::model::{
    AdminKey, Database, Member, OidcProvider, SdkKey, ServiceAccount, DbId,
};

pub mod mock {
    use super::*;

    /// Return a mock list of databases for the account.
    pub fn databases() -> Vec<Database> {
        Vec::new()
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
