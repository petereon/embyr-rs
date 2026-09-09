// @driving_port @real-io @adapter-integration @infrastructure-failure @US-01
#![allow(dead_code)]
//! realtime-listener-reconnect (US-01) — a transient Postgres connection
//! failure against a project's real-time listener no longer permanently and
//! silently ends that project's real-time delivery, does not leak or
//! accumulate connection resources, does not cause unbounded reconnect
//! attempts against Postgres, and — if failure is sustained well beyond a
//! normal transient blip — becomes visible to the operator.
//!
//! Acceptance criteria verified here (feature-delta.md, DISCUSS+DESIGN;
//! ADR-071):
//!   AC-RLR-01: a transient connection failure against a real Postgres
//!              instance recovers automatically — the listener resumes
//!              delivering `DocumentChange` events without a new `Listen`
//!              RPC, client action, or operator action. This is the
//!              feature's own walking skeleton AND — per ADR-071 § Peer
//!              Review's "High" finding resolution — the empirical
//!              confirmation that `sqlx`'s own `PgListener` auto-reconnects
//!              transparently on the SAME binding (ADR-071 § Context).
//!   AC-RLR-02: repeated reconnect cycles do not accumulate dangling
//!              background tasks/connections that duplicate delivery of the
//!              same committed write.
//!   AC-RLR-03: during a sustained outage, reconnect attempts are spaced by
//!              a capped backoff (1s -> 30s exponential doubling, ADR-071),
//!              not a tight loop — verified via the reconnect-attempt
//!              metric's count after a fixed wait window, not by sleeping
//!              and assuming.
//!   AC-RLR-04: after a listener has been failing to reconnect beyond the
//!              sustained-failure threshold (5 consecutive failures,
//!              ADR-071's `RECONNECT_ALERT_THRESHOLD`), an operator-visible
//!              signal distinguishes this state from a healthy listener —
//!              and a second, unrelated project's listener (dedicated
//!              connection per project, AD-08) is unaffected.
//!   AC-RLR-05 (regression guard): a project's real-time delivery is not
//!              permanently blocked once connectivity is resolved, even
//!              after the outage crossed the sustained-failure threshold —
//!              no full-process restart required.
//!   AC-RLR-06 (regression guard) is NOT a new test here — the existing
//!              non-failure-path suite (`tests/acceptance/us_05_listen_realtime.rs`)
//!              already proves a listener that never fails is unaffected by
//!              this feature; DESIGN's own Handoff Package names it as the
//!              regression guard to keep green, not to duplicate.
//!   AC-RLR-07 (full regression suite) is NOT a new test here — the
//!              orchestrator runs the full workspace suite once, at the
//!              pre-commit gate (mirrors pr07's own AC-WBL-05 treatment).
//!
//! Mechanism under test (ADR-071, locked): `PostgresNotifyListener::start()`'s
//! spawned-task loop (`postgres_notify_listener.rs`, today's lines 57-72)
//! stops `break`ing on `recv()` error; it applies a capped exponential
//! backoff sleep and calls `.recv()` again on the SAME `pg_listener`
//! binding — `sqlx`'s own `PgListener` transparently reconnects and
//! re-`LISTEN`s internally. No new `PgListener`, pool, or connection is
//! ever created for a reconnect cycle.
//!
//! Postgres disruption mechanism: `ContainerAsync::stop()` then `::start()`
//! on the SAME testcontainer (not `rm` + recreate) — a genuine TCP-level
//! connection failure and recovery against a real Postgres instance, per
//! this session's own "real, not mocked" proof standard (DoR item 1). Docker
//! preserves the container's host port mapping across stop/start, so the
//! already-resolved customer DSN stays valid after recovery. No established
//! "pause Postgres" helper exists yet in this workspace to reuse
//! (`tests/distributed_rate_limiting/acceptance/b13_fallback_on_pg_failure.rs`
//! is itself still an unimplemented `todo!()` scaffold with no working
//! mechanism) — stop/start is written fresh here, the smallest real,
//! non-mocked mechanism available via `testcontainers-rs` (0.21.1, this
//! workspace's pinned version) without introducing a new dependency
//! (toxiproxy, etc.) for a single feature.
//!
//! Layer: WS/`@wiring_e2e` (real Postgres container, real in-process gRPC +
//! admin server, ~seconds each) — per `nw-test-design-mandates` Layered Test
//! Discipline, this layer uses traditional assertions (not
//! `assert_state_delta`), example-only (no PBT), matching this same file
//! family's own established style (pr01-pr07).
//!
//! Why an in-process `embyr_server::TestServer` (mirrors
//! `tests/acceptance/us_05_listen_realtime.rs`) rather than this directory's
//! own subprocess `ServerProcess` harness: this feature's own driving port
//! is the existing `Listen` streaming RPC plus the existing `/metrics`
//! admin endpoint, both already exercised end-to-end by the in-process
//! harness with zero new process-spawn overhead; the subprocess harness
//! exists for this suite's OTHER features (env-var config, TLS, Docker,
//! graceful shutdown) where the actual OS process boundary is the thing
//! under test — it is not, here. `TestServer` already exposes `admin_addr`
//! for the same `/metrics` scrape convention `b15_project_id_metric_label_cardinality.rs`
//! established (`GET /metrics` with the fixed test operator Bearer key).
//!
//! Scaffold state: NONE created by this DISTILL pass — no new production
//! module is imported (this feature's own fix lives entirely inside
//! `postgres_notify_listener.rs`'s spawned-task closure, which already
//! compiles and links). All 5 tests are expected to FAIL against today's
//! code for the right reason (the listener task dies permanently on the
//! first `recv()` error and never resumes; the two new metrics do not yet
//! exist), not a test-setup bug.
//!
//! First test enabled (not `#[ignore]`) is the walking skeleton
//! (AC-RLR-01) — DELIVER unskips the remaining 4 one at a time.

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    listen_request, listen_response,
    target::{self, query_target},
    structured_query::CollectionSelector,
    CreateDocumentRequest, Document, ListenRequest, ListenResponse, StructuredQuery, Target,
    target_change::TargetChangeType,
};
use embyr_server::adapters::system_db::SystemDb;
use std::{collections::HashMap, sync::Arc, time::Duration};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{
        core::IntoContainerPort, runners::AsyncRunner, ContainerAsync, ImageExt,
    },
};
use tokio_stream::StreamExt;

use crate::common::find_free_port;

/// Fixed operator Bearer key every `start_test_server_with_*` variant
/// configures the admin router with (`crates/embyr-server/src/lib.rs`) —
/// same constant `b15_project_id_metric_label_cardinality.rs` uses.
const ADMIN_KEY: &str = "test-admin-key-secret";

// ─── Setup (shared across all scenarios — Pillar 2) ───────────────────────────

/// Fixed (not ephemeral) host port mapping — required so the customer DSN
/// stays valid across `stop()`/`start()`. Docker only preserves a
/// container's host port mapping across a restart when the mapping was
/// created with a FIXED host port; an ephemeral/dynamic mapping (Docker's
/// default, and this file's own earlier draft) is reassigned to a NEW
/// random host port on every `start()` after a `stop()` — verified
/// empirically against this sandbox's own Docker daemon during DISTILL
/// (a real Postgres restart never changes its own host:port in production;
/// this is a testcontainers/Docker ephemeral-port artifact this test must
/// route around, not a real-world scenario this feature needs to handle).
async fn start_postgres() -> (ContainerAsync<Postgres>, String) {
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

/// A single provisioned project, backed by its OWN dedicated customer
/// Postgres instance (not merely a separate schema) — proving AD-08's
/// "dedicated connection per project" isolation requires genuinely separate
/// Postgres instances, not just separate rows.
struct ProjectFixture {
    cust_container: ContainerAsync<Postgres>,
    cust_url: String,
    project_id: String,
    api_key: String,
}

struct TestEnv {
    _sys_container: ContainerAsync<Postgres>,
    server: embyr_server::TestServer,
}

/// Start one system-DB-backed `embyr-server` instance and provision one
/// project per `(project_id, api_key)` pair, each with its own dedicated
/// customer Postgres container.
async fn setup(projects: &[(&str, &str)]) -> (TestEnv, Vec<ProjectFixture>) {
    let (sys_container, sys_url) = start_postgres().await;
    let system_db = Arc::new(SystemDb::new(&sys_url).await.expect("system db connect"));
    system_db.migrate().await.expect("system db migrate");
    let sys_pool = sqlx::PgPool::connect(&sys_url).await.expect("system pool connect");

    let mut fixtures = Vec::with_capacity(projects.len());
    for (project_id, api_key) in projects {
        let (cust_container, cust_url) = start_postgres().await;
        let cust_pool = sqlx::PgPool::connect(&cust_url).await.expect("customer pool connect");
        sqlx::migrate!("../../migrations/customer")
            .run(&cust_pool)
            .await
            .expect("customer migrate");

        let api_key_hash = argon2::hash_api_key(api_key.as_bytes()).expect("hash api key");
        let pub_key = ecies::derive_public_key(api_key.as_bytes());
        let encrypted_dsn = ecies::encrypt(&pub_key, cust_url.as_bytes()).expect("encrypt dsn");

        sqlx::query(
            "INSERT INTO projects \
             (id, status, backend_mode, api_key_hash_current, ecies_encrypted_dsn) \
             VALUES ($1, 'active', 'direct_pg', $2, $3)",
        )
        .bind(*project_id)
        .bind(&api_key_hash)
        .bind(&encrypted_dsn)
        .execute(&sys_pool)
        .await
        .expect("insert project row");

        fixtures.push(ProjectFixture {
            cust_container,
            cust_url,
            project_id: project_id.to_string(),
            api_key: api_key.to_string(),
        });
    }

    let server =
        embyr_server::start_test_server_with_keepalive(system_db, Duration::from_millis(500)).await;

    (TestEnv { _sys_container: sys_container, server }, fixtures)
}

fn make_channel(addr: std::net::SocketAddr) -> tonic::transport::Channel {
    tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .expect("valid gRPC channel URI")
        .connect_lazy()
}

fn add_target_request(project_id: &str, collection: &str) -> ListenRequest {
    ListenRequest {
        database: format!("projects/{project_id}/databases/(default)"),
        target_change: Some(listen_request::TargetChange::AddTarget(Target {
            target_id: 1,
            target_type: Some(target::TargetType::Query(target::QueryTarget {
                parent: format!("projects/{project_id}/databases/(default)/documents"),
                query_type: Some(query_target::QueryType::StructuredQuery(StructuredQuery {
                    from: vec![CollectionSelector {
                        collection_id: collection.to_string(),
                        all_descendants: false,
                    }],
                    ..Default::default()
                })),
            })),
            ..Default::default()
        })),
        ..Default::default()
    }
}

async fn seed_document(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    collection: &str,
    doc_id: &str,
) {
    let parent = format!("projects/{project_id}/databases/(default)/documents");
    let mut req = tonic::Request::new(CreateDocumentRequest {
        parent,
        collection_id: collection.to_string(),
        document_id: doc_id.to_string(),
        document: Some(Document { name: String::new(), fields: HashMap::new(), ..Default::default() }),
        ..Default::default()
    });
    req.metadata_mut()
        .insert("authorization", format!("bearer {api_key}").parse().expect("valid header"));
    client.create_document(req).await.expect("seed document should succeed");
}

/// Open a Listen stream with an `AddTarget` for `collection` and drain
/// messages until the initial-snapshot `CURRENT` marker, returning the
/// stream ready to observe subsequent `DocumentChange` events.
async fn open_listen_stream_and_wait_for_current(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    collection: &str,
) -> tonic::Streaming<ListenResponse> {
    let req_stream = tokio_stream::once(add_target_request(project_id, collection));
    let mut request = tonic::Request::new(req_stream);
    request
        .metadata_mut()
        .insert("authorization", format!("bearer {api_key}").parse().expect("valid header"));

    let mut stream = client.listen(request).await.expect("listen should succeed").into_inner();

    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("timed out waiting for CURRENT")
            .expect("stream ended before CURRENT")
            .expect("stream error before CURRENT");
        if let Some(listen_response::ResponseType::TargetChange(tc)) = msg.response_type {
            let change_type = TargetChangeType::try_from(tc.target_change_type)
                .unwrap_or(TargetChangeType::NoChange);
            if change_type == TargetChangeType::Current {
                return stream;
            }
        }
    }
}

/// Repeatedly seed a freshly-named document and wait a short window for its
/// delivery, until one succeeds or `overall_deadline` elapses.
///
/// Postgres NOTIFY is fire-and-forget with no replay: a write committed in
/// the narrow window between "the customer Postgres port is reachable again"
/// (all this test file's own `wait_until_reachable` can observe) and "this
/// project's listener has actually re-issued `LISTEN`" (an internal
/// reconnect-loop state with no external signal below the sustained-failure
/// alert threshold) is lost forever, no matter how long the test then waits
/// for it. A single write+wait pair is therefore an inherently flaky way to
/// test "the listener recovers" — retrying the WRITE itself (not just the
/// wait) is what actually tests that claim robustly, without weakening it:
/// the assertion is still "a write eventually gets delivered once the
/// listener recovers," not "the very first write after an approximate
/// external reachability check is guaranteed delivered."
async fn seed_until_delivered(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    stream: &mut tonic::Streaming<ListenResponse>,
    project_id: &str,
    api_key: &str,
    collection: &str,
    doc_id_prefix: &str,
    overall_deadline: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + overall_deadline;
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        seed_document(client, project_id, api_key, collection, &format!("{doc_id_prefix}-{attempt}")).await;
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let per_attempt_wait = remaining.min(Duration::from_secs(3));
        if next_document_change(stream, per_attempt_wait).await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
    }
}

/// Wait up to `timeout` for the next `DocumentChange` event on `stream`,
/// ignoring other message types (e.g. keep-alive `NO_CHANGE`).
async fn next_document_change(stream: &mut tonic::Streaming<ListenResponse>, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(Ok(msg))) => {
                if let Some(listen_response::ResponseType::DocumentChange(_)) = msg.response_type {
                    return true;
                }
            }
            _ => return false,
        }
    }
}

/// Drain `stream` for `window` and count how many `DocumentChange` events
/// arrive — used to detect double-delivery from accumulated dangling tasks.
async fn count_document_changes_within(stream: &mut tonic::Streaming<ListenResponse>, window: Duration) -> usize {
    let deadline = tokio::time::Instant::now() + window;
    let mut count = 0usize;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return count;
        }
        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(Ok(msg))) => {
                if let Some(listen_response::ResponseType::DocumentChange(_)) = msg.response_type {
                    count += 1;
                }
            }
            _ => return count,
        }
    }
}

/// Poll `db_url` with a fresh `sqlx::PgPool::connect` until it succeeds or
/// `timeout` elapses — used after restarting a customer Postgres container
/// to confirm it has finished coming back up before proceeding.
async fn wait_until_reachable(db_url: &str, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if sqlx::PgPool::connect(db_url).await.is_ok() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("customer Postgres container did not become reachable again within {timeout:?}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// GET /metrics with the fixed test operator key — same convention as
/// `b15_project_id_metric_label_cardinality.rs`'s own `get_metrics`.
async fn get_metrics(admin_client: &reqwest::Client, admin_addr: std::net::SocketAddr) -> String {
    admin_client
        .get(format!("http://{admin_addr}/metrics"))
        .header("Authorization", format!("Bearer {ADMIN_KEY}"))
        .send()
        .await
        .expect("GET /metrics request failed")
        .text()
        .await
        .expect("GET /metrics body read failed")
}

/// Extract the numeric value of `metric_name{project_id="<project_id>",...}`
/// from a scraped Prometheus text body. Returns `None` if the metric/label
/// combination is absent (a metric that has never fired yet is legitimately
/// absent in Prometheus text format, not zero).
fn metric_value_for_label(body: &str, metric_name: &str, project_id: &str) -> Option<f64> {
    let name_prefix = format!("{metric_name}{{");
    let label_needle = format!("project_id=\"{project_id}\"");
    for line in body.lines() {
        if line.starts_with('#') || !line.starts_with(&name_prefix) {
            continue;
        }
        if !line.contains(&label_needle) {
            continue;
        }
        return line.rsplit(' ').next().and_then(|v| v.parse::<f64>().ok());
    }
    None
}

// ─── AC-RLR-01 (walking skeleton) ──────────────────────────────────────────────

/// A transient Postgres blip does not permanently end real-time delivery.
///
/// This test also serves as the empirical confirmation of ADR-071's central
/// technical claim (`sqlx`'s own `PgListener` auto-reconnects transparently
/// on the same binding) — see ADR-071 § Peer Review's "High" finding
/// resolution.
///
/// Given: Trailmark's project has an active onSnapshot listener receiving
///   live updates.
/// When: the underlying Postgres connection for that listener drops for a
///   few seconds and then recovers.
/// Then: the listener automatically resumes delivering DocumentChange
///   events — no new Listen RPC, client action, or operator action was
///   required to resume delivery.
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-RLR-01
#[tokio::test]
async fn transient_postgres_blip_does_not_permanently_end_realtime_delivery() {
    let (env, mut fixtures) = setup(&[("trailmark-prod", "test-sk-trailmark-01")]).await;
    let trailmark = fixtures.remove(0);

    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));
    let mut listen_stream = open_listen_stream_and_wait_for_current(
        &mut client,
        &trailmark.project_id,
        &trailmark.api_key,
        "dashboard",
    )
    .await;

    // When: the customer's Postgres connection drops for a few seconds
    // (US-01 Example 1) and then recovers.
    trailmark.cust_container.stop().await.expect("stop customer postgres");
    tokio::time::sleep(Duration::from_secs(2)).await;
    trailmark.cust_container.start().await.expect("restart customer postgres");
    wait_until_reachable(&trailmark.cust_url, Duration::from_secs(15)).await;

    // Then: without any new Listen RPC or client/operator action, a write
    // committed after recovery is eventually delivered on the SAME open
    // stream. `wait_until_reachable` only confirms the customer Postgres
    // PORT is reachable again — it says nothing about whether THIS
    // project's listener has finished its own internal reconnect (no
    // external signal exists for that below the sustained-failure alert
    // threshold), and a write committed in that gap is lost forever (NOTIFY
    // has no replay). `seed_until_delivered` retries the WRITE itself
    // (never just re-waiting on the same one) so this test robustly proves
    // "the listener recovers," not "the very first write after an
    // approximate external check is always delivered."
    assert!(
        seed_until_delivered(
            &mut client,
            &mut listen_stream,
            &trailmark.project_id,
            &trailmark.api_key,
            "dashboard",
            "post-blip-doc",
            Duration::from_secs(20),
        )
        .await,
        "expected the SAME Listen stream to eventually deliver a DocumentChange event for a \
         write committed after the Postgres blip resolved — real-time delivery must self-heal, \
         not die permanently on a transient blip"
    );
}

// ─── AC-RLR-02 ──────────────────────────────────────────────────────────────

/// Repeated transient blips do not accumulate dangling background tasks or
/// connections that duplicate delivery of the same committed write.
///
/// Given: Trailmark's project has an active onSnapshot listener.
/// When: the underlying Postgres connection drops and recovers 3 times in a
///   row (reduced from DISCUSS's illustrative 20 for wall-clock cost — the
///   accumulation failure mode this AC targets, N dangling tasks all still
///   fanning out, is already fully exposed by 3 cycles: a single surviving
///   extra task from ANY prior cycle would double-deliver every subsequent
///   write).
/// Then: a single write committed after the 3rd recovery produces EXACTLY
///   ONE DocumentChange event — not one per accumulated dangling task.
///
/// @driving_port @real-io @US-01 @AC-RLR-02
#[tokio::test]
async fn repeated_transient_blips_do_not_duplicate_delivered_changes() {
    let (env, mut fixtures) = setup(&[("trailmark-flapping", "test-sk-trailmark-02")]).await;
    let trailmark = fixtures.remove(0);
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));
    let mut listen_stream = open_listen_stream_and_wait_for_current(
        &mut client,
        &trailmark.project_id,
        &trailmark.api_key,
        "flap",
    )
    .await;

    for cycle in 1..=3 {
        trailmark
            .cust_container
            .stop()
            .await
            .unwrap_or_else(|e| panic!("stop cycle {cycle}: {e}"));
        tokio::time::sleep(Duration::from_millis(500)).await;
        trailmark
            .cust_container
            .start()
            .await
            .unwrap_or_else(|e| panic!("start cycle {cycle}: {e}"));
        wait_until_reachable(&trailmark.cust_url, Duration::from_secs(15)).await;
    }

    // `wait_until_reachable` only confirms EXTERNAL reachability (a fresh,
    // standalone connect) — it says nothing about whether THIS listener's
    // own reconnect loop has caught up yet. A reconnect attempt mid-flight
    // when a cycle's blip started can itself take up to
    // RECONNECT_ATTEMPT_TIMEOUT (5s) before erroring, so settle generously
    // before seeding rather than assuming the listener is already caught up
    // the instant external reachability returns.
    tokio::time::sleep(Duration::from_secs(10)).await;

    seed_document(&mut client, &trailmark.project_id, &trailmark.api_key, "flap", "post-flap-doc").await;

    let received = count_document_changes_within(&mut listen_stream, Duration::from_secs(15)).await;
    assert_eq!(
        received, 1,
        "expected exactly 1 DocumentChange for 1 committed write after 3 reconnect cycles \
         — {received} events means dangling listener tasks from earlier cycles are still \
         fanning out the same NOTIFY"
    );
}

// ─── AC-RLR-03 ──────────────────────────────────────────────────────────────

/// During a sustained outage, reconnect attempts are spaced by a capped
/// backoff, not a tight loop.
///
/// Given: Trailmark's Postgres instance is unavailable for an extended
///   period.
/// When: the listener repeatedly attempts to reconnect during that outage.
/// Then: the reconnect-attempt count observed via
///   `embyr_pg_notify_listener_reconnect_attempts_total{project_id}` after a
///   fixed wait window is small and bounded — a tight retry loop would
///   instead produce hundreds/thousands of increments in the same window.
///
/// @driving_port @real-io @US-01 @AC-RLR-03
#[tokio::test]
async fn sustained_outage_reconnect_attempts_are_bounded_not_a_tight_loop() {
    let (env, mut fixtures) = setup(&[("trailmark-backoff", "test-sk-trailmark-03")]).await;
    let trailmark = fixtures.remove(0);
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));
    let _listen_stream = open_listen_stream_and_wait_for_current(
        &mut client,
        &trailmark.project_id,
        &trailmark.api_key,
        "outage",
    )
    .await;

    trailmark.cust_container.stop().await.expect("stop customer postgres");

    // Backoff curve (ADR-071's `reconnect_backoff`): 1s, 2s, 4s, 8s, ... —
    // after 9s of sustained outage, at most a handful of reconnect attempts
    // should have fired.
    tokio::time::sleep(Duration::from_secs(9)).await;

    let admin_client = reqwest::Client::new();
    let body = get_metrics(&admin_client, env.server.admin_addr).await;
    let attempts = metric_value_for_label(
        &body,
        "embyr_pg_notify_listener_reconnect_attempts_total",
        &trailmark.project_id,
    )
    .unwrap_or(0.0);

    assert!(
        attempts >= 1.0,
        "expected at least 1 reconnect attempt to be recorded during a 9s outage, got {attempts}"
    );
    assert!(
        attempts <= 6.0,
        "expected reconnect attempts to be bounded by exponential backoff (<=6 in 9s), \
         got {attempts} — this many attempts in 9s indicates a tight retry loop, not \
         capped backoff"
    );

    trailmark.cust_container.start().await.expect("restart customer postgres for cleanup");
}

// ─── AC-RLR-04 + per-project isolation (AD-08) ─────────────────────────────────

/// Sustained failure of ONE project's listener becomes operator-visible and
/// distinguishable from a healthy listener, and does NOT affect a second,
/// unrelated project's real-time delivery (dedicated connection per
/// project, AD-08).
///
/// Given: two provisioned projects, each with its own dedicated customer
///   Postgres instance and an active onSnapshot listener.
/// When: only the first project's Postgres instance is down long enough to
///   cross the sustained-failure threshold (5 consecutive failures,
///   ADR-071's `RECONNECT_ALERT_THRESHOLD`).
/// Then: the down project's `embyr_pg_notify_listener_reconnecting` gauge
///   reads 1 while the second, healthy project's gauge reads 0/absent.
/// And: the second project's Listen stream still delivers a normal write
///   with no observable disruption from the first project's outage.
///
/// @driving_port @real-io @US-01 @AC-RLR-04
#[tokio::test]
async fn sustained_failure_of_one_project_is_operator_visible_and_does_not_affect_a_second_project() {
    let (env, mut fixtures) = setup(&[
        ("trailmark-down", "test-sk-trailmark-down"),
        ("acme-healthy", "test-sk-acme-healthy"),
    ])
    .await;
    let acme = fixtures.remove(1);
    let trailmark = fixtures.remove(0);

    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));
    let mut trailmark_stream = open_listen_stream_and_wait_for_current(
        &mut client,
        &trailmark.project_id,
        &trailmark.api_key,
        "billing",
    )
    .await;
    let mut acme_stream =
        open_listen_stream_and_wait_for_current(&mut client, &acme.project_id, &acme.api_key, "billing").await;

    trailmark.cust_container.stop().await.expect("stop trailmark's postgres");

    // Wait past the 5-consecutive-failure sustained-failure threshold.
    // Worst case is NOT just the 1+2+4+8=15s backoff-sleep sum (ADR-071's
    // own illustrative estimate) — while the target is genuinely down
    // (not merely slow), each of the 4 reconnect attempts before the 5th
    // failure can itself consume the full RECONNECT_ATTEMPT_TIMEOUT (5s)
    // before its backoff sleep even starts (empirically confirmed: a
    // stopped testcontainers Postgres can leave a connect() attempt
    // "blackholed" rather than fast-failing with ECONNREFUSED). Worst case:
    // 4 attempts * 5s + (2+4+8)s backoff between them = 20 + 14 = 34s.
    // 60s gives real margin over that (not the optimistic 15s figure) plus
    // headroom for scheduling jitter under load.
    tokio::time::sleep(Duration::from_secs(60)).await;

    let admin_client = reqwest::Client::new();
    let body = get_metrics(&admin_client, env.server.admin_addr).await;

    let down_gauge = metric_value_for_label(&body, "embyr_pg_notify_listener_reconnecting", &trailmark.project_id)
        .unwrap_or(0.0);
    let healthy_gauge = metric_value_for_label(&body, "embyr_pg_notify_listener_reconnecting", &acme.project_id)
        .unwrap_or(0.0);

    assert_eq!(
        down_gauge, 1.0,
        "expected the down project's reconnecting gauge to read 1 after crossing the \
         sustained-failure threshold — Sam Chen must never see the same signal for \
         silently-dead-for-hours as for healthy"
    );
    assert_eq!(
        healthy_gauge, 0.0,
        "expected the healthy project's reconnecting gauge to read 0/absent — \
         distinguishable at a glance from the down project"
    );

    // Isolation (AD-08): the healthy project's own dedicated listener is
    // unaffected by the other project's outage.
    seed_document(&mut client, &acme.project_id, &acme.api_key, "billing", "acme-doc-during-outage").await;
    assert!(
        next_document_change(&mut acme_stream, Duration::from_secs(5)).await,
        "expected the second, healthy project's Listen stream to keep delivering normally \
         while the first project's listener is failing — dedicated per-project connections \
         (AD-08) must isolate the two"
    );

    let _ = &mut trailmark_stream; // keep the failing project's stream alive for the test's duration
    trailmark.cust_container.start().await.expect("restart trailmark's postgres for cleanup");
}

// ─── AC-RLR-05 (regression guard) ───────────────────────────────────────────

/// A project's listener is not permanently blocked once its underlying
/// connectivity issue is resolved, even after the outage was long enough to
/// cross the operator-visible sustained-failure threshold.
///
/// Given: Trailmark's Postgres instance has been down long enough that its
///   listener already crossed the sustained-failure threshold.
/// When: the underlying connectivity issue is resolved.
/// Then: the SAME listener resumes delivering DocumentChange events — no
///   full `embyr-server` process restart, no new Listen RPC — and the
///   `embyr_pg_notify_listener_reconnecting` gauge resets to 0.
///
/// @driving_port @real-io @US-01 @AC-RLR-05
#[tokio::test]
async fn listener_recovers_after_crossing_the_sustained_failure_threshold() {
    let (env, mut fixtures) = setup(&[("trailmark-recovers", "test-sk-trailmark-05")]).await;
    let trailmark = fixtures.remove(0);
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));
    let mut listen_stream = open_listen_stream_and_wait_for_current(
        &mut client,
        &trailmark.project_id,
        &trailmark.api_key,
        "resize",
    )
    .await;

    trailmark.cust_container.stop().await.expect("stop customer postgres");
    // 60s, not 20/45s: wait long enough that the listener has settled into
    // the CAPPED 30s steady-state backoff (consecutive_failures >= 6), not
    // merely crossed the 5-failure alert threshold. Reaching cf=5 alone can
    // take up to ~34s worst case (4 attempts * RECONNECT_ATTEMPT_TIMEOUT=5s
    // plus 2+4+8s backoff between them — see AC-RLR-04's own comment); one
    // more full backoff(5)=16s sleep plus another attempt lands around
    // ~55s. Once truly in the capped-30s steady state, the REMAINING
    // worst-case wait after connectivity resolves is bounded by that same
    // 30s cap (not an ever-growing tail), which is what the post-recovery
    // wait below needs to reason about.
    tokio::time::sleep(Duration::from_secs(60)).await;

    trailmark.cust_container.start().await.expect("restart customer postgres");
    wait_until_reachable(&trailmark.cust_url, Duration::from_secs(15)).await;

    // `wait_until_reachable` only confirms the customer Postgres PORT is
    // reachable again — a write committed before this project's listener
    // has actually finished its own reconnect (still possibly mid the
    // capped-30s steady-state backoff sleep) is lost forever (NOTIFY has no
    // replay). `seed_until_delivered` retries the WRITE itself rather than
    // re-waiting on one, so this robustly proves eventual recovery instead
    // of being sensitive to exactly when in its backoff cycle the listener
    // happened to be. 45s overall budget: worst case is connectivity
    // resolving the instant a capped 30s sleep begins, plus margin.
    assert!(
        seed_until_delivered(
            &mut client,
            &mut listen_stream,
            &trailmark.project_id,
            &trailmark.api_key,
            "resize",
            "post-recovery-doc",
            Duration::from_secs(45),
        )
        .await,
        "expected the SAME Listen stream to resume delivery after connectivity was restored, \
         even though the outage was long enough to cross the sustained-failure threshold — no \
         process restart, no new Listen RPC required"
    );

    let admin_client = reqwest::Client::new();
    let body = get_metrics(&admin_client, env.server.admin_addr).await;
    let gauge_after_recovery =
        metric_value_for_label(&body, "embyr_pg_notify_listener_reconnecting", &trailmark.project_id)
            .unwrap_or(0.0);
    assert_eq!(gauge_after_recovery, 0.0, "expected the reconnecting gauge to reset to 0 once delivery resumed");
}
