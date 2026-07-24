//! AppModel and all domain types for the embyr admin UI.
//!
//! Minimal field set — sufficient for `update()` logic to compile and for
//! proptest strategies to construct values. Expand during DELIVER.

use std::collections::HashMap;
use uuid::Uuid;
use chrono::{DateTime, Utc};

// ── Newtype identifiers ─────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct DbId(pub Uuid);

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct UserId(pub Uuid);

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct KeyId(pub Uuid);

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct OidcId(pub Uuid);

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ToastId(pub Uuid);

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct AccountId(pub Uuid);

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ServiceAccountId(pub Uuid);

// ── Enumerations ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DbStatus {
    #[default]
    Active,
    Suspended,
    Deleted,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DbBackendMode {
    #[default]
    DirectPg,
    AgentMode,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DbTab {
    #[default]
    Overview,
    Connections,
    Keys,
    Logs,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Login,
    Dashboard,
    Databases,
    DbDetail(DbId),
    Identities,
    ApiKeys,
    Billing,
    Settings,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Role {
    Owner,
    Admin,
    #[default]
    Viewer,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LogRetention {
    #[default]
    OneDay,
    SevenDays,
    ThirtyDays,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ToastLevel {
    #[default]
    Info,
    Warning,
    Error,
}

// ── DbPatch — partial update to database backend config ────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum DbPatch {
    /// Update DSN (direct_pg mode)
    Dsn(String),
    /// Update agent endpoint (agent_mode)
    AgentEndpoint(String),
    /// Enable or disable query logging
    LoggingEnabled(bool, Option<LogRetention>),
    /// Update backend mode
    BackendMode(DbBackendMode),
    /// Toggle suspended ↔ active
    Suspended(bool),
}

// ── Domain structs ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Database {
    pub id: DbId,
    pub name: String,
    pub status: DbStatus,
    pub backend_mode: DbBackendMode,
    pub logging_enabled: bool,
    pub log_retention: Option<LogRetention>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Member {
    pub id: UserId,
    pub email: String,
    pub display_name: Option<String>,
    pub role: Role,
    pub pending: bool,
    pub mfa_enabled: bool,
    pub last_login: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SdkKey {
    pub id: KeyId,
    pub db_id: DbId,
    pub name: String,
    pub prefix: String,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServiceAccount {
    pub id: ServiceAccountId,
    pub name: String,
    pub description: Option<String>,
    pub role: Role,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AdminKey {
    pub id: KeyId,
    pub name: String,
    pub service_account_id: Option<ServiceAccountId>,
    pub member_id: Option<UserId>,
    pub role: Role,
    pub prefix: String,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OidcProvider {
    pub id: OidcId,
    pub issuer: String,
    pub client_id: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Toast {
    pub id: ToastId,
    pub message: String,
    pub level: ToastLevel,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavState {
    pub section: Section,
    pub db_tab: DbTab,
}

// ── AppModel ────────────────────────────────────────────────────────────────

/// The single source of truth for all UI state.
///
/// All fields are pub for test visibility. Production code mutates only via
/// `update(&mut AppModel, Msg)`.
#[derive(Clone, Debug, Default)]
pub struct AppModel {
    /// Whether the session is authenticated.
    pub authed: bool,
    /// All databases in the account.
    pub databases: Vec<Database>,
    /// Members (including pending invitations).
    pub members: Vec<Member>,
    /// SDK keys keyed by database id.
    pub sdk_keys: HashMap<DbId, Vec<SdkKey>>,
    /// Service accounts.
    pub service_accounts: Vec<ServiceAccount>,
    /// Account-level admin API keys.
    pub admin_keys: Vec<AdminKey>,
    /// OIDC providers.
    pub oidc_providers: Vec<OidcProvider>,
    /// Toast notification queue.
    pub toasts: Vec<Toast>,
    /// Current navigation state.
    pub nav: NavState,
    /// TOTP failure counter (resets on success).
    pub totp_failures: u8,
    /// Whether the account is temporarily locked.
    pub account_locked: bool,
}

impl AppModel {
    /// Build an initial model from mock data.
    ///
    /// Returns the default model pre-populated with mock data from the data layer.
    pub fn from_mock() -> Self {
        use crate::data::mock;
        Self {
            authed: false,
            databases: mock::databases(),
            members: mock::members(),
            sdk_keys: std::collections::HashMap::new(),
            service_accounts: mock::service_accounts(),
            admin_keys: mock::admin_keys(),
            oidc_providers: mock::oidc_providers(),
            toasts: Vec::new(),
            nav: NavState::default(),
            totp_failures: 0,
            account_locked: false,
        }
    }
}
