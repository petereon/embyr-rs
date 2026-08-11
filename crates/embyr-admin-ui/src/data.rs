//! Mock data layer — `data::mock::*()` functions.
//!
//! V1: all data is in-process mock returning hardcoded collections.
//! Individual slices populate this data as they are implemented.
//! V2 plan: replace with `#[server]` function calls. Component code is unchanged (ADR-007).

/// Hardcoded display-only stats for a database at a given index (V1 mock).
///
/// Used by dashboard cards and the db-detail overview. Not part of the domain model.
pub struct DisplayStats {
    pub region: &'static str,
    pub p50: u32,
    pub p95: u32,
    pub p99: u32,
    pub reads: u64,
    pub writes: u64,
    pub deletes: u64,
    /// 12-point sparkline heights (0-40) for the db card latency preview.
    pub spark: [u8; 12],
    /// 12-bar ops series for the overview bar chart.
    pub ops: [u16; 12],
    /// Human-readable connection detail string.
    pub connection_detail: &'static str,
    pub backend_mode_label: &'static str,
    pub created: &'static str,
}

pub fn display_stats(idx: usize) -> DisplayStats {
    match idx {
        0 => DisplayStats {
            region: "us-east-1", p50: 4, p95: 11, p99: 28,
            reads: 1_240_000, writes: 342_000, deletes: 12_400,
            spark: [20,22,18,25,15,28,22,30,24,18,26,20],
            ops:   [3200,4100,5400,6800,7200,6400,5100,5800,7000,8100,7600,6300],
            connection_detail: "db.prod.internal:5432/embyr_prod",
            backend_mode_label: "direct_pg",
            created: "2025-08-14",
        },
        1 => DisplayStats {
            region: "us-east-1", p50: 7, p95: 19, p99: 44,
            reads: 88_000, writes: 41_000, deletes: 3_100,
            spark: [10,14,12,18,16,14,20,16,18,14,16,14],
            ops:   [420,510,680,720,640,590,710,680,760,820,710,590],
            connection_detail: "agent.staging.embyr.dev:8443",
            backend_mode_label: "agent_mode",
            created: "2025-09-02",
        },
        2 => DisplayStats {
            region: "europe-west1", p50: 12, p95: 33, p99: 71,
            reads: 612_000, writes: 9_800, deletes: 220,
            spark: [25,30,28,35,32,30,38,34,28,32,36,30],
            ops:   [2600,3100,4200,5100,4800,4400,5200,4900,5400,5900,5100,4600],
            connection_detail: "agent.analytics.embyr.dev:8443",
            backend_mode_label: "agent_mode",
            created: "2025-10-21",
        },
        _ => DisplayStats {
            region: "us-east-1", p50: 3, p95: 6, p99: 14,
            reads: 4_200, writes: 1_800, deletes: 90,
            spark: [5,6,4,8,5,7,4,6,5,7,6,5],
            ops:   [12,18,24,30,22,18,26,22,28,32,24,20],
            connection_detail: "localhost:5432/playground",
            backend_mode_label: "direct_pg",
            created: "2026-01-09",
        },
    }
}

pub fn fmt_num(n: u64) -> String {
    if n >= 1_000_000_000 { format!("{:.1}B", n as f64 / 1_000_000_000.0) }
    else if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1_000_000.0) }
    else if n >= 1_000     { format!("{:.1}K", n as f64 / 1_000.0) }
    else                   { n.to_string() }
}

pub fn sparkline_points(spark: &[u8]) -> String {
    let w = 300.0f32;
    let h = 40.0f32;
    let max = spark.iter().copied().max().unwrap_or(1) as f32;
    spark.iter().enumerate().map(|(i, &v)| {
        let x = i as f32 * w / (spark.len() - 1) as f32;
        let y = h - (v as f32 / max * h * 0.85);
        format!("{:.1},{:.1}", x, y)
    }).collect::<Vec<_>>().join(" ")
}

pub fn bar_chart_points(ops: &[u16]) -> Vec<(f32, f32, f32)> {
    // Returns (x, width, height_frac) tuples for each bar (0..1 height fraction)
    let max = ops.iter().copied().max().unwrap_or(1) as f32;
    let n = ops.len() as f32;
    ops.iter().enumerate().map(|(i, &v)| {
        let x = i as f32 / n;
        let w = 0.7 / n;
        let h = v as f32 / max;
        (x, w, h)
    }).collect()
}

use crate::model::{
    AdminKey, Card, Database, DbBackendMode, DbId, DbStatus, Invoice, KeyId, Member, OidcId,
    OidcProvider, Plan, Role, SdkKey, ServiceAccount, ServiceAccountId, Subscription,
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
                // card-payments (DELIVER, 02-02): healthy daily usage — well
                // under every FREE_CAPS dimension once projected ×30.
                usage: crate::model::UsageStats {
                    reads: 20_000,
                    writes: 3_000,
                    deletes: 200,
                    storage_gb: 0.4,
                },
            },
            Database {
                id: DbId(Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap()),
                name: "staging".to_string(),
                status: DbStatus::Active,
                backend_mode: DbBackendMode::AgentMode,
                logging_enabled: false,
                log_retention: None,
                created_at: None,
                // card-payments (DELIVER, 02-02): near-cap daily usage on
                // writes/deletes (84%/90% of FREE_CAPS ×30) for a realistic
                // amber demo on the Cap Usage card.
                usage: crate::model::UsageStats {
                    reads: 15_000,
                    writes: 14_000,
                    deletes: 3_000,
                    storage_gb: 1.1,
                },
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
    ///
    /// One hardcoded entry (Google) with enabled=false, representing a provider
    /// that has been configured but not yet activated.
    pub fn oidc_providers() -> Vec<OidcProvider> {
        vec![OidcProvider {
            id: OidcId(Uuid::parse_str("00000000-0000-0000-0000-000000000030").unwrap()),
            issuer: "https://accounts.google.com".to_string(),
            client_id: "mock-google-client-id".to_string(),
            enabled: false,
        }]
    }

    // ── card-payments (DISTILL RED scaffold, 2026-08-10) ────────────────────
    // SCAFFOLD: true
    //
    // DESIGN's literal Model Changes snippet names these `mock::subscription(
    // scenario) -> Subscription` / `mock::invoices(plan) -> Vec<Invoice>`.
    // Left RED (panicking) deliberately: `AppModel::from_mock()` was NOT
    // wired to call these (see model.rs comment) so the already-shipped app
    // and user-admin-ui test suite keep compiling and passing. DELIVER
    // implements these, then wires `from_mock()` to call them.

    pub fn subscription(_plan: Plan, _card: Option<Card>, _payment_failure: bool) -> Subscription {
        panic!("RED scaffold (card-payments): mock::subscription not yet implemented")
    }

    pub fn invoices(_plan: &Plan) -> Vec<Invoice> {
        panic!("RED scaffold (card-payments): mock::invoices not yet implemented")
    }
}

// ── card-payments (DISTILL RED scaffold, 2026-08-10) ────────────────────────
// Constants placed here per DESIGN DDD-4 / D7 (Shared Artifacts Registry):
// "FREE_CAPS/PRICING constants live in data.rs, matching the DISCUSS Shared
// Artifacts Registry's explicit source-of-truth assignment." Real (non-
// panicking) constant data — only the *derivation logic* consuming them is
// RED-scaffolded (see model.rs's `impl AppModel` block, and the pure
// functions below).

/// Per-dimension monthly volume shape shared by `FREE_CAPS` and
/// `Pricing::pro_included`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FreeCaps {
    pub reads: u64,
    pub writes: u64,
    pub deletes: u64,
    pub storage_gb: f64,
}

/// Free plan included monthly volume per dimension (D-7: all 4 dimensions
/// metered separately). Illustrative placeholder values (feature-delta.md
/// § Out of Scope: "Actual price points and included allowances ... are
/// illustrative placeholders throughout").
pub const FREE_CAPS: FreeCaps = FreeCaps {
    reads: 2_000_000,
    writes: 500_000,
    deletes: 100_000,
    storage_gb: 2.0,
};

/// Pro plan pricing: base + per-dimension overage rate beyond `pro_included`.
/// AC-103-02 worked example: 620,000 overage reads / 100,000 ×
/// `overage_rate_per_100k_reads` ($0.05) = $0.31.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pricing {
    pub pro_base: f64,
    pub pro_included: FreeCaps,
    pub overage_rate_per_100k_reads: f64,
    pub overage_rate_per_100k_writes: f64,
    pub overage_rate_per_100k_deletes: f64,
    pub overage_rate_per_gb_storage: f64,
}

pub const PRICING: Pricing = Pricing {
    pro_base: 49.00,
    pro_included: FREE_CAPS,
    overage_rate_per_100k_reads: 0.05,
    overage_rate_per_100k_writes: 0.05,
    overage_rate_per_100k_deletes: 0.05,
    overage_rate_per_gb_storage: 0.10,
};

/// Free vs Pro feature comparison content for UpgradeModal's compare step
/// (AC-106-01). D-4: Free + Pro tiers only, never a third tier.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanFeatures {
    pub free_included: FreeCaps,
    pub pro_base: f64,
}

pub const PLAN_FEATURES: PlanFeatures = PlanFeatures {
    free_included: FREE_CAPS,
    pro_base: PRICING.pro_base,
};

/// AC-103-02/03: per-dimension overage line items + total for the Next
/// Invoice card estimate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InvoiceEstimate {
    pub base: f64,
    pub overage_reads: f64,
    pub overage_writes: f64,
    pub overage_deletes: f64,
    pub overage_storage: f64,
    pub total: f64,
    /// AC-103-04: true when any overage line item is > 0.
    pub has_overage: bool,
}

/// AC-102-02: bar color thresholds — accent <80%, amber 80-99%, red ≥100%.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BarColor {
    Accent,
    Amber,
    Red,
}

// SCAFFOLD: true — pure functions below panic (RED), per Mandate 7. DELIVER
// implements the formula documented in each doc comment.

/// AC-103-02/03: `overage = max(0, usage - pro_included) / unit * rate`;
/// `total = base + sum(overage)`; storage overage uses `usage_gb *
/// overage_rate_per_gb_storage` directly (no /100k unit, per DESIGN's
/// literal formula: `storage = usage_gb * storagePerGB`).
pub fn next_invoice_estimate(usage: &crate::model::UsageTotals) -> InvoiceEstimate {
    let included = PRICING.pro_included;

    let overage_reads = (usage.reads.saturating_sub(included.reads)) as f64 / 100_000.0
        * PRICING.overage_rate_per_100k_reads;
    let overage_writes = (usage.writes.saturating_sub(included.writes)) as f64 / 100_000.0
        * PRICING.overage_rate_per_100k_writes;
    let overage_deletes = (usage.deletes.saturating_sub(included.deletes)) as f64 / 100_000.0
        * PRICING.overage_rate_per_100k_deletes;
    let overage_storage =
        (usage.storage_gb - included.storage_gb).max(0.0) * PRICING.overage_rate_per_gb_storage;

    let total = PRICING.pro_base + overage_reads + overage_writes + overage_deletes + overage_storage;
    let has_overage = overage_reads > 0.0 || overage_writes > 0.0 || overage_deletes > 0.0 || overage_storage > 0.0;

    InvoiceEstimate {
        base: PRICING.pro_base,
        overage_reads,
        overage_writes,
        overage_deletes,
        overage_storage,
        total,
        has_overage,
    }
}

/// AC-102-02: maps a cap ratio (0.0 = 0%, 1.0 = 100%) to a bar color.
pub fn bar_color(ratio: f64) -> BarColor {
    if ratio >= 1.0 {
        return BarColor::Red;
    }
    if ratio >= 0.80 {
        return BarColor::Amber;
    }
    BarColor::Accent
}

/// AC-105-02: brand auto-detected from the card number prefix (e.g. "4" →
/// Visa, "5" → Mastercard).
pub fn detect_card_brand(number: &str) -> crate::model::CardBrand {
    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
    match digits.chars().next() {
        Some('4') => crate::model::CardBrand::Visa,
        Some('5') => crate::model::CardBrand::Mastercard,
        Some('3') => crate::model::CardBrand::Amex,
        Some('6') => crate::model::CardBrand::Discover,
        _ => crate::model::CardBrand::Unknown,
    }
}

/// AC-105-05: true only for a complete, plausible card number (V1: length
/// check sufficient — no Luhn/real validation, Rust-native form only).
pub fn card_number_is_complete(number: &str) -> bool {
    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
    digits.len() == 16
}
