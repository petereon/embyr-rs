//! Rate-limit domain types.
//!
//! Pure value types — no IO, no async.  These structs are returned by
//! `RateLimiter::check()` in embyr-server and serialised into gRPC
//! trailing metadata headers (`x-ratelimit-*`).

/// Result from a rate-limit check — populates x-ratelimit-* response headers.
///
/// Returned as `Ok(info)` when the request is allowed, `Err(info)` when the
/// token bucket is empty and the request is rejected.
#[derive(Debug, Clone)]
pub struct RateLimitInfo {
    /// Token count after this request (may be fractional; can be negative on
    /// in-process fallback when the per-project bucket is shared across threads).
    pub remaining: f64,
    /// Configured capacity (rps ceiling).  Echoed back to clients as
    /// `x-ratelimit-limit`.
    pub limit: f64,
    /// Milliseconds until at least 1 token will be available.
    /// Zero when `remaining >= 1.0`.
    pub reset_ms: u64,
}
