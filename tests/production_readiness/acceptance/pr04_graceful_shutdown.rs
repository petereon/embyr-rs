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
//! All tests are #[ignore] — run explicitly via `--ignored` (real-io suite).

use std::time::Duration;

use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    listen_request,
    listen_response::ResponseType,
    structured_query::CollectionSelector,
    target::{query_target::QueryType, QueryTarget, TargetType},
    target_change::TargetChangeType,
    ListenRequest, StructuredQuery, Target,
};
use tokio_stream::wrappers::ReceiverStream;

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
    assert!(
        healthy,
        "server must be healthy before testing graceful shutdown"
    );

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
/// Implementation:
///   Uses a real Listen gRPC stream as the "slow" request. `handle_add_target`
///   (crates/embyr-server/src/realtime/listen_handler.rs) never completes on
///   its own — after the initial snapshot it loops forever on a keepalive
///   timer / NOTIFY events / RESET — so the moment the stream is open, the
///   server-side handler task is *genuinely* still executing, with no
///   artificial delay added to production code.
///
///   Sequence:
///     1. Start server (ServerProcess::start) + provision a real project via
///        the admin API (Listen requires authentication to reach the loop).
///     2. Open a tonic gRPC Listen stream (AddTarget) — headers accepted,
///        but no response body message is read yet.
///     3. Sleep 500ms (stream is open and being serviced — in flight).
///     4. sigterm()
///     5. Read the CURRENT + NO_CHANGE handshake messages *after* SIGTERM —
///        must arrive, not error with a connection reset.
///     6. Close the client-side stream, then assert wait_for_exit() == Some(0).
///
/// @real-io @US-PR-01 @AC-PR-04-02
#[tokio::test]
#[ignore]
async fn in_flight_request_completes_before_shutdown() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );

    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(
        healthy,
        "server must be healthy before testing in-flight drain"
    );

    // Provisioning applies `migrations/customer` to the given DSN's database.
    // It cannot reuse the system DB's own database — sqlx tracks applied
    // migrations in a single `_sqlx_migrations` table per database, and the
    // system (`migrations/`) and customer (`migrations/customer/`) migration
    // sets both start at version 1, so applying both to one database
    // collides. Create a second, sibling database on the same Postgres
    // container for the project's document backend.
    let sys_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .expect("connect to system postgres to create customer database");
    sqlx::query("CREATE DATABASE pr04_customer")
        .execute(&sys_pool)
        .await
        .expect("create sibling customer database");
    drop(sys_pool);
    let last_slash = db_url
        .rfind('/')
        .expect("db_url must contain a path separator");
    let customer_db_url = format!("{}/pr04_customer", &db_url[..last_slash]);

    // Provision a real project (Listen requires authenticate() to succeed to
    // reach its unbounded handler loop) against the sibling customer database.
    let project_id = "pr04-inflight";
    let admin_base = format!("http://127.0.0.1:{}", server.admin_port);
    let http = reqwest::Client::new();
    let provision_resp = http
        .post(format!("{admin_base}/admin/v1/projects"))
        .header("Authorization", "Bearer testkey")
        .json(&serde_json::json!({
            "project_id": project_id,
            "dsn": customer_db_url,
            "backend_mode": "direct_pg",
        }))
        .send()
        .await
        .expect("POST /admin/v1/projects request failed");
    let provision_status = provision_resp.status();
    let provision_body_text = provision_resp
        .text()
        .await
        .expect("provision response body text");
    assert!(
        provision_status.is_success(),
        "POST /admin/v1/projects must return 2xx; got {provision_status}: {provision_body_text}"
    );
    let provisioned: serde_json::Value =
        serde_json::from_str(&provision_body_text).expect("provision response body must be JSON");
    let api_key = provisioned["api_key"]
        .as_str()
        .expect("provision response must include api_key")
        .to_string();

    // Open the Listen stream: send AddTarget, accept the RPC (headers), but
    // do NOT read any response body message before SIGTERM.
    let channel =
        tonic::transport::Channel::from_shared(format!("http://127.0.0.1:{}", server.grpc_port))
            .expect("valid gRPC channel URI")
            .connect_lazy();
    let mut client = FirestoreClient::new(channel);

    let target = Target {
        target_id: 1,
        once: false,
        target_type: Some(TargetType::Query(QueryTarget {
            parent: format!("projects/{project_id}/databases/(default)/documents"),
            query_type: Some(QueryType::StructuredQuery(StructuredQuery {
                from: vec![CollectionSelector {
                    collection_id: "pr04_col".to_string(),
                    all_descendants: false,
                }],
                ..Default::default()
            })),
        })),
        resume_type: None,
    };
    let add_target = ListenRequest {
        database: format!("projects/{project_id}/databases/(default)"),
        labels: Default::default(),
        target_change: Some(listen_request::TargetChange::AddTarget(target)),
    };

    let (req_tx, req_rx) = tokio::sync::mpsc::channel::<ListenRequest>(4);
    req_tx
        .send(add_target)
        .await
        .expect("send AddTarget message");

    let mut request = tonic::Request::new(ReceiverStream::new(req_rx));
    request.metadata_mut().insert(
        "authorization",
        format!("bearer {api_key}")
            .parse()
            .expect("valid metadata value"),
    );

    let response = client
        .listen(request)
        .await
        .expect("Listen RPC must be accepted (in-flight subscription established)");
    let mut inbound = response.into_inner();

    // The handler's loop is unbounded — this sleep is much shorter than the
    // call's (never-ending on its own) natural duration, and the request is
    // genuinely still executing server-side for the whole window.
    tokio::time::sleep(Duration::from_millis(500)).await;

    server.sigterm();

    // The client is deliberately still holding the Listen stream open at
    // this point. A shutdown that merely sleeps a fixed short duration and
    // exits regardless of open connections (rather than genuinely draining
    // in-flight requests) would have already exited by now. Assert the
    // process is still alive — it must wait for the still-open stream.
    let exit_before_close = server.wait_for_exit(Duration::from_secs(2)).await;
    assert_eq!(
        exit_before_close, None,
        "server exited before the in-flight Listen stream was closed by the client — \
         shutdown must drain genuinely open in-flight requests, not just sleep a fixed \
         duration regardless of connection state"
    );

    // Read the handshake responses AFTER SIGTERM was sent. If the server
    // aborts in-flight connections instead of draining them, this errors
    // with a transport/connection-reset failure instead of yielding the
    // expected messages.
    let mut got_current = false;
    let mut got_no_change_after_current = false;
    for _ in 0..5 {
        let msg = match inbound.message().await {
            Ok(Some(m)) => m,
            Ok(None) => break,
            Err(status) => panic!(
                "Listen stream errored after SIGTERM \
                 (expected graceful drain, got abort): {status}"
            ),
        };
        if let Some(ResponseType::TargetChange(tc)) = msg.response_type {
            match TargetChangeType::try_from(tc.target_change_type) {
                Ok(TargetChangeType::Current) => got_current = true,
                Ok(TargetChangeType::NoChange) if got_current => {
                    got_no_change_after_current = true;
                    break;
                }
                _ => {}
            }
        }
    }
    assert!(
        got_current && got_no_change_after_current,
        "expected CURRENT then NO_CHANGE to be delivered after SIGTERM \
         (in-flight request must complete, not be reset)"
    );

    // End the stream from the client side so the drained connection can
    // close and the server can finish exiting.
    drop(inbound);
    drop(req_tx);
    drop(client);

    let exit_code = server.wait_for_exit(Duration::from_secs(15)).await;
    assert_eq!(
        exit_code,
        Some(0),
        "server must exit 0 after draining the in-flight Listen request; got {exit_code:?}"
    );
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
    assert!(
        healthy,
        "server must be healthy before testing drain window"
    );

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
