// SCAFFOLD: true
//! Slice 03 — Invoice Forecast + Usage Breakdown acceptance scenarios.
//! Stories: US-103 (Next Invoice estimate), US-104 (per-database Usage tab)
//!
//! All tests are #[ignore] (RED). Enable one at a time in DELIVER.
//!
//! Out of model-layer test scope (view-rendering concerns, not modeled in
//! AppModel — same class as Tabs' view-local active-tab state in
//! user-admin-ui): AC-103-05 (footer disclosure text), AC-103-06 ("Estimated
//! — projecting to period end" label), AC-104-02 (3-color stacked bar
//! proportions), AC-104-05 (existing BillingRange time-range selector,
//! already view-local per views/billing.rs).

use embyr_admin_ui::data;
use embyr_admin_ui::model::{Plan, UsageTotals};

#[path = "../common/mod.rs"]
mod common;
use common::{database_with_daily_usage, model_on_plan};

// ─────────────────────────────────────────────────────────────────────────────
// US-103 / AC-103-02/03/04: Next Invoice overage estimate.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-103-02: overage estimate breaks down by dimension — 620,000 overage
/// reads beyond the Pro-included allowance = $0.31 (620,000 / 100,000 ×
/// $0.05), matching the DISCUSS domain example verbatim.
#[test]
fn overage_estimate_breaks_down_by_dimension_for_reads() {
    // Given: Northwind Data (Pro plan) has used 620,000 reads beyond its
    // included allowance this period (2,000,000 included + 620,000 over).
    let usage = UsageTotals {
        reads: data::PRICING.pro_included.reads + 620_000,
        writes: 0,
        deletes: 0,
        storage_gb: 0.0,
    };

    // When: Dana views the Next Invoice card.
    let estimate = data::next_invoice_estimate(&usage);

    // Then: the itemized breakdown shows an estimated reads overage of $0.31.
    assert!(
        (estimate.overage_reads - 0.31).abs() < 0.001,
        "AC-103-02: reads overage must be ~$0.31, got {}",
        estimate.overage_reads
    );
}

/// AC-103-04: no overage (usage fully within the Pro included allowance)
/// projects a base-only invoice.
#[test]
fn no_overage_projects_base_only_invoice() {
    // Given: Northwind Data has stayed within its Pro included allowance
    // for reads, writes, and deletes this period.
    let usage = UsageTotals {
        reads: data::PRICING.pro_included.reads,
        writes: data::PRICING.pro_included.writes,
        deletes: data::PRICING.pro_included.deletes,
        storage_gb: data::PRICING.pro_included.storage_gb,
    };

    // When: Dana views the Next Invoice card.
    let estimate = data::next_invoice_estimate(&usage);

    // Then: the card shows "$49.00 base · no overage projected".
    assert!(!estimate.has_overage, "AC-103-04: has_overage must be false at/under the included allowance");
    assert_eq!(estimate.total, data::PRICING.pro_base, "AC-103-04: total must equal base with no overage");
}

/// AC-103-03: total always equals base + sum of every overage line item.
#[test]
fn total_equals_base_plus_sum_of_overage_line_items() {
    // Given: usage with overage on multiple dimensions simultaneously.
    let usage = UsageTotals {
        reads: data::PRICING.pro_included.reads + 200_000,
        writes: data::PRICING.pro_included.writes + 50_000,
        deletes: data::PRICING.pro_included.deletes,
        storage_gb: data::PRICING.pro_included.storage_gb + 1.0,
    };

    // When: the Next Invoice estimate is computed.
    let estimate = data::next_invoice_estimate(&usage);

    // Then: total = base + sum(overage line items).
    let expected_total = estimate.base
        + estimate.overage_reads
        + estimate.overage_writes
        + estimate.overage_deletes
        + estimate.overage_storage;
    assert!(
        (estimate.total - expected_total).abs() < 0.001,
        "AC-103-03: total must equal base + sum(overage line items)"
    );
    assert!(estimate.has_overage, "precondition: this scenario has overage on 3 dimensions");
}

/// AC-103-02, Error/Boundary: storage overage uses a flat `usage_gb *
/// storagePerGB` rate — NOT the same `/100,000 unit` formula as
/// reads/writes/deletes. A naive copy-paste of the /100k formula to storage
/// would silently under/over-charge by orders of magnitude.
#[test]
fn storage_overage_uses_flat_gb_rate_not_per_100k_unit() {
    // Given: 1.5 GB of storage beyond the Pro included allowance.
    let usage = UsageTotals {
        reads: data::PRICING.pro_included.reads,
        writes: data::PRICING.pro_included.writes,
        deletes: data::PRICING.pro_included.deletes,
        storage_gb: data::PRICING.pro_included.storage_gb + 1.5,
    };

    // When: the Next Invoice estimate is computed.
    let estimate = data::next_invoice_estimate(&usage);

    // Then: storage overage = 1.5 * overage_rate_per_gb_storage (flat rate,
    // not divided by 100,000).
    let expected = 1.5 * data::PRICING.overage_rate_per_gb_storage;
    assert!(
        (estimate.overage_storage - expected).abs() < 0.001,
        "AC-103-02: storage overage must use the flat per-GB rate, expected {}, got {}",
        expected,
        estimate.overage_storage
    );
}

// ── Mutation-killing: writes/deletes overage arithmetic + has_overage ──────
//
// The reads (worked example) and storage (flat-rate) dimensions already had
// exact-value coverage above; writes/deletes shared the same /100k-unit
// formula as reads but had no dimension-specific pinned value, and
// `has_overage`'s per-dimension `> 0.0` / `||` chain was only exercised by
// all-true and all-false cases, never an isolated single-dimension case.

/// AC-103-02: writes and deletes overage use the identical /100,000-unit ×
/// rate formula as reads — verified independently per dimension so a
/// copy-paste bug in one dimension's divisor/rate does not hide behind
/// reads' coverage alone.
#[test]
fn overage_estimate_uses_same_per_100k_formula_for_writes_and_deletes() {
    let included = data::PRICING.pro_included;

    // 300,000 overage writes / 100,000 × $0.05 = $0.15
    let writes_usage = UsageTotals {
        reads: included.reads,
        writes: included.writes + 300_000,
        deletes: included.deletes,
        storage_gb: included.storage_gb,
    };
    let writes_estimate = data::next_invoice_estimate(&writes_usage);
    assert!(
        (writes_estimate.overage_writes - 0.15).abs() < 0.001,
        "AC-103-02: writes overage must be $0.15, got {}",
        writes_estimate.overage_writes
    );

    // 400,000 overage deletes / 100,000 × $0.05 = $0.20
    let deletes_usage = UsageTotals {
        reads: included.reads,
        writes: included.writes,
        deletes: included.deletes + 400_000,
        storage_gb: included.storage_gb,
    };
    let deletes_estimate = data::next_invoice_estimate(&deletes_usage);
    assert!(
        (deletes_estimate.overage_deletes - 0.20).abs() < 0.001,
        "AC-103-02: deletes overage must be $0.20, got {}",
        deletes_estimate.overage_deletes
    );
}

/// AC-103-04: `has_overage` is true when ANY single dimension exceeds its
/// included allowance, even with the other three exactly at their included
/// allowance — isolates each dimension's `> 0.0` comparison and its `||`
/// link in the has_overage chain (an all-true/all-false case cannot tell
/// `||` from `&&`, or `>` from `<`, apart).
#[test]
fn has_overage_true_when_any_single_dimension_exceeds_included_allowance() {
    let included = data::PRICING.pro_included;
    let cases = [
        ("reads", UsageTotals { reads: included.reads + 1, writes: included.writes, deletes: included.deletes, storage_gb: included.storage_gb }),
        ("writes", UsageTotals { reads: included.reads, writes: included.writes + 1, deletes: included.deletes, storage_gb: included.storage_gb }),
        ("deletes", UsageTotals { reads: included.reads, writes: included.writes, deletes: included.deletes + 1, storage_gb: included.storage_gb }),
        ("storage_gb", UsageTotals { reads: included.reads, writes: included.writes, deletes: included.deletes, storage_gb: included.storage_gb + 0.001 }),
    ];

    for (dimension, usage) in cases {
        let estimate = data::next_invoice_estimate(&usage);
        assert!(
            estimate.has_overage,
            "AC-103-04: has_overage must be true when only {dimension} exceeds its included allowance"
        );
    }
}

/// AC-103-03: total must reflect every overage line item independently —
/// computed by hand here (NOT re-summed from `estimate`'s own returned
/// fields, which would be circular and blind to a `+`→`-` mutation on any
/// single term of the total's own summation).
#[test]
fn total_reflects_every_overage_line_item_independently_computed() {
    let included = data::PRICING.pro_included;
    let usage = UsageTotals {
        reads: included.reads + 100_000,     // +$0.05
        writes: included.writes + 200_000,   // +$0.10
        deletes: included.deletes + 300_000, // +$0.15
        storage_gb: included.storage_gb + 2.0, // +$0.20
    };

    let estimate = data::next_invoice_estimate(&usage);
    let expected_total = 49.00 + 0.05 + 0.10 + 0.15 + 0.20;

    assert!(
        (estimate.total - expected_total).abs() < 0.001,
        "AC-103-03: total must be ${}, got {}",
        expected_total, estimate.total
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// US-104 / AC-104-01/03/04: per-database Usage tab.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-104-01: the Usage table shows real, differentiated per-database
/// numbers — no "—" placeholders.
#[test]
fn usage_table_shows_real_per_database_numbers() {
    // Given: Aperture Labs has 3 databases with recorded usage this period.
    let mut model = model_on_plan(Plan::Free);
    model.databases = vec![
        database_with_daily_usage("prod-orders", 10_333, 2_966, 400, 0.4),
        database_with_daily_usage("prod-inventory", 4_000, 1_000, 100, 0.2),
        database_with_daily_usage("staging-orders", 133, 20, 5, 0.01),
    ];

    // When: Chris navigates to Billing → Usage.
    let rows = model.usage_table_rows();

    // Then: each database row shows its own, differentiated usage.
    assert_eq!(rows.len(), 3, "AC-104-01: one row per database");
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"prod-orders"), "AC-104-01: prod-orders must appear");
    assert!(names.contains(&"prod-inventory"), "AC-104-01: prod-inventory must appear");
    assert!(names.contains(&"staging-orders"), "AC-104-01: staging-orders must appear");
    assert_ne!(
        rows[0].usage, rows[2].usage,
        "AC-104-01: rows must show real, differentiated values — not identical placeholders"
    );
}

/// AC-104-03: the 5 KPI summary tiles total exactly across all rows.
#[test]
fn kpi_summary_tiles_total_across_all_databases() {
    // Given: Aperture Labs' 3 databases have distinct daily usage.
    let mut model = model_on_plan(Plan::Free);
    model.databases = vec![
        database_with_daily_usage("prod-orders", 10_000, 3_000, 400, 0.4),
        database_with_daily_usage("prod-inventory", 4_000, 1_000, 100, 0.2),
        database_with_daily_usage("staging-orders", 133, 20, 5, 0.01),
    ];

    // When: Chris views the Usage tab.
    let rows = model.usage_table_rows();
    let totals = model.usage_totals();

    // Then: the summary totals equal the sum of the table rows (×30
    // projection for reads/writes/deletes; storage_gb is a snapshot sum).
    let summed_reads: u64 = rows.iter().map(|r| r.usage.reads).sum::<u64>() * 30;
    let summed_storage: f64 = rows.iter().map(|r| r.usage.storage_gb).sum();
    assert_eq!(summed_reads, totals.reads, "AC-104-03: reads KPI tile must match the row sum");
    assert!(
        (summed_storage - totals.storage_gb).abs() < 0.001,
        "AC-104-03: storage KPI tile must match the row sum"
    );
}

/// AC-104-01, Edge: a database created 2 days ago shows its proportionally
/// low real usage, not a dash or a misleading zero.
#[test]
fn newly_created_database_shows_proportionally_low_not_missing_usage() {
    // Given: "staging-orders" was created 2 days ago with 4,000 reads
    // recorded so far (a low daily rate, not zero).
    let mut model = model_on_plan(Plan::Free);
    model.databases = vec![database_with_daily_usage("staging-orders", 2_000, 0, 0, 0.0)];

    // When: Chris views the Usage tab.
    let rows = model.usage_table_rows();

    // Then: the row shows real, nonzero reads — not a dash or blank.
    assert_eq!(rows.len(), 1);
    assert!(rows[0].usage.reads > 0, "AC-104-01: newly created db must show nonzero, not a dash");
}

/// AC-104-04, Error/Boundary: an account with zero databases keeps today's
/// unchanged empty state — this is a REGRESSION check (the empty state
/// existed before this feature; card-payments must not break it).
#[test]
fn account_with_no_databases_keeps_existing_empty_state() {
    // Given: an account has zero databases.
    let model = model_on_plan(Plan::Free);
    assert!(model.databases.is_empty(), "precondition: zero databases");

    // When: its admin navigates to Billing → Usage.
    let rows = model.usage_table_rows();

    // Then: the table has zero rows (the view renders "No databases —
    // nothing to bill" exactly as it does today, per views/billing.rs).
    assert!(rows.is_empty(), "AC-104-04: zero databases must yield zero usage rows");
}
