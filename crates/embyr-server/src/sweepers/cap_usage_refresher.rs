//! `CapUsageRefresher` — background task computing cumulative Free-plan usage
//! vs. cap and enforcing suspension on cap-crossing (ADR-020).
//!
//! Per ADR-020: runs every `EMBYR_CAP_CHECK_INTERVAL_SECS` (default 30s),
//! guarded by a Postgres advisory lock so redundant computation is avoided
//! across instances (not required for correctness — each instance computing
//! independently and reaching the same suspend/no-op decision is safe by
//! construction, since `set_project_status`'s `WHERE status IN
//! ('active','suspended')` clause is idempotent — but avoids wasted Postgres
//! load, consistent with the sweeper shape ADR-020 describes).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use embyr_core::admin::{cap_exceeded, compute_cap_status, UsageDimension};

use crate::adapters::cap_status_cache::CapStatusCache;
use crate::adapters::system_db::SystemDb;
use crate::admin::handlers::lifecycle::{self, LifecycleDeps};

/// FNV-1a hash of the literal string `"embyr_cap_check"`, used as the
/// `pg_try_advisory_lock` key (ADR-020). Computed once, at compile time
/// would be ideal but FNV-1a-64 has no const-fn stdlib implementation
/// available here — computed inline via the same algorithm every cycle
/// (cheap, no allocation, negligible cost relative to the DB round-trip it
/// guards).
fn advisory_lock_key(s: &str) -> i64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET_BASIS;
    for byte in s.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash as i64
}

/// Spawn the `CapUsageRefresher` background task. Returns a `JoinHandle` the
/// composition root should hold for the process lifetime (not awaited —
/// mirrors the existing OBS-05 pool-gauge task in `lib.rs`, which is
/// fire-and-forget for the duration of the process).
pub fn spawn(
    system_db: Arc<SystemDb>,
    cap_status_cache: Arc<CapStatusCache>,
    lifecycle_deps: LifecycleDeps,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        loop {
            tick.tick().await;

            let lock_key = advisory_lock_key("embyr_cap_check");
            let locked: Option<bool> = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
                .bind(lock_key)
                .fetch_one(system_db.pool())
                .await
                .ok();

            if locked != Some(true) {
                // Lock held by another instance (or the probe query itself
                // failed) — skip this cycle, retry next tick. Mirrors the
                // sweeper shape ADR-020 describes: no error surfaced, no
                // panic on a benign "someone else is already running this
                // cycle" outcome.
                continue;
            }

            run_cycle(&system_db, &cap_status_cache, &lifecycle_deps).await;

            let _: Option<bool> = sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
                .bind(lock_key)
                .fetch_one(system_db.pool())
                .await
                .ok();
        }
    })
}

/// One refresh cycle: for every Free-plan account, compute this cycle's
/// cumulative per-dimension usage (summed across all of the account's
/// projects, `daily_project_metrics` joined through `projects.account_id`,
/// filtered to the current UTC calendar month per ADR-020 § Billing Cycle
/// Boundary), write the result into `cap_status_cache`, and — on a
/// cap-crossing (`cap_exceeded`, AC-207-01/02) for an account not already
/// `free_cap_exceeded` — call `lifecycle::suspend_account_projects` (the
/// SAME function US-204's dunning trigger calls, AC-207-04 — D-12 literal
/// reuse) and set `subscriptions.status = 'free_cap_exceeded'`.
///
/// Pro-plan accounts never reach this loop (the account-selection query
/// below filters to `plan = 'free'`, AC-207-03). The transition guard
/// (`status != 'free_cap_exceeded'`) is an optimization only — a repeat
/// suspend call across cycles is harmless because `set_project_status`'s own
/// `WHERE status IN ('active', 'suspended')` clause is idempotent.
async fn run_cycle(
    system_db: &Arc<SystemDb>,
    cap_status_cache: &Arc<CapStatusCache>,
    lifecycle_deps: &LifecycleDeps,
) {
    let accounts: Vec<(uuid::Uuid, String)> = sqlx::query_as(
        "SELECT account_id, status FROM subscriptions \
         WHERE plan = 'free' AND status IN ('active', 'free_cap_exceeded')",
    )
    .fetch_all(system_db.pool())
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "CapUsageRefresher: failed to list Free-plan accounts");
        Vec::new()
    });

    for (account_id, subscription_status) in accounts {
        // AC-206-01: summed across ALL of the account's projects, keyed by
        // account_id. AC-206-05: current UTC calendar month only (ADR-020 §
        // Billing Cycle Boundary — Stripe's current_period_end is never
        // consulted for Free accounts).
        // Postgres SUM(bigint) widens to NUMERIC (overflow-safety) — cast back
        // to BIGINT explicitly, otherwise sqlx's i64 decode fails at runtime.
        let usage_row = sqlx::query_as::<_, (i64, i64, i64)>(
            "SELECT COALESCE(SUM(m.read_ops), 0)::BIGINT, COALESCE(SUM(m.write_ops), 0)::BIGINT, \
             COALESCE(SUM(m.delete_ops), 0)::BIGINT \
             FROM daily_project_metrics m \
             JOIN projects p ON p.id = m.project_id \
             WHERE p.account_id = $1 AND m.date >= DATE_TRUNC('month', CURRENT_DATE)",
        )
        .bind(account_id)
        .fetch_one(system_db.pool())
        .await;

        let (read_ops, write_ops, delete_ops) = match usage_row {
            Ok(row) => row,
            Err(e) => {
                tracing::warn!(
                    account_id = %account_id,
                    error = %e,
                    "CapUsageRefresher: failed to sum usage for account"
                );
                continue;
            }
        };

        let mut usage_this_cycle = HashMap::new();
        usage_this_cycle.insert(UsageDimension::Reads, read_ops.max(0) as u64);
        usage_this_cycle.insert(UsageDimension::Writes, write_ops.max(0) as u64);
        usage_this_cycle.insert(UsageDimension::Deletes, delete_ops.max(0) as u64);

        let status = compute_cap_status(account_id, &usage_this_cycle);
        let crossed_cap = cap_exceeded(&status);
        cap_status_cache.set(account_id, status).await;

        // AC-207-01/02: enforcement — suspend on cap-crossing, guarded so a
        // repeat crossing on a later cycle doesn't redo work every tick
        // (harmless if it did, per set_project_status's idempotent WHERE).
        if crossed_cap && subscription_status != "free_cap_exceeded" {
            // AC-207-04: identical function US-204's dunning trigger calls —
            // reused unchanged, not reimplemented.
            if let Err(e) =
                lifecycle::suspend_account_projects(account_id, lifecycle_deps).await
            {
                tracing::error!(
                    account_id = %account_id,
                    status = ?e,
                    "CapUsageRefresher: failed to suspend account projects on cap crossing"
                );
                continue;
            }

            if let Err(e) = sqlx::query(
                "UPDATE subscriptions SET status = 'free_cap_exceeded', updated_at = now() \
                 WHERE account_id = $1",
            )
            .bind(account_id)
            .execute(system_db.pool())
            .await
            {
                tracing::error!(
                    account_id = %account_id,
                    error = %e,
                    "CapUsageRefresher: failed to persist free_cap_exceeded status"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advisory_lock_key_is_deterministic() {
        assert_eq!(
            advisory_lock_key("embyr_cap_check"),
            advisory_lock_key("embyr_cap_check")
        );
    }

    #[test]
    fn advisory_lock_key_differs_for_different_input() {
        assert_ne!(
            advisory_lock_key("embyr_cap_check"),
            advisory_lock_key("something_else")
        );
    }
}
