// SCAFFOLD: true
//! Integration test binary entry point — card-payments acceptance tests.
//!
//! Registers the TEA state scenario module and per-slice scenario modules.
//! All tests carry `#[ignore]` (RED) — DELIVER enables them one at a time.
//! Mirrors `tests/user_admin_ui/user_admin_ui.rs`'s wiring pattern exactly.
//!
//! Registered in `crates/embyr-admin-ui/Cargo.toml` as:
//!   [[test]]
//!   name = "card_payments_tea_state"
//!   path = "../../tests/card_payments/card_payments.rs"
//!
//! Run with: `cargo test --package embyr-admin-ui --test card_payments_tea_state`
//! Run ignored: `cargo test --package embyr-admin-ui --test card_payments_tea_state -- --include-ignored`
//! Run ignored only: `cargo test --package embyr-admin-ui --test card_payments_tea_state -- --ignored`

// Shared test infrastructure and proptest strategies.
#[path = "common/mod.rs"]
mod common;

// TEA state machine scenarios — status-derivation properties (layer 1-2 PBT).
#[path = "acceptance/tea_state_scenarios.rs"]
mod tea_state_scenarios;

// Per-slice focused scenarios.
#[path = "acceptance/slice_02_cap_usage_visibility.rs"]
mod slice_02_cap_usage_visibility;

#[path = "acceptance/slice_03_invoice_forecast_usage_breakdown.rs"]
mod slice_03_invoice_forecast_usage_breakdown;

#[path = "acceptance/slice_04_payment_method_capture.rs"]
mod slice_04_payment_method_capture;

#[path = "acceptance/slice_05_plan_change.rs"]
mod slice_05_plan_change;

#[path = "acceptance/slice_06_invoice_history.rs"]
mod slice_06_invoice_history;

#[path = "acceptance/slice_07_suspension_banner.rs"]
mod slice_07_suspension_banner;

#[path = "acceptance/slice_08_test_mode_simulator.rs"]
mod slice_08_test_mode_simulator;
