// SCAFFOLD: true
//! distributed-rate-limiting — acceptance test module root.
//!
//! Each acceptance test file is its own `[[test]]` binary entry in
//! `crates/embyr-server/Cargo.toml`. This file is a documentation
//! marker only — it is not referenced by any `[[test]]` entry directly.
//!
//! Test placement: `tests/distributed_rate_limiting/acceptance/`
//! Precedent: mirrors the `tests/admin_api_v2/` layout established
//! by the admin-api-v2 feature.
//!
//! Walking skeleton: b12 `distributed_rate_limit_rejects_when_bucket_exhausted`
//! (DRL-02 is the first slice with observable behaviour at the gRPC driving port).
//!
//! Implementation order (matches slice dependency graph):
//!   1. b11 — DRL-05: provisioning inserts rate_buckets row (depends on DRL-01 schema)
//!   2. b12 — DRL-02: atomic Postgres UPDATE enforces distributed limit
//!   3. b13 — DRL-03: 20ms timeout + per-instance fallback
//!   4. b14 — DRL-04: x-ratelimit-* trailing metadata headers
