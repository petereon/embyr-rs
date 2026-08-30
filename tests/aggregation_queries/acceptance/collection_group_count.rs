//! aggregation-queries Slice 01 (US-01, ADR-038/039/040) — Collection-Group
//! COUNT Aggregation (Walking Skeleton).
//!
//! Acceptance criteria verified here (feature-delta.md US-01):
//!   AC-01-03: a collection-group COUNT aggregation (`all_descendants=true`)
//!             is governed by `group_access_rules`, never by a same-named
//!             exact-path rule, mirroring RunQuery's own dual-arm
//!             composition (ADR-032) unchanged.
//!
//! Driving port: gRPC :8080 `RunAggregationQuery` (via `SecurityRulesFullContext`
//! + this feature's own `run_count_aggregation` helper — Pillar 3, real
//! Postgres, real gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    mint_client_identity_token, now_unix, run_count_aggregation, seed_document_at_path,
    seed_group_access_rule_full, string_field, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use std::collections::HashMap;

/// Journey:
///   Given: `expenses` has a same-named EXACT-PATH rule that denies
///          everything (`false`) AND an independent, permissive
///          collection-group rule (`true`)
///   And:   two `expenses` documents exist, nested under two different trips
///          (`trips/trip-1/expenses`, `trips/trip-2/expenses`)
///   When:  Alex's app runs a collection-group COUNT aggregation across all
///          `expenses` sub-collections
///   Then:  the aggregation is admitted (governed by the permissive GROUP
///          rule) and the count reflects every matching document across
///          every trip — proving the same-named exact-path DenyAll rule was
///          never consulted
///
/// AC-01-03
///
/// @driving_port @real-io @US-01 @AC-01-03
#[tokio::test]
async fn collection_group_count_is_governed_by_group_rule_not_same_named_exact_path_rule() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg01-cg").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    // Same-named exact-path rule that would deny every request — proves the
    // aggregation is governed by the GROUP rule below, never this one.
    ctx.seed_access_rule("expenses", "false").await;
    seed_group_access_rule_full(&ctx, "expenses", "true").await;

    let mut fields1 = HashMap::new();
    fields1.insert("amount_cents".to_string(), string_field("1200"));
    seed_document_at_path(&ctx, "trips/agg01-trip-1/expenses", "exp-1", fields1).await;

    let mut fields2 = HashMap::new();
    fields2.insert("amount_cents".to_string(), string_field("3400"));
    seed_document_at_path(&ctx, "trips/agg01-trip-2/expenses", "exp-2", fields2).await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let count = run_count_aggregation(&ctx, "expenses", true, &[], Some(&marias_token))
        .await
        .expect(
            "AC-01-03: a collection-group COUNT governed by a permissive group rule must be \
             admitted, never rejected by the same-named DenyAll exact-path rule",
        );

    assert_eq!(
        count, 2,
        "expected both expenses documents across both trips, got {count}"
    );
}
