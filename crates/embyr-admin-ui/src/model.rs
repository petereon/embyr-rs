//! AppModel and all domain types for the embyr admin UI.
//!
//! `AppModel` is the single source of truth for UI state. Production code
//! mutates it only via `update(&mut AppModel, Msg)` (see `update.rs`); all
//! other reads flow through the pure projection methods below.

use crate::api::CallKind;
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

// ── Newtype identifiers ─────────────────────────────────────────────────────

/// API project ids are operator-chosen strings, not UUIDs (G-05).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct DbId(pub String);

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

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct InvoiceId(pub Uuid);

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
    /// Secret-manager modes are display-only: the create form offers `direct_pg` and `agent` (G-06).
    AwsSecret,
    GcpSecret,
}

impl DbBackendMode {
    /// The value the API uses for this mode.
    pub fn api_value(&self) -> &'static str {
        match self {
            DbBackendMode::DirectPg => "direct_pg",
            DbBackendMode::AgentMode => "agent",
            DbBackendMode::AwsSecret => "aws_secret",
            DbBackendMode::GcpSecret => "gcp_secret",
        }
    }
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
    /// Any other server-side value (G-27): rendered as "N days", never offered in the selector.
    Days(u32),
}

impl LogRetention {
    pub fn days(&self) -> u32 {
        match self {
            LogRetention::OneDay => 1,
            LogRetention::SevenDays => 7,
            LogRetention::ThirtyDays => 30,
            LogRetention::Days(n) => *n,
        }
    }

    pub fn from_days(days: u32) -> Self {
        match days {
            1 => LogRetention::OneDay,
            7 => LogRetention::SevenDays,
            30 => LogRetention::ThirtyDays,
            n => LogRetention::Days(n),
        }
    }

    pub fn label(&self) -> String {
        match self.days() {
            1 => "1 day".to_string(),
            n => format!("{n} days"),
        }
    }
}

/// Hourly-metrics window (`?window=`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MetricsWindow {
    #[default]
    Day,
    Week,
}

impl MetricsWindow {
    pub fn api_value(&self) -> &'static str {
        match self {
            MetricsWindow::Day => "24h",
            MetricsWindow::Week => "7d",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ToastLevel {
    #[default]
    Info,
    Warning,
    Error,
}

// ── card-payments: billing enumerations ─────────────────────────────────────
// Source: feature-delta.md § Wave: DESIGN / [REF] Component Decomposition →
// Model Changes.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Plan {
    #[default]
    Free,
    Pro,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CardBrand {
    Visa,
    Mastercard,
    Amex,
    Discover,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum InvoiceStatus {
    #[default]
    Upcoming,
    Paid,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UpgradeModalStep {
    #[default]
    Compare,
    ConfirmDowngrade,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum EffectiveStatus {
    #[default]
    Active,
    FreeCapExceeded,
    PastDue,
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
    /// Per-database daily usage counters. `AppModel::usage_totals()` sums
    /// and projects these to monthly (×30), mirroring the
    /// design-reference.md prototype's `sum(db.reads) * 30`. Extended per
    /// DESIGN DDD-3 / feature-delta.md Model Changes.
    pub usage: UsageStats,
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

/// Read-only rotation metadata of a project's API key. Holds timestamps only,
/// never a key or a hash (ADR-085).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectApiKeyInfo {
    /// `None` = never rotated.
    pub rotated_at: Option<DateTime<Utc>>,
    /// `None` = no previous key is still accepted.
    pub previous_expires_at: Option<DateTime<Utc>>,
}

/// One-time plaintext secret shown in the reveal modal. `Debug` redacts it; `update` drops it when
/// the modal closes, on navigation, sign-out and session expiry (AC-AUW-20).
#[derive(Clone, PartialEq, Eq)]
pub struct RevealedSecret {
    pub db: DbId,
    pub secret: String,
}

impl std::fmt::Debug for RevealedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RevealedSecret")
            .field("db", &self.db)
            .field("secret", &"<redacted>")
            .finish()
    }
}

/// Percentiles of one op class. `None` (server `null`) means no samples: shown as "—", never 0 ms (AC-AUW-30).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LatencyStats {
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub samples: u64,
    /// A percentile fell in the overflow bucket: reported values are lower bounds.
    pub saturated: bool,
}

/// How a percentile is shown: "—" without samples, ">5000" when it is only a lower bound.
pub fn latency_label(value: Option<f64>, saturated: bool) -> String {
    match value {
        None => "—".to_string(),
        Some(v) if saturated && v >= 5000.0 => ">5000".to_string(),
        Some(v) if v.fract() == 0.0 => format!("{v:.0}"),
        Some(v) => format!("{v:.1}"),
    }
}

/// One hour of the activity series.
#[derive(Clone, Debug, PartialEq)]
pub struct SeriesPoint {
    pub hour: DateTime<Utc>,
    pub reads: u64,
    pub writes: u64,
    pub deletes: u64,
    pub errors: u64,
}

/// `GET /projects/:id/metrics` for one database.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DbMetrics {
    pub window: MetricsWindow,
    pub reads_today: u64,
    pub writes_today: u64,
    pub deletes_today: u64,
    pub read_latency: LatencyStats,
    pub write_latency: LatencyStats,
    pub series: Vec<SeriesPoint>,
}

/// One entry of `GET /metrics/summary`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricsSummaryRow {
    pub reads_today: u64,
    pub writes_today: u64,
    pub deletes_today: u64,
    pub p95_read_ms: Option<f64>,
    pub p95_write_ms: Option<f64>,
}

/// Masked connection view: never credentials (ADR-083).
#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionSummary {
    Dsn { host: String, port: u16, database: Option<String>, tls: bool },
    Agent { endpoint: String, cert_expires_at: Option<DateTime<Utc>> },
    /// Secret-manager mode: the secret's name/ARN.
    SecretRef(String),
    /// The server holds the secret but cannot summarise it (legacy encryption or unreadable).
    NotReadable,
}

impl ConnectionSummary {
    /// One-line, credential-free description for the overview and connections views.
    pub fn describe(&self) -> String {
        match self {
            ConnectionSummary::Dsn { host, port, database, tls } => {
                let target = format!("{host}:{port}/{}", database.as_deref().unwrap_or(""));
                if *tls { target } else { format!("{target} (TLS off)") }
            }
            ConnectionSummary::Agent { endpoint, cert_expires_at } => match cert_expires_at {
                Some(t) => format!("{endpoint} (client certificate expires {})", t.format("%Y-%m-%d")),
                None => endpoint.clone(),
            },
            ConnectionSummary::SecretRef(reference) => reference.clone(),
            ConnectionSummary::NotReadable => "stored encrypted".to_string(),
        }
    }
}

/// Account-wide figures for the dashboard KPI row.
#[derive(Clone, Debug, PartialEq)]
pub struct SummaryTotals {
    pub reads_today: u64,
    pub writes_today: u64,
    /// Mean of the databases' last-24 h read p95; `None` when no database has samples.
    pub avg_p95_read_ms: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub mode: DbBackendMode,
    pub summary: ConnectionSummary,
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

/// The signed-in person, as reported by `GET /auth/session`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CurrentUser {
    pub user_id: String,
    pub account_id: String,
    pub account_name: String,
    pub email: String,
    pub display_name: String,
    pub role: Role,
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

// ── card-payments: billing domain structs ───────────────────────────────────
// Source: feature-delta.md § Wave: DESIGN / [REF] Component Decomposition →
// Model Changes.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageStats {
    /// Daily rate, projected ×30 by `AppModel::usage_totals()`.
    pub reads: u64,
    /// Daily rate, projected ×30 by `AppModel::usage_totals()`.
    pub writes: u64,
    /// Daily rate, projected ×30 by `AppModel::usage_totals()`.
    pub deletes: u64,
    /// Point-in-time snapshot (NOT a daily rate) — see feature-delta.md OQ-CP-04.
    pub storage_gb: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Card {
    pub brand: CardBrand,
    pub last4: String,
    pub exp_month: u8,
    pub exp_year: u16,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Subscription {
    pub plan: Plan,
    pub stripe_customer_id: String,
    pub current_period_end: Option<DateTime<Utc>>,
    pub card: Option<Card>,
    pub payment_failure: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Invoice {
    pub id: InvoiceId,
    pub date: Option<DateTime<Utc>>,
    pub period_label: String,
    pub base: f64,
    pub overage: f64,
    pub total: f64,
    pub status: InvoiceStatus,
}

// ── card-payments: pure projection/view-model types ─────────────────────────
// Not literally named in DESIGN's 5-method list — introduced to give each AC
// a concrete, testable driving-port surface per Mandate 1/4. One
// single-sourced projection per UI card/component; field shapes may evolve
// with the views that consume them.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageTotals {
    pub reads: u64,
    pub writes: u64,
    pub deletes: u64,
    pub storage_gb: f64,
}

/// Field-for-field conversion from the data layer's `FreeCaps` shape.
/// Shared by `plan_summary()` and `upgrade_modal_view()`, both of which
/// project `data::FREE_CAPS`/`data::PLAN_FEATURES` into a `UsageTotals`.
impl From<crate::data::FreeCaps> for UsageTotals {
    fn from(caps: crate::data::FreeCaps) -> Self {
        UsageTotals {
            reads: caps.reads,
            writes: caps.writes,
            deletes: caps.deletes,
            storage_gb: caps.storage_gb,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CapRatios {
    pub reads: f64,
    pub writes: f64,
    pub deletes: f64,
    pub storage_gb: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanSummary {
    pub plan: Plan,
    /// Free plan: included volume per dimension (sourced from `data::FREE_CAPS`).
    pub included: Option<UsageTotals>,
    /// Pro plan: monthly base price (sourced from `data::PRICING`).
    pub base_price: Option<f64>,
    /// Pro plan: renewal date.
    pub renews_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaymentMethodSummary {
    pub card: Option<Card>,
    pub on_file: bool,
    pub stripe_customer_id: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageTableRow {
    pub db_id: DbId,
    pub name: String,
    pub usage: UsageStats,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SuspensionBannerView {
    pub status: EffectiveStatus,
    pub message: &'static str,
    pub cta_label: &'static str,
    /// true → CTA dispatches `Msg::OpenUpgradeModal`; false → `Msg::OpenCardModal`.
    pub cta_opens_upgrade_modal: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpgradeModalView {
    pub step: UpgradeModalStep,
    pub free_included: UsageTotals,
    pub pro_base_price: f64,
    /// true only when the pending transition is Pro → Free (AC-106-03).
    pub shows_downgrade_warning: bool,
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
    /// Project API key rotation metadata keyed by database id.
    pub api_key_info: HashMap<DbId, ProjectApiKeyInfo>,
    /// Hourly metrics per database (`GET /projects/:id/metrics`).
    pub db_metrics: HashMap<DbId, DbMetrics>,
    /// Dashboard summary per database (`GET /metrics/summary`).
    pub metrics_summary: HashMap<DbId, MetricsSummaryRow>,
    /// Masked connection per database (`GET /projects/:id/connection`).
    pub connections: HashMap<DbId, Connection>,
    /// Bumped on every server-confirmed connection save; views close their edit form on change.
    pub connection_saves: u32,
    /// One-time plaintext secret for the reveal modal; cleared on close/navigation/sign-out.
    pub revealed_secret: Option<RevealedSecret>,
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
    /// The server reported the account locked (429 without `retry-after-ms`); the server owns the counter.
    pub account_locked: bool,
    /// Boot session check still in flight: show a splash, not the sign-in form (AC-AUW-14).
    pub booting: bool,
    /// Boot session check could not reach the server: show "cannot reach server" + Retry (AC-AUW-14).
    pub server_unreachable: bool,
    /// Who is signed in (set with `authed`).
    pub current_user: Option<CurrentUser>,
    /// Message shown on the sign-in view (server text, throttle, or "session expired").
    pub sign_in_error: Option<String>,
    /// API calls in flight; drives disabled buttons and loading states (AC-AUW-16).
    pub pending: HashSet<CallKind>,
    /// Monotonic counter minting deterministic toast ids inside the pure `update`.
    pub toast_seq: u64,
    /// Plan/card/renewal snapshot.
    pub subscription: Subscription,
    /// card-payments: invoice history (persists across plan changes, AC-107-04).
    pub invoices: Vec<Invoice>,
    /// card-payments: global modal-open state (ADR-019 — cross-cutting
    /// SuspensionBanner must open CardModal directly, not view-local state).
    pub card_modal_open: bool,
    /// card-payments: global modal-open state (ADR-019).
    pub upgrade_modal_open: bool,
    /// card-payments: drives UpgradeModal's compare → confirm-downgrade step.
    pub upgrade_modal_step: UpgradeModalStep,
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
            api_key_info: std::collections::HashMap::new(),
            db_metrics: HashMap::new(),
            metrics_summary: HashMap::new(),
            connections: HashMap::new(),
            connection_saves: 0,
            revealed_secret: None,
            service_accounts: mock::service_accounts(),
            admin_keys: mock::admin_keys(),
            oidc_providers: mock::oidc_providers(),
            toasts: Vec::new(),
            nav: NavState::default(),
            account_locked: false,
            booting: false,
            server_unreachable: false,
            current_user: None,
            sign_in_error: None,
            pending: HashSet::new(),
            toast_seq: 0,
            // card-payments: demo account is Pro-plan with a card on file and
            // invoice history, to showcase the billing views' full feature set.
            subscription: mock::subscription(
                Plan::Pro,
                Some(Card {
                    brand: CardBrand::Visa,
                    last4: "4242".to_string(),
                    exp_month: 12,
                    exp_year: 2027,
                }),
                false,
            ),
            invoices: mock::invoices(&Plan::Pro),
            card_modal_open: false,
            upgrade_modal_open: false,
            upgrade_modal_step: UpgradeModalStep::default(),
        }
    }
}

impl AppModel {
    /// Owner and Admin may change databases; Viewers see disabled controls (AC-AUW-21).
    /// The server enforces the same rule.
    pub fn can_manage_databases(&self) -> bool {
        self.current_user.as_ref().is_some_and(|u| u.role != Role::Viewer)
    }

    /// Dashboard totals from `GET /metrics/summary`; `None` until it has loaded.
    pub fn summary_totals(&self) -> Option<SummaryTotals> {
        if self.metrics_summary.is_empty() {
            return None;
        }
        let rows = self.metrics_summary.values();
        let p95s: Vec<f64> = rows.clone().filter_map(|r| r.p95_read_ms).collect();
        Some(SummaryTotals {
            reads_today: rows.clone().map(|r| r.reads_today).sum(),
            writes_today: rows.map(|r| r.writes_today).sum(),
            avg_p95_read_ms: (!p95s.is_empty()).then(|| p95s.iter().sum::<f64>() / p95s.len() as f64),
        })
    }
}

// ── card-payments ─────────────────────────────────────────────────────────
//
// Pure derivation methods over `AppModel` fields — satisfies the DISCUSS/
// CLAUDE.md constraint verbatim: "Status derivation MUST be a pure function
// over AppModel fields, not duplicated stored booleans" (see
// docs/feature/card-payments/discuss/wave-decisions.md and DDD-8).
impl AppModel {
    /// AC-104-01/03: sums `self.databases[].usage`, projects reads/writes/
    /// deletes ×30 (storage_gb stays a point-in-time snapshot, OQ-CP-04).
    pub fn usage_totals(&self) -> UsageTotals {
        let daily_reads: u64 = self.databases.iter().map(|db| db.usage.reads).sum();
        let daily_writes: u64 = self.databases.iter().map(|db| db.usage.writes).sum();
        let daily_deletes: u64 = self.databases.iter().map(|db| db.usage.deletes).sum();
        let storage_gb: f64 = self.databases.iter().map(|db| db.usage.storage_gb).sum();
        UsageTotals {
            reads: daily_reads * 30,
            writes: daily_writes * 30,
            deletes: daily_deletes * 30,
            storage_gb,
        }
    }

    /// AC-102-01: `usage_totals() / data::FREE_CAPS`, per dimension.
    pub fn cap_ratios(&self) -> CapRatios {
        let totals = self.usage_totals();
        let caps = crate::data::FREE_CAPS;
        CapRatios {
            reads: totals.reads as f64 / caps.reads as f64,
            writes: totals.writes as f64 / caps.writes as f64,
            deletes: totals.deletes as f64 / caps.deletes as f64,
            storage_gb: totals.storage_gb / caps.storage_gb,
        }
    }

    /// AC-108-06 (D-6): `subscription.plan == Free && max(cap_ratios()) >= 1.0`.
    pub fn cap_exceeded(&self) -> bool {
        if self.subscription.plan != Plan::Free {
            return false;
        }
        let ratios = self.cap_ratios();
        ratios.reads >= 1.0
            || ratios.writes >= 1.0
            || ratios.deletes >= 1.0
            || ratios.storage_gb >= 1.0
    }

    /// AC-108-06: `cap_exceeded()` → FreeCapExceeded; else `payment_failure`
    /// → PastDue; else Active. Single-sourced D-6/D-12 hard-stop logic.
    pub fn effective_status(&self) -> EffectiveStatus {
        if self.cap_exceeded() {
            EffectiveStatus::FreeCapExceeded
        } else if self.subscription.payment_failure {
            EffectiveStatus::PastDue
        } else {
            EffectiveStatus::Active
        }
    }

    /// AC-108-01: `effective_status() != Active`.
    pub fn read_only(&self) -> bool {
        self.effective_status() != EffectiveStatus::Active
    }

    /// AC-101-01/02/03: Plan card projection (Free included volume from
    /// `data::FREE_CAPS`, or Pro base price + renewal date).
    pub fn plan_summary(&self) -> PlanSummary {
        match self.subscription.plan {
            Plan::Free => PlanSummary {
                plan: Plan::Free,
                included: Some(crate::data::FREE_CAPS.into()),
                base_price: None,
                renews_at: None,
            },
            Plan::Pro => PlanSummary {
                plan: Plan::Pro,
                included: None,
                base_price: Some(crate::data::PRICING.pro_base),
                renews_at: self.subscription.current_period_end,
            },
        }
    }

    /// AC-101-04/05/07: Payment Method card projection.
    pub fn payment_method_summary(&self) -> PaymentMethodSummary {
        PaymentMethodSummary {
            card: self.subscription.card.clone(),
            on_file: self.subscription.card.is_some(),
            stripe_customer_id: self.subscription.stripe_customer_id.clone(),
        }
    }

    /// AC-104-01/02/03: per-database Usage tab table rows.
    pub fn usage_table_rows(&self) -> Vec<UsageTableRow> {
        self.databases
            .iter()
            .map(|db| UsageTableRow {
                db_id: db.id.clone(),
                name: db.name.clone(),
                usage: db.usage.clone(),
            })
            .collect()
    }

    /// AC-107-03: `Some(copy)` for Free-plan accounts with no invoice
    /// history; `None` otherwise (table renders instead).
    pub fn invoices_empty_state(&self) -> Option<&'static str> {
        if self.subscription.plan == Plan::Free && self.invoices.is_empty() {
            Some("The Free plan has no recurring charges — invoices appear once you're on Pro.")
        } else {
            None
        }
    }

    /// AC-108-01/02/03: `None` when `effective_status() == Active`; amber
    /// (FreeCapExceeded) or red (PastDue) projection otherwise.
    pub fn suspension_banner_view(&self) -> Option<SuspensionBannerView> {
        if !self.read_only() {
            return None;
        }
        if self.effective_status() == EffectiveStatus::FreeCapExceeded {
            return Some(SuspensionBannerView {
                status: EffectiveStatus::FreeCapExceeded,
                message: "You've reached your Free plan limits for this cycle.",
                cta_label: "Upgrade to Pro",
                cta_opens_upgrade_modal: true,
            });
        }
        Some(SuspensionBannerView {
            status: EffectiveStatus::PastDue,
            message: "We couldn't process your last payment.",
            cta_label: "Update payment method",
            cta_opens_upgrade_modal: false,
        })
    }

    /// AC-106-01/03: UpgradeModal compare/confirm-downgrade projection.
    pub fn upgrade_modal_view(&self) -> UpgradeModalView {
        let plan_features = crate::data::PLAN_FEATURES;
        UpgradeModalView {
            step: self.upgrade_modal_step.clone(),
            free_included: plan_features.free_included.into(),
            pro_base_price: plan_features.pro_base,
            shows_downgrade_warning: self.upgrade_modal_step == UpgradeModalStep::ConfirmDowngrade,
        }
    }
}
