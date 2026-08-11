// SCAFFOLD: true
//! Slice 02 — Cap Usage Visibility acceptance scenarios.
//! Story: US-102 (See How Close I Am to My Free-Plan Limits)
//!
//! All tests are #[ignore] (RED). Enable one at a time in DELIVER.
//!
//! V1 note: `AppModel::cap_ratios()`/`data::bar_color()` are RED scaffolds
//! (see model.rs/data.rs). Tests document the expected contract; DELIVER
//! implements the `usage_totals()×30 / FREE_CAPS` formula and the
//! accent/amber/red thresholds.

use embyr_admin_ui::data::{self, BarColor};
use embyr_admin_ui::model::Plan;

#[path = "../common/mod.rs"]
mod common;
use common::{database_with_daily_usage, model_on_plan};

// ─────────────────────────────────────────────────────────────────────────────
// AC-102-01/02/03: per-dimension cap ratio + bar color + percentage label.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-102-01/02: a dimension approaching its cap (84% of the Free writes
/// cap, adapted from the domain example's 412K/500K = 82% for clean ×30
/// arithmetic) renders its bar in amber.
#[test]
#[ignore] // RED — enable in DELIVER
fn writes_bar_goes_amber_approaching_cap() {
    // Given: Aperture Labs (Free plan) has used 420,000 of 500,000 monthly
    // writes (14,000/day × 30).
    let mut model = model_on_plan(Plan::Free);
    model.databases = vec![database_with_daily_usage("db-1", 0, 14_000, 0, 0.0)];

    // When: Chris views the Cap Usage card.
    let ratios = model.cap_ratios();

    // Then: the Writes bar is in the amber band (80-99%) and colored amber.
    assert!(
        (0.80..1.0).contains(&ratios.writes),
        "AC-102-01: writes ratio must land in the amber band, got {}",
        ratios.writes
    );
    assert_eq!(data::bar_color(ratios.writes), BarColor::Amber, "AC-102-02: amber at 80-99%");
}

/// AC-102-01/02: a dimension at or above its cap (Solstice Analytics'
/// deletes, adapted to 102,000/100,000) renders its bar in red.
#[test]
#[ignore] // RED — enable in DELIVER
fn deletes_bar_goes_red_at_or_above_cap() {
    // Given: Solstice Analytics (Free plan) has used 102,000 of 100,000
    // monthly deletes (3,400/day × 30) — already `free_cap_exceeded`.
    let mut model = model_on_plan(Plan::Free);
    model.databases = vec![database_with_daily_usage("db-1", 0, 0, 3_400, 0.0)];

    // When: Priya views the Cap Usage card.
    let ratios = model.cap_ratios();

    // Then: the Deletes bar is >= 100% and colored red.
    assert!(ratios.deletes >= 1.0, "AC-102-01: deletes ratio must be >= 100%, got {}", ratios.deletes);
    assert_eq!(data::bar_color(ratios.deletes), BarColor::Red, "AC-102-02: red at >= 100%");
}

/// AC-102-01/02: usage well under cap (~40% of the Free reads cap) renders
/// in the default accent color, not amber/red.
#[test]
#[ignore] // RED — enable in DELIVER
fn reads_well_under_cap_renders_accent() {
    // Given: Aperture Labs has used 810,000 of 2,000,000 monthly reads
    // (27,000/day × 30).
    let mut model = model_on_plan(Plan::Free);
    model.databases = vec![database_with_daily_usage("db-1", 27_000, 0, 0, 0.0)];

    // When: Chris views the Cap Usage card.
    let ratios = model.cap_ratios();

    // Then: the Reads bar is under 80% and colored with the default accent.
    assert!(ratios.reads < 0.80, "AC-102-01: reads ratio must be under 80%, got {}", ratios.reads);
    assert_eq!(data::bar_color(ratios.reads), BarColor::Accent, "AC-102-02: default accent under 80%");
}

/// AC-102-01/02, Error/Boundary: a brand-new account ("Bramble & Co") with
/// zero usage across all four dimensions must show 0% everywhere, with no
/// false amber/red — a naive `ratio >= threshold` check without a
/// zero-usage guard could misfire on degenerate inputs.
#[test]
#[ignore] // RED — enable in DELIVER
fn brand_new_account_shows_zero_usage_without_false_alarms() {
    // Given: Bramble & Co has recorded zero reads, writes, deletes, and
    // storage this cycle (no databases yet).
    let model = model_on_plan(Plan::Free);
    assert!(model.databases.is_empty(), "precondition: brand-new account has no databases");

    // When: Marcus views the Cap Usage card.
    let ratios = model.cap_ratios();

    // Then: all four bars are 0% and none are amber/red.
    for (dimension, ratio) in [
        ("reads", ratios.reads),
        ("writes", ratios.writes),
        ("deletes", ratios.deletes),
        ("storage_gb", ratios.storage_gb),
    ] {
        assert_eq!(ratio, 0.0, "AC-102-01: {dimension} ratio must be exactly 0.0 for zero usage");
        assert_eq!(
            data::bar_color(ratio),
            BarColor::Accent,
            "AC-102-02: zero usage must never render amber/red for {dimension}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-102-05: expanding a dimension reveals the per-database breakdown.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-102-05: expanding the Writes row lists per-database write counts
/// that sum to the card's total.
#[test]
#[ignore] // RED — enable in DELIVER
fn per_database_write_breakdown_sums_to_card_total() {
    // Given: Aperture Labs has 3 databases contributing to its write count.
    let mut model = model_on_plan(Plan::Free);
    model.databases = vec![
        database_with_daily_usage("prod-orders", 0, 5_000, 0, 0.0),
        database_with_daily_usage("prod-inventory", 0, 3_000, 0, 0.0),
        database_with_daily_usage("staging-orders", 0, 500, 0, 0.0),
    ];

    // When: Chris expands the Writes row on the Cap Usage card.
    let rows = model.usage_table_rows();
    let totals = model.usage_totals();

    // Then: the per-database write counts sum to the card's total.
    let summed: u64 = rows.iter().map(|r| r.usage.writes).sum::<u64>() * 30;
    assert_eq!(
        summed, totals.writes,
        "AC-102-05: per-database breakdown must sum to the card's total"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-102-04 / AC-108-06: cap-exceeded logic is Free-plan-only (D-6).
// ─────────────────────────────────────────────────────────────────────────────

/// AC-102-04: the Cap Usage card's underlying logic (`cap_exceeded()`)
/// never fires for a Pro-plan account, even when usage would exceed the
/// Free thresholds — Pro shows the Next Invoice card instead (US-103), and
/// D-6's hard-stop is Free-plan-only.
#[test]
#[ignore] // RED — enable in DELIVER
fn pro_plan_never_treated_as_cap_exceeded_even_over_free_thresholds() {
    // Given: a Pro-plan account with usage far beyond every Free cap.
    let mut model = model_on_plan(Plan::Pro);
    model.databases = vec![database_with_daily_usage("db-1", 500_000, 100_000, 20_000, 50.0)];

    // When: the account's cap-exceeded status is derived.
    let exceeded = model.cap_exceeded();

    // Then: Pro is never cap_exceeded — the Cap Usage card must not render
    // for this account (NextInvoiceCard renders instead, US-103).
    assert!(!exceeded, "AC-102-04/AC-108-06: cap_exceeded() must be Free-plan-only (D-6)");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-102-02 boundary tests — the exact threshold values.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-102-02, boundary: ratio exactly 0.8 (80%) is the first amber value.
#[test]
fn bar_color_boundary_at_exactly_80_percent_is_amber() {
    assert_eq!(data::bar_color(0.8), BarColor::Amber, "80% is the amber threshold, inclusive");
}

/// AC-102-02, boundary: ratio exactly 1.0 (100%) is the first red value.
#[test]
fn bar_color_boundary_at_exactly_100_percent_is_red() {
    assert_eq!(data::bar_color(1.0), BarColor::Red, "100% is the red threshold, inclusive");
}

/// AC-102-02, boundary: just under 80% stays accent (not amber).
#[test]
fn bar_color_just_under_80_percent_is_accent() {
    assert_eq!(data::bar_color(0.7999), BarColor::Accent, "just under 80% must stay accent");
}

/// AC-102-02, boundary: just under 100% stays amber (not red).
#[test]
fn bar_color_just_under_100_percent_is_amber() {
    assert_eq!(data::bar_color(0.9999), BarColor::Amber, "just under 100% must stay amber");
}
