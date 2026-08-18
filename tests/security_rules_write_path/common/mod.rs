//! Common test infrastructure — security-rules-write-path acceptance tests
//! (Slice 01, ADR-030).
//!
//! Reuses `SecurityRulesAdminContext` from `security-rules`' own fixture
//! module via a path import (per the DISTILL/DELIVER dispatch instructions
//! — this feature extends the SAME admin surface, no new driving-port
//! context class needed). `access_rules`, its adapter methods, and its own
//! test file are never touched — only imported, read-only, from here.
//!
//! `write_access_rules` is a NEW, independent table (ADR-030 § Decision —
//! Storage Shape). Helpers below query/seed it directly via raw SQL against
//! `ctx.pool` (public field on `SecurityRulesAdminContext`) — mirroring how
//! `security-rules`' own fixture reads `access_rules` directly rather than
//! through the `SystemDb` adapter, since no test in THIS slice drives
//! `SystemDb::get_write_access_rule` (that method has no caller until
//! Slice 02-04's write-time evaluation call sites — Iron Law: no production
//! code without a failing test requiring it).

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules/common/mod.rs"]
mod security_rules_common;
pub use security_rules_common::{
    assert_state_delta, set_to, unchanged, universe, SecurityRulesAdminContext,
};

/// Directly seed a `write_access_rules` row (bypassing the define
/// endpoint) — used by AC-17-21's redefine-precondition setup, mirroring
/// `SecurityRulesAdminContext::seed_access_rule`'s identical
/// bypass-the-endpoint allowance for the read-rule table.
pub async fn seed_write_access_rule(
    ctx: &SecurityRulesAdminContext,
    project_id: &str,
    collection_path: &str,
    condition_source: &str,
) {
    sqlx::query(
        "INSERT INTO write_access_rules (project_id, collection_path, condition_source) \
         VALUES ($1, $2, $3)",
    )
    .bind(project_id)
    .bind(collection_path)
    .bind(condition_source)
    .execute(&ctx.pool)
    .await
    .expect("insert write_access_rules row");
}

/// Read the currently-stored `condition_source` for `(project_id,
/// collection_path)` in `write_access_rules` — the state-delta observable
/// for AC-17-20/21, structurally disjoint from
/// `SecurityRulesAdminContext::access_rule_condition_source` (a DIFFERENT
/// table, `FROM write_access_rules` only).
pub async fn write_access_rule_condition_source(
    ctx: &SecurityRulesAdminContext,
    project_id: &str,
    collection_path: &str,
) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT condition_source FROM write_access_rules WHERE project_id = $1 AND collection_path = $2",
    )
    .bind(project_id)
    .bind(collection_path)
    .fetch_optional(&ctx.pool)
    .await
    .unwrap_or(None)
}
