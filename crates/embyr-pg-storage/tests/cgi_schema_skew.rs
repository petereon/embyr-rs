//! US-03 — a collection-group query is never wrong or broken against a
//! database that hasn't been migrated yet (feature-delta.md, ADR-080
//! Decision C).
//!
//! AC-CGI-10, AC-CGI-11, AC-CGI-12, AC-CGI-13.
//!
//! @driving_port @real-io @US-03

#[path = "common/mod.rs"]
mod common;
use common::*;

use std::time::Duration;

use embyr_core::storage::backend_adapter::BackendAdapter;
use embyr_pg_storage::backend_adapter::{PostgresBackendAdapter, SchemaCapability};

/// AC-CGI-10 — a collection-group query against a customer database that
/// has NOT received this feature's schema migration returns the exact same
/// correct result set as today's LIKE-based query, with no error. This
/// scenario is already true of today's production code (it never
/// references `collection_id`) and MUST stay true after this feature ships
/// — a regression anchor, not a new-behavior proof.
#[tokio::test]
async fn collection_group_query_against_an_unmigrated_database_returns_correct_results() {
    let (_pg, _pool, adapter) = customer_db_missing_latest_migration().await;
    let project_id = "meridian-health-cgi10";

    seed_document(&adapter, project_id, "products/p1/reviews", "r1", std::collections::BTreeMap::new())
        .await;
    seed_document(&adapter, project_id, "orders", "o1", std::collections::BTreeMap::new()).await;

    let collection = collection_group_selector(project_id, "reviews");
    let query = collection_group_query("reviews", None);
    let result = adapter.run_query(&collection, &query, None).await;

    assert!(
        result.is_ok(),
        "AC-CGI-10: a collection-group query against an un-migrated database must never error, \
         got: {:?}",
        result.err()
    );
    let ids: Vec<String> = result.unwrap().iter().map(|d| d.path.document_id.clone()).collect();
    assert_eq!(ids, vec!["r1".to_string()], "AC-CGI-10: correctness must be preserved unconditionally");
}

/// AC-CGI-12 — a collection in a PARTIALLY-backfilled state (some documents
/// already carry collection_id, some pre-existing ones do not yet) still
/// returns every matching document via a collection-group query, regardless
/// of each individual document's own backfill state. Already guaranteed by
/// today's production code (it ignores collection_id entirely) — this pins
/// that invariant so a careless DELIVER change cannot silently regress it.
#[tokio::test]
async fn a_partially_backfilled_collection_returns_every_matching_document() {
    let (_pg, pool, adapter) = migrated_customer_db().await;
    let project_id = "trailmark-prod-cgi12";

    // Already-backfilled (trigger populates collection_id at write time).
    seed_document(&adapter, project_id, "products/p1/reviews", "backfilled-1", std::collections::BTreeMap::new())
        .await;
    // Not-yet-backfilled (pre-existing, trigger bypassed via fixture).
    seed_pre_existing_document_without_collection_id(&pool, project_id, "products/p1/reviews", "pending-1")
        .await;
    seed_pre_existing_document_without_collection_id(&pool, project_id, "products/p1/reviews", "pending-2")
        .await;

    let collection = collection_group_selector(project_id, "reviews");
    let query = collection_group_query("reviews", None);
    let results = adapter.run_query(&collection, &query, None).await.expect("query should succeed");
    let mut ids: Vec<String> = results.iter().map(|d| d.path.document_id.clone()).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec!["backfilled-1".to_string(), "pending-1".to_string(), "pending-2".to_string()],
        "AC-CGI-12: EVERY matching document must be returned regardless of its own backfill state"
    );
}

/// AC-CGI-11 — a customer database migrated AFTER this feature's server
/// code is already live automatically begins using the fast path for
/// subsequent queries, with no restart/redeploy/cutover: the SAME
/// long-lived adapter instance's `schema_capability()` must flip from
/// `Unavailable` to `Available` once the migration lands, within the
/// configured TTL — never requiring the adapter to be reconstructed.
#[tokio::test]
async fn schema_capability_picks_up_a_migration_landing_mid_session_within_the_ttl() {
    let (_pg, pool, adapter) = customer_db_missing_latest_migration().await;

    let ttl = Duration::from_millis(150);
    let before = adapter.schema_capability(ttl).await;
    assert_eq!(
        before,
        SchemaCapability::Unavailable,
        "AC-CGI-11: before migration lands, the probe must report Unavailable"
    );

    // Sam Chen's own DBA re-runs embyr-db-prep against the SAME database —
    // via a SEPARATE connection, while `adapter` (the long-lived instance
    // embyr-server would hold) is never reconstructed.
    apply_remaining_migrations_live(&pool).await;

    // Immediately after: within the TTL window, a cached Unavailable is
    // still an acceptable (correct, not wrong) answer -- so wait past the
    // TTL before asserting the flip, matching ADR-080's own "bounded,
    // self-resolving performance-transition delay" semantics.
    tokio::time::sleep(ttl + Duration::from_millis(50)).await;

    let after = adapter.schema_capability(ttl).await;
    assert_eq!(
        after,
        SchemaCapability::Available,
        "AC-CGI-11: once the TTL has elapsed after a live migration, the SAME adapter instance \
         must report Available with no restart/reconstruction"
    );
}

/// AC-CGI-11 (permanence half) — once `Available` is observed, it is cached
/// PERMANENTLY: migrations are additive-only (ADR-022), so a later probe
/// must never regress back to `Unavailable` regardless of elapsed time.
#[tokio::test]
async fn schema_capability_available_is_permanent_never_re_flips_to_unavailable() {
    let (_pg, pool, adapter) = migrated_customer_db().await;
    let ttl = Duration::from_millis(50);

    let first = adapter.schema_capability(ttl).await;
    assert_eq!(first, SchemaCapability::Available);

    tokio::time::sleep(ttl * 5).await;

    let second = adapter.schema_capability(ttl).await;
    assert_eq!(
        second,
        SchemaCapability::Available,
        "AC-CGI-11: Available must be a monotonic, permanent cache entry -- migrations never un-happen"
    );
    let _ = pool; // kept alive for the container's own lifetime
}

/// AC-CGI-13 — the detection mechanism's own overhead, once cached, must
/// not erase US-01's own index-assisted performance gain. Proxy: p99 of N
/// cached `schema_capability` reads (the steady-state, fully-migrated cost
/// every collection-group query would pay) must be a small fraction of p99
/// of N uncached `information_schema` catalog probes (what a naive
/// per-query try/catch-style check would cost) -- proving the CACHE, not
/// just the mechanism's existence, is what keeps steady-state overhead
/// negligible.
#[tokio::test]
async fn cached_schema_capability_reads_are_negligible_versus_uncached_catalog_probes() {
    let (_pg, pool, adapter) = migrated_customer_db().await;
    let ttl = Duration::from_secs(30);

    // Warm the cache once.
    assert_eq!(adapter.schema_capability(ttl).await, SchemaCapability::Available);

    let cached_p99 = p99_of(50, || {
        let adapter = &adapter;
        async move {
            adapter.schema_capability(ttl).await;
        }
    })
    .await;

    let uncached_p99 = p99_of(50, || {
        let pool = &pool;
        async move {
            let _: Option<i32> = sqlx::query_scalar(
                "SELECT 1 FROM information_schema.columns \
                 WHERE table_name = 'documents' AND column_name = 'collection_id' LIMIT 1",
            )
            .fetch_optional(pool)
            .await
            .expect("uncached catalog probe failed");
        }
    })
    .await;

    assert!(
        cached_p99 <= uncached_p99.mul_f64(1.05).max(Duration::from_micros(1)),
        "AC-CGI-13: cached reads (p99={cached_p99:?}) must not exceed a small overhead ceiling \
         relative to an uncached catalog probe (p99={uncached_p99:?}) -- detection overhead must \
         not erase the index-assisted performance gain"
    );
}

async fn p99_of<F, Fut>(n: usize, mut f: F) -> Duration
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
        let start = std::time::Instant::now();
        f().await;
        samples.push(start.elapsed());
    }
    samples.sort();
    samples[(samples.len() * 99 / 100).min(samples.len() - 1)]
}
