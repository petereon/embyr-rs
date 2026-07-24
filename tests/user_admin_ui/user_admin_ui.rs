// SCAFFOLD: true
//! Integration test binary entry point — user-admin-ui acceptance tests.
//!
//! Registers all TEA state scenario modules and per-slice scenario modules.
//! All tests carry `#[ignore]` (RED) — DELIVER enables them one at a time.
//!
//! Registered in `crates/embyr-admin-ui/Cargo.toml` as:
//!   [[test]]
//!   name = "user_admin_ui_tea_state"
//!   path = "../../tests/user_admin_ui/user_admin_ui.rs"
//!
//! Run with: `cargo test --package embyr-admin-ui --test user_admin_ui_tea_state`
//! Run ignored: `cargo test --package embyr-admin-ui --test user_admin_ui_tea_state -- --include-ignored`

// Shared test infrastructure and proptest strategies.
#[path = "common/mod.rs"]
mod common;

// TEA state machine scenarios — all US covered via proptest.
#[path = "acceptance/tea_state_scenarios.rs"]
mod tea_state_scenarios;

// Per-slice focused scenarios.
#[path = "acceptance/slice_01_auth_dashboard.rs"]
mod slice_01_auth_dashboard;

#[path = "acceptance/slice_02_database_management.rs"]
mod slice_02_database_management;

#[path = "acceptance/slice_03_connections_sdk_keys.rs"]
mod slice_03_connections_sdk_keys;

#[path = "acceptance/slice_04_identity_admin_keys.rs"]
mod slice_04_identity_admin_keys;

#[path = "acceptance/slice_05_billing_logs.rs"]
mod slice_05_billing_logs;

#[path = "acceptance/slice_06_settings.rs"]
mod slice_06_settings;
