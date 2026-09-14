// @driving_port @real-io @US-01 @US-02
//! healthz-dependency-checks (US-01 liveness / US-02 readiness) — `/healthz` is
//! redefined as readiness (real `SystemDb::probe()` against the shared system
//! Postgres pool); a new `/livez` is added for liveness (zero I/O, unconditional
//! 200). ADR-078.
//!
//! Acceptance criteria verified here (feature-delta.md, DISCUSS+DESIGN; ADR-078):
//!   AC-HDC-01: liveness returns success whenever the process can handle HTTP,
//!              independent of Postgres reachability.
//!   AC-HDC-02: liveness handler makes zero calls to Postgres/any external
//!              dependency — verified structurally (source inspection of the
//!              compiled handler's own module text), not by timing. A timing-
//!              based proof would be a fragile, contention-sensitive assertion
//!              (this session's own established lesson, e.g. the
//!              admin-signin-hardening TOTP/CPU-contention finding); the
//!              zero-I/O guarantee is a STRUCTURAL fact about the handler's
//!              code path, not a latency budget, so a structural check is both
//!              cheaper and non-flaky. ADR-078 § Enforcement recommends
//!              exactly this: "a cheap structural guard against a future PR
//!              silently wiring a dependency into liveness."
//!   AC-HDC-03: during a real, sustained Postgres outage, liveness keeps
//!              responding successfully on both :8081 and :9090.
//!   AC-HDC-04/AC-HDC-09 (regression guards): NOT new tests here — satisfied by
//!              `pr01_config_from_env.rs::server_starts_with_all_required_env_vars_set`,
//!              `::non_default_ports_respected`, and `pr04_graceful_shutdown.rs`
//!              continuing to pass UNCHANGED (verified during RED/GREEN, not
//!              re-implemented as a duplicate test — mirrors pr08's own
//!              AC-RLR-06 "not a new test here" treatment).
//!   AC-HDC-05/06/07/08: readiness turns unhealthy during a real outage and
//!              recovers, consistently on both :8081 and :9090, within the
//!              walking skeleton's own single combined run (DISCUSS's own
//!              named Walking Skeleton Strategy: US-02 Scenarios 2+3 proven
//!              TOGETHER with US-01 Scenario 2, one real outage, both mounts).
//!   AC-HDC-10: readiness is scoped to the SHARED SYSTEM Postgres only — a dead
//!              PER-TENANT customer database must not flip `/healthz` unhealthy.
//!   AC-HDC-11: the 503 response body is a fixed, generic string — never the
//!              raw `CoreError::BackendUnavailable` driver/schema text (new
//!              leak surface, not covered by ADR-075's tonic-only sweep).
//!
//! Probe-timeout (OQ-HDC-03, 3s `tokio::time::timeout`) — DELIBERATELY NOT
//! independently tested here. Reliably forcing an in-flight `SystemDb::probe()`
//! call to HANG (as opposed to fail fast) against an already-established pool
//! connection requires blackholing an established TCP connection's traffic
//! (iptables/tc netem/toxiproxy) — DISCUSS/DESIGN explicitly ruled out adding
//! a new dependency (toxiproxy) for this feature, and this sandbox has no
//! iptables access to fake it another way. `pr08_realtime_listener_reconnect.rs`
//! itself empirically notes a stopped testcontainers Postgres CAN sometimes
//! leave a `connect()` "blackholed" rather than fast-failing — so the walking
//! skeleton's own outage window opportunistically exercises that path — but
//! asserting on the EXACT 3s bound on top of that non-deterministic behavior
//! would be exactly the fragile timing assertion this session has repeatedly
//! flagged as a bad pattern (admin-signin-hardening TOTP/CPU-contention
//! finding). The timeout's presence is instead verified as an implementation-
//! shape fact at GREEN (the handler must wrap `system_db.probe()` in
//! `tokio::time::timeout(Duration::from_secs(3), ...)`, per DESIGN's Component
//! Design section) rather than re-proven via a flaky black-box race.
//!
//! Driving port: `embyr-server` binary subprocess, same `ServerProcess` harness
//! as pr01/pr04/pr08. `GET /livez` and `GET /healthz` on both the REST
//! (`rest_port`, :8081-equivalent) and admin (`admin_port`, :9090-equivalent)
//! HTTP surfaces — neither route requires an Authorization header (confirmed:
//! both are chained onto their router AFTER any auth `route_layer` already
//! applied inside `build_admin_router`/`spawn_all_servers`, matching pr01's
//! own unauthenticated `/healthz` poll).
//!
//! Layer: WS/`@wiring_e2e` (real Postgres container(s), real subprocess,
//! seconds each) — per `nw-test-design-mandates` Layered Test Discipline, this
//! layer uses traditional assertions (not `assert_state_delta`), example-only
//! (no PBT), matching this same file family's own established style
//! (pr01-pr09; the `tests/common/state_delta.rs` port is bootstrapped in this
//! project but, per Mandate 8, is a layer-1-3 requirement — layer 4+ WS tests
//! MAY use traditional assertions, and every existing pr0x file already does).
//!
//! Scaffold state: NONE created by this DISTILL pass. All tests here drive the
//! system exclusively over HTTP against the compiled `embyr-server` binary
//! (`CARGO_BIN_EXE_embyr-server`) — no Rust-level import of an unimplemented
//! production symbol, so there is nothing to scaffold per Mandate 7 (that
//! mandate applies when a test IMPORTS an unimplemented module; a black-box
//! HTTP test importing nothing new has no compile-time dependency to stub).
//! `/livez` does not exist yet on today's code — every test polling it gets a
//! real HTTP 404/connection behavior, which is a legitimate RED (assertion on
//! status code / body fails), not a BROKEN test.
//!
//! First test enabled (not `#[ignore]`) is the walking skeleton (AC-HDC-01,
//! 03, 05, 06, 07, 08). DELIVER unskips the remaining tests one at a time.

use std::time::Duration;

use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{core::IntoContainerPort, runners::AsyncRunner, ContainerAsync, ImageExt},
};

use crate::common::{find_free_port, ServerProcess, TEST_ENCRYPTION_KEY};

// ─── Setup helpers (shared across scenarios — Pillar 2 chained narrative) ──────

/// Start a Postgres 15-alpine testcontainer with a FIXED host port mapping.
///
/// A fixed mapping (not the default ephemeral one `start_postgres_container`
/// uses) is required so the server's `DATABASE_URL` stays valid across a
/// `stop()`/`start()` cycle — Docker only preserves a container's host port
/// across a restart when the mapping was created with a fixed host port;
/// otherwise `start()` reassigns a brand-new random port
/// (empirically confirmed and documented in
/// `pr08_realtime_listener_reconnect.rs::start_postgres`, whose exact
/// mechanism this duplicates — Rust integration test binaries don't share
/// code across sibling private modules without a dedicated support crate, so
/// re-declaring this ~10-line helper here is the smaller diff, matching this
/// suite's own established precedent, e.g. `common::sign_stripe_payload`'s
/// doc comment).
async fn start_postgres_fixed_port() -> (ContainerAsync<Postgres>, String) {
    let host_port = find_free_port();
    let container = Postgres::default()
        .with_tag("15-alpine")
        .with_mapped_port(host_port, 5432.tcp())
        .start()
        .await
        .expect("failed to start Postgres container");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres");
    (container, url)
}

/// GET `path` on `port`, returning the status code and body text, or `None`
/// if the connection itself failed (server down / route unreachable).
///
/// Client-side timeout is 6s — deliberately ABOVE `/healthz`'s own
/// DESIGN-mandated 3s `tokio::time::timeout` bound (ADR-078 OQ-HDC-03), with
/// margin. A 2s client timeout (this helper's original value) races the
/// server's legitimate worst-case latency: empirically confirmed in this
/// sandbox (and independently noted by `pr08_realtime_listener_reconnect.rs`'s
/// own module doc) that a stopped testcontainers Postgres can leave
/// `connect()` "blackholed" rather than fast-failing, so `/healthz` can
/// legitimately take up to ~3s to answer 503 during an outage. A 2s client
/// timeout would abort before that answer ever arrives, making the 503
/// transition unobservable regardless of server correctness — a test setup
/// bug, not a behavioral assertion change.
async fn get(port: u16, path: &str) -> Option<(u16, String)> {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{port}{path}"))
        .timeout(Duration::from_secs(6))
        .send()
        .await
        .ok()?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    Some((status, body))
}

/// Poll `path` on `port` until it returns `want_status`, or `timeout` elapses.
/// Returns `true` once observed, `false` on timeout.
async fn wait_for_status(port: u16, path: &str, want_status: u16, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some((status, _)) = get(port, path).await {
            if status == want_status {
                return true;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

// ─── Walking Skeleton ───────────────────────────────────────────────────────

/// Sam Chen's orchestrator can tell "process is wedged, restart it" apart
/// from "process is fine but Postgres is down, stop routing traffic" — proven
/// against a real, sustained Postgres outage, on BOTH the REST and admin HTTP
/// surfaces.
///
/// Given: embyr-server is running and healthy, Postgres fully reachable —
///        `/livez` and `/healthz` both report healthy on both mounts.
/// When:  the shared system Postgres becomes completely unreachable (real
///        container `stop()`), then becomes reachable again (`start()`).
/// Then:  `/livez` responds successfully on both mounts throughout the ENTIRE
///        outage (never depends on Postgres) — an orchestrator would never
///        restart this pod because of the outage.
/// And:   `/healthz` turns unhealthy on both mounts during the outage, and
///        recovers to healthy on both mounts once Postgres is reachable again
///        — an orchestrator would stop, then resume, routing traffic, without
///        ever restarting the pod.
///
/// @walking_skeleton @driving_port @real-io @US-01 @US-02
/// @AC-HDC-01 @AC-HDC-03 @AC-HDC-05 @AC-HDC-06 @AC-HDC-07 @AC-HDC-08
#[tokio::test]
async fn readiness_and_liveness_across_a_real_postgres_outage_on_both_mounts() {
    let (pg, db_url) = start_postgres_fixed_port().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );

    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(healthy, "server did not become healthy with Postgres reachable");

    // Given: baseline — both signals healthy on both mounts.
    for port in [server.admin_port, server.rest_port] {
        let (status, _) = get(port, "/livez").await.expect("GET /livez baseline");
        assert_eq!(status, 200, "/livez must be 200 on port {port} while Postgres is reachable");
        let (status, _) = get(port, "/healthz").await.expect("GET /healthz baseline");
        assert_eq!(status, 200, "/healthz must be 200 on port {port} while Postgres is reachable");
    }

    // When: the shared system Postgres becomes completely unreachable.
    pg.stop().await.expect("stop system postgres");

    // Then: /livez never leaves 200 on either mount for the duration of the
    // outage — sampled repeatedly, not asserted once, so a liveness handler
    // that happens to succeed on the first poll but later starts touching
    // Postgres would still be caught.
    for _ in 0..5 {
        for port in [server.admin_port, server.rest_port] {
            let (status, _) = get(port, "/livez")
                .await
                .unwrap_or_else(|| panic!("GET /livez on port {port} must never fail to connect during a Postgres outage"));
            assert_eq!(
                status, 200,
                "/livez on port {port} must stay 200 during a Postgres outage — a restart-storm \
                 would follow if liveness ever depended on Postgres reachability"
            );
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    // And: /healthz turns unhealthy on both mounts within a bounded window.
    for port in [server.admin_port, server.rest_port] {
        let turned_unhealthy = wait_for_status(port, "/healthz", 503, Duration::from_secs(10)).await;
        assert!(
            turned_unhealthy,
            "/healthz on port {port} must turn 503 while the shared system Postgres is unreachable"
        );
    }

    // When: Postgres becomes reachable again.
    pg.start().await.expect("restart system postgres");

    // Then: /healthz recovers to healthy on both mounts.
    for port in [server.admin_port, server.rest_port] {
        let recovered = wait_for_status(port, "/healthz", 200, Duration::from_secs(15)).await;
        assert!(
            recovered,
            "/healthz on port {port} must recover to 200 once Postgres is reachable again"
        );
    }

    // And: /livez was never, at any point, anything other than 200.
    for port in [server.admin_port, server.rest_port] {
        let (status, _) = get(port, "/livez").await.expect("GET /livez post-recovery");
        assert_eq!(status, 200, "/livez on port {port} must remain 200 after recovery too");
    }

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-HDC-02 ──────────────────────────────────────────────────────────────

/// Liveness's own handler makes zero calls to Postgres or any external
/// dependency — verified structurally (source text of the handler function),
/// not by timing (a hung/slow Postgres could make a timing-only proof flaky
/// or, worse, falsely pass on a fast machine while the handler secretly calls
/// out). This mirrors ADR-078 § Enforcement's own recommended structural
/// guard.
///
/// Given: `crates/embyr-server/src/grpc/healthz.rs` defines `livez_handler`.
/// Then:  that function's own body contains no reference to `SystemDb`,
///        `sqlx`, `system_db`, or `.probe(` — no code path exists by which it
///        could reach Postgres.
///
/// @driving_port @AC-HDC-02
#[tokio::test]
async fn livez_handler_body_has_zero_postgres_or_external_calls() {
    let source = include_str!("../../../crates/embyr-server/src/grpc/healthz.rs");

    let start = source
        .find("fn livez_handler")
        .expect("livez_handler not found in healthz.rs — not yet implemented");

    // Extract from the function signature to the next top-level `pub async fn`
    // (or EOF) — a coarse but sufficient boundary for this single-file module.
    let after_start = &source[start..];
    let end = after_start[1..]
        .find("pub async fn")
        .map(|i| i + 1)
        .unwrap_or(after_start.len());
    let body = &after_start[..end];

    for forbidden in ["SystemDb", "sqlx", "system_db", ".probe(", "State<"] {
        assert!(
            !body.contains(forbidden),
            "livez_handler must never reference '{forbidden}' — liveness must have zero code \
             path to Postgres or any external dependency (restart-storm prevention, ADR-078); \
             found in:\n{body}"
        );
    }
}

// ─── AC-HDC-10 ──────────────────────────────────────────────────────────────

/// Readiness is scoped to the SHARED SYSTEM Postgres only — a dead PER-TENANT
/// customer database must never flip `/healthz` unhealthy for the whole pod.
///
/// Given: embyr-server is running with the shared system Postgres reachable,
///        and one tenant project provisioned against its OWN dedicated
///        customer Postgres.
/// When:  the SHARED SYSTEM Postgres itself is confirmed to actually drive
///        `/healthz` (a real outage flips it 503, a real recovery flips it
///        back) — chained-narrative reuse of the walking skeleton's own
///        proof, so THIS test cannot pass by accident against a still-
///        hardcoded handler.
/// And:   only the TENANT's customer Postgres then becomes unreachable (the
///        shared system Postgres is left fully up).
/// Then:  `/healthz` stays healthy — a single dead tenant database must not
///        remove the pod from serving every OTHER tenant.
///
/// @driving_port @real-io @AC-HDC-10
///
/// Uses the real subprocess `ServerProcess` harness (production `main.rs` +
/// `spawn_all_servers` wiring), NOT the in-process `start_test_server*`
/// helpers — DESIGN's own Reading Confirmation established that
/// `start_test_server`/`start_test_server_with_keepalive` never mount
/// `/healthz` at all (a pre-existing, explicitly out-of-scope gap). Using
/// that harness here would fail for the WRONG reason (route absent by
/// harness design, not by missing feature code) — the real subprocess binary
/// is the only harness in this workspace where `/healthz`/`/livez` actually
/// exist.
///
/// Deliberately proves the SYSTEM-db-flips-it-unhealthy fact first (not just
/// the scope guard alone) — a scope-only assertion would trivially pass
/// today against the current hardcoded-200 handler with zero production
/// change (Fixture Theater: the test would be green while testing nothing).
#[tokio::test]
async fn healthz_stays_healthy_when_only_a_tenant_customer_database_is_down() {
    let (sys_pg, sys_url) = start_postgres_fixed_port().await;

    let mut server = ServerProcess::start(
        &sys_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(healthy, "server did not become healthy with system Postgres reachable");

    // Prove the check is real before proving the scope guard: a SYSTEM db
    // outage must flip /healthz unhealthy, and recovery must flip it back.
    sys_pg.stop().await.expect("stop system postgres");
    let turned_unhealthy = wait_for_status(server.admin_port, "/healthz", 503, Duration::from_secs(10)).await;
    assert!(
        turned_unhealthy,
        "/healthz must turn 503 when the SHARED SYSTEM Postgres is down — this must be proven \
         before the scope guard below means anything"
    );
    sys_pg.start().await.expect("restart system postgres");
    let recovered = wait_for_status(server.admin_port, "/healthz", 200, Duration::from_secs(15)).await;
    assert!(recovered, "/healthz must recover to 200 once the system Postgres is reachable again");

    // A dedicated, independently stoppable customer Postgres for one tenant.
    let (cust_pg, cust_url) = start_postgres_fixed_port().await;
    let cust_pool = sqlx::PgPool::connect(&cust_url).await.expect("customer pool connect");
    sqlx::migrate!("../../migrations/customer")
        .run(&cust_pool)
        .await
        .expect("customer migrate");
    drop(cust_pool);

    // Provision the tenant via the real admin API (same shape as pr04's
    // in-flight test) — proves the scope guard against a project the server
    // actually knows about, not merely an unregistered database.
    let admin_base = format!("http://127.0.0.1:{}", server.admin_port);
    let http = reqwest::Client::new();
    let provision_resp = http
        .post(format!("{admin_base}/admin/v1/projects"))
        .header("Authorization", "Bearer testkey")
        .json(&serde_json::json!({
            "project_id": "hdc-tenant-down",
            "dsn": cust_url,
            "backend_mode": "direct_pg",
        }))
        .send()
        .await
        .expect("POST /admin/v1/projects request failed");
    assert!(
        provision_resp.status().is_success(),
        "provisioning the tenant project must succeed before this test can prove the scope guard"
    );

    // Given: readiness healthy with everything up.
    let (status, _) = get(server.admin_port, "/healthz").await.expect("GET /healthz baseline");
    assert_eq!(status, 200, "/healthz must be healthy before the tenant DB is stopped");

    // When: only the TENANT's customer Postgres goes down.
    cust_pg.stop().await.expect("stop tenant customer postgres");
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Then: /healthz is unaffected — it never touches per-tenant databases.
    let (status, body) = get(server.admin_port, "/healthz")
        .await
        .expect("GET /healthz while tenant DB is down");
    assert_eq!(
        status, 200,
        "/healthz must stay healthy when only a single TENANT's customer database is \
         unreachable — the shared system Postgres, which readiness actually checks, is still \
         fully up; body: {body}"
    );

    cust_pg.start().await.expect("restart tenant customer postgres for cleanup");
    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-HDC-11 ──────────────────────────────────────────────────────────────

/// The unhealthy (503) response body from `/healthz` is a fixed, generic
/// string — it never contains raw Postgres driver text, schema/table names,
/// or `CoreError`'s own `Display` text. This is a NEW leak-surface boundary
/// ADR-075's tonic-only sanitization sweep never covered (HTTP/axum JSON
/// response, not a `tonic::Status` conversion site).
///
/// Given: embyr-server is running with the shared system Postgres reachable.
/// When:  the shared system Postgres becomes unreachable and `/healthz` is
///        polled until it reports 503.
/// Then:  the 503 response body contains none of `SystemDb::probe()`'s own
///        raw error fragments ("system DB unreachable", "schema check
///        failed", "information_schema", the underlying `sqlx`/`tokio-postgres`
///        driver error text) — a fixed, generic body only.
///
/// @driving_port @real-io @AC-HDC-11
#[tokio::test]
async fn healthz_503_body_never_leaks_postgres_driver_or_schema_error_text() {
    let (pg, db_url) = start_postgres_fixed_port().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(healthy, "server did not become healthy with Postgres reachable");

    pg.stop().await.expect("stop system postgres");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut unhealthy_body = None;
    while tokio::time::Instant::now() < deadline {
        if let Some((503, body)) = get(server.admin_port, "/healthz").await {
            unhealthy_body = Some(body);
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let body = unhealthy_body.expect("/healthz never reported 503 during the outage");
    let body_lower = body.to_lowercase();

    // Exact raw fragments SystemDb::probe() embeds today (system_db.rs:314,322,326).
    for leak in [
        "system db unreachable",
        "schema check failed",
        "information_schema",
        "projects table missing",
        "connection refused",
        "backendunavailable",
    ] {
        assert!(
            !body_lower.contains(leak),
            "503 body must never contain raw driver/schema error text ('{leak}' found) — \
             this reopens finding #10's leak class at a new HTTP boundary ADR-075 never \
             covered; body was: {body}"
        );
    }
    assert!(
        body.len() < 200,
        "503 body should be a short, fixed generic string, not a rendered error chain \
         (got {} bytes): {body}",
        body.len()
    );

    pg.start().await.expect("restart system postgres for cleanup");
    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}
