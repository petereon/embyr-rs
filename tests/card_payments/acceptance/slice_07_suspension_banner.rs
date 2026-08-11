// SCAFFOLD: true
//! Slice 07 — Suspension Banner acceptance scenarios.
//! Story: US-108 (Know Why I'm Locked Out and How to Fix It)
//!
//! All tests are #[ignore] (RED). Enable one at a time in DELIVER.
//!
//! Out of model-layer test scope (view-rendering concern): AC-108-04
//! (banner rendered above `<Show>`-routed content in ShellView) — the
//! cross-cutting *placement* is a Leptos composition concern; what IS
//! testable at this layer, and covered by `banner_visible_regardless_of_
//! active_section` below, is that `suspension_banner_view()` never depends
//! on `model.nav.section` — the actual guarantee AC-108-04 depends on.

use embyr_admin_ui::model::{EffectiveStatus, Plan, Section};
use embyr_admin_ui::msg::Msg;
use embyr_admin_ui::update::update;

#[path = "../common/mod.rs"]
mod common;
use common::{database_with_daily_usage, model_on_plan, model_with_payment_failure};

/// AC-108-02: `free_cap_exceeded` shows an amber banner with an
/// "Upgrade to Pro" CTA.
#[test]
fn cap_exceeded_shows_amber_banner_with_upgrade_cta() {
    // Given: Solstice Analytics' effective status is "free_cap_exceeded".
    let mut model = model_on_plan(Plan::Free);
    model.databases = vec![database_with_daily_usage(
        "db-1", 500_000, 100_000, 20_000, 50.0,
    )];

    // When: Priya views any section of the console.
    let banner = model.suspension_banner_view();

    // Then: an amber banner with an "Upgrade to Pro" CTA is shown.
    let banner = banner.expect("AC-108-01/02: a suspended account must show a banner");
    assert_eq!(
        banner.status,
        EffectiveStatus::FreeCapExceeded,
        "AC-108-02: status must be FreeCapExceeded"
    );
    assert!(
        banner.cta_opens_upgrade_modal,
        "AC-108-02: CTA must open the Upgrade modal"
    );
    assert_eq!(
        banner.message, "You've reached your Free plan limits for this cycle.",
        "AC-108-02: amber banner copy must match verbatim"
    );
}

/// AC-108-03: `past_due` (payment failure) shows a red banner with an
/// "Update payment method" CTA.
#[test]
fn payment_failed_shows_red_banner_with_payment_cta() {
    // Given: Northwind Data's effective status is "past_due".
    let model = model_with_payment_failure(Plan::Pro);

    // When: Dana views any section of the console.
    let banner = model.suspension_banner_view();

    // Then: a red banner with an "Update payment method" CTA is shown.
    let banner = banner.expect("AC-108-01/03: a suspended account must show a banner");
    assert_eq!(
        banner.status,
        EffectiveStatus::PastDue,
        "AC-108-03: status must be PastDue"
    );
    assert!(
        !banner.cta_opens_upgrade_modal,
        "AC-108-03: CTA must open the Card modal, not Upgrade"
    );
    assert_eq!(
        banner.message, "We couldn't process your last payment.",
        "AC-108-03: red banner copy must match verbatim"
    );
}

/// AC-108-01, Error/Boundary: a healthy account sees no banner anywhere —
/// zero visual footprint when not needed.
#[test]
fn healthy_account_shows_no_banner() {
    // Given: Aperture Labs' effective status is "active".
    let model = model_on_plan(Plan::Free);

    // When: Chris views any section of the console.
    let banner = model.suspension_banner_view();

    // Then: no suspension banner renders on any page.
    assert!(
        banner.is_none(),
        "AC-108-01: an active account must never show a banner"
    );
}

/// AC-108-04 (model-layer guarantee): the banner projection is independent
/// of the active `Section` — it must remain visible after navigating from
/// Billing to Databases, since `suspension_banner_view()` reads only
/// `subscription`/`databases`, never `nav.section`.
#[test]
fn banner_visible_regardless_of_active_section() {
    // Given: Solstice Analytics is suspended, viewing Billing.
    let mut model = model_on_plan(Plan::Free);
    model.databases = vec![database_with_daily_usage(
        "db-1", 500_000, 100_000, 20_000, 50.0,
    )];
    update(&mut model, Msg::NavigateTo(Section::Billing));
    assert!(
        model.suspension_banner_view().is_some(),
        "precondition: banner visible on Billing"
    );

    // When: Priya navigates from Billing to the Databases section.
    update(&mut model, Msg::NavigateTo(Section::Databases));

    // Then: the amber banner remains visible above the Databases content.
    assert!(
        model.suspension_banner_view().is_some(),
        "AC-108-04: banner must remain visible after navigating away from Billing"
    );
}

/// AC-108-05: clicking the banner's "Upgrade to Pro" CTA opens the Upgrade
/// modal directly.
#[test]
fn clicking_upgrade_cta_opens_upgrade_modal() {
    // Given: Solstice Analytics is suspended with status
    // "free_cap_exceeded".
    let mut model = model_on_plan(Plan::Free);
    model.databases = vec![database_with_daily_usage(
        "db-1", 500_000, 100_000, 20_000, 50.0,
    )];

    // When: Priya clicks "Upgrade to Pro" on the banner.
    update(&mut model, Msg::OpenUpgradeModal);

    // Then: the Upgrade modal opens directly (US-106) — no intermediate
    // navigation to Billing required.
    assert!(
        model.upgrade_modal_open,
        "AC-108-05: banner CTA must open the Upgrade modal directly"
    );
}

/// AC-108-05: clicking a `past_due` banner's "Update payment method" CTA
/// opens the Card modal instead.
#[test]
fn clicking_payment_cta_opens_card_modal() {
    // Given: an account is suspended with status "past_due".
    let mut model = model_with_payment_failure(Plan::Pro);

    // When: the admin clicks "Update payment method" on the banner.
    update(&mut model, Msg::OpenCardModal);

    // Then: the Card modal opens directly (US-105).
    assert!(
        model.card_modal_open,
        "AC-108-05: past_due banner CTA must open the Card modal directly"
    );
}

/// AC-108-06, Error/Boundary: when BOTH `free_cap_exceeded` and
/// `payment_failure` are true simultaneously, `effective_status()`
/// deterministically prioritizes `FreeCapExceeded` (per DESIGN's literal
/// derivation order: "cap_exceeded() → FreeCapExceeded; else
/// payment_failure → PastDue") — not an unspecified/ambiguous state.
#[test]
fn both_conditions_true_prioritizes_cap_exceeded() {
    // Given: an account is both cap-exceeded AND has a failed payment.
    let mut model = model_with_payment_failure(Plan::Free);
    model.databases = vec![database_with_daily_usage(
        "db-1", 500_000, 100_000, 20_000, 50.0,
    )];
    assert!(
        model.subscription.payment_failure,
        "precondition: payment_failure is true"
    );

    // When: the effective status is derived.
    let status = model.effective_status();

    // Then: FreeCapExceeded wins deterministically — never an ambiguous or
    // PastDue-first result.
    assert_eq!(
        status,
        EffectiveStatus::FreeCapExceeded,
        "AC-108-06: cap_exceeded must take priority over payment_failure"
    );
}
