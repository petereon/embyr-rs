//! Display helpers and the billing-simulation constants. Everything shown to the signed-in user comes
//! from the API; nothing here is fixture data (AC-AUW-18).

pub fn fmt_num(n: u64) -> String {
    if n >= 1_000_000_000 { format!("{:.1}B", n as f64 / 1_000_000_000.0) }
    else if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1_000_000.0) }
    else if n >= 1_000     { format!("{:.1}K", n as f64 / 1_000.0) }
    else                   { n.to_string() }
}

/// SVG polyline points (`x,y …`) for `values` inside a `width` x `height` box; larger = higher.
/// Fewer than two values draw nothing.
pub fn line_points(values: &[u64], width: f32, height: f32) -> String {
    if values.len() < 2 {
        return String::new();
    }
    let max = values.iter().copied().max().unwrap_or(0).max(1) as f32;
    let step = width / (values.len() - 1) as f32;
    values
        .iter()
        .enumerate()
        .map(|(i, &v)| format!("{:.1},{:.1}", i as f32 * step, height - v as f32 / max * height * 0.9))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `(x, width, height_fraction)` per bar, each in 0..1.
pub fn bar_chart_points(values: &[u64]) -> Vec<(f32, f32, f32)> {
    let max = values.iter().copied().max().unwrap_or(0).max(1) as f32;
    let n = values.len().max(1) as f32;
    values.iter().enumerate().map(|(i, &v)| (i as f32 / n, 0.7 / n, v as f32 / max)).collect()
}

// Constants placed here per DESIGN DDD-4 / D7 (Shared Artifacts Registry):
// "FREE_CAPS/PRICING constants live in data.rs, matching the DISCUSS Shared
// Artifacts Registry's explicit source-of-truth assignment."

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

    let total =
        PRICING.pro_base + overage_reads + overage_writes + overage_deletes + overage_storage;
    let has_overage = overage_reads > 0.0
        || overage_writes > 0.0
        || overage_deletes > 0.0
        || overage_storage > 0.0;

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

/// Strip all non-digit characters (spaces, dashes) from a raw card number
/// input. Shared by `detect_card_brand` and `card_number_is_complete`.
fn digits_only(number: &str) -> String {
    number.chars().filter(char::is_ascii_digit).collect()
}

/// AC-105-02: brand auto-detected from the card number prefix (e.g. "4" →
/// Visa, "5" → Mastercard).
pub fn detect_card_brand(number: &str) -> crate::model::CardBrand {
    match digits_only(number).chars().next() {
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
    digits_only(number).len() == 16
}
