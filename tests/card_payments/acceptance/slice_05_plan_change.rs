// SCAFFOLD: true
//! Slice 05 — Plan Change acceptance scenarios.
//! Story: US-106 (Change My Plan With Clear Consequences)
//!
//! All tests are #[ignore] (RED). Enable one at a time in DELIVER.

use embyr_admin_ui::data;
use embyr_admin_ui::model::{EffectiveStatus, Plan, UpgradeModalStep};
use embyr_admin_ui::msg::Msg;
use embyr_admin_ui::update::update;

#[path = "../common/mod.rs"]
mod common;
use common::{database_with_daily_usage, model_on_plan};

/// AC-106-01: the compare step renders Free and Pro columns sourced from
/// `data::PLAN_FEATURES` (D-4: Free + Pro tiers only, never a third).
#[test]
fn compare_step_shows_free_and_pro_columns_from_plan_features() {
    // Given: Chris has the Upgrade modal open on the compare step.
    let mut model = model_on_plan(Plan::Free);
    update(&mut model, Msg::OpenUpgradeModal);

    // When: Chris reviews the Free vs Pro comparison.
    let view = model.upgrade_modal_view();

    // Then: a Free column (included volume) and a Pro column (base price)
    // are shown side by side.
    assert_eq!(view.step, UpgradeModalStep::Compare, "precondition: compare step");
    assert_eq!(
        view.free_included.reads, data::PLAN_FEATURES.free_included.reads,
        "AC-106-01: Free column must source included volume from PLAN_FEATURES"
    );
    assert_eq!(
        view.pro_base_price, data::PLAN_FEATURES.pro_base,
        "AC-106-01: Pro column must source the base price from PLAN_FEATURES"
    );
}

/// AC-106-02: confirming Free → Pro updates the plan and closes the modal.
#[test]
fn confirming_upgrade_updates_plan_to_pro_and_closes_modal() {
    // Given: Chris Okafor's account "Aperture Labs" is on the Free plan
    // with the Upgrade modal open.
    let mut model = model_on_plan(Plan::Free);
    update(&mut model, Msg::OpenUpgradeModal);

    // When: Chris confirms upgrading to Pro.
    update(&mut model, Msg::SetPlan(Plan::Pro));
    update(&mut model, Msg::CloseUpgradeModal);

    // Then: the modal closes and the Plan card shows "Pro plan · $49/mo
    // base".
    assert_eq!(model.subscription.plan, Plan::Pro, "AC-106-02: plan must become Pro");
    assert!(!model.upgrade_modal_open, "AC-106-02: modal must close after confirming");
}

/// AC-106-03: selecting downgrade shows the explicit hard-cap warning as a
/// required, unskippable confirmation step.
#[test]
fn selecting_downgrade_shows_mandatory_hard_cap_warning() {
    // Given: Dana Whitfield's account "Northwind Data" is on the Pro plan
    // with the Upgrade modal open.
    let mut model = model_on_plan(Plan::Pro);
    update(&mut model, Msg::OpenUpgradeModal);

    // When: Dana selects "Downgrade to Free".
    update(&mut model, Msg::SetUpgradeModalStep(UpgradeModalStep::ConfirmDowngrade));
    let view = model.upgrade_modal_view();

    // Then: a confirmation step shows the hard-cap warning, mandatory
    // before Dana can confirm.
    assert_eq!(view.step, UpgradeModalStep::ConfirmDowngrade, "precondition: downgrade step selected");
    assert!(
        view.shows_downgrade_warning,
        "AC-106-03: the hard-cap warning must be shown as a required confirmation step"
    );
}

/// AC-106-04: confirming Pro → Free updates the plan and closes the modal.
#[test]
fn confirming_downgrade_updates_plan_to_free_and_closes_modal() {
    // Given: Dana's account is on Pro, downgrade warning acknowledged
    // (chained from the prior scenario's Given + When).
    let mut model = model_on_plan(Plan::Pro);
    update(&mut model, Msg::OpenUpgradeModal);
    update(&mut model, Msg::SetUpgradeModalStep(UpgradeModalStep::ConfirmDowngrade));

    // When: Dana explicitly confirms the downgrade.
    update(&mut model, Msg::SetPlan(Plan::Free));
    update(&mut model, Msg::CloseUpgradeModal);

    // Then: the plan becomes Free and the modal closes.
    assert_eq!(model.subscription.plan, Plan::Free, "AC-106-04: plan must become Free");
    assert!(!model.upgrade_modal_open, "AC-106-04: modal must close after confirming");
}

/// AC-106-05: upgrading from a suspended (`free_cap_exceeded`) state clears
/// the suspension — `effective_status()` re-derives to Active automatically
/// (no separate "clear suspension" message needed, per DESIGN's msg.rs
/// comment).
#[test]
fn upgrading_from_cap_exceeded_clears_suspended_state() {
    // Given: Priya Raman's account "Solstice Analytics" is suspended with
    // status "free_cap_exceeded" (usage well beyond every Free cap).
    let mut model = model_on_plan(Plan::Free);
    model.databases = vec![database_with_daily_usage("db-1", 500_000, 100_000, 20_000, 50.0)];
    assert!(model.cap_exceeded(), "precondition: account is cap-exceeded");
    assert_eq!(model.effective_status(), EffectiveStatus::FreeCapExceeded, "precondition: suspended");

    // When: Priya confirms upgrading to Pro from the Upgrade modal.
    update(&mut model, Msg::SetPlan(Plan::Pro));

    // Then: the account's effective status becomes "active" and the
    // SuspensionBanner no longer renders.
    assert_eq!(
        model.effective_status(),
        EffectiveStatus::Active,
        "AC-106-05: upgrading must clear the free_cap_exceeded suspension"
    );
}

/// AC-106-06: closing the modal without confirming makes no plan changes.
#[test]
fn closing_without_confirming_makes_no_plan_change() {
    // Given: Chris has the Upgrade modal open on the compare step.
    let mut model = model_on_plan(Plan::Free);
    update(&mut model, Msg::OpenUpgradeModal);

    // When: Chris closes the modal without confirming (no SetPlan
    // dispatched).
    update(&mut model, Msg::CloseUpgradeModal);

    // Then: Aperture Labs remains on the Free plan.
    assert_eq!(model.subscription.plan, Plan::Free, "AC-106-06: closing without confirming must not change the plan");
    assert!(!model.upgrade_modal_open, "AC-106-06: modal must be closed");
}

/// AC-106-03, Error/Boundary (negative case): the downgrade warning must
/// NOT show on the ordinary upgrade path (Free → Pro via the compare step)
/// — it is specific to the Pro → Free transition, not shown unconditionally.
#[test]
fn downgrade_warning_does_not_show_on_upgrade_path() {
    // Given: Chris (Free plan) has the Upgrade modal open on the compare
    // step (the ordinary upgrade path, never touching ConfirmDowngrade).
    let mut model = model_on_plan(Plan::Free);
    update(&mut model, Msg::OpenUpgradeModal);

    // When: Chris reviews the compare step (has not selected downgrade).
    let view = model.upgrade_modal_view();

    // Then: the hard-cap warning must not show on this path.
    assert!(
        !view.shows_downgrade_warning,
        "AC-106-03: the downgrade warning must be scoped to the Pro→Free transition only"
    );
}
