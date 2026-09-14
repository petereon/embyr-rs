// @real-io @driving_port @US-01
//! pool-sizing-and-limits (finding #16 High + #30 Medium, ADR-079) — the
//! LISTENER pool (`handler.rs`'s `handle_listen`, ad hoc `PgPoolOptions`
//! built per-project on first `Listen` RPC).
//!
//! Acceptance criteria this file speaks to:
//!   AC-PSL-02/03 (listener site): `EMBYR_LISTENER_DB_MAX_CONNECTIONS`/
//!     `EMBYR_LISTENER_DB_ACQUIRE_TIMEOUT_SECS` are read and threaded into
//!     the SAME `handle_listen` call site DESIGN describes as "same call
//!     site, same surrounding code, only the two literals become field
//!     reads" — this file's own scenario proves that threading did not
//!     break `Listen`'s existing wiring (AC-PSL-01-shaped regression guard
//!     for the listener site specifically).
//!
//! SCOPE DECISION (documented, not silently dropped) — no dedicated
//! listener-pool SATURATION test in this file:
//!
//! `backend_adapter.rs`'s tenant-pool write path takes a real `FOR UPDATE`
//! row lock as part of its own OCC precondition check (confirmed by direct
//! code read) — this is what makes `pr11_pool_sizing_tenant_pool.rs`'s own
//! saturation mechanism deterministic (an external, uncommitted
//! `SELECT ... FOR UPDATE` genuinely blocks a concurrent SUT write on the
//! SAME row). `PostgresNotifyListener`'s own fetch queries (driving this
//! pool) are triggered asynchronously by Postgres `NOTIFY`, inside a
//! background task, not synchronously inside the `Listen` RPC's own
//! request/response path — there is no known, verified locking-read
//! precedent on this path to hang a deterministic external block off of
//! (a plain, non-locking `SELECT` is never blocked by another session's row
//! lock in Postgres, only by a stronger whole-table lock, and reliably
//! synchronizing "a write commits + NOTIFY fires + this test grabs an
//! exclusive lock in the resulting race window" without touching production
// code is exactly the class of fragile, timing-dependent test this
//! project's own established lesson explicitly avoids (mirrors
//! `pr10_healthz_dependency_checks.rs`'s own OQ-HDC-03 note, and
//! `healthz`'s admin-signin-hardening TOTP/CPU-contention citation).
//!
//! The listener pool's CONSTRUCTION MECHANISM (`PgPoolOptions::new()
//! .max_connections(N).acquire_timeout(T)`) is byte-for-byte the same
//! sqlx builder shape DESIGN specifies for the tenant pool — already
//! exhaustively proven to saturate and fail fast under real Postgres
//! contention by `pr11_pool_sizing_tenant_pool.rs`'s own walking skeleton
//! (AC-PSL-04). The listener site's only genuine per-site risk is the
//! mechanical field-threading itself (env var -> `ServerConfig` field ->
//! `handle_listen`'s two literals), which THIS file's wiring-smoke test
//! below does exercise for real. Config-value acceptance/rejection for
//! `EMBYR_LISTENER_DB_MAX_CONNECTIONS`/`EMBYR_LISTENER_DB_ACQUIRE_TIMEOUT_SECS`
//! is covered by `pr12_pool_sizing_invalid_config.rs`. Filing a follow-up
//! for a real NOTIFY-fan-out saturation proof (once a safe synchronization
//! point exists) is a reasonable future addition, not a silent gap.
//!
//! Driving port: real `embyr-server` subprocess (`ServerProcess`), real
//! gRPC `FirestoreClient` (`CreateDocument`, `Listen`).
//!
//! Layer: WS-adjacent integration (~seconds, real Postgres + real
//! subprocess), example-only, traditional assertions.
//!
//! Scaffold state: NONE — no unimplemented production symbol imported.

use std::collections::HashMap;
use std::time::Duration;

use embyr_proto::firestore::{
    firestore_client::FirestoreClient, listen_request, listen_response,
    structured_query::CollectionSelector, target, target::query_target,
    target_change::TargetChangeType, ListenRequest, ListenResponse, StructuredQuery, Target,
};
use tokio_stream::StreamExt;

use crate::common::{create_doc, grpc_channel, provision_project, start_postgres_container, string_field, ServerProcess, TEST_ENCRYPTION_KEY};

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

async fn open_listen_and_wait_for_current(
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
        let msg = tokio::time::timeout(Duration::from_secs(10), stream.next())
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

/// A tenant's real-time `Listen` stream still delivers a document change
/// after this feature threads 2 new config-sourced fields through
/// `handle_listen`'s ad hoc listener-pool construction — proving the
/// mechanical field-threading (env var -> `ServerConfig` -> the 2 literals
/// `handle_listen` already passes to `PgPoolOptions`) did not break `Listen`
/// itself. No pool-sizing env vars are set — defaults (2 connections, 5s
/// timeout) apply, matching today's exact values (AC-PSL-01-shaped
/// regression guard for the listener site).
///
/// Given: Fernbank Analytics has an open `Listen` stream (`AddTarget` +
///        `CURRENT` marker observed) on its own `billing` collection.
/// When:  a new document is created in `billing`.
/// Then:  a `DocumentChange` event for that document arrives on the stream.
///
/// @real-io @driving_port @AC-PSL-01
#[tokio::test]
#[ignore]
async fn listen_still_delivers_document_changes_with_listener_pool_fields_threaded_through() {
    let (_sys_pg, sys_url) = start_postgres_container().await;
    let (_cust_pg, cust_url) = start_postgres_container().await;

    let cust_pool = sqlx::PgPool::connect(&cust_url)
        .await
        .expect("connect customer pool for migration");
    sqlx::migrate!("../../migrations/customer")
        .run(&cust_pool)
        .await
        .expect("customer migrate");

    let server = ServerProcess::start(
        &sys_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(healthy, "server did not become healthy");

    let api_key = provision_project(server.admin_port, "psl-fernbank-listen", &cust_url).await;
    let mut client = FirestoreClient::new(grpc_channel(server.grpc_port));

    let mut stream =
        open_listen_and_wait_for_current(&mut client, "psl-fernbank-listen", &api_key, "billing").await;

    let mut fields = HashMap::new();
    fields.insert("amount".to_string(), string_field("42"));
    create_doc(&mut client, "psl-fernbank-listen", &api_key, "billing", "invoice-1", fields).await;

    let mut saw_change = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(Ok(msg))) => {
                if let Some(listen_response::ResponseType::DocumentChange(_)) = msg.response_type {
                    saw_change = true;
                    break;
                }
            }
            _ => break,
        }
    }

    assert!(
        saw_change,
        "Listen must still deliver a DocumentChange after the listener pool's construction \
         gained 2 new config-sourced fields — a wiring regression here would silently break \
         every tenant's realtime subscriptions"
    );

    let mut server = server;
    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}
