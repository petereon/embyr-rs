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
                Ok(Err(e)) => tracing::warn!(
                    error = %e,
                    "signin_rate_limit: pg query error; falling back to in-process"
                ),
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
