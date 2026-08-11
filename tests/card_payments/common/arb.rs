// SCAFFOLD: true
//! proptest strategies for card-payments billing domain types.
//!
//! Used by tea_state_scenarios.rs (layer 1-2 PBT full, per
//! nw-test-design-mandates Mandate 9) and any slice test wanting a
//! generated fixture. Mirrors `tests/user_admin_ui/common/arb.rs`'s style.

#![allow(dead_code)]

use embyr_admin_ui::model::{
    AppModel, Card, CardBrand, Database, DbBackendMode, DbId, DbStatus, Invoice, InvoiceId,
    InvoiceStatus, Plan, Subscription, UsageStats,
};
use proptest::prelude::*;
use uuid::Uuid;

// ── Enum strategies ───────────────────────────────────────────────────────

pub fn arb_plan() -> impl Strategy<Value = Plan> {
    prop_oneof![Just(Plan::Free), Just(Plan::Pro)]
}

pub fn arb_card_brand() -> impl Strategy<Value = CardBrand> {
    prop_oneof![
        Just(CardBrand::Visa),
        Just(CardBrand::Mastercard),
        Just(CardBrand::Amex),
        Just(CardBrand::Discover),
        Just(CardBrand::Unknown),
    ]
}

pub fn arb_invoice_status() -> impl Strategy<Value = InvoiceStatus> {
    prop_oneof![Just(InvoiceStatus::Upcoming), Just(InvoiceStatus::Paid)]
}

// ── Newtype strategies ─────────────────────────────────────────────────────

prop_compose! {
    pub fn arb_invoice_id()(bytes in any::<[u8; 16]>()) -> InvoiceId {
        InvoiceId(Uuid::from_bytes(bytes))
    }
}

// ── Domain struct strategies ──────────────────────────────────────────────

prop_compose! {
    pub fn arb_card()(
        brand in arb_card_brand(),
        last4 in "[0-9]{4}",
        exp_month in 1u8..=12,
        exp_year in 2026u16..=2032,
    ) -> Card {
        Card { brand, last4, exp_month, exp_year }
    }
}

// Daily usage rates, bounded well above a single dimension's Free cap so
// that ×30-projected properties can exercise both under- and over-cap
// territory (`FREE_CAPS.reads` = 2,000,000/mo ≈ 66,667/day).
prop_compose! {
    pub fn arb_usage_stats()(
        reads in 0u64..200_000,
        writes in 0u64..50_000,
        deletes in 0u64..10_000,
        storage_gb in 0.0f64..5.0,
    ) -> UsageStats {
        UsageStats { reads, writes, deletes, storage_gb }
    }
}

prop_compose! {
    pub fn arb_database_with_usage()(
        name in "[a-z][a-z0-9-]{2,19}",
        usage in arb_usage_stats(),
    ) -> Database {
        Database {
            id: DbId(Uuid::new_v4()),
            name,
            status: DbStatus::Active,
            backend_mode: DbBackendMode::DirectPg,
            logging_enabled: false,
            log_retention: None,
            created_at: None,
            usage,
        }
    }
}

prop_compose! {
    pub fn arb_subscription()(
        plan in arb_plan(),
        payment_failure in any::<bool>(),
        has_card in any::<bool>(),
        card in arb_card(),
    ) -> Subscription {
        Subscription {
            plan,
            stripe_customer_id: "cus_test".to_string(),
            current_period_end: None,
            card: if has_card { Some(card) } else { None },
            payment_failure,
        }
    }
}

prop_compose! {
    pub fn arb_invoice()(
        id in arb_invoice_id(),
        period_label in "[A-Z][a-z]{2} 20[2-3][0-9]",
        base in 0.0f64..200.0,
        overage in 0.0f64..50.0,
        status in arb_invoice_status(),
    ) -> Invoice {
        Invoice {
            id,
            date: None,
            period_label,
            base,
            overage,
            total: base + overage,
            status,
        }
    }
}

// ── AppModel strategies ───────────────────────────────────────────────────

// Generate an authenticated AppModel on an arbitrary plan with 0–4
// databases contributing arbitrary daily usage.
prop_compose! {
    pub fn arb_model_with_databases()(
        plan in arb_plan(),
        payment_failure in any::<bool>(),
        dbs in prop::collection::vec(arb_database_with_usage(), 0..=4),
    ) -> AppModel {
        let mut model = AppModel::default();
        model.authed = true;
        model.subscription.plan = plan;
        model.subscription.payment_failure = payment_failure;
        model.databases = dbs;
        model
    }
}

// Generate a Free-plan model only (US-102/US-108 cap-exceeded properties
// are meaningless on Pro, per D-6/AC-108-06: `cap_exceeded()` requires
// `plan == Free`).
prop_compose! {
    pub fn arb_free_model_with_databases()(
        dbs in prop::collection::vec(arb_database_with_usage(), 0..=4),
    ) -> AppModel {
        let mut model = AppModel::default();
        model.authed = true;
        model.subscription.plan = Plan::Free;
        model.databases = dbs;
        model
    }
}
