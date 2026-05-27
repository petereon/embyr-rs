//! Per-project token bucket rate limiter.
//!
//! Pure in-process implementation — no external dependencies.
//! Thread-safe via `tokio::sync::Mutex`. The `check()` call is sub-microsecond
//! on uncontested paths (no I/O, no allocation on the hot path after first access).

use std::{collections::HashMap, sync::Arc, time::Instant};

use tokio::sync::Mutex;

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

/// Per-project token bucket rate limiter.
pub struct RateLimiter {
    buckets: Mutex<HashMap<String, TokenBucket>>,
    capacity: f64,
    refill_rate: f64,
    pub enabled: bool,
}

impl RateLimiter {
    /// Create a new enabled rate limiter with the given capacity and refill rate.
    ///
    /// - `capacity`: maximum burst size (tokens)
    /// - `refill_rate`: tokens added per second
    pub fn new(capacity: f64, refill_rate: f64) -> Arc<Self> {
        Arc::new(Self {
            buckets: Mutex::new(HashMap::new()),
            capacity,
            refill_rate,
            enabled: true,
        })
    }

    /// Create a disabled rate limiter — all requests are allowed unconditionally.
    pub fn disabled() -> Arc<Self> {
        Arc::new(Self {
            buckets: Mutex::new(HashMap::new()),
            capacity: f64::MAX,
            refill_rate: f64::MAX,
            enabled: false,
        })
    }

    /// Check the rate limit for `project_id`.
    ///
    /// Returns `Ok(())` if the request is allowed, `Err(())` if rate limited.
    pub async fn check(&self, project_id: &str) -> Result<(), ()> {
        if !self.enabled {
            return Ok(());
        }
        let mut buckets = self.buckets.lock().await;
        let bucket = buckets
            .entry(project_id.to_string())
            .or_insert_with(|| TokenBucket::new(self.capacity, self.refill_rate));
        if bucket.try_consume() {
            Ok(())
        } else {
            Err(())
        }
    }
}
