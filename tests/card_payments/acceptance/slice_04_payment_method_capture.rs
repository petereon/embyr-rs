// SCAFFOLD: true
//! Slice 04 — Payment Method Capture acceptance scenarios.
//! Story: US-105 (Add or Update My Payment Method)
//!
//! All tests are #[ignore] (RED). Enable one at a time in DELIVER.
//!
//! Out of model-layer test scope (view-rendering concerns): AC-105-01
//! (Stripe-Elements-styled `.stripe-el` markup), AC-105-06 (PCI reassurance
//! copy + "Secured by Stripe" footer — always-visible static text, not
//! derived from model state).

use embyr_admin_ui::data;
use embyr_admin_ui::model::{CardBrand, Plan};
use embyr_admin_ui::msg::Msg;
use embyr_admin_ui::update::update;

#[path = "../common/mod.rs"]
mod common;
use common::{model_on_plan, model_with_card, sample_mastercard, sample_visa_card};

// ─────────────────────────────────────────────────────────────────────────────
// AC-105-02: brand auto-detected from the card number prefix.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-105-02: a number starting with "4" is detected as Visa.
#[test]
fn brand_detected_as_visa_from_number_prefix() {
    // Given/When: Chris enters "4242 4242 4242 4242" in the Card modal.
    let brand = data::detect_card_brand("4242 4242 4242 4242");

    // Then: the modal displays the Visa brand icon.
    assert_eq!(brand, CardBrand::Visa, "AC-105-02: '4...' prefix must detect Visa");
}

/// AC-105-02: a number starting with "5" is detected as Mastercard, once
/// the first 6 digits are typed (domain example 2).
#[test]
fn brand_detected_as_mastercard_from_number_prefix() {
    // Given/When: Chris finishes typing the first 6 digits of a number
    // starting with "5".
    let brand = data::detect_card_brand("550000");

    // Then: the modal displays the Mastercard brand icon.
    assert_eq!(brand, CardBrand::Mastercard, "AC-105-02: '5...' prefix must detect Mastercard");
}

/// Mutation-killing: a number starting with "3" is detected as Amex —
/// Visa/Mastercard/Unknown coverage above does not exercise this match arm.
#[test]
fn brand_detected_as_amex_from_number_prefix() {
    let brand = data::detect_card_brand("340000000000009");
    assert_eq!(brand, CardBrand::Amex, "AC-105-02: '3...' prefix must detect Amex");
}

/// Mutation-killing: a number starting with "6" is detected as Discover —
/// Visa/Mastercard/Unknown coverage above does not exercise this match arm.
#[test]
fn brand_detected_as_discover_from_number_prefix() {
    let brand = data::detect_card_brand("6011000000000004");
    assert_eq!(brand, CardBrand::Discover, "AC-105-02: '6...' prefix must detect Discover");
}

/// AC-105-02, Error/Boundary: a prefix that matches no known brand detects
/// as Unknown, not a false-positive brand.
#[test]
fn brand_unknown_for_unrecognized_prefix() {
    // Given/When: an unrecognized prefix is entered.
    let brand = data::detect_card_brand("9999999999999999");

    // Then: no brand icon renders (Unknown, the safe default).
    assert_eq!(brand, CardBrand::Unknown, "AC-105-02: unrecognized prefix must default to Unknown");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-105-03/04: submitting a valid card updates/replaces the card on file.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-105-03: first-time card capture completes successfully — submitting
/// updates `model.subscription.card`.
#[test]
fn submitting_valid_card_updates_subscription_card() {
    // Given: Chris Okafor's account has no card on file.
    let mut model = model_on_plan(Plan::Free);
    assert!(model.subscription.card.is_none(), "precondition: no card on file");

    // When: Chris enters card number 4242 4242 4242 4242, expiry 08/27,
    // CVC 123, and submits.
    update(&mut model, Msg::SetCard(sample_visa_card()));

    // Then: the Payment Method card shows "Visa •••• 4242 · Expires
    // 08/2027 · On file".
    assert_eq!(
        model.subscription.card,
        Some(sample_visa_card()),
        "AC-105-03: submitting a valid card must update model.subscription.card"
    );
}

/// AC-105-04: updating an existing card replaces it, not duplicates it —
/// no trace of the old Mastercard remains.
#[test]
fn submitting_replaces_not_appends_existing_card() {
    // Given: Dana Whitfield's account has a Mastercard ending in 9012 on
    // file.
    let mut model = model_with_card(Plan::Pro, sample_mastercard());
    assert_eq!(model.subscription.card, Some(sample_mastercard()), "precondition: Mastercard on file");

    // When: Dana opens the Card modal, enters a new Visa card, and submits.
    update(&mut model, Msg::SetCard(sample_visa_card()));

    // Then: the Payment Method card shows only the new Visa card, with no
    // trace of the Mastercard.
    assert_eq!(
        model.subscription.card,
        Some(sample_visa_card()),
        "AC-105-04: the new card must replace, not append to, the existing one"
    );
    assert_ne!(
        model.subscription.card,
        Some(sample_mastercard()),
        "AC-105-04: the old Mastercard must not remain"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-105-05: incomplete/invalid card number blocks submission.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-105-05: a 12-digit (incomplete) card number fails completeness
/// validation — Save must stay disabled.
#[test]
fn incomplete_twelve_digit_card_number_is_not_complete() {
    // Given/When: Priya enters only 12 digits and attempts to submit.
    let complete = data::card_number_is_complete("424242424242");

    // Then: an inline validation message appears and Save stays disabled.
    assert!(!complete, "AC-105-05: 12 digits must fail completeness validation");
}

/// AC-105-05, happy counterpart: a full 16-digit number (formatted with
/// spaces, matching the DISCUSS domain example verbatim) passes validation.
#[test]
fn formatted_sixteen_digit_card_number_is_complete() {
    // Given/When: Chris enters "4242 4242 4242 4242" (space-formatted, the
    // literal domain-example format).
    let complete = data::card_number_is_complete("4242 4242 4242 4242");

    // Then: the number is recognized as complete — Save is enabled.
    assert!(complete, "AC-105-05: a full, space-formatted 16-digit number must be complete");
}

/// AC-105-05, Error/Boundary: an empty card number is never complete.
#[test]
fn empty_card_number_is_not_complete() {
    let complete = data::card_number_is_complete("");
    assert!(!complete, "AC-105-05: empty input must never validate as complete");
}
