// @walking_skeleton @driving_port @real-io @US-01
//! pool-sizing-and-limits (finding #16 High + #30 Medium, ADR-079) — the
//! PER-TENANT customer-document pool (`PostgresBackendAdapter::with_pool_config`,
//! `backend_adapter.rs`), reached here via `handler.rs`'s `authenticate()`
//! (gRPC Firestore path — DISCUSS's original citation).
//!
//! Acceptance criteria verified here (feature-delta.md; ADR-079):
//!   AC-PSL-02: the tenant pool accepts an `EMBYR_TENANT_DB_MAX_CONNECTIONS`
//!              override, applied at pool-construction time, no code change.
//!   AC-PSL-03: the tenant pool (one of the 2 sites missing `acquire_timeout`
//!              today) gains one, overridable via
//!              `EMBYR_TENANT_DB_ACQUIRE_TIMEOUT_SECS`.
//!   AC-PSL-04: a request against a saturated tenant pool fails with a
//!              clean, typed error within the configured `acquire_timeout`
//!              — proven against a REAL saturated Postgres-backed pool.
//!   AC-PSL-01: with no new env vars set, the tenant pool's `max_connections`
//!              stays exactly 5 (today's value) — regression guard.
//!   AC-PSL-06: one tenant's saturated pool does not affect another
//!              tenant's separate pool.
//!
//! Saturation mechanism (deterministic, not a query-speed race): a real,
//! uncommitted `SELECT ... FOR UPDATE` opened directly against the tenant's
//! Postgres (bypassing the SUT) holds a row lock that `commit_transaction`'s
//! own OCC existence check (`backend_adapter.rs`, `MustExist` branch) will
//! block on when the SUT issues a real `Commit` RPC carrying a
//! `current_document.exists = true` precondition against that SAME
//! document. The blocked `Commit` genuinely checks out and holds one of the
//! SUT's own pool connections for as long as the external lock is held —
//! this is what "saturates" the pool, not a race between fast queries. See
//! `commit_update_requiring_exists`/`lock_document_row_for_update` in
//! `tests/production_readiness/common/mod.rs`.
//!
//! Driving port: real `embyr-server` subprocess (`ServerProcess`, the ONLY
//! harness in this workspace whose pool construction actually reads
//! `ServerConfig::from_env()` — the in-process `start_test_server_*`
//! variants are DESIGN-confirmed to keep today's hardcoded literals
//! regardless of env vars), real gRPC `FirestoreClient` (`CreateDocument`,
//! `Commit`), real admin-API project provisioning
//! (`POST /admin/v1/projects`, mirrors `pr10`'s own call).
//!
//! Layer: WS/integration (~seconds, real Postgres + real subprocess) — per
//! `nw-test-design-mandates` Layered Test Discipline, example-only (no PBT),
//! traditional assertions (not `assert_state_delta`), matching every
//! existing `pr0X` file's own established style in this suite.
//!
//! Scaffold state: NONE created by this DISTILL pass. Every symbol these
//! tests import already exists today (`ServerProcess`, `SystemDb::new`,
//! `PostgresBackendAdapter::new` via the real binary, `FirestoreClient`) —
//! no Rust-level import of an unimplemented production symbol (mirrors
//! `pr10`'s own "NONE" scaffold-state precedent). The NEW behavior
//! (`EMBYR_TENANT_DB_MAX_CONNECTIONS`/`EMBYR_TENANT_DB_ACQUIRE_TIMEOUT_SECS`
//! actually applied) does not exist yet — today the real subprocess ignores
//! both vars and keeps `max_connections(5)` with NO `acquire_timeout`, so
//! the walking skeleton's own assertions (small override honored, fast
//! failure) fail for the right reason: `elapsed` is small but the 3rd
//! request unexpectedly SUCCEEDS (5 hardcoded connections comfortably cover
//! 2 locked + 1 free), not the expected `Err` — MISSING_FUNCTIONALITY, not a
//! setup bug.
//!
//! Test 3 (`tenant_pool_default_max_connections_preserved_when_unset`) is a
//! REGRESSION GUARD, not new functionality — its assertions already hold
//! today (today's hardcoded default IS 5, coincidentally == the new
//! documented default), mirroring `pr10`'s own explicit "NOT new tests
//! here" treatment for a like-shaped guard (AC-HDC-04/09). Documented here,
//! not silently passed off as a RED scenario it structurally cannot be.
//!
//! First test enabled (not `#[ignore]`) is the walking skeleton
//! (AC-PSL-02/03/04). DELIVER unskips the remaining 2 one at a time.

use std::collections::HashMap;
use std::time::Duration;

use embyr_proto::firestore::firestore_client::FirestoreClient;
use testcontainers_modules::{postgres::Postgres, testcontainers::ContainerAsync};

use crate::common::{
    commit_update_requiring_exists, create_doc, grpc_channel, lock_document_row_for_update,
    provision_project, start_postgres_container, ServerProcess, TEST_ENCRYPTION_KEY,
};

/// Start a real subprocess `embyr-server` against a real system Postgres,
/// with `extra_env` layered on top of the required admin/encryption vars —
/// shared Given across all 3 scenarios in this file (Pillar 2: no
/// copy-pasted fixture setup).
async fn start_server(extra_env: &[(&str, &str)]) -> (ContainerAsync<Postgres>, String, ServerProcess) {
    let (sys_pg, sys_url) = start_postgres_container().await;
    let mut env: Vec<(&str, &str)> = vec![
        ("EMBYR_ADMIN_KEY", "testkey"),
        ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
    ];
    env.extend_from_slice(extra_env);
    let server = ServerProcess::start(&sys_url, &env);
    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(healthy, "server did not become healthy");
    (sys_pg, sys_url, server)
}

/// Provision one tenant project with its own dedicated customer Postgres,
/// migrated and ready. Returns the customer pool (kept alive for raw locks),
/// the project's api_key, and a fresh `FirestoreClient`.
async fn provision_tenant(
    server: &ServerProcess,
    project_id: &str,
) -> (
    ContainerAsync<Postgres>,
    sqlx::PgPool,
    String,
    FirestoreClient<tonic::transport::Channel>,
) {
    let (cust_pg, cust_url) = start_postgres_container().await;
    let cust_pool = sqlx::PgPool::connect(&cust_url)
        .await
        .expect("connect customer pool for migration");
    sqlx::migrate!("../../migrations/customer")
        .run(&cust_pool)
        .await
        .expect("customer migrate");

    let api_key = provision_project(server.admin_port, project_id, &cust_url).await;
    let client = FirestoreClient::new(grpc_channel(server.grpc_port));
    (cust_pg, cust_pool, api_key, client)
}

async fn seed(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    collection: &str,
    doc_id: &str,
) {
    let mut fields = HashMap::new();
    fields.insert("seeded".to_string(), crate::common::string_field("true"));
    create_doc(client, project_id, api_key, collection, doc_id, fields).await;
}

// ─── Walking Skeleton — AC-PSL-02, AC-PSL-03, AC-PSL-04 ────────────────────────

/// Sam Chen sets a small `max_connections`/`acquire_timeout` override for
/// Solstice Retail's dedicated pool before a flash sale. Once that pool is
/// genuinely saturated, the next request fails fast and cleanly — never
/// anywhere close to sqlx's 30-second default.
///
/// Given: Solstice Retail's tenant pool is configured for max_connections=2,
///        acquire_timeout=1s via environment variables.
/// And:   2 of Solstice Retail's own documents are locked by real,
///        concurrent, in-flight writes (holding both pool connections).
/// When:  a 3rd request for a DIFFERENT, unlocked Solstice Retail document
///        is attempted.
/// Then:  it fails with a clean, typed error well within a few seconds —
///        never close to 30 seconds.
///
/// @walking_skeleton @driving_port @real-io @AC-PSL-02 @AC-PSL-03 @AC-PSL-04
#[tokio::test]
async fn tenant_pool_override_honored_and_saturated_request_fails_fast() {
    let (_sys_pg, _sys_url, server) = start_server(&[
        ("EMBYR_TENANT_DB_MAX_CONNECTIONS", "2"),
        ("EMBYR_TENANT_DB_ACQUIRE_TIMEOUT_SECS", "1"),
    ])
    .await;

    let (_cust_pg, cust_pool, api_key, mut client) =
        provision_tenant(&server, "psl-flash-sale").await;

    // Seed 3 documents — also forces the CredentialCache miss path, which
    // constructs the tenant pool with our override values.
    for doc_id in ["doc-a", "doc-b", "doc-c"] {
        seed(&mut client, "psl-flash-sale", &api_key, "orders", doc_id).await;
    }

    // Given: doc-a and doc-b are locked by real, uncommitted transactions —
    // held directly against Postgres, bypassing the SUT entirely.
    let lock_a = lock_document_row_for_update(&cust_pool, "psl-flash-sale", "orders", "doc-a").await;
    let lock_b = lock_document_row_for_update(&cust_pool, "psl-flash-sale", "orders", "doc-b").await;

    // When: 2 real, concurrent Commit RPCs each try to update the locked
    // rows — each blocks on Postgres, holding one of the SUT's 2 pool
    // connections for the duration of the external lock.
    let grpc_port = server.grpc_port;
    let key_a = api_key.clone();
    let task_a = tokio::spawn(async move {
        let mut c = FirestoreClient::new(grpc_channel(grpc_port));
        commit_update_requiring_exists(&mut c, "psl-flash-sale", &key_a, "orders", "doc-a", "locked").await
    });
    let key_b = api_key.clone();
    let task_b = tokio::spawn(async move {
        let mut c = FirestoreClient::new(grpc_channel(grpc_port));
        commit_update_requiring_exists(&mut c, "psl-flash-sale", &key_b, "orders", "doc-b", "locked").await
    });

    // Give both blocked writes time to reach the FOR UPDATE wait state.
    tokio::time::sleep(Duration::from_millis(400)).await;

    // Self-check (empirically confirmed during DISTILL, 2026-09-14): both
    // writes must still be genuinely pending at this point — if either
    // already finished, the row-lock mechanism did not engage and this
    // test's later assertions would be meaningless (Fixture Theater risk).
    assert!(
        !task_a.is_finished() && !task_b.is_finished(),
        "test setup invariant: doc-a/doc-b's writes must still be blocked on the external \
         FOR UPDATE lock at this point — if not, the saturation mechanism itself is broken, \
         not the feature under test"
    );

    // When: a 3rd request for an UNLOCKED document is attempted — it must
    // acquire a connection from the (now fully checked-out) pool.
    let start = std::time::Instant::now();
    let result =
        commit_update_requiring_exists(&mut client, "psl-flash-sale", &api_key, "orders", "doc-c", "probe")
            .await;
    let elapsed = start.elapsed();

    // Release the locks so the 2 blocked writes can complete, then clean up.
    let _ = lock_a.rollback().await;
    let _ = lock_b.rollback().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), task_a).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), task_b).await;

    assert!(
        elapsed < Duration::from_secs(3),
        "AC-PSL-04: a saturated tenant pool must fail within a few seconds, never anywhere \
         close to sqlx's 30-second default; took {elapsed:?}"
    );
    assert!(
        result.is_err(),
        "AC-PSL-02/AC-PSL-04: with max_connections=2 genuinely honored and both connections \
         held by the 2 locked writes, the 3rd request must fail with a clean, typed error \
         (not silently succeed on a hardcoded, larger default pool); result was {result:?}"
    );

    server_shutdown(server).await;
}

// ─── AC-PSL-06 — isolation regression guard ────────────────────────────────────

/// One tenant's saturated pool must never delay or fail another tenant's
/// separate, unrelated pool.
///
/// Given (reused from the walking skeleton): Solstice Retail's tenant pool
///        is configured for max_connections=2, acquire_timeout=1s, and is
///        fully saturated by 2 locked, in-flight writes.
/// When:  Fernbank Analytics — a completely separate tenant, own dedicated
///        Postgres, own dedicated pool — makes a normal request.
/// Then:  Fernbank Analytics' request succeeds quickly, unaffected by
///        Solstice Retail's saturation.
///
/// @real-io @error @AC-PSL-06
#[tokio::test]
#[ignore]
async fn one_tenants_saturated_pool_does_not_affect_another_tenants_pool() {
    let (_sys_pg, _sys_url, server) = start_server(&[
        ("EMBYR_TENANT_DB_MAX_CONNECTIONS", "2"),
        ("EMBYR_TENANT_DB_ACQUIRE_TIMEOUT_SECS", "1"),
    ])
    .await;

    let (_cust_pg_a, cust_pool_a, key_a, mut client_a) =
        provision_tenant(&server, "psl-solstice-retail").await;
    let (_cust_pg_b, _cust_pool_b, key_b, mut client_b) =
        provision_tenant(&server, "psl-fernbank-analytics").await;

    for doc_id in ["doc-a", "doc-b"] {
        seed(&mut client_a, "psl-solstice-retail", &key_a, "orders", doc_id).await;
    }
    seed(&mut client_b, "psl-fernbank-analytics", &key_b, "orders", "doc-x").await;

    // Given: Solstice Retail's pool is fully saturated (same mechanism as
    // the walking skeleton).
    let lock_a = lock_document_row_for_update(&cust_pool_a, "psl-solstice-retail", "orders", "doc-a").await;
    let lock_b = lock_document_row_for_update(&cust_pool_a, "psl-solstice-retail", "orders", "doc-b").await;

    let grpc_port = server.grpc_port;
    let solstice_key_a = key_a.clone();
    let task_a = tokio::spawn(async move {
        let mut c = FirestoreClient::new(grpc_channel(grpc_port));
        commit_update_requiring_exists(&mut c, "psl-solstice-retail", &solstice_key_a, "orders", "doc-a", "locked").await
    });
    let solstice_key_b = key_a.clone();
    let task_b = tokio::spawn(async move {
        let mut c = FirestoreClient::new(grpc_channel(grpc_port));
        commit_update_requiring_exists(&mut c, "psl-solstice-retail", &solstice_key_b, "orders", "doc-b", "locked").await
    });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !task_a.is_finished() && !task_b.is_finished(),
        "test setup invariant: Solstice Retail's 2 locked writes must still be blocked at \
         this point, or the saturation mechanism did not engage"
    );

    // When: Fernbank Analytics — a separate tenant — makes a normal request.
    let start = std::time::Instant::now();
    let result = commit_update_requiring_exists(
        &mut client_b,
        "psl-fernbank-analytics",
        &key_b,
        "orders",
        "doc-x",
        "unaffected",
    )
    .await;
    let elapsed = start.elapsed();

    let _ = lock_a.rollback().await;
    let _ = lock_b.rollback().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), task_a).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), task_b).await;

    assert!(
        result.is_ok(),
        "AC-PSL-06: Fernbank Analytics' own, separate pool must be unaffected by Solstice \
         Retail's saturation; result was {result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "AC-PSL-06: an unrelated tenant's request must be served promptly, not queued behind \
         another tenant's saturated pool; took {elapsed:?}"
    );

    server_shutdown(server).await;
}

// ─── AC-PSL-01 — defaults preserved when unset (regression guard) ─────────────

/// With none of the new pool-sizing environment variables set, the tenant
/// pool's `max_connections` stays exactly 5 — today's value.
///
/// NOTE: this scenario's own assertions already hold against today's code
/// (today's hardcoded default IS 5) — a legitimate regression guard, not
/// new functionality, mirroring `pr10_healthz_dependency_checks.rs`'s own
/// explicit "NOT new tests here" treatment for AC-HDC-04/09. Kept as an
/// explicit, falsifiable scenario (not merely inferred) because a future
/// change to the new default constant WOULD legitimately red this test.
///
/// @real-io @AC-PSL-01
#[tokio::test]
#[ignore]
async fn tenant_pool_default_max_connections_preserved_when_unset() {
    let (_sys_pg, _sys_url, server) = start_server(&[]).await;

    let (_cust_pg, cust_pool, api_key, mut client) =
        provision_tenant(&server, "psl-default-size").await;

    for doc_id in ["doc-a", "doc-b", "doc-c", "doc-d", "doc-e"] {
        seed(&mut client, "psl-default-size", &api_key, "orders", doc_id).await;
    }

    // Hold 2 of the 5 default connections busy.
    let lock_a = lock_document_row_for_update(&cust_pool, "psl-default-size", "orders", "doc-a").await;
    let lock_b = lock_document_row_for_update(&cust_pool, "psl-default-size", "orders", "doc-b").await;

    let grpc_port = server.grpc_port;
    let ka = api_key.clone();
    let task_a = tokio::spawn(async move {
        let mut c = FirestoreClient::new(grpc_channel(grpc_port));
        commit_update_requiring_exists(&mut c, "psl-default-size", &ka, "orders", "doc-a", "locked").await
    });
    let kb = api_key.clone();
    let task_b = tokio::spawn(async move {
        let mut c = FirestoreClient::new(grpc_channel(grpc_port));
        commit_update_requiring_exists(&mut c, "psl-default-size", &kb, "orders", "doc-b", "locked").await
    });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !task_a.is_finished() && !task_b.is_finished(),
        "test setup invariant: doc-a/doc-b's writes must still be blocked at this point, or \
         the saturation mechanism did not engage"
    );

    // Then: 3 more, UNLOCKED, concurrent requests must all succeed quickly
    // — proving at least 5 connections are genuinely available (2 held +
    // these 3 = 5), i.e. the default was not accidentally shrunk.
    let mut probes = Vec::new();
    for doc_id in ["doc-c", "doc-d", "doc-e"] {
        let kp = api_key.clone();
        let doc_id = doc_id.to_string();
        probes.push(tokio::spawn(async move {
            let mut c = FirestoreClient::new(grpc_channel(grpc_port));
            let start = std::time::Instant::now();
            let r = commit_update_requiring_exists(&mut c, "psl-default-size", &kp, "orders", &doc_id, "probe").await;
            (r, start.elapsed())
        }));
    }

    let mut results = Vec::new();
    for p in probes {
        results.push(tokio::time::timeout(Duration::from_secs(5), p).await);
    }

    let _ = lock_a.rollback().await;
    let _ = lock_b.rollback().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), task_a).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), task_b).await;

    for outcome in results {
        let (result, elapsed) = outcome.expect("probe task must not time out").expect("probe task must not panic");
        assert!(
            result.is_ok(),
            "AC-PSL-01: default max_connections must still be 5 — a 3rd/4th/5th concurrent \
             request while 2 connections are held must succeed, not be starved; result was \
             {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(3),
            "AC-PSL-01: a request within the default 5-connection budget must not queue; \
             took {elapsed:?}"
        );
    }

    server_shutdown(server).await;
}

async fn server_shutdown(mut server: ServerProcess) {
    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}
