//! Common test infrastructure — security-rules-operations acceptance tests
//! (Slice 01, US-01, ADR-035).
//!
//! Reuses `SecurityRulesAdminContext` from `security-rules`' own fixture
//! module via a path import (per the established `security_rules_write_path`/
//! `security_rules_collection_group_rules` reuse-chain precedent — this
//! feature extends the SAME admin surface, `POST .../access_rules`, no new
//! driving-port context class needed). `access_rules`/`define_access_rule`
//! and their own test file are never touched — only imported, read-only,
//! from here.
//!
//! `access_rule_history` is a NEW, additive table (ADR-035 § Decision —
//! Schema), fused into the existing `upsert_access_rule` transaction. The
//! helper below reads it directly via raw SQL — the real domain-observable
//! proof this slice's ACs require — ordered by `id` (the authoritative
//! ordering key, not `captured_at`, per ADR-035).

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules/common/mod.rs"]
mod security_rules_common;
pub use security_rules_common::{
    assert_state_delta, mint_client_identity_token, now_unix, set_to, unchanged, universe,
    SecurityRulesAdminContext, SecurityRulesFullContext,
};

/// One `access_rule_history` row, read directly via SQL — the port-exposed
/// observable surface for AC-17-156/157/158 (a driven-port boundary read,
/// never an internal struct field).
#[derive(Debug, Clone)]
pub struct AccessRuleHistoryRow {
    pub id: i64,
    pub condition_source: String,
    pub actor_account_id: uuid::Uuid,
    pub captured_at: chrono::DateTime<chrono::Utc>,
}

/// Read every `access_rule_history` row for `(project_id, collection_path)`,
/// ordered `id ASC` (oldest first — the authoritative chronological
/// ordering key, ADR-035 § Decision — Schema).
pub async fn access_rule_history_rows(
    ctx: &SecurityRulesAdminContext,
    project_id: &str,
    collection_path: &str,
) -> Vec<AccessRuleHistoryRow> {
    let rows: Vec<(i64, String, uuid::Uuid, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, condition_source, actor_account_id, captured_at \
         FROM access_rule_history WHERE project_id = $1 AND collection_path = $2 \
         ORDER BY id ASC",
    )
    .bind(project_id)
    .bind(collection_path)
    .fetch_all(&ctx.pool)
    .await
    .expect("query access_rule_history");

    rows.into_iter()
        .map(
            |(id, condition_source, actor_account_id, captured_at)| AccessRuleHistoryRow {
                id,
                condition_source,
                actor_account_id,
                captured_at,
            },
        )
        .collect()
}
