//! AppModel and all domain types for the embyr admin UI.
//!
//! Minimal field set — sufficient for `update()` logic to compile and for
//! proptest strategies to construct values. Expand during DELIVER.

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use uuid::Uuid;

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
    /// card-payments (DISTILL, 2026-08-10): per-database daily usage counters.
    /// `AppModel::usage_totals()` sums and projects these to monthly (×30),
    /// mirroring the design-reference.md prototype's `sum(db.reads) * 30`.
    /// Extended per DESIGN DDD-3 / feature-delta.md Model Changes.
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
// DISTILL-introduced (not literally named in DESIGN's 5-method list) to give
// each AC a concrete, testable driving-port surface per Mandate 1/4. DELIVER
// may refine field shapes; the *existence* of a single-sourced projection per
// card/component is the DISTILL contract, not these exact field names.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageTotals {
    pub reads: u64,
    pub writes: u64,
    pub deletes: u64,
    pub storage_gb: f64,
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
    /// card-payments (DISTILL, 2026-08-10): plan/card/renewal snapshot.
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
            sdk_keys: std::collections::HashMap::new(),
            service_accounts: mock::service_accounts(),
            admin_keys: mock::admin_keys(),
            oidc_providers: mock::oidc_providers(),
            toasts: Vec::new(),
            nav: NavState::default(),
            totp_failures: 0,
            account_locked: false,
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
                included: Some(UsageTotals {
                    reads: crate::data::FREE_CAPS.reads,
                    writes: crate::data::FREE_CAPS.writes,
                    deletes: crate::data::FREE_CAPS.deletes,
                    storage_gb: crate::data::FREE_CAPS.storage_gb,
                }),
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
            free_included: UsageTotals {
                reads: plan_features.free_included.reads,
                writes: plan_features.free_included.writes,
                deletes: plan_features.free_included.deletes,
                storage_gb: plan_features.free_included.storage_gb,
            },
            pro_base_price: plan_features.pro_base,
            shows_downgrade_warning: self.upgrade_modal_step == UpgradeModalStep::ConfirmDowngrade,
        }
    }
}
