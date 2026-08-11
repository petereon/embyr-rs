// SCAFFOLD: true
//! Common test infrastructure for card-payments acceptance tests.
//!
//! Mirrors `tests/user_admin_ui/common/mod.rs`'s pattern exactly: builder
//! functions construct `AppModel` via direct struct literals (NOT via
//! `update()`/`Msg` dispatch), so "Given" preconditions never touch the
//! RED-scaffolded `update()` match arms. Only the "When" step under test
//! dispatches a new `Msg` or calls a new `impl AppModel`/`data` function —
//! that is where each test is expected to panic (RED), per
//! docs/feature/card-payments/distill/red-classification.md.

// Not every slice file calls every helper; some are unused in a given
// compilation unit that includes this module via `#[path]`.
#![allow(dead_code)]

pub mod arb;

use embyr_admin_ui::model::{
    AppModel, Card, CardBrand, Database, DbBackendMode, DbId, DbStatus, Plan, UsageStats,
};
use uuid::Uuid;

/// Build a minimal authenticated AppModel on the given plan, with no card
/// on file and no databases. Baseline "Given" state for most scenarios.
pub fn model_on_plan(plan: Plan) -> AppModel {
    let mut model = AppModel::default();
    model.authed = true;
    model.subscription.plan = plan;
    model
}

/// The canonical Visa card used throughout the DISCUSS UAT scenarios
/// ("4242 4242 4242 4242", expires 08/2027).
pub fn sample_visa_card() -> Card {
    Card {
        brand: CardBrand::Visa,
        last4: "4242".to_string(),
        exp_month: 8,
        exp_year: 2027,
    }
}

/// A Mastercard on file, used by the "updating an existing card" scenarios
/// (US-105 domain example 2 — Dana Whitfield).
pub fn sample_mastercard() -> Card {
    Card {
        brand: CardBrand::Mastercard,
        last4: "9012".to_string(),
        exp_month: 6,
        exp_year: 2026,
    }
}

/// Build a model with a card already on file.
pub fn model_with_card(plan: Plan, card: Card) -> AppModel {
    let mut model = model_on_plan(plan);
    model.subscription.card = Some(card);
    model
}

/// Build a model whose account is suspended for the given reason
/// (`payment_failure` drives `PastDue`; a `Free` plan model with usage
/// beyond `FREE_CAPS`, built via `database_with_daily_usage`, drives
/// `FreeCapExceeded` once `effective_status()` is implemented).
pub fn model_with_payment_failure(plan: Plan) -> AppModel {
    let mut model = model_on_plan(plan);
    model.subscription.payment_failure = true;
    model
}

/// Build a database with the given *daily* reads/writes/deletes and a
/// point-in-time `storage_gb` snapshot (US-102/103/104 cap/usage
/// scenarios). `AppModel::usage_totals()` projects reads/writes/deletes
/// ×30 once implemented — callers pass daily rates here, and compute the
/// expected monthly total in the test body (`daily * 30`) for assertions.
pub fn database_with_daily_usage(
    name: &str,
    reads_per_day: u64,
    writes_per_day: u64,
    deletes_per_day: u64,
    storage_gb: f64,
) -> Database {
    Database {
        id: DbId(Uuid::new_v4()),
        name: name.to_string(),
        status: DbStatus::Active,
        backend_mode: DbBackendMode::DirectPg,
        logging_enabled: false,
        log_retention: None,
        created_at: None,
        usage: UsageStats {
            reads: reads_per_day,
            writes: writes_per_day,
            deletes: deletes_per_day,
            storage_gb,
        },
    }
}
