// @error @US-01 @AC-PSL-05
//! pool-sizing-and-limits (finding #16 High + #30 Medium, ADR-079) — invalid
//! pool-sizing environment variable values cause STARTUP FAILURE, never a
//! silent fallback to default (AC-PSL-05).
//!
//! Mirrors `pr01_config_from_env.rs`'s own established shape exactly
//! (`AC-PR-01-02` through `AC-PR-01-05`): a syntactically-valid-but-
//! unreachable `DATABASE_URL` is enough — config validation happens BEFORE
//! any DB connection attempt, so no Postgres container is needed for any
//! scenario in this file (cheap, fast, no Docker required).
//!
//! Per ADR-079 Decision 1, `parse_positive_u32` (mirroring `parse_port`'s
//! shape) is reused 5 times: absent -> documented default; present but
//! non-numeric or `0` -> `ConfigError::InvalidPoolConfig`. Two distinct
//! failure MODES exist (non-numeric, zero) — per Mandate 11, one example
//! per failure mode is sufficient once the underlying shared-helper
//! mechanism is established; this file proves non-numeric on EVERY one of
//! the 5 new vars (confirming each is actually wired to the shared parser,
//! not just the first), and zero-value on 2 representative vars (one
//! `max_connections`-family, one `acquire_timeout`-family) rather than
//! repeating the identical zero-check 5x for marginal signal.
//!
//! Driving port: real `embyr-server` subprocess (`ServerProcess::start_env_only`).
//!
//! Layer: subprocess/FS acceptance (~100ms-3s each, real binary, no
//! Postgres) — example-only per Mandate 11 (sad paths are never
//! PBT-generated), traditional assertions.
//!
//! Scaffold state: NONE — every test drives the real, pre-built
//! `embyr-server` binary over its process boundary (env vars in, exit
//! code + stderr out); no Rust-level import of an unimplemented production
//! symbol. Every scenario here fails today for the right reason: today
//! `ServerConfig::from_env()` never reads any of these 5 new var names at
//! all, so the server starts up successfully (or fails for an unrelated,
//! pre-existing reason) regardless of what garbage value is set — exit
//! code is never the expected `1`, and stderr never names the pool var.
//!
//! All tests except the first are `#[ignore]` — DELIVER unskips one at a
//! time. The first (`non_numeric_tenant_max_connections_fails_startup`) is
//! representative and left enabled as this file's own regression anchor,
//! following `pr06`/`pr07`'s own precedent of a non-walking-skeleton file
//! still needing ONE enabled anchor test (this feature's true walking
//! skeleton lives in `pr11_pool_sizing_tenant_pool.rs`).

use std::time::Duration;

use crate::common::{ServerProcess, TEST_ENCRYPTION_KEY};

/// Unreachable-but-syntactically-valid DSN — config validation must reject
/// the bad pool var BEFORE any connection attempt is made, matching
/// `pr01_config_from_env.rs::exits_1_when_encryption_key_not_64_hex_chars`'s
/// own precedent.
const PLACEHOLDER_DSN: &str = "postgres://postgres:postgres@127.0.0.1:65535/embyr";

async fn assert_invalid_pool_var_fails_startup(var: &str, value: &str) {
    let mut server = ServerProcess::start_env_only(&[
        ("DATABASE_URL", PLACEHOLDER_DSN),
        ("EMBYR_ADMIN_KEY", "testkey"),
        ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        (var, value),
    ]);

    let exit_code = server.wait_for_exit(Duration::from_secs(3)).await;
    let stderr = server.drain_stderr();

    assert_eq!(
        exit_code,
        Some(1),
        "AC-PSL-05: server must exit 1 when {var}={value:?} is invalid; got {exit_code:?}\n\
         stderr: {stderr}"
    );
    assert!(
        stderr.contains(var),
        "AC-PSL-05: stderr must name the specific invalid variable ({var}); got: {stderr}"
    );
}

/// @error @AC-PSL-05
#[tokio::test]
async fn non_numeric_tenant_max_connections_fails_startup() {
    assert_invalid_pool_var_fails_startup("EMBYR_TENANT_DB_MAX_CONNECTIONS", "not_a_number").await;
}

/// @error @AC-PSL-05
#[tokio::test]
#[ignore]
async fn non_numeric_system_max_connections_fails_startup() {
    assert_invalid_pool_var_fails_startup("EMBYR_SYSTEM_DB_MAX_CONNECTIONS", "not_a_number").await;
}

/// @error @AC-PSL-05
#[tokio::test]
#[ignore]
async fn non_numeric_tenant_acquire_timeout_fails_startup() {
    assert_invalid_pool_var_fails_startup("EMBYR_TENANT_DB_ACQUIRE_TIMEOUT_SECS", "not_a_number").await;
}

/// @error @AC-PSL-05
#[tokio::test]
#[ignore]
async fn non_numeric_listener_max_connections_fails_startup() {
    assert_invalid_pool_var_fails_startup("EMBYR_LISTENER_DB_MAX_CONNECTIONS", "not_a_number").await;
}

/// @error @AC-PSL-05
#[tokio::test]
#[ignore]
async fn non_numeric_listener_acquire_timeout_fails_startup() {
    assert_invalid_pool_var_fails_startup("EMBYR_LISTENER_DB_ACQUIRE_TIMEOUT_SECS", "not_a_number").await;
}

/// Zero is explicitly non-positive — `parse_positive_u32` must reject it,
/// never silently substitute a default or a zero-sized (permanently
/// unusable) pool.
///
/// @error @boundary @AC-PSL-05
#[tokio::test]
#[ignore]
async fn zero_tenant_max_connections_fails_startup() {
    assert_invalid_pool_var_fails_startup("EMBYR_TENANT_DB_MAX_CONNECTIONS", "0").await;
}

/// @error @boundary @AC-PSL-05
#[tokio::test]
#[ignore]
async fn zero_listener_acquire_timeout_fails_startup() {
    assert_invalid_pool_var_fails_startup("EMBYR_LISTENER_DB_ACQUIRE_TIMEOUT_SECS", "0").await;
}
