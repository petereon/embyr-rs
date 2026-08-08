# Slice DRL-04 — Rate-Limit Response Headers

**Feature:** distributed-rate-limiting
**Slice:** DRL-04 of DRL-05
**Estimate:** 0.5 days
**Stories:** US-DRL-02 — Rate-limit response headers for SDK adaptive backoff
**Depends on:** DRL-02 (return type `Result<RateLimitInfo, RateLimitInfo>` must exist), DRL-03 (fallback path also carries `RateLimitInfo` — headers must work on both paths)

---

## Goal

Attach `x-ratelimit-limit`, `x-ratelimit-remaining`, `x-ratelimit-reset` to all gRPC response trailing metadata at all 9 handler call sites. Add `retry-after-ms` to RESOURCE_EXHAUSTED responses only. Replaced the placeholder stubs left in DRL-02 with inline header attachment via `attach_rate_limit_headers()`. IMPLEMENTED.

## Learning Hypothesis

Disproves: "tonic trailing metadata attachment at all 9 handler call sites requires per-handler boilerplate that is error-prone and easy to miss."
Confirms if succeeds: A single `attach_rate_limit_headers(metadata_map, &info)` helper function keeps all 9 sites identical; a compile-time check or test validates coverage.

## IN Scope

- Helper function `attach_rate_limit_headers(metadata: &mut tonic::metadata::MetadataMap, info: &RateLimitInfo)`:
  - Inserts `x-ratelimit-limit: {info.limit as u64}` as ASCII metadata value
  - Inserts `x-ratelimit-remaining: {info.remaining.max(0.0) as u64}` as ASCII metadata value
  - Inserts `x-ratelimit-reset: {info.reset_ms}` as ASCII metadata value (epoch milliseconds)
- Helper function `rate_limited_response(info: &RateLimitInfo) -> Status`:
  - Creates `Status::resource_exhausted("rate limit exceeded")`
  - Adds `retry-after-ms: {ms_until_next_token(info)}` to status metadata
  - `retry-after-ms` = `ceil((1.0 - info.remaining.max(0.0)) / refill_rate * 1000.0)`, minimum 1ms
- Update all 9 handler call sites in `crates/embyr-server/src/grpc/handler.rs`:
  ```rust
  match self.rate_limiter.check(&project_id).await {
      Ok(info) => {
          // attach headers to response later in handler body (before return)
          // or use response extension / wrapper
          attach_rate_limit_headers(response.metadata_mut(), &info); // IMPLEMENTED — tonic 0.12 inline attachment
      }
      Err(info) => return Err(rate_limited_response(&info)),
  }
  ```
  IMPLEMENTED: inline attachment at each call site via `attach_rate_limit_headers()` and `rate_limit_rejection()`. tonic 0.12 `Response<T>::metadata_mut()` for unary handlers; `Status::metadata_mut()` for rejections. See ADR-015 for streaming handler asymmetry.
- New acceptance test `us_drl_02_rate_limit_headers.rs`:
  - `GetDocument` response includes `x-ratelimit-limit`, `x-ratelimit-remaining`, `x-ratelimit-reset`
  - Rate-limited `GetDocument` response includes `retry-after-ms`
  - `RunQuery` response includes the same set of headers (spot-check non-GetDocument path)

## OUT Scope

- Changing the `x-ratelimit-*` header names (they follow the de-facto standard used by GitHub/Stripe/Fastly)
- Per-window rate-limit semantics (e.g., `x-ratelimit-policy` header) — token bucket has no hard window
- gRPC-Web header translation validation (tonic-web passes trailing metadata through; no extra code needed, but validation is out of scope for this slice)

## Acceptance Criteria

- AC-DRL-02: all 9 gRPC handler methods attach `x-ratelimit-limit`, `x-ratelimit-remaining`, `x-ratelimit-reset` as trailing metadata on allowed responses
- AC-DRL-02: RESOURCE_EXHAUSTED responses additionally attach `retry-after-ms` as trailing metadata
- AC-DRL-02: `x-ratelimit-remaining` is never negative (floor at 0)
- AC-DRL-02: `x-ratelimit-limit` equals current `EMBYR_RATE_LIMIT_RPS` value
- AC-DRL-02: `retry-after-ms` is ≥ 1 (never 0 even if bucket is nearly full)
- Acceptance test exercises at minimum `GetDocument` and `RunQuery` for header presence

## Dependencies

- DRL-02: `RateLimitInfo` type exists with `remaining`, `limit`, `reset_ms` fields
- DRL-03: `check()` returns `RateLimitInfo` on the fallback path too — headers must be populated from the fallback path's `TokenBucket` values
- tonic 0.12 metadata API CONFIRMED — `Response<T>::metadata_mut()` for unary trailing metadata; `Response<BoxStream>::metadata_mut()` for streaming initial metadata; `Status::metadata_mut()` for error frames. Documented in ADR-015.

## Effort Estimate

0.5 days. The helper functions are ~30 LOC. Updating 9 call sites is mechanical once the helper signature is agreed. The primary work is writing the integration test that verifies trailing metadata is accessible to the tonic test client.
