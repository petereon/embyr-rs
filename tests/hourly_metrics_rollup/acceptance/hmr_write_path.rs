//! hourly-metrics-rollup (ADR-084) slice 2 — buffer, flush, handler hook.
//!
//! AC-HMR-01 (real gRPC requests, logging off -> rollup rows, Σ buckets = requests),
//! AC-HMR-05 (several instances, one project-hour -> counts add),
//! AC-HMR-06 (graceful shutdown flushes the buffer),
//! AC-HMR-07 (system DB failing: flush fails, buffer bounded, drop counter),
//! AC-HMR-12 (streams are not sampled; mapping asserted in rollup.rs unit tests).
//!
//! Flushes are driven explicitly (`embyr_server::rollup::flush`); the 10 s
//! timer is never waited on.

#[path = "../common/mod.rs"]
mod common;

use std::sync::Arc;

use common::{hour_ago, insert_project, rollup_row, rollup_row_count, start_system_db};
use embyr_core::metrics_rollup::Op;
use embyr_proto::firestore::{
    firestore_client::FirestoreClient, CreateDocumentRequest, DeleteDocumentRequest, Document,
    GetDocumentRequest,
};
use embyr_server::adapters::system_db::SystemDb;
use embyr_server::rollup::{flush, spawn_flusher};

fn authed<T>(mut req: tonic::Request<T>, key: &str) -> tonic::Request<T> {
    req.metadata_mut().insert("authorization", format!("Bearer {key}").parse().unwrap());
    req
}

/// AC-HMR-05: two server instances (own buffers) flush the same project-hour
/// concurrently; counts add, no lost update, no duplicate-key error.
// @AC-HMR-05 @real-io
#[tokio::test]
async fn instances_flushing_the_same_project_hour_add_up() {
    let (_container, url, db1) = start_system_db().await;
    let db2 = Arc::new(SystemDb::new(&url).await.expect("second instance"));
    insert_project(db1.pool(), "hmr-multi").await;
    let hour = hour_ago(1);

    for _ in 0..2 {
        db1.rollup().record("hmr-multi", hour, Op::Read, 3.0, 2, false);
    }
    for i in 0..3 {
        db2.rollup().record("hmr-multi", hour, Op::Read, 900.0, 2, i == 0);
    }
    let (a, b) = tokio::join!(flush(&db1), flush(&db2));
    a.expect("instance 1 flush");
    b.expect("instance 2 flush");

    assert_eq!(rollup_row(db1.pool(), "hmr-multi", hour, 0).await, Some((5, 10, 1, 5)));
}

/// AC-HMR-06: graceful shutdown flushes whatever is buffered, even if the
/// periodic flush has not fired yet.
// @AC-HMR-06 @real-io
#[tokio::test]
async fn graceful_shutdown_flushes_the_buffer() {
    let (_container, _url, db) = start_system_db().await;
    insert_project(db.pool(), "hmr-shutdown").await;
    let hour = hour_ago(0);

    let flusher = spawn_flusher(Arc::clone(&db), std::time::Duration::from_secs(3600));
    db.rollup().record("hmr-shutdown", hour, Op::Write, 8.0, 1, false);
    assert_eq!(rollup_row_count(db.pool(), "hmr-shutdown").await, 0, "nothing flushed yet");

    flusher.shutdown().await;

    assert_eq!(rollup_row(db.pool(), "hmr-shutdown", hour, 1).await, Some((1, 1, 0, 1)));
    assert!(db.rollup().is_empty());
}

/// AC-HMR-07: with the rollup table unavailable the flush fails without
/// losing data, recording stays non-blocking and bounded, drops are counted,
/// and everything kept lands once the DB is back.
// @AC-HMR-07 @real-io
#[tokio::test]
async fn failing_flush_keeps_data_stays_bounded_and_counts_drops() {
    let (_container, _url, db) = start_system_db().await;
    insert_project(db.pool(), "hmr-down").await;
    let hour = hour_ago(0);
    db.rollup().set_max_keys(2);
    db.rollup().record("hmr-down", hour, Op::Read, 3.0, 1, false);
    db.rollup().record("hmr-down", hour, Op::Write, 3.0, 1, false);

    sqlx::query("ALTER TABLE hourly_project_metrics RENAME TO hourly_project_metrics_offline")
        .execute(db.pool())
        .await
        .expect("take the rollup table offline");

    assert!(flush(&db).await.is_err(), "flush must fail while the table is unavailable");
    assert_eq!(db.rollup().len(), 2, "unflushed data is kept for the next flush");
    // The request-path call is synchronous and drops (never blocks/fails) at the bound.
    assert!(!db.rollup().record("hmr-down", hour, Op::Delete, 3.0, 1, false));
    assert_eq!(db.rollup().dropped(), 1);
    assert_eq!(db.rollup().len(), 2, "buffer stays bounded");

    sqlx::query("ALTER TABLE hourly_project_metrics_offline RENAME TO hourly_project_metrics")
        .execute(db.pool())
        .await
        .expect("bring the rollup table back");
    flush(&db).await.expect("flush after recovery");

    assert_eq!(rollup_row(db.pool(), "hmr-down", hour, 0).await, Some((1, 1, 0, 1)));
    assert_eq!(rollup_row(db.pool(), "hmr-down", hour, 1).await, Some((1, 1, 0, 1)));
    assert_eq!(rollup_row(db.pool(), "hmr-down", hour, 2).await, None);
}

/// A flush must not fail because a project was deleted while its data sat in
/// the buffer; the rest of the chunk is still written.
// @AC-HMR-05 @real-io
#[tokio::test]
async fn project_deleted_before_flush_is_skipped_not_fatal() {
    let (_container, _url, db) = start_system_db().await;
    insert_project(db.pool(), "hmr-keep").await;
    let hour = hour_ago(0);
    db.rollup().record("hmr-gone", hour, Op::Read, 3.0, 1, false);
    db.rollup().record("hmr-keep", hour, Op::Read, 3.0, 1, false);

    flush(&db).await.expect("flush with a vanished project");

    assert_eq!(rollup_row(db.pool(), "hmr-keep", hour, 0).await, Some((1, 1, 0, 1)));
    assert_eq!(rollup_row(db.pool(), "hmr-gone", hour, 0).await, None);
}

/// AC-HMR-01 (+ AC-HMR-12): real gRPC requests against a project with query
/// logging off produce rollup rows per op class whose bucket total equals
/// `requests`; a request with a wrong key is not attributed to the project.
// @AC-HMR-01 @AC-HMR-12 @driving_port @real-io
#[tokio::test]
async fn grpc_requests_are_rolled_up_per_op_with_logging_off() {
    use embyr_core::auth::{argon2, ecies};

    let (_container, url, db) = start_system_db().await;
    let api_key = "hmr-test-key";
    let encrypted_dsn =
        ecies::encrypt(&ecies::derive_public_key(api_key.as_bytes()), url.as_bytes())
            .expect("encrypt dsn");
    sqlx::query(
        "INSERT INTO projects \
         (id, status, backend_mode, api_key_hash_current, ecies_encrypted_dsn) \
         VALUES ('hmr-grpc', 'active', 'direct_pg', $1, $2)",
    )
    .bind(argon2::hash_api_key(api_key.as_bytes()).expect("hash api key"))
    .bind(&encrypted_dsn)
    .execute(db.pool())
    .await
    .expect("insert project");
    let logging_enabled: bool =
        sqlx::query_scalar("SELECT logging_enabled FROM projects WHERE id = 'hmr-grpc'")
            .fetch_one(db.pool())
            .await
            .expect("read logging flag");
    assert!(!logging_enabled, "precondition: query logging is off");

    let server = embyr_server::start_test_server(Arc::clone(&db)).await;
    let channel = tonic::transport::Endpoint::new(format!("http://{}", server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);
    let docs = "projects/hmr-grpc/databases/(default)/documents";

    // Outcomes (ok / not found / backend error) are irrelevant: every
    // authenticated request counts, errors are flagged in `errors`.
    for _ in 0..3 {
        let get = GetDocumentRequest { name: format!("{docs}/c/d"), ..Default::default() };
        let _ = client.get_document(authed(tonic::Request::new(get), api_key)).await;
    }
    for _ in 0..2 {
        let del = DeleteDocumentRequest { name: format!("{docs}/c/d"), ..Default::default() };
        let _ = client.delete_document(authed(tonic::Request::new(del), api_key)).await;
    }
    let create = CreateDocumentRequest {
        parent: docs.to_string(),
        collection_id: "c".to_string(),
        document_id: "d".to_string(),
        document: Some(Document::default()),
        ..Default::default()
    };
    let _ = client.create_document(authed(tonic::Request::new(create), api_key)).await;
    let bad = GetDocumentRequest { name: format!("{docs}/c/d"), ..Default::default() };
    let _ = client.get_document(authed(tonic::Request::new(bad), "wrong-key")).await;

    flush(&db).await.expect("explicit flush");

    let rows: Vec<(i16, i64, i64)> = sqlx::query_as(
        "SELECT op, requests, \
                (b00+b01+b02+b03+b04+b05+b06+b07+b08+b09+b10+b11+b12)::bigint \
         FROM hourly_project_metrics WHERE project_id = 'hmr-grpc' ORDER BY op",
    )
    .fetch_all(db.pool())
    .await
    .expect("read rollup rows");
    let requests_by_op: Vec<(i16, i64)> = rows.iter().map(|(op, r, _)| (*op, *r)).collect();
    assert_eq!(requests_by_op, vec![(0, 3), (1, 1), (2, 2)], "wrong-key request is not recorded");
    assert!(rows.iter().all(|(_, requests, bucket_total)| requests == bucket_total));
}
