//! Per-project token bucket rate limiter.
//!
//! Supports two enforcement modes:
//!   - **Postgres-backed** (`with_pg`): atomic `UPDATE` on `rate_buckets` shared
//!     across all embyr-server instances.  Falls back to per-instance on timeout.
//!   - **In-process only** (`new`): pure in-memory token bucket; sufficient for
//!     single-instance deployments and tests that don't need cross-node enforcement.
//!
//! The 20 ms Postgres timeout ensures a slow/unavailable DB never adds latency
//! to gRPC hot paths.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderValue, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use embyr_core::rate_limit::RateLimitInfo;

/// Hard cap on Postgres round-trip for distributed rate-limit enforcement.
const RATE_LIMIT_PG_TIMEOUT_MS: u64 = 20;

// ---------------------------------------------------------------------------
// Token bucket (in-process, per project)
// ---------------------------------------------------------------------------

/// A single token bucket for one project.
struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_rate: f64, // tokens per second
    last_refill: Instant,
}

impl TokenBucket {
    fn new(capacity: f64, refill_rate: f64) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    /// Try to consume one token. Returns `true` if allowed.
    fn try_consume(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// RateLimiter
// ---------------------------------------------------------------------------

/// Per-project token bucket rate limiter.
///
/// When a `pg_pool` is configured, each `check()` call first attempts a
/// Postgres-level atomic UPDATE within a 20 ms timeout.  On timeout or
/// Postgres error the call falls through to the per-instance in-memory
/// bucket (`check_in_process`).
pub struct RateLimiter {
    /// Per-project in-process token buckets (fallback / solo-instance mode).
    buckets: Mutex<HashMap<String, TokenBucket>>,
    capacity: f64,
    refill_rate: f64,
    enabled: bool,
    pg_pool: Option<sqlx::PgPool>,
}

impl RateLimiter {
    /// Create a new enabled rate limiter with the given capacity and refill rate.
    ///
    /// Uses in-process token buckets only (no Postgres).
    ///
    /// - `capacity`: maximum burst size (tokens)
    /// - `refill_rate`: tokens added per second
    pub fn new(capacity: f64, refill_rate: f64) -> Arc<Self> {
        Arc::new(Self {
            buckets: Mutex::new(HashMap::new()),
            capacity,
            refill_rate,
            enabled: true,
            pg_pool: None,
        })
    }

    /// Create an enabled rate limiter backed by a shared Postgres `rate_buckets` table.
    ///
    /// Falls back to per-instance buckets if the Postgres call times out (20 ms).
    pub fn with_pg(capacity: f64, refill_rate: f64, pool: sqlx::PgPool) -> Arc<Self> {
        Arc::new(Self {
            buckets: Mutex::new(HashMap::new()),
            capacity,
            refill_rate,
            enabled: true,
            pg_pool: Some(pool),
        })
    }

    /// Create a disabled rate limiter — all requests are allowed unconditionally.
    pub fn disabled() -> Arc<Self> {
        Arc::new(Self {
            buckets: Mutex::new(HashMap::new()),
            capacity: f64::MAX,
            refill_rate: f64::MAX,
            enabled: false,
            pg_pool: None,
        })
    }

    /// Check the rate limit for `project_id`.
    ///
    /// Returns `Ok(RateLimitInfo)` if the request is allowed,
    /// `Err(RateLimitInfo)` if the bucket is exhausted.
    ///
    /// When a `pg_pool` is configured the check is attempted against Postgres
    /// first.  On timeout the call falls through to the in-process bucket so
    /// gRPC latency is bounded by `RATE_LIMIT_PG_TIMEOUT_MS`.
    ///
    /// Increments `embyr_rate_limit_requests_total{project_id, outcome}` on
    /// every return (OBS-04).  The pg-timeout counter is incremented alongside
    /// the existing `tracing::warn!` inside `check_inner`.
    pub async fn check(&self, project_id: &str) -> Result<RateLimitInfo, RateLimitInfo> {
        let result = self.check_inner(project_id).await;
        let outcome = if result.is_ok() { "allowed" } else { "rejected" };
        metrics::counter!(
            "embyr_rate_limit_requests_total",
            "project_id" => project_id.to_owned(),
            "outcome" => outcome
        )
        .increment(1);
        result
    }

    /// Inner implementation of `check` — extracted so the metric wrapper has a
    /// single call site.
    async fn check_inner(&self, project_id: &str) -> Result<RateLimitInfo, RateLimitInfo> {
        if !self.enabled {
            return Ok(RateLimitInfo {
                remaining: self.capacity,
                limit: self.capacity,
                reset_ms: 0,
            });
        }

        if let Some(pool) = &self.pg_pool {
            match tokio::time::timeout(
                std::time::Duration::from_millis(RATE_LIMIT_PG_TIMEOUT_MS),
                self.check_pg(project_id, pool),
            )
            .await
            {
                Ok(result) => return result,
                Err(_timeout) => {
                    tracing::warn!(
                        project_id = project_id,
                        "rate_limit_pg_timeout: Postgres check exceeded {}ms, falling back to in-process bucket",
                        RATE_LIMIT_PG_TIMEOUT_MS,
                    );
                    // OBS-04: pg-timeout counter alongside the warn! log (ADR-016).
                    metrics::counter!("embyr_rate_limit_pg_timeout_total").increment(1);
                    // Fall through to in-process
                }
            }
        }

        self.check_in_process(project_id)
    }

    /// Attempt an atomic UPDATE against the `rate_buckets` Postgres table.
    ///
    /// On success, returns `Ok(info)` or `Err(info)` based on whether tokens
    /// were available.  On Postgres error, returns `Ok(info)` with a synthetic
    /// full-capacity token count (fail-open behaviour: DB errors are not
    /// penalised).
    async fn check_pg(
        &self,
        project_id: &str,
        pool: &sqlx::PgPool,
    ) -> Result<RateLimitInfo, RateLimitInfo> {
        let capacity = self.capacity;
        let refill_rate = self.refill_rate;

        // Atomic UPDATE: refill based on elapsed time then try to consume 1 token.
        // Returns the new token count iff tokens were available (≥ 1.0 before deduction).
        let allowed: Option<f64> = sqlx::query_scalar(
            "UPDATE rate_buckets \
             SET tokens = LEAST($1::float8, \
                               tokens + EXTRACT(EPOCH FROM (now() - last_refill)) * $2::float8) - 1.0, \
                 last_refill = now() \
             WHERE project_id = $3 \
               AND tokens + EXTRACT(EPOCH FROM (now() - last_refill)) * $2::float8 >= 1.0 \
             RETURNING tokens",
        )
        .bind(capacity)
        .bind(refill_rate)
        .bind(project_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

        match allowed {
            Some(remaining) => {
                let reset_ms = if remaining < 1.0 {
                    ((1.0 - remaining) / refill_rate * 1000.0) as u64
                } else {
                    0
                };
                Ok(RateLimitInfo { remaining, limit: capacity, reset_ms })
            }
            None => {
                // No row updated: either rate-limited or project row absent.
                // Check for row existence — absent means project predates migration 0018.
                let row_exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM rate_buckets WHERE project_id = $1)",
                )
                .bind(project_id)
                .fetch_one(pool)
                .await
                .unwrap_or(false);

                if !row_exists {
                    // Project existed before migration 0018 — insert a default row and allow.
                    let _ = sqlx::query(
                        "INSERT INTO rate_buckets (project_id, tokens, last_refill) \
                         VALUES ($1, $2, now()) ON CONFLICT DO NOTHING",
                    )
                    .bind(project_id)
                    .bind(capacity - 1.0)
                    .execute(pool)
                    .await;
                    return Ok(RateLimitInfo {
                        remaining: capacity - 1.0,
                        limit: capacity,
                        reset_ms: 0,
                    });
                }

                // Genuinely rate-limited — query current token level for headers.
                let current: f64 = sqlx::query_scalar(
                    "SELECT LEAST($1::float8, \
                                  tokens + EXTRACT(EPOCH FROM (now() - last_refill)) * $2::float8) \
                     FROM rate_buckets WHERE project_id = $3",
                )
                .bind(capacity)
                .bind(refill_rate)
                .bind(project_id)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten()
                .unwrap_or(0.0);

                let reset_ms = if current < 1.0 {
                    ((1.0 - current) / refill_rate * 1000.0) as u64
                } else {
                    0
                };
                Err(RateLimitInfo { remaining: current, limit: capacity, reset_ms })
            }
        }
    }

    /// Per-instance in-process token bucket check (sync).
    ///
    /// Used when Postgres is unavailable or not configured.
    /// Caps the in-process bucket at 1× capacity to prevent token accumulation
    /// during long idle periods.
    fn check_in_process(&self, project_id: &str) -> Result<RateLimitInfo, RateLimitInfo> {
        let capacity = self.capacity;
        let refill_rate = self.refill_rate;
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let bucket = buckets
            .entry(project_id.to_string())
            .or_insert_with(|| TokenBucket::new(capacity, refill_rate));
        // Cap to prevent unbounded accumulation during idle periods.
        if bucket.tokens > capacity {
            bucket.tokens = capacity;
        }
        if bucket.try_consume() {
            let remaining = bucket.tokens;
            let reset_ms = if remaining < 1.0 {
                ((1.0 - remaining) / refill_rate * 1000.0) as u64
            } else {
                0
            };
            Ok(RateLimitInfo { remaining, limit: capacity, reset_ms })
        } else {
            let reset_ms =
                ((1.0 - bucket.tokens.min(1.0)) / refill_rate * 1000.0) as u64;
            Err(RateLimitInfo { remaining: bucket.tokens, limit: capacity, reset_ms })
        }
    }
}

// ---------------------------------------------------------------------------
// REST axum middleware — closes the gRPC-only enforcement gap (ADR-043
// Decision 7: `grep` across `crates/embyr-server/src/rest/` for
// `RateLimiter` returned zero matches; every gRPC handler method gates via
// `rate_limiter.check()`, but no REST route did).
// ---------------------------------------------------------------------------

/// Axum middleware enforcing the per-project rate limit on REST routes that
/// carry a `:project_id` path parameter — today, the `accounts:<verb>`
/// identity-bridge route (`accounts_bridge_dispatch` in `lib.rs`).
///
/// Mounted via `Router::route_layer` (applied AFTER path matching, so
/// `Path` extraction succeeds) so it runs once, uniformly, before every
/// dispatched handler — mirroring `FirestoreService::handle_get_document`'s
/// own single `rate_limiter.check()` call site, instead of duplicating the
/// check inside each REST handler. Uses the SAME `Arc<RateLimiter>` the
/// caller passes in (the gRPC side's own instance — see `lib.rs::spawn_all_servers`),
/// never a second bucket.
///
/// gRPC-Web requests are NOT routed through this middleware — they dispatch
/// to the tonic `FirestoreServer` (see `rest/grpc_web.rs::HybridService`),
/// which reuses the SAME per-RPC `rate_limiter.check()` calls the native
/// gRPC port already has. Only the plain-HTTP axum routes needed this gate.
pub async fn rest_rate_limit_middleware(
    State(rate_limiter): State<Arc<RateLimiter>>,
    Path(params): Path<HashMap<String, String>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(project_id) = params.get("project_id") else {
        // This middleware is only mounted on routes that declare
        // `:project_id` — nothing to gate on if it's ever absent.
        return next.run(request).await;
    };

    match rate_limiter.check(project_id).await {
        Ok(_info) => next.run(request).await,
        Err(info) => rest_rate_limit_rejection(project_id, &info),
    }
}

/// Build the HTTP 429 rejection response.
///
/// Body shape is `docs/SPEC.md`'s § Rate Limiting documented REST
/// convention: `{"error":{"code":429,"message":"rate limit exceeded: <resource_name>","status":"RESOURCE_EXHAUSTED"}}`.
/// Also attaches `x-ratelimit-*`/`retry-after-ms` headers, mirroring
/// `FirestoreService::rate_limit_rejection`'s gRPC trailing-metadata shape.
fn rest_rate_limit_rejection(project_id: &str, info: &RateLimitInfo) -> Response {
    let body = serde_json::json!({
        "error": {
            "code": 429,
            "message": format!("rate limit exceeded: {project_id}"),
            "status": "RESOURCE_EXHAUSTED",
        }
    });
    let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
    let headers = response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&info.limit.to_string()) {
        headers.insert("x-ratelimit-limit", v);
    }
    if let Ok(v) = HeaderValue::from_str(&(info.remaining.floor() as i64).to_string()) {
        headers.insert("x-ratelimit-remaining", v);
    }
    if let Ok(v) = HeaderValue::from_str(&info.reset_ms.to_string()) {
        headers.insert("x-ratelimit-reset", v.clone());
        headers.insert("retry-after-ms", v);
    }
    response
}
