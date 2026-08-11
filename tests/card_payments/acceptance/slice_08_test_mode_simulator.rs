// SCAFFOLD: true
//! Slice 08 — Test Mode Simulator acceptance scenarios.
//! Story: US-109 (Verify Suspension and Recovery Flows in Test Mode)
//!
//! All tests are #[ignore] (RED). Enable one at a time in DELIVER.
//!
//! Out of model-layer test scope (view-rendering concerns): AC-109-03
//! (dashed border + amber "TEST MODE" badge), AC-109-05 (build-level
//! dev/staging gating — a DESIGN-wave decision per feature-delta.md, not a
//! model-layer concern).

use embyr_admin_ui::model::{EffectiveStatus, Plan};
use embyr_admin_ui::msg::Msg;
use embyr_admin_ui::update::update;

#[path = "../common/mod.rs"]
mod common;
use common::{model_on_plan, sample_visa_card};

/// AC-109-01/02: toggling the TestClockCard to "Payment fails" sets
/// `subscription.payment_failure` and immediately re-derives the
/// SuspensionBanner into its red `past_due` state.
#[test]
fn toggling_to_payment_fails_triggers_red_banner() {
    // Given: Chris is viewing Billing → Overview in a dev/staging build,
    // with a card on file and no failure yet.
    let mut model = model_on_plan(Plan::Pro);
    model.subscription.card = Some(sample_visa_card());
    assert_eq!(model.effective_status(), EffectiveStatus::Active, "precondition: healthy account");

    // When: Chris toggles the TestClockCard segmented control to
    // "Payment fails".
    update(&mut model, Msg::SetPaymentFailure(true));

    // Then: the SuspensionBanner immediately renders in its red "past_due"
    // state, with no page reload (pure re-derivation).
    assert!(model.subscription.payment_failure, "AC-109-01: payment_failure must be set to true");
    assert_eq!(
        model.effective_status(),
        EffectiveStatus::PastDue,
        "AC-109-02: effective_status() must re-derive to PastDue immediately"
    );
}

/// AC-109-01/02: toggling back to "Payment succeeds" clears the banner
/// immediately (chained from the prior scenario's Given + When).
#[test]
fn toggling_back_to_payment_succeeds_clears_banner() {
    // Given: the TestClockCard is currently set to "Payment fails".
    let mut model = model_on_plan(Plan::Pro);
    update(&mut model, Msg::SetPaymentFailure(true));

    // When: Chris toggles it back to "Payment succeeds".
    update(&mut model, Msg::SetPaymentFailure(false));

    // Then: the banner disappears immediately.
    assert!(!model.subscription.payment_failure, "AC-109-01: payment_failure must be set to false");
    assert_eq!(
        model.effective_status(),
        EffectiveStatus::Active,
        "AC-109-02: effective_status() must re-derive to Active immediately"
    );
}

/// AC-109-04, Error/Boundary: toggling the TestClockCard mutates ONLY
/// `subscription.payment_failure` — plan, card, and databases are
/// unaffected (the toggle affects only the currently-viewed account's mock
/// state, per AC-109-04's letter, and no unrelated field as a side effect).
#[test]
fn toggle_mutates_only_payment_failure_field() {
    // Given: Aperture Labs' TestClockCard, with a known plan and card.
    let mut model = model_on_plan(Plan::Free);
    model.subscription.card = Some(sample_visa_card());
    let plan_before = model.subscription.plan.clone();
    let card_before = model.subscription.card.clone();

    // When: Chris toggles Aperture Labs' TestClockCard to "Payment fails".
    update(&mut model, Msg::SetPaymentFailure(true));

    // Then: plan and card are unaffected — only payment_failure changed.
    assert_eq!(model.subscription.plan, plan_before, "AC-109-04: plan must be unaffected by the toggle");
    assert_eq!(model.subscription.card, card_before, "AC-109-04: card must be unaffected by the toggle");
}

/// AC-109-01, Error/Boundary: setting the same value twice is idempotent
/// — no double-toggle/flip-flop bug.
#[test]
fn toggle_to_same_value_twice_is_idempotent() {
    let mut model = model_on_plan(Plan::Pro);

    update(&mut model, Msg::SetPaymentFailure(true));
    update(&mut model, Msg::SetPaymentFailure(true));

    assert!(model.subscription.payment_failure, "AC-109-01: repeated SetPaymentFailure(true) must remain true");
}
