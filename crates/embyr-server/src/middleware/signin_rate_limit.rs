//! Per-source-IP token bucket rate limiter for `POST /admin/v1/auth/signin`
//! (admin-signin-hardening, ADR-076).
//!
//! Distinct from `RateLimiter` (per-project, `rate_buckets` table, hard FK to
//! `projects`) — a source IP is not a project and has no `projects` row.
//! Reuses only the `TokenBucket` continuous-refill ALGORITHM from
//! `rate_limit.rs` (bumped to `pub(crate)`), backed by its own FK-free
//! `signin_rate_limits` table. Postgres-backed with a 20ms-timeout
//! in-process fallback, mirroring ADR-015's own fail-open shape.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use sqlx::PgPool;

use super::rate_limit::TokenBucket;

const SIGNIN_RATE_LIMIT_PG_TIMEOUT_MS: u64 = 20; // mirrors ADR-015's own bound

pub struct SigninRateLimiter {
    buckets: Mutex<HashMap<String, TokenBucket>>,
    capacity: f64,
    refill_rate: f64,
    pg_pool: Option<PgPool>,
}

impl SigninRateLimiter {
    /// In-process only (no Postgres) — used by every test-server wrapper so
    /// no acceptance test run under a shared testcontainers Postgres instance
    /// can pollute another test's throttle counters via a shared table row.
    pub fn new(capacity: f64, refill_rate: f64) -> Arc<Self> {
        Arc::new(Self {
            buckets: Mutex::new(HashMap::new()),
            capacity,
            refill_rate,
            pg_pool: None,
        })
    }

    /// Postgres-backed — cross-instance enforcement (production).
    pub fn with_pg(capacity: f64, refill_rate: f64, pool: PgPool) -> Arc<Self> {
        Arc::new(Self {
            buckets: Mutex::new(HashMap::new()),
            capacity,
            refill_rate,
            pg_pool: Some(pool),
        })
    }

    /// `Ok(())` if allowed; `Err(retry_after_ms)` if throttled.
    pub async fn check(&self, source_key: &str) -> Result<(), u64> {
        if let Some(pool) = &self.pg_pool {
            match tokio::time::timeout(
                std::time::Duration::from_millis(SIGNIN_RATE_LIMIT_PG_TIMEOUT_MS),
                self.check_pg(source_key, pool),
            )
            .await
            {
                Ok(Ok(result)) => {
                    Self::record(result.is_ok());
                    return result;
                }
                Ok(Err(e)) => {
                    metrics::counter!("embyr_signin_rate_limit_pg_error_total").increment(1);
                    tracing::warn!(
                        error = %e,
                        "signin_rate_limit: pg query error; falling back to in-process"
                    );
                }
                Err(_timeout) => {
                    metrics::counter!("embyr_signin_rate_limit_pg_timeout_total").increment(1);
                    tracing::warn!(
                        "signin_rate_limit: pg check exceeded {SIGNIN_RATE_LIMIT_PG_TIMEOUT_MS}ms, \
                         falling back to in-process"
                    );
                }
            }
        }
        let result = self.check_in_process(source_key);
        Self::record(result.is_ok());
        result
    }

    fn record(allowed: bool) {
        metrics::counter!(
            "embyr_signin_rate_limit_requests_total",
            "outcome" => if allowed { "allowed" } else { "rejected" }
        )
        .increment(1);
    }

    async fn check_pg(
        &self,
        source_key: &str,
        pool: &PgPool,
    ) -> Result<Result<(), u64>, sqlx::Error> {
        let (capacity, refill_rate) = (self.capacity, self.refill_rate);
        let allowed: Option<f64> = sqlx::query_scalar(
            "INSERT INTO signin_rate_limits (source_key, tokens, last_refill) \
             VALUES ($3, $1::float8 - 1.0, now()) \
             ON CONFLICT (source_key) DO UPDATE \
             SET tokens = LEAST($1::float8, \
                                signin_rate_limits.tokens \
                                  + EXTRACT(EPOCH FROM (now() - signin_rate_limits.last_refill)) * $2::float8 \
                               ) - 1.0, \
                 last_refill = now() \
             WHERE signin_rate_limits.tokens \
                     + EXTRACT(EPOCH FROM (now() - signin_rate_limits.last_refill)) * $2::float8 \
                   >= 1.0 \
             RETURNING tokens",
        )
        .bind(capacity)
        .bind(refill_rate)
        .bind(source_key)
        .fetch_optional(pool)
        .await?;

        match allowed {
            Some(_remaining) => Ok(Ok(())),
            None => {
                let current: f64 = sqlx::query_scalar(
                    "SELECT LEAST($1::float8, tokens + EXTRACT(EPOCH FROM (now() - last_refill)) * $2::float8) \
                     FROM signin_rate_limits WHERE source_key = $3",
                )
                .bind(capacity)
                .bind(refill_rate)
                .bind(source_key)
                .fetch_optional(pool)
                .await?
                .unwrap_or(0.0);
                let retry_after_ms = if current < 1.0 {
                    ((1.0 - current) / refill_rate * 1000.0) as u64
                } else {
                    0
                };
                Ok(Err(retry_after_ms))
            }
        }
    }

    fn check_in_process(&self, source_key: &str) -> Result<(), u64> {
        let (capacity, refill_rate) = (self.capacity, self.refill_rate);
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let bucket = buckets
            .entry(source_key.to_string())
            .or_insert_with(|| TokenBucket::new(capacity, refill_rate));
        if bucket.tokens > capacity {
            bucket.tokens = capacity;
        }
        if bucket.try_consume() {
            Ok(())
        } else {
            Err(((1.0 - bucket.tokens.min(1.0)) / refill_rate * 1000.0) as u64)
        }
    }
}

// admin-signin-hardening QUALITY_GATE: `check_pg` (the Postgres-backed path,
// production-only — every acceptance-test wrapper in this workspace
// constructs `SigninRateLimiter::new(...)`, in-process only) is never
// exercised by the integration suite. cargo-mutants confirmed two missed
// mutants here (whole-function stub -> `Ok(Ok(()))`; `<` -> `==` in the
// retry_after_ms boundary at line 130). These direct unit tests close that
// gap, mirroring the testcontainers-Postgres idiom already established in
// `crate::adapters::system_db`'s own `#[cfg(test)]` module.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::system_db::SystemDb;
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt};

    async fn start_pg() -> (ContainerAsync<Postgres>, PgPool) {
        let container = Postgres::default()
            .with_tag("15-alpine")
            .start()
            .await
            .expect("Failed to start Postgres container");
        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get port");
        let url = format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres");
        let db = SystemDb::new(&url).await.expect("connect");
        db.migrate().await.expect("migrate"); // applies 0037_signin_rate_limits.sql
        (container, db.pool().clone())
    }

    /// Reads a labelless Prometheus counter's current value out of a
    /// `PrometheusHandle::render()` body — mirrors
    /// `tests/distributed_rate_limiting/acceptance/b19_fail_open_on_pg_error.rs`'s
    /// own `labelless_metric` helper (same `name value` line shape, since both
    /// counters are registered with no labels).
    fn metric_value(body: &str, name: &str) -> f64 {
        let prefix = format!("{name} ");
        body.lines()
            .find_map(|line| line.strip_prefix(&prefix))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0.0)
    }

    /// finding #56: `embyr_signin_rate_limit_pg_error_total` increments on a
    /// genuine Postgres error (dropped table -> immediate `sqlx::Error`, not
    /// a 20ms timeout), and the pre-existing timeout counter stays untouched
    /// — mirrors finding #20's own error-vs-timeout distinction in
    /// `rate_limit.rs`'s `embyr_rate_limit_pg_error_total`.
    #[tokio::test]
    async fn check_increments_pg_error_counter_on_genuine_db_error_not_timeout() {
        let (_container, pool) = start_pg().await;
        sqlx::query("DROP TABLE signin_rate_limits")
            .execute(&pool)
            .await
            .expect("drop table to force a genuine query error");

        let limiter = SigninRateLimiter::with_pg(5.0, 1.0, pool);
        let handle = crate::observability::get_or_install_prometheus_handle();

        let before_errors =
            metric_value(&handle.render(), "embyr_signin_rate_limit_pg_error_total");
        let before_timeouts =
            metric_value(&handle.render(), "embyr_signin_rate_limit_pg_timeout_total");

        let result = limiter.check("198.51.100.4").await;
        assert!(
            result.is_ok(),
            "a genuine DB error must fall open to the fresh in-process bucket"
        );

        let after = handle.render();
        let after_errors = metric_value(&after, "embyr_signin_rate_limit_pg_error_total");
        let after_timeouts = metric_value(&after, "embyr_signin_rate_limit_pg_timeout_total");
        assert_eq!(
            after_errors,
            before_errors + 1.0,
            "expected embyr_signin_rate_limit_pg_error_total to increment on a genuine DB error"
        );
        assert_eq!(
            after_timeouts, before_timeouts,
            "a genuine DB error must not also increment the timeout counter"
        );
    }

    #[tokio::test]
    async fn check_pg_allows_when_under_capacity() {
        let (_container, pool) = start_pg().await;
        let limiter = SigninRateLimiter::with_pg(5.0, 1.0, pool.clone());
        let result = limiter
            .check_pg("198.51.100.1", &pool)
            .await
            .expect("no sqlx error");
        assert!(
            result.is_ok(),
            "first request under capacity must be allowed"
        );
    }

    #[tokio::test]
    async fn check_pg_throttles_once_capacity_exhausted() {
        let (_container, pool) = start_pg().await;
        // capacity=1, near-zero refill: the 2nd immediate request must throttle.
        let limiter = SigninRateLimiter::with_pg(1.0, 1.0 / 3600.0, pool.clone());
        let first = limiter
            .check_pg("198.51.100.2", &pool)
            .await
            .expect("no sqlx error");
        assert!(first.is_ok(), "first request consumes the only token");

        let second = limiter
            .check_pg("198.51.100.2", &pool)
            .await
            .expect("no sqlx error");
        match second {
            Err(retry_after_ms) => assert!(
                retry_after_ms > 0,
                "retry_after_ms must be > 0 once throttled"
            ),
            Ok(()) => panic!("second request must be throttled once capacity is exhausted"),
        }
    }

    /// Pins the exact `retry_after_ms` VALUE (not just its sign) with a
    /// non-degenerate `current` (~0.5, not ~0.0) so `1.0 - current` and
    /// `1.0 + current`/`1.0 / current` diverge, and `/ refill_rate * 1000.0`
    /// vs `+`/`/` variants land far outside the expected range. Closes 4
    /// arithmetic mutants (`*`->`+`, `*`->`/`, `-`->`+`, `-`->`/` on the
    /// `retry_after_ms` formula) that `check_pg_throttles_once_capacity_
    /// exhausted`'s `> 0` assertion alone did not catch.
    #[tokio::test]
    async fn check_pg_retry_after_ms_matches_expected_formula() {
        let (_container, pool) = start_pg().await;
        // capacity=1.5, refill=1/hour: first call leaves current ~= 0.5 tokens.
        let limiter = SigninRateLimiter::with_pg(1.5, 1.0 / 3600.0, pool.clone());
        let first = limiter
            .check_pg("198.51.100.3", &pool)
            .await
            .expect("no sqlx error");
        assert!(first.is_ok(), "first request leaves ~0.5 tokens, still allowed");

        let second = limiter
            .check_pg("198.51.100.3", &pool)
            .await
            .expect("no sqlx error");
        // Expected: (1.0 - 0.5) / (1/3600) * 1000.0 = 1_800_000ms, with slack
        // for the negligible refill + real query latency between the 2 calls.
        match second {
            Err(retry_after_ms) => assert!(
                (1_700_000..=1_900_000).contains(&retry_after_ms),
                "retry_after_ms = {retry_after_ms}, expected ~1_800_000 \
                 ((1.0 - 0.5) / (1/3600) * 1000.0)"
            ),
            Ok(()) => panic!("second request must be throttled (only ~0.5 tokens remained)"),
        }
    }
}
