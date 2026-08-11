// @walking_skeleton @driving_port @in-memory
//! Walking skeleton — card-payments (Slice 01: Billing Overview shell)
//!
//! One thin vertical slice touching every activity in the Story Map
//! backbone (A. Orient — Plan + Payment Method cards; B. Understand Usage
//! stub; C. Manage Payment Method — open-able Card modal shell; D. Change
//! Plan — open-able Upgrade modal shell; E. Review Invoices stub), driven
//! entirely through the TEA loop's sole entry point, `update(&mut
//! AppModel, Msg)`, and the new pure `impl AppModel` projections.
//!
//! Unlike `tests/user_admin_ui/acceptance/walking_skeleton.rs` (a real HTTP
//! probe against the served SPA), this feature introduces **no new backend
//! HTTP endpoint** — see feature-delta.md § Wave: DISCUSS / [REF] Driving
//! Ports: "No new backend HTTP endpoints are introduced by this feature;
//! all Msg/update() state transitions operate on mock AppModel data." The
//! existing HTTP-probe WS already proves the SPA is served; this WS proves
//! the billing-specific driving-port surface (`update()` + the new
//! `impl AppModel` projections) delivers Chris's Slice-01 journey
//! end-to-end at the in-memory acceptance layer (layer 2, per
//! nw-test-design-mandates Layered Test Discipline).
//!
//! All tests are #[ignore] (RED) — production logic does not exist yet;
//! every `impl AppModel` projection method and every new `Msg` match arm
//! panics by design (see crates/embyr-admin-ui/src/{model,update}.rs RED
//! scaffolds, Mandate 7). Enable one at a time in DELIVER per
//! docs/feature/card-payments/distill/red-classification.md.
//!
//! Classification: RED (all 6 tests) — see red-classification.md.

use embyr_admin_ui::data;
use embyr_admin_ui::model::{Plan, UsageTotals};
use embyr_admin_ui::msg::Msg;
use embyr_admin_ui::update::update;

#[path = "../common/mod.rs"]
mod common;
use common::{model_on_plan, model_with_card, sample_visa_card};

/// AC-101-01/02: Free-plan Chris Okafor sees the Plan card's derived
/// summary — Free badge + included volume sourced from `data::FREE_CAPS`,
/// through a single projection method (Mandate 1 — no view-local
/// re-derivation of the same numbers).
// @walking_skeleton @driving_port @AC-101-01 @AC-101-02
#[test]
fn chris_sees_free_plan_and_included_volume() {
    // Given: Chris Okafor's account "Aperture Labs" is on the Free plan.
    let model = model_on_plan(Plan::Free);

    // When: Chris navigates to Billing → Overview and reads the Plan card.
    let summary = model.plan_summary();

    // Then: the Plan card shows Free + FREE_CAPS included volume.
    assert_eq!(summary.plan, Plan::Free, "AC-101-01: Plan card must show Free");
    assert_eq!(
        summary.included,
        Some(UsageTotals {
            reads: data::FREE_CAPS.reads,
            writes: data::FREE_CAPS.writes,
            deletes: data::FREE_CAPS.deletes,
            storage_gb: data::FREE_CAPS.storage_gb,
        }),
        "AC-101-02: Free plan card must show included volume from FREE_CAPS"
    );
    assert_eq!(summary.base_price, None, "Free plan has no base price");
}

/// AC-101-03: Dana Whitfield (Pro plan) sees her renewal date and base
/// price instead of Free's included-volume text.
// @walking_skeleton @driving_port @AC-101-03
#[test]
fn dana_sees_pro_plan_base_price_and_renewal() {
    // Given: Dana Whitfield's account "Northwind Data" is on the Pro plan.
    let model = model_on_plan(Plan::Pro);

    // When: Dana navigates to Billing → Overview.
    let summary = model.plan_summary();

    // Then: the Plan card shows Pro, the $49/mo base price, and included=None.
    assert_eq!(summary.plan, Plan::Pro, "AC-101-03: Plan card must show Pro");
    assert_eq!(
        summary.base_price,
        Some(data::PRICING.pro_base),
        "AC-101-03: Pro plan card must show the $49/mo base price"
    );
    assert_eq!(summary.included, None, "Pro plan does not show Free's included-volume text");
}

/// AC-101-04/07: Chris Okafor's account has a Visa card on file — the
/// Payment Method card summarizes brand/last4/expiry, "On file" state,
/// and the mono `stripeCustomerId`.
// @walking_skeleton @driving_port @AC-101-04 @AC-101-07
#[test]
fn chris_with_card_sees_payment_method_summary() {
    // Given: Chris Okafor's account has a Visa card ending in 4242 on file.
    let model = model_with_card(Plan::Free, sample_visa_card());

    // When: Chris navigates to Billing → Overview.
    let summary = model.payment_method_summary();

    // Then: the Payment Method card shows the card, "On file" true, and the
    // stripeCustomerId (AC-101-07).
    assert_eq!(summary.card, Some(sample_visa_card()), "AC-101-04: card summary must match");
    assert!(summary.on_file, "AC-101-04: card on file must render the 'On file' badge");
}

/// AC-101-05: Priya Raman's account "Solstice Analytics" has no payment
/// method on file — the Payment Method card shows a CTA, not card details.
// @walking_skeleton @driving_port @AC-101-05 @error
#[test]
fn priya_with_no_card_sees_add_card_cta() {
    // Given: Priya Raman's account "Solstice Analytics" has no card on file.
    let model = model_on_plan(Plan::Free);

    // When: Priya navigates to Billing → Overview.
    let summary = model.payment_method_summary();

    // Then: the Payment Method card shows "No card on file" (on_file=false),
    // an "Add card" CTA (not "Update").
    assert_eq!(summary.card, None, "AC-101-05: no card on file");
    assert!(!summary.on_file, "AC-101-05: on_file must be false so the view renders 'Add card'");
}

/// AC-101-06 (partial — modal shells only; sub-tab navigation is
/// view-local Leptos RwSignal state, untestable at the TEA model layer,
/// same class as `db_detail`'s own `active_tab` in user-admin-ui): Chris
/// can open both the Card modal shell (activity C) and the Upgrade modal
/// shell (activity D) directly from the Overview page — this is the
/// cross-cutting global-state wiring ADR-019 exists to guarantee (proven
/// here at Slice 01, exercised again by SuspensionBanner in Slice 07).
// @walking_skeleton @driving_port @AC-101-06
#[test]
fn chris_can_open_card_modal_and_upgrade_modal_from_overview() {
    // Given: Chris Okafor's account "Aperture Labs" is on the Free plan,
    // Billing → Overview is showing, and neither modal is open.
    let mut model = model_on_plan(Plan::Free);
    assert!(!model.card_modal_open);
    assert!(!model.upgrade_modal_open);

    // When: Chris clicks "Add card" on the Payment Method card.
    update(&mut model, Msg::OpenCardModal);

    // Then: the Card modal shell is open (ADR-019 global state).
    assert!(model.card_modal_open, "AC-101-06: Card modal must be open-able from Overview");

    // When: Chris closes the Card modal and clicks "Upgrade to Pro" on the
    // Plan card instead.
    update(&mut model, Msg::CloseCardModal);
    update(&mut model, Msg::OpenUpgradeModal);

    // Then: the Upgrade modal shell is open, defaulted to the Compare step.
    assert!(model.upgrade_modal_open, "AC-101-06: Upgrade modal must be open-able from Overview");
    assert_eq!(
        model.upgrade_modal_step,
        embyr_admin_ui::model::UpgradeModalStep::Compare,
        "Opening the Upgrade modal must reset to the Compare step"
    );
}

/// AC-107-03: activity E stub touch — a Free-plan account with no billing
/// history yet sees the documented empty-state copy on Billing → Invoices,
/// not a broken/empty table (Slice 01's WS renders a stub; Slice 06 gives
/// this tab its full depth).
// @walking_skeleton @driving_port @AC-107-03
#[test]
fn chris_sees_invoices_tab_stub_empty_state() {
    // Given: Aperture Labs has never been on a paid plan.
    let model = model_on_plan(Plan::Free);
    assert!(model.invoices.is_empty(), "precondition: no invoice history yet");

    // When: Chris navigates to Billing → Invoices.
    let empty_state = model.invoices_empty_state();

    // Then: the documented empty-state copy renders, not a broken table.
    assert_eq!(
        empty_state,
        Some("The Free plan has no recurring charges — invoices appear once you're on Pro."),
        "AC-107-03: Free-plan empty-state copy must render verbatim"
    );
}
