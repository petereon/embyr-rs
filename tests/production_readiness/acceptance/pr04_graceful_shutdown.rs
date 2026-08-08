// @real-io @US-PR-01
//! US-PR-01 (graceful shutdown) — SIGTERM causes graceful drain before exit.
//!
//! Acceptance criteria verified here:
//!   AC-PR-04-01: SIGTERM causes graceful shutdown — server exits 0.
//!   AC-PR-04-02: An in-flight gRPC request completes before the server exits.
//!   AC-PR-04-03: SIGTERM drain completes within 30 seconds.
//!
//! Driving port: embyr-server subprocess + tonic gRPC client for the in-flight
//!               request scenario.
//!
//! All tests are #[ignore] — DELIVER unskips them one at a time.
//!
//! Scaffold classification target: RED (todo! panics; all tests compile).

use std::time::Duration;

use crate::common::{start_postgres_container, ServerProcess, TEST_ENCRYPTION_KEY};

// ─── AC-PR-04-01: SIGTERM → graceful exit 0 ──────────────────────────────────

/// SIGTERM causes the server to drain in-flight requests and exit with code 0.
///
/// Journey:
///   Given: embyr-server is running and healthy (GET /healthz returns 200)
///   When:  the OS sends SIGTERM to the embyr-server process
///   Then:  the server exits with code 0 within 10 seconds
///   And:   no new connections are accepted after SIGTERM
///
/// @real-io @US-PR-01 @AC-PR-04-01
#[tokio::test]
#[ignore]
async fn sigterm_causes_graceful_shutdown() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );

    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(healthy, "server must be healthy before testing graceful shutdown");

    server.sigterm();
    let exit_code = server.wait_for_exit(Duration::from_secs(10)).await;

    assert_eq!(
        exit_code,
        Some(0),
        "server must exit 0 after SIGTERM (graceful drain); got {exit_code:?}"
    );
}

// ─── AC-PR-04-02: in-flight gRPC request completes before shutdown ───────────

/// A gRPC request started before SIGTERM completes successfully.
///
/// Journey (chained from AC-PR-04-01):
///   Given: embyr-server is running and accepting gRPC connections
///   And:   a Firebase SDK client has issued a long-running gRPC request
///   When:  the OS sends SIGTERM mid-request
///   Then:  the in-flight gRPC request receives a response (not an error)
///   And:   the server exits 0 after the response is delivered
///
/// Implementation note for DELIVER:
///   Use a slow gRPC endpoint (e.g. Listen stream or a 2-second delayed GetDocument)
///   to ensure the request is in-flight when SIGTERM arrives.
///   Sequence:
///     1. Start server (ServerProcess::start)
///     2. Open tonic gRPC connection to grpc_port
///     3. Issue a request that takes >1s (e.g. ListenStream subscription)
///     4. Sleep 500ms (request in flight)
///     5. sigterm()
///     6. Assert: the gRPC response arrives (not a connection reset)
///     7. Assert: server.wait_for_exit() == Some(0)
///
/// @real-io @US-PR-01 @AC-PR-04-02
#[tokio::test]
#[ignore]
async fn in_flight_request_completes_before_shutdown() {
    todo!(
        "PR-04 DELIVER: \
         1. start server \
         2. issue slow gRPC request (Listen stream or simulated delay) \
         3. sigterm() while request is in flight \
         4. assert response arrives before server exits \
         5. assert exit code 0"
    )
}

// ─── AC-PR-04-03: drain completes within 30 seconds ─────────────────────────

/// The graceful shutdown drain window is bounded to 30 seconds (Kubernetes default).
///
/// Journey (chained from AC-PR-04-02):
///   Given: embyr-server is running with active connections
///   When:  SIGTERM is received
///   Then:  the server exits within 30 seconds
///   And:   exit code is 0 (drain completed, not killed by timeout)
///
/// @real-io @US-PR-01 @AC-PR-04-03
#[tokio::test]
#[ignore]
async fn sigterm_drains_within_30s() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );

    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(healthy, "server must be healthy before testing drain window");

    let t0 = tokio::time::Instant::now();
    server.sigterm();
    let exit_code = server.wait_for_exit(Duration::from_secs(35)).await;
    let elapsed = t0.elapsed();

    assert_eq!(
        exit_code,
        Some(0),
        "server must exit 0 after SIGTERM; got {exit_code:?}"
    );
    assert!(
        elapsed < Duration::from_secs(30),
        "server took {elapsed:?} to shut down — must drain within 30s (Kubernetes grace period)"
    );
}
