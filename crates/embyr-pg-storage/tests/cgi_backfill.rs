//! US-02 — backfilling existing documents never blocks a live customer's
//! reads or writes (feature-delta.md, ADR-080 Decision B).
//!
//! AC-CGI-06, AC-CGI-07, AC-CGI-08, AC-CGI-09.
//!
//! Resource-conscious: batch_size 5, dozens of documents (not thousands) —
//! large enough that a "backfill in progress" state is observable and
//! interruptible in a test, small enough to stay cheap on an 8GB machine.
//!
//! @driving_port @real-io @US-02

#[path = "common/mod.rs"]
mod common;
use common::*;

use std::time::Duration;

use embyr_core::storage::backend_adapter::BackendAdapter;
use embyr_pg_storage::backend_adapter::PostgresBackendAdapter;

/// AC-CGI-06 — backfilling a collection under active, concurrent write
/// traffic does not block or fail those writes. Real concurrency: the
/// backfill runs in a background task on the SAME pool while the foreground
/// task issues real `create_document` writes to the SAME collection.
#[tokio::test]
async fn backfilling_pre_existing_documents_does_not_block_concurrent_writes() {
    let (_pg, pool, adapter) = migrated_customer_db().await;
    let project_id = "trailmark-prod-cgi06";

    for n in 0..30 {
        seed_pre_existing_document_without_collection_id(
            &pool,
            project_id,
            "products/p1/reviews",
            &format!("pre-{n}"),
        )
        .await;
    }

    let backfill_adapter = PostgresBackendAdapter::new_from_pool(pool.clone());
    let backfill_handle = tokio::spawn(async move {
        backfill_adapter
            .backfill_collection_id(5, Duration::from_millis(50))
            .await
            .expect("backfill_collection_id should succeed")
    });

    // Concurrent writes to the SAME collection while the backfill runs.
    let mut write_latencies = Vec::new();
    for n in 0..10 {
        let start = std::time::Instant::now();
        seed_document(
            &adapter,
            project_id,
            "products/p1/reviews",
            &format!("during-{n}"),
            std::collections::BTreeMap::new(),
        )
        .await;
        write_latencies.push(start.elapsed());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let summary = backfill_handle.await.expect("backfill task should not panic");
    assert_eq!(summary.rows_backfilled, 30, "AC-CGI-06/09: every pre-existing document must be backfilled");

    let max_latency = write_latencies.iter().max().copied().unwrap_or_default();
    assert!(
        max_latency < Duration::from_secs(2),
        "AC-CGI-06: no write may be blocked/measurably stalled by the backfill in progress \
         (FOR UPDATE SKIP LOCKED must defer contended rows to a later batch, never block); \
         slowest concurrent write took {max_latency:?}"
    );

    // No blocked backend attributable to lock contention on `documents`.
    let blocked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_locks WHERE NOT granted AND relation = 'documents'::regclass",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(0);
    assert_eq!(blocked, 0, "AC-CGI-06: zero lock contention attributable to the backfill process");
}

/// AC-CGI-07 — a document written WHILE a backfill for its own collection is
/// still in progress already carries `collection_id` at write time (the
/// trigger, not the backfill loop, is what populates it) and is immediately
/// correctly served by a collection-group query.
#[tokio::test]
async fn a_document_written_mid_backfill_is_already_correctly_indexed() {
    let (_pg, pool, adapter) = migrated_customer_db().await;
    let project_id = "trailmark-prod-cgi07";

    for n in 0..20 {
        seed_pre_existing_document_without_collection_id(
            &pool,
            project_id,
            "products/p1/reviews",
            &format!("pre-{n}"),
        )
        .await;
    }

    let backfill_adapter = PostgresBackendAdapter::new_from_pool(pool.clone());
    let backfill_handle = tokio::spawn(async move {
        backfill_adapter.backfill_collection_id(5, Duration::from_millis(100)).await
    });

    // Give the backfill a moment to genuinely be mid-flight before writing.
    tokio::time::sleep(Duration::from_millis(60)).await;

    seed_document(
        &adapter,
        project_id,
        "products/p1/reviews",
        "written-during-backfill",
        std::collections::BTreeMap::new(),
    )
    .await;

    assert_eq!(
        collection_id_of(&pool, project_id, "products/p1/reviews", "written-during-backfill").await,
        Some("reviews".to_string()),
        "AC-CGI-07: a document written mid-backfill must already carry collection_id at write time"
    );

    let collection = collection_group_selector(project_id, "reviews");
    let query = collection_group_query("reviews", None);
    let results = adapter.run_query(&collection, &query, None).await.expect("query should succeed");
    assert!(
        results.iter().any(|d| d.path.document_id == "written-during-backfill"),
        "AC-CGI-07: the mid-backfill write must be immediately visible to a collection-group query"
    );

    let _ = backfill_handle.await;
}

/// AC-CGI-08 — an interrupted-then-resumed backfill is safely resumable:
/// tests the `FOR UPDATE SKIP LOCKED` / `WHERE collection_id IS NULL`
/// resumability claim directly, by killing the backfill future after only a
/// couple of batches and re-running it to completion.
#[tokio::test]
async fn interrupted_backfill_resumes_without_reprocessing_or_leaving_gaps() {
    let (_pg, pool, adapter) = migrated_customer_db().await;
    let project_id = "trailmark-prod-cgi08";

    const TOTAL: i64 = 30;
    for n in 0..TOTAL {
        seed_pre_existing_document_without_collection_id(
            &pool,
            project_id,
            "products/p1/reviews",
            &format!("pre-{n}"),
        )
        .await;
    }

    // Interrupt: abort the backfill task partway through (simulates
    // embyr-server restart / a transient DB blip mid-run) by racing it
    // against a short timeout, then dropping the future — `tokio::spawn`'s
    // task is aborted, its in-flight batch transaction is never committed.
    let interrupt_adapter = PostgresBackendAdapter::new_from_pool(pool.clone());
    let handle = tokio::spawn(async move {
        interrupt_adapter.backfill_collection_id(5, Duration::from_millis(150)).await
    });
    tokio::time::sleep(Duration::from_millis(220)).await; // allow ~1 batch to land
    handle.abort();
    let _ = handle.await;

    let null_after_interrupt = count_null_collection_id(&pool, project_id).await;
    assert!(
        null_after_interrupt > 0 && null_after_interrupt < TOTAL,
        "the interrupt must land genuinely mid-run (some backfilled, some not) for this test to \
         prove resumability rather than vacuously pass — got {null_after_interrupt} still NULL of {TOTAL}"
    );

    // Snapshot which rows are already backfilled, and their values + version/
    // update_time, before resuming — the no-double-write invariant (c) is
    // verified against this snapshot after resume.
    let mut backfilled_before_resume = Vec::new();
    for n in 0..TOTAL {
        let doc_id = format!("pre-{n}");
        if let Some(v) = collection_id_of(&pool, project_id, "products/p1/reviews", &doc_id).await {
            let (version, update_time) =
                version_and_update_time_of(&pool, project_id, "products/p1/reviews", &doc_id).await;
            backfilled_before_resume.push((doc_id, v, version, update_time));
        }
    }

    // Resume.
    let resumed_summary = adapter
        .backfill_collection_id(5, Duration::from_millis(10))
        .await
        .expect("resumed backfill should complete");

    // (a) Zero NULLs remain.
    assert_eq!(
        count_null_collection_id(&pool, project_id).await,
        0,
        "AC-CGI-08(a): every document must carry a populated collection_id after resume completes"
    );

    // (d) Resume must not redundantly reprocess already-backfilled rows —
    // its own row-touch count must equal exactly the remaining NULL count.
    assert_eq!(
        resumed_summary.rows_backfilled,
        null_after_interrupt as u64,
        "AC-CGI-08(d): the resumed run's own row-touch count must exclude already-completed \
         documents — expected exactly the {null_after_interrupt} rows still NULL before resume"
    );

    // (b) + (c): already-backfilled rows are untouched by the resume — same
    // collection_id value, same version, same update_time (the backfill
    // UPDATE statement never bumps version/update_time, ADR-080 Decision B
    // SQL — only `collection_id` is SET).
    for (doc_id, expected_value, expected_version, expected_update_time) in &backfilled_before_resume {
        let actual_value = collection_id_of(&pool, project_id, "products/p1/reviews", doc_id).await;
        assert_eq!(
            actual_value.as_deref(),
            Some(expected_value.as_str()),
            "AC-CGI-08(b): {doc_id}'s collection_id must be unchanged by the resumed run"
        );
        let (actual_version, actual_update_time) =
            version_and_update_time_of(&pool, project_id, "products/p1/reviews", doc_id).await;
        assert_eq!(
            (actual_version, actual_update_time),
            (*expected_version, *expected_update_time),
            "AC-CGI-08(c): {doc_id} must not be written twice by the backfill process itself \
             (version/update_time must be identical before and after resume)"
        );
    }

    // Control comparison: every backfilled value equals the pure,
    // deterministic last-path-segment extraction — "reviews" for every
    // document in "products/p1/reviews", with no exceptions.
    for n in 0..TOTAL {
        let doc_id = format!("pre-{n}");
        assert_eq!(
            collection_id_of(&pool, project_id, "products/p1/reviews", &doc_id).await,
            Some("reviews".to_string()),
            "AC-CGI-08: {doc_id}'s final collection_id must match the deterministic control value"
        );
    }
}

/// AC-CGI-09 — once a collection's backfill fully completes, `EXPLAIN` of a
/// collection-group query against it shows an index-assisted plan covering
/// documents that existed BEFORE the backfill ran, not just ones written
/// afterward.
#[tokio::test]
async fn a_fully_backfilled_collection_is_index_assisted_for_pre_existing_documents() {
    let (_pg, pool, adapter) = migrated_customer_db().await;
    let project_id = "trailmark-prod-cgi09";

    for n in 0..20 {
        seed_pre_existing_document_without_collection_id(
            &pool,
            project_id,
            "products/p1/reviews",
            &format!("pre-{n}"),
        )
        .await;
    }

    adapter.ensure_collection_group_indexes().await.expect("index build should succeed");
    let summary = adapter
        .backfill_collection_id(10, Duration::from_millis(5))
        .await
        .expect("backfill should complete");
    assert_eq!(summary.rows_backfilled, 20);

    assert!(documents_collection_group_idx_exists(&pool).await, "the collection-group index must exist");

    let explain = explain_all_descendants(&pool, project_id, "reviews", true).await;
    assert!(
        plan_is_index_assisted(&explain),
        "AC-CGI-09: EXPLAIN must show an index-assisted plan covering the pre-existing, \
         now-backfilled documents, got:\n{}",
        explain.join("\n")
    );
}
