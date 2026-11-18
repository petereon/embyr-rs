//! AppModel and all domain types for the embyr admin UI.
//!
//! `AppModel` is the single source of truth for UI state. Production code
//! mutates it only via `update(&mut AppModel, Msg)` (see `update.rs`); all
//! other reads flow through the pure projection methods below.

use crate::api::CallKind;
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Delete confirmation: the typed text must be exactly the database id (surrounding blanks ignored).
pub fn confirms_delete(typed: &str, id: &DbId) -> bool {
    typed.trim() == id.0
}

/// Account-delete confirmation: the typed text must be exactly the account name (the server compares
/// byte for byte, so no trimming).
pub fn confirms_account_delete(typed: &str, account_name: &str) -> bool {
    !account_name.is_empty() && typed == account_name
}

// ── Newtype identifiers ─────────────────────────────────────────────────────

/// API project ids are operator-chosen strings, not UUIDs (G-05).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct DbId(pub String);

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct UserId(pub Uuid);

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct KeyId(pub Uuid);

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

/// Which signed-out screen is showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AuthScreen {
    #[default]
    SignIn,
    /// Ask for a reset link by email.
    ForgotPassword,
    /// Choose a new password (reached from the emailed `?reset=` link; the token never enters the model).
    ResetPassword,
    /// Redeem an invitation (reached from the emailed `?invite=` link; the token never enters the model).
    AcceptInvite,
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
pub enum EffectiveStatus {
    #[default]
    Active,
    FreeCapExceeded,
    PastDue,
}

/// The server's own subscription status (`GET /billing/subscription`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubscriptionStatus {
    Active,
    PastDue,
    FreeCapExceeded,
    Canceled,
}

impl SubscriptionStatus {
    pub fn label(&self) -> &'static str {
        match self {
            SubscriptionStatus::Active => "Active",
            SubscriptionStatus::PastDue => "Past due",
            SubscriptionStatus::FreeCapExceeded => "Free limit reached",
            SubscriptionStatus::Canceled => "Canceled",
        }
    }
}

/// One Free-plan cap dimension as the server reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapUsage {
    pub dimension: String,
    pub used: u64,
    pub cap: u64,
    pub pct: u64,
}

/// Why the browser came back from Stripe Checkout (`?checkout=`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckoutReturn {
    Success,
    Cancelled,
}

/// Server-owned billing state. Everything here comes from the API; nothing is derived from usage mocks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BillingState {
    /// `None` until `GET /billing/subscription` has answered.
    pub status: Option<SubscriptionStatus>,
    /// Free plan only; `None` when the server has no figures yet.
    pub cap_status: Option<Vec<CapUsage>>,
    /// The server answered 502: Stripe is not configured or not reachable (G-52).
    pub unavailable: bool,
    /// Month-to-date usage per database (`GET /billing?range=month`); `None` until loaded.
    pub usage: Option<Vec<UsageTableRow>>,
}

/// TOTP enrolment secret issued by `invitations/accept`. `Debug` redacts it.
#[derive(Clone, PartialEq, Eq)]
pub struct Enrolment {
    /// Base32 secret for manual entry.
    pub totp_secret: String,
    /// `otpauth://` URI for authenticator apps.
    pub otpauth_uri: String,
}

/// One-time recovery codes issued by `invitations/confirm`. `Debug` redacts them.
#[derive(Clone, PartialEq, Eq)]
pub struct RecoveryCodes(pub Vec<String>);

impl std::fmt::Debug for Enrolment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Enrolment(<redacted>)")
    }
}

impl std::fmt::Debug for RecoveryCodes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RecoveryCodes(<{} redacted>)", self.0.len())
    }
}

/// Where the invitation screen is: details form, then authenticator enrolment, then recovery codes.
/// Dropped whenever the screen is left (`ShowAuth`), on sign-out and on session expiry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum InviteStep {
    #[default]
    Details,
    /// `invitations/lookup` is in flight: neither form shows yet.
    Checking,
    /// An SSO invitation: no password or authenticator steps, just "Continue with SSO".
    Sso(ProviderIds),
    Enrol(Enrolment),
    Codes(RecoveryCodes),
}

/// SSO provider ids an invitation may be redeemed through. `Debug` hides them: they come from a
/// public lookup keyed by the invitation token, so they stay out of logged model dumps.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderIds(pub Vec<String>);

impl std::fmt::Debug for ProviderIds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ProviderIds(<{} redacted>)", self.0.len())
    }
}

/// What `invitations/lookup` says about an invitation token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InviteKind {
    Password,
    Sso(ProviderIds),
}

/// Where the browser came back from an SSO round trip (`?error=sso_failed`, `?sso_linked=1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SsoReturn {
    Failed,
    Linked,
}

/// One SSO provider of the account (`GET /oidc_providers`, Owner only). The client secret is
/// write-only: the API never returns it and this type has no field for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OidcProvider {
    pub id: String,
    pub issuer: String,
    pub client_id: String,
    pub enabled: bool,
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

/// What a revealed one-time secret is, so the reveal modal can word itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretKind {
    /// A project's SDK API key (`db` names the database).
    ApiKey,
    /// An account-level admin API key (`label` is the key name).
    AdminKey,
    /// An invitation token (`label` is the invitee's email); the view turns it into a link.
    InviteToken,
}

/// One-time plaintext secret shown in the reveal modal. `Debug` redacts it; `update` drops it when
/// the modal closes, on navigation, sign-out and session expiry (AC-AUW-20).
#[derive(Clone, PartialEq, Eq)]
pub struct RevealedSecret {
    /// The database an API key belongs to; empty for the other kinds.
    pub db: DbId,
    pub secret: String,
    pub kind: SecretKind,
    /// What the secret is for: key name or invitee email. Empty for API keys.
    pub label: String,
}

impl RevealedSecret {
    pub fn api_key(db: DbId, secret: String) -> Self {
        Self { db, secret, kind: SecretKind::ApiKey, label: String::new() }
    }

    pub fn other(kind: SecretKind, label: String, secret: String) -> Self {
        Self { db: DbId::default(), secret, kind, label }
    }
}

impl std::fmt::Debug for RevealedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RevealedSecret")
            .field("db", &self.db)
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("secret", &"<redacted>")
            .finish()
    }
}

/// One row of `GET /projects/:id/query_logs`.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryLogEntry {
    pub id: String,
    /// `read | write | delete | listen | run_query` (G-34).
    pub op: String,
    pub collection_path: Option<String>,
    pub latency_ms: Option<i64>,
    pub status: String,
    pub error_code: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// The loaded pages of one database's query log (newest first).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueryLogs {
    /// Size of the whole filtered set on the server, not of `entries`.
    pub total: u64,
    pub entries: Vec<QueryLogEntry>,
}

impl QueryLogs {
    /// The cursor for the next page: the last loaded entry's id, while the server still has more.
    pub fn next_cursor(&self) -> Option<&str> {
        let more = (self.entries.len() as u64) < self.total;
        self.entries.last().filter(|_| more).map(|e| e.id.as_str())
    }
}

/// Query-log operation filter values the API accepts (`?op=`), in display order.
pub const LOG_OPS: [&str; 5] = ["read", "write", "delete", "listen", "run_query"];

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

/// The signed-in person, as reported by `GET /auth/session`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CurrentUser {
    pub user_id: String,
    pub account_id: String,
    pub account_name: String,
    pub email: String,
    pub display_name: String,
    pub role: Role,
    /// How this session was minted. Servers before `sso_required` omit it: `Password`.
    pub auth_method: AuthMethod,
    /// The user has at least one SSO link. Absent from older servers: `false`.
    pub sso_linked: bool,
    /// The account requires SSO for non-Owner members. Absent from older servers: `false`.
    pub sso_required: bool,
}

/// How the current session was established (`auth_method` of `GET /auth/session`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AuthMethod {
    #[default]
    Password,
    Sso,
}

/// What the "Your single sign-on" card offers the signed-in member (`AppModel::link_control`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkControl {
    /// No link yet: the re-authenticate-and-link form.
    Link,
    /// Linked and signed in with a password: the Unlink button.
    Unlink,
    /// Signed in through SSO: no password to re-enter, so neither Link nor Unlink (decision O4).
    SsoSession,
    /// Linked, non-Owner, and the account requires SSO: unlinking would lock them out.
    RequiredByAccount,
}

/// The persistent SSO-policy notice above the page (`AppModel::sso_notice`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SsoNotice {
    /// Non-Owner, linked: informational, no action.
    MembersUseSso,
    /// Non-Owner, not linked (a password session that predates the policy): link before it ends.
    LinkNow,
    /// Owner: reminder that the policy does not apply to them.
    OwnerExempt,
}

impl SsoNotice {
    pub fn message(self) -> &'static str {
        match self {
            SsoNotice::MembersUseSso => {
                "Your account requires single sign-on. You sign in through your identity provider."
            }
            SsoNotice::LinkNow => {
                "Your account now requires single sign-on, but your login is not linked to it. Link it under Account Settings before this session ends, or you will be locked out."
            }
            SsoNotice::OwnerExempt => {
                "SSO is required for members. As an Owner you are exempt and can still sign in with your password and authenticator."
            }
        }
    }
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
/// Used by `plan_summary()` to project `data::FREE_CAPS` into a `UsageTotals`.
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
    /// true → the CTA starts Stripe Checkout (Free-plan limit); false → opens the billing portal (past due).
    pub cta_starts_checkout: bool,
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
    /// Query-log pages per database (`GET /projects/:id/query_logs`).
    pub query_logs: HashMap<DbId, QueryLogs>,
    /// Databases whose hosted identity was enabled in this session (the API has no read for it).
    pub hosted_identity: HashSet<DbId>,
    /// Service accounts.
    pub service_accounts: Vec<ServiceAccount>,
    /// Account-level admin API keys.
    pub admin_keys: Vec<AdminKey>,
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
    /// Which signed-out screen is showing.
    pub auth_screen: AuthScreen,
    /// Error on the forgot/reset screens (server text or throttle).
    pub reset_error: Option<String>,
    /// Neutral confirmation on the signed-out screens ("link sent", "password updated").
    pub auth_notice: Option<String>,
    /// API calls in flight; drives disabled buttons and loading states (AC-AUW-16).
    pub pending: HashSet<CallKind>,
    /// Monotonic counter minting deterministic toast ids inside the pure `update`.
    pub toast_seq: u64,
    /// Plan/card/renewal snapshot.
    pub subscription: Subscription,
    /// card-payments: invoice history (persists across plan changes, AC-107-04).
    pub invoices: Vec<Invoice>,
    /// Create-database modal is open (the form fields are view-local; secrets never enter the model).
    pub create_db_modal_open: bool,
    /// Server-owned subscription status, caps and month usage.
    pub billing: BillingState,
    /// The browser just came back from Stripe Checkout: shown once on the Billing screen.
    pub checkout_return: Option<CheckoutReturn>,
    /// Which step of the invitation screen is showing.
    pub invite_step: InviteStep,
    /// Delete-account modal is open (the password / code fields are view-local; secrets never enter the model).
    pub delete_account_modal_open: bool,
    /// Error inside the delete-account modal (a wrong password is not a session expiry).
    pub account_delete_error: Option<String>,
    /// The account's SSO providers (Owner only; empty for everyone else).
    pub oidc_providers: Vec<OidcProvider>,
    /// Bumped on every confirmed provider create / secret rotation; views clear their secret fields on change.
    pub oidc_provider_saves: u32,
    /// Members who sign in through SSO (no password), from `GET /members`.
    pub sso_members: HashSet<UserId>,
    /// Error on the "Link SSO" form (a wrong password or code is not a session expiry).
    pub sso_link_error: Option<String>,
}

impl AppModel {
    /// Only the account Owner manages SSO providers (the server enforces the same rule).
    pub fn can_manage_sso(&self) -> bool {
        self.can_manage_billing()
    }

    /// The account requires SSO for non-Owner members (from the session probe).
    pub fn sso_required(&self) -> bool {
        self.current_user.as_ref().is_some_and(|u| u.sso_required)
    }

    /// Whether a password invitation may be created for `role`: while SSO is required only Owners (who
    /// are exempt) may get one; the server answers 422 otherwise.
    pub fn password_invite_allowed(&self, role: &Role) -> bool {
        !self.sso_required() || *role == Role::Owner
    }

    /// Which link control the signed-in member sees. `None` when signed out.
    pub fn link_control(&self) -> Option<LinkControl> {
        let u = self.current_user.as_ref()?;
        Some(if u.auth_method == AuthMethod::Sso {
            LinkControl::SsoSession
        } else if !u.sso_linked {
            LinkControl::Link
        } else if u.sso_required && u.role != Role::Owner {
            LinkControl::RequiredByAccount
        } else {
            LinkControl::Unlink
        })
    }

    /// The SSO-policy banner for the signed-in member; `None` while the account does not require SSO.
    pub fn sso_notice(&self) -> Option<SsoNotice> {
        let u = self.current_user.as_ref().filter(|u| u.sso_required)?;
        Some(match (&u.role, u.sso_linked) {
            (Role::Owner, _) => SsoNotice::OwnerExempt,
            (_, true) => SsoNotice::MembersUseSso,
            (_, false) => SsoNotice::LinkNow,
        })
    }

    /// Whether the invite dialog may offer SSO. The provider list is Owner-only, so an Admin cannot see
    /// it: for them the server decides (422 without an enabled provider). Viewers cannot invite at all.
    pub fn can_invite_sso(&self) -> bool {
        match self.current_user.as_ref().map(|u| &u.role) {
            Some(Role::Owner) => self.oidc_providers.iter().any(|p| p.enabled),
            Some(Role::Admin) => true,
            _ => false,
        }
    }

    /// Owner and Admin may change databases; Viewers see disabled controls (AC-AUW-21).
    /// The server enforces the same rule.
    pub fn can_manage_databases(&self) -> bool {
        self.current_user.as_ref().is_some_and(|u| u.role != Role::Viewer)
    }

    /// Owner and Admin may manage members, service accounts and admin keys (the server enforces it).
    pub fn can_manage_identities(&self) -> bool {
        self.can_manage_databases()
    }

    /// Roles the signed-in user may hand out: never above their own (the server rejects escalation).
    pub fn assignable_roles(&self) -> Vec<Role> {
        match self.current_user.as_ref().map(|u| &u.role) {
            Some(Role::Owner) => vec![Role::Owner, Role::Admin, Role::Viewer],
            Some(Role::Admin) => vec![Role::Admin, Role::Viewer],
            _ => vec![],
        }
    }

    /// Whether the signed-in user may change the role of, remove or revoke `target`:
    /// an Admin may not touch an Owner.
    pub fn can_edit_member(&self, target: &Member) -> bool {
        self.can_manage_identities()
            && (target.role != Role::Owner || self.current_user.as_ref().is_some_and(|u| u.role == Role::Owner))
    }

    /// Only the account Owner may delete a database (the server enforces the same rule).
    pub fn can_delete_databases(&self) -> bool {
        self.current_user.as_ref().is_some_and(|u| u.role == Role::Owner)
    }

    /// Only the account Owner may start Checkout or open the billing portal (the server enforces the same rule).
    pub fn can_manage_billing(&self) -> bool {
        self.current_user.as_ref().is_some_and(|u| u.role == Role::Owner)
    }

    /// Only the account Owner may delete the account (the server enforces the same rule).
    pub fn can_delete_account(&self) -> bool {
        self.can_manage_billing()
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
        // Once the server has answered, its status is authoritative (G-48).
        if let Some(status) = self.billing.status {
            return match status {
                SubscriptionStatus::FreeCapExceeded => EffectiveStatus::FreeCapExceeded,
                SubscriptionStatus::PastDue => EffectiveStatus::PastDue,
                SubscriptionStatus::Active | SubscriptionStatus::Canceled => EffectiveStatus::Active,
            };
        }
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
        if let Some(rows) = &self.billing.usage {
            return rows.clone();
        }
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
                cta_starts_checkout: true,
            });
        }
        Some(SuspensionBannerView {
            status: EffectiveStatus::PastDue,
            message: "We couldn't process your last payment.",
            cta_label: "Update payment method",
            cta_starts_checkout: false,
        })
    }
}
