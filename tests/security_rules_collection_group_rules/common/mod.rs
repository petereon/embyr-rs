//! Common test infrastructure — security-rules-collection-group-rules
//! acceptance tests (Slice 01, US-01, ADR-032).
//!
//! Reuses `SecurityRulesAdminContext`/`assert_state_delta`/`set_to`/
//! `unchanged` from `security-rules-write-path`'s own fixture module via a
//! path import (this feature's Slice 01 is admin-API-only, the same
//! "Activity A" shape write-path's own Slice 01 was — no new driving-port
//! context class needed, mirroring ADR-032 § Enforcement, "No new driven
//! port"). `tests/security_rules/`, `tests/security_rules_write_path/`, and
//! `tests/security_rules_query_path/` files are never touched — only
//! imported, read-only, from here.
//!
//! `group_access_rules` is a NEW, disjoint table (ADR-032 § Decision —
//! Schema). Seed/read helpers below write/read it directly via raw SQL
//! (bypassing the define endpoint and the adapter), mirroring
//! `seed_write_access_rule`/`write_access_rule_condition_source`'s identical
//! shape for `write_access_rules`.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_write_path/common/mod.rs"]
mod security_rules_write_path_common;
pub use security_rules_write_path_common::{
    assert_state_delta, seed_write_access_rule, set_to, unchanged,
    write_access_rule_condition_source, SecurityRulesAdminContext,
};

/// Directly seed a `group_access_rules` row (bypassing the define endpoint
/// and the adapter) — used by AC-17-78's redefine-precondition setup and
/// AC-17-79's independence setup, mirroring
/// `seed_write_access_rule`'s identical bypass-the-endpoint allowance.
pub async fn seed_group_access_rule(
    ctx: &SecurityRulesAdminContext,
    project_id: &str,
    collection_id: &str,
    condition_source: &str,
) {
    sqlx::query(
        "INSERT INTO group_access_rules (project_id, collection_id, condition_source) \
         VALUES ($1, $2, $3)",
    )
    .bind(project_id)
    .bind(collection_id)
    .bind(condition_source)
    .execute(&ctx.pool)
    .await
    .expect("insert group_access_rules row");
}

/// Read the currently-stored `condition_source` for `(project_id,
/// collection_id)` in `group_access_rules` — the state-delta observable for
/// AC-17-77/78/79, structurally disjoint from
/// `SecurityRulesAdminContext::access_rule_condition_source`/
/// `write_access_rule_condition_source` (a DIFFERENT table, `FROM
/// group_access_rules` only).
pub async fn group_access_rule_condition_source(
    ctx: &SecurityRulesAdminContext,
    project_id: &str,
    collection_id: &str,
) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT condition_source FROM group_access_rules WHERE project_id = $1 AND collection_id = $2",
    )
    .bind(project_id)
    .bind(collection_id)
    .fetch_optional(&ctx.pool)
    .await
    .unwrap_or(None)
}
