// SCAFFOLD: true
//! TEA state machine scenarios — card-payments status-derivation properties.
//!
//! Tests the pure `impl AppModel` derivation methods directly (no browser,
//! no WASM, no Leptos). All tests are #[ignore] (RED) until DELIVER enables
//! them one at a time.
//!
//! Layer: in-memory acceptance (layer 2). Per nw-test-design-mandates
//! Mandate 9, layers 1-2 use PBT full (`proptest!` with generative
//! strategies, 256+ cases per property by default) — this is exactly the
//! "quantifiable invariant" case the DISTILL Property recipe calls for:
//! `capExceeded`/`effectiveStatus`/`readOnly` must be internally consistent
//! for ANY valid usage/plan/card combination, not just the worked examples
//! already covered as `Scenario:`-style examples in the slice_NN files.
//!
//! Per Mandate 11, layer 3+ sad paths stay example-based — this file is
//! exclusively layer 1-2 in-memory, so PBT machinery is appropriate
//! throughout.

#![allow(unused_imports)]

use proptest::prelude::*;

use embyr_admin_ui::data;
use embyr_admin_ui::model::{AppModel, CardBrand, EffectiveStatus, Plan};
use embyr_admin_ui::msg::Msg;
use embyr_admin_ui::update::update;

#[path = "../common/mod.rs"]
mod common;
use common::arb::*;

// ── @property: readOnly ⟺ status != active (AC-108-01/06) ──────────────────

proptest! {
    /// @property: for ANY subscription/usage combination, `read_only()` is
    /// true if and only if `effective_status()` is not `Active` — this is
    /// the literal contract AC-108-01 depends on ("Banner renders only
    /// when derived readOnly is true").
    #[test]
    #[ignore] // RED — enable in DELIVER
    fn read_only_iff_status_not_active(model in arb_model_with_databases()) {
        let status = model.effective_status();
        let read_only = model.read_only();
        prop_assert_eq!(
            read_only, status != EffectiveStatus::Active,
            "read_only() must be exactly (effective_status() != Active)"
        );
    }
}

// ── @property: free_cap_exceeded only possible on Free plan (D-6/D-7) ──────

proptest! {
    /// @property: for ANY model, `effective_status() == FreeCapExceeded`
    /// implies `subscription.plan == Free` — the D-6 hard-stop is
    /// structurally impossible to trigger on Pro, by construction of
    /// `cap_exceeded()`'s own `plan == Free` guard.
    #[test]
    #[ignore] // RED — enable in DELIVER
    fn free_cap_exceeded_implies_free_plan(model in arb_model_with_databases()) {
        if model.effective_status() == EffectiveStatus::FreeCapExceeded {
            prop_assert_eq!(model.subscription.plan, Plan::Free);
        }
    }
}

proptest! {
    /// @property: Pro-plan accounts NEVER show `free_cap_exceeded`,
    /// regardless of usage magnitude — the direct converse of the property
    /// above, stated as its own property per the DISTILL brief's explicit
    /// invariant list ("Pro plan never shows free_cap_exceeded regardless
    /// of usage").
    #[test]
    #[ignore] // RED — enable in DELIVER
    fn pro_plan_never_shows_free_cap_exceeded(dbs in prop::collection::vec(arb_database_with_usage(), 0..=6)) {
        let mut model = AppModel::default();
        model.authed = true;
        model.subscription.plan = Plan::Pro;
        model.databases = dbs;

        prop_assert_ne!(model.effective_status(), EffectiveStatus::FreeCapExceeded);
        prop_assert!(!model.cap_exceeded());
    }
}

// ── @property: cap_exceeded ⟺ some dimension's ratio >= 1.0, Free-only ─────

proptest! {
    /// @property: on the Free plan, `cap_exceeded()` is true if and only if
    /// AT LEAST ONE dimension's cap ratio is >= 1.0 (D-7: all 4 dimensions
    /// metered separately — any single dimension crossing its cap is
    /// sufficient to trigger the hard-stop).
    #[test]
    #[ignore] // RED — enable in DELIVER
    fn free_plan_cap_exceeded_iff_any_dimension_at_or_over_cap(model in arb_free_model_with_databases()) {
        let ratios = model.cap_ratios();
        let any_over = ratios.reads >= 1.0
            || ratios.writes >= 1.0
            || ratios.deletes >= 1.0
            || ratios.storage_gb >= 1.0;

        prop_assert_eq!(model.cap_exceeded(), any_over);
    }
}

// ── @property: usage_totals sums database usage and projects ×30 ───────────

proptest! {
    /// @property: `usage_totals().reads` always equals the sum of every
    /// database's daily `usage.reads`, projected ×30 — for ANY list of 0-6
    /// databases, not just the worked 3-database examples in slice_02/03.
    #[test]
    fn usage_totals_reads_equals_sum_times_thirty(dbs in prop::collection::vec(arb_database_with_usage(), 0..=6)) {
        let mut model = AppModel::default();
        model.authed = true;
        model.databases = dbs.clone();

        let totals = model.usage_totals();
        let expected: u64 = dbs.iter().map(|d| d.usage.reads).sum::<u64>() * 30;

        prop_assert_eq!(totals.reads, expected);
    }
}

// ── @property: cap_ratios is exactly usage_totals / FREE_CAPS ──────────────

proptest! {
    /// @property: for ANY usage, `cap_ratios().reads` equals
    /// `usage_totals().reads as f64 / FREE_CAPS.reads as f64` — the single-
    /// sourced formula AC-102-01 depends on, checked generatively rather
    /// than pinned to one worked example.
    #[test]
    fn cap_ratio_reads_matches_usage_totals_over_free_caps(model in arb_free_model_with_databases()) {
        let ratios = model.cap_ratios();
        let totals = model.usage_totals();
        let expected = totals.reads as f64 / data::FREE_CAPS.reads as f64;

        prop_assert!(
            (ratios.reads - expected).abs() < 1e-9,
            "cap_ratios().reads ({}) must equal usage_totals().reads / FREE_CAPS.reads ({})",
            ratios.reads, expected
        );
    }
}

// ── @property: SetCard always replaces, never accumulates (AC-105-04) ──────

proptest! {
    /// @property: for ANY sequence of two `SetCard` dispatches, the final
    /// `subscription.card` equals the SECOND card exactly — never a merge,
    /// never the first card, regardless of how many cards were on file
    /// before. Generalizes the single worked example (Mastercard → Visa)
    /// in slice_04 to arbitrary card pairs.
    #[test]
    #[ignore] // RED — enable in DELIVER
    fn set_card_always_replaces_never_accumulates(first in arb_card(), second in arb_card()) {
        let mut model = AppModel::default();
        model.authed = true;

        update(&mut model, Msg::SetCard(first));
        update(&mut model, Msg::SetCard(second.clone()));

        prop_assert_eq!(model.subscription.card, Some(second));
    }
}

// ── @property: SetPlan is idempotent ────────────────────────────────────────

proptest! {
    /// @property: dispatching `SetPlan(p)` twice in a row yields the same
    /// model state as dispatching it once — no double-charge/double-toggle
    /// side effect hiding in `update()`.
    #[test]
    #[ignore] // RED — enable in DELIVER
    fn set_plan_is_idempotent(plan in arb_plan()) {
        let mut once = AppModel::default();
        once.authed = true;
        update(&mut once, Msg::SetPlan(plan.clone()));

        let mut twice = AppModel::default();
        twice.authed = true;
        update(&mut twice, Msg::SetPlan(plan.clone()));
        update(&mut twice, Msg::SetPlan(plan));

        prop_assert_eq!(once.subscription.plan, twice.subscription.plan);
    }
}

// ── Negative-testing workflow (Hebert ch.6): relax the "has databases"
//    assumption and confirm the property still holds on the empty case ─────

proptest! {
    /// Negative-testing companion to `read_only_iff_status_not_active`:
    /// deliberately relax the "at least one database" assumption implicit
    /// in most fixtures — an account with ZERO databases must still derive
    /// a well-defined, non-ambiguous status (Active, since zero usage never
    /// exceeds a cap), not a vacuously-true or panicking edge case.
    #[test]
    #[ignore] // RED — enable in DELIVER
    fn zero_databases_never_falsely_trip_cap_exceeded(plan in arb_plan(), payment_failure in any::<bool>()) {
        let mut model = AppModel::default();
        model.authed = true;
        model.subscription.plan = plan;
        model.subscription.payment_failure = payment_failure;
        // model.databases intentionally left empty.

        prop_assert!(!model.cap_exceeded(), "zero usage must never trip cap_exceeded()");
    }
}

// ── Pinned canonical example (domain-readable, alongside the properties) ───

/// Pinned example (Mandate 9: "Pinned example preserves a domain-readable
/// canonical case for reviewers") — the exact AC-108-06 worked case:
/// Free plan, one dimension at exactly 100% (deletes), everything else
/// under cap. `cap_exceeded()` must be true from this single dimension
/// alone, not require all 4 dimensions to be over.
#[test]
#[ignore] // RED — enable in DELIVER
fn pinned_example_single_dimension_at_cap_is_sufficient() {
    let mut model = AppModel::default();
    model.authed = true;
    model.subscription.plan = Plan::Free;
    model.databases = vec![common::database_with_daily_usage("db-1", 0, 0, 3_334, 0.0)]; // ~100,020 monthly deletes

    assert!(model.cap_exceeded(), "AC-108-06 pinned example: one dimension at cap is sufficient");
}
