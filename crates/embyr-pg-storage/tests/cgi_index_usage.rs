//! US-01 — a collection-group query uses a real index instead of scanning
//! the whole project (feature-delta.md, ADR-080 Decision D).
//!
//! AC-CGI-01, AC-CGI-02, AC-CGI-03, AC-CGI-04, AC-CGI-05.
//!
//! @walking_skeleton @driving_port @real-io @US-01

#[path = "common/mod.rs"]
mod common;
use common::*;

use std::collections::BTreeMap;

use embyr_core::domain::query::QueryFilter;
use embyr_core::storage::backend_adapter::BackendAdapter;

/// AC-CGI-01 (walking skeleton) — Domain Example 1: a "reviews" collection
/// group nested under many "products" parents, among unrelated documents in
/// other collections. The collection-group query's own result set must be
/// identical to what the pre-existing LIKE-based query returns (unchanged
/// business outcome — this feature only changes the PLAN, never the
/// result). Also covers Domain Example 2 (top-level + nested collections of
/// the same name both returned).
#[tokio::test]
async fn collection_group_query_returns_the_same_documents_a_customer_already_sees_today() {
    let (_pg, _pool, adapter) = migrated_customer_db().await;
    let project_id = "trailmark-prod-cgi01";

    // Nested "reviews" under two different products.
    seed_document(&adapter, project_id, "products/p1/reviews", "r1", rating(5.0)).await;
    seed_document(&adapter, project_id, "products/p2/reviews", "r2", rating(4.0)).await;
    // Unrelated documents in other collections — must never appear.
    seed_document(&adapter, project_id, "products", "p1", BTreeMap::new()).await;
    seed_document(&adapter, project_id, "orders", "o1", BTreeMap::new()).await;

    // Domain Example 2: top-level AND nested collections sharing a name.
    seed_document(&adapter, project_id, "logs", "top1", BTreeMap::new()).await;
    seed_document(&adapter, project_id, "orgs/org1/logs", "nested1", BTreeMap::new()).await;
    seed_document(&adapter, project_id, "orgs/org2/logs", "nested2", BTreeMap::new()).await;

    let collection = collection_group_selector(project_id, "reviews");
    let query = collection_group_query("reviews", None);
    let results = adapter
        .run_query(&collection, &query, None)
        .await
        .expect("collection-group RunQuery should succeed");
    let mut doc_ids: Vec<String> = results.iter().map(|d| d.path.document_id.clone()).collect();
    doc_ids.sort();
    assert_eq!(
        doc_ids,
        vec!["r1".to_string(), "r2".to_string()],
        "AC-CGI-01: result set must exactly match every document whose collection identity is \
         'reviews', regardless of nesting depth, and nothing else"
    );

    let logs_collection = collection_group_selector(project_id, "logs");
    let logs_query = collection_group_query("logs", None);
    let logs_results = adapter
        .run_query(&logs_collection, &logs_query, None)
        .await
        .expect("collection-group RunQuery for 'logs' should succeed");
    let mut log_ids: Vec<String> = logs_results.iter().map(|d| d.path.document_id.clone()).collect();
    log_ids.sort();
    assert_eq!(
        log_ids,
        vec!["nested1".to_string(), "nested2".to_string(), "top1".to_string()],
        "AC-CGI-01 (Example 2): both the top-level 'logs' collection and every nested 'logs' \
         collection must be returned"
    );
}

/// AC-CGI-01 (Example 3, boundary) — a collection group matching zero
/// documents returns an empty result, never an error.
#[tokio::test]
async fn collection_group_query_for_a_name_with_no_matches_returns_an_empty_result() {
    let (_pg, _pool, adapter) = migrated_customer_db().await;
    let project_id = "trailmark-prod-cgi01-empty";
    seed_document(&adapter, project_id, "products", "p1", BTreeMap::new()).await;

    let collection = collection_group_selector(project_id, "nonexistent_collection");
    let query = collection_group_query("nonexistent_collection", None);
    let results = adapter
        .run_query(&collection, &query, None)
        .await
        .expect("query for a name with no matches must succeed, not error");
    assert!(results.is_empty(), "AC-CGI-01: no collection anywhere named this must return empty");
}

/// AC-CGI-01 (DISCUSS UAT "compound filter still uses the index-assisted
/// plan") — a compound filter on top of the collection-identity predicate
/// still returns the correct, unchanged result set.
#[tokio::test]
async fn collection_group_query_with_a_compound_filter_returns_only_matching_documents() {
    let (_pg, _pool, adapter) = migrated_customer_db().await;
    let project_id = "trailmark-prod-cgi01-compound";

    seed_document(&adapter, project_id, "products/p1/reviews", "high_verified", rating_verified(5.0, true))
        .await;
    seed_document(&adapter, project_id, "products/p1/reviews", "high_unverified", rating_verified(5.0, false))
        .await;
    seed_document(&adapter, project_id, "products/p2/reviews", "low_verified", rating_verified(2.0, true))
        .await;

    let collection = collection_group_selector(project_id, "reviews");
    let compound = QueryFilter::Composite(vec![rating_gte_filter(4.0), verified_equal_true_filter()]);
    let query = collection_group_query("reviews", Some(compound));
    let results = adapter
        .run_query(&collection, &query, None)
        .await
        .expect("compound-filter collection-group query should succeed");
    let ids: Vec<String> = results.iter().map(|d| d.path.document_id.clone()).collect();
    assert_eq!(
        ids,
        vec!["high_verified".to_string()],
        "AC-CGI-01: compound filter (rating>=4 AND verified==true) must apply on top of the \
         collection-identity predicate, not replace it"
    );
}

/// AC-CGI-02 — `EXPLAIN` of a collection-group query against a
/// schema-current database shows an index-assisted plan, not a sequential
/// scan of the entire `documents` table.
#[tokio::test]
async fn explain_of_a_schema_current_collection_group_query_is_index_assisted() {
    let (_pg, pool, adapter) = migrated_customer_db().await;
    let project_id = "trailmark-prod-cgi02";

    for n in 0..20 {
        seed_document(
            &adapter,
            project_id,
            &format!("products/p{n}/reviews"),
            &format!("r{n}"),
            rating(4.0),
        )
        .await;
    }
    // Unrelated documents, larger than the target collection group, so a
    // Seq Scan would be the ONLY way today's LIKE predicate could ever be
    // satisfied for these rows too.
    for n in 0..40 {
        seed_document(&adapter, project_id, "orders", &format!("o{n}"), BTreeMap::new()).await;
    }

    let explain = explain_all_descendants(&pool, project_id, "reviews", true).await;
    assert!(
        plan_is_index_assisted(&explain),
        "AC-CGI-02: expected an index-assisted plan for a schema-current database, got:\n{}",
        explain.join("\n")
    );
}

/// AC-CGI-02 (DISCUSS UAT "remains index-assisted regardless of result-set
/// size") — several thousand... kept to dozens per this feature's own
/// resource-conscious test-design instruction, still enough to distinguish
/// an index scan from a full scan by EXPLAIN's own plan type, not by row
/// count.
#[tokio::test]
async fn explain_stays_index_assisted_for_a_larger_collection_group_result_set() {
    let (_pg, pool, adapter) = migrated_customer_db().await;
    let project_id = "trailmark-prod-cgi02-large";

    for n in 0..60 {
        seed_document(
            &adapter,
            project_id,
            &format!("products/p{}/reviews", n % 10),
            &format!("r{n}"),
            rating(4.0),
        )
        .await;
    }

    let explain = explain_all_descendants(&pool, project_id, "reviews", true).await;
    assert!(
        plan_is_index_assisted(&explain),
        "AC-CGI-02: plan must remain index-assisted regardless of matching-row count, got:\n{}",
        explain.join("\n")
    );
}

/// AC-CGI-03 — `RunAggregationQuery` Count/Sum/Avg for a collection group
/// return the same values the pre-existing LIKE-based query would, and are
/// index-assisted (the underlying predicate is identical to `run_query`'s
/// own, ADR-080 Decision D — proven once via `explain_all_descendants`
/// above; this test proves VALUE correctness for all 3 arms).
#[tokio::test]
async fn aggregation_count_sum_avg_for_a_collection_group_return_correct_values() {
    let (_pg, _pool, adapter) = migrated_customer_db().await;
    let project_id = "trailmark-prod-cgi03";

    seed_document(&adapter, project_id, "products/p1/reviews", "r1", rating(4.0)).await;
    seed_document(&adapter, project_id, "products/p2/reviews", "r2", rating(5.0)).await;
    seed_document(&adapter, project_id, "products/p3/reviews", "r3", rating(3.0)).await;
    seed_document(&adapter, project_id, "products", "p1", BTreeMap::new()).await; // must not count

    let collection = collection_group_selector(project_id, "reviews");

    let count = adapter
        .run_aggregation_query(&collection, &count_aggregation("reviews", None), None)
        .await
        .expect("Count aggregation should succeed");
    assert_eq!(count, embyr_core::domain::query::AggregateValue::Count(3), "AC-CGI-03: Count");

    let sum = adapter
        .run_aggregation_query(&collection, &sum_aggregation("reviews", "rating"), None)
        .await
        .expect("Sum aggregation should succeed");
    assert_eq!(sum, embyr_core::domain::query::AggregateValue::Sum(12.0), "AC-CGI-03: Sum");

    let avg = adapter
        .run_aggregation_query(&collection, &avg_aggregation("reviews", "rating"), None)
        .await
        .expect("Avg aggregation should succeed");
    assert_eq!(avg, embyr_core::domain::query::AggregateValue::Avg(Some(4.0)), "AC-CGI-03: Avg");
}

/// AC-CGI-04 (regression guard) — this feature touches ONLY the
/// `all_descendants=true` branch; an ordinary (non-collection-group) query
/// is byte-for-byte unaffected.
#[tokio::test]
async fn ordinary_non_collection_group_queries_are_unaffected() {
    let (_pg, _pool, adapter) = migrated_customer_db().await;
    let project_id = "trailmark-prod-cgi04";

    seed_document(&adapter, project_id, "reviews", "top_level_review", rating(5.0)).await;
    seed_document(&adapter, project_id, "products/p1/reviews", "nested_review", rating(5.0)).await;

    let collection = collection_group_selector(project_id, "reviews");
    let query = ordinary_collection_query("reviews");
    let results = adapter
        .run_query(&collection, &query, None)
        .await
        .expect("ordinary query should succeed");
    let ids: Vec<String> = results.iter().map(|d| d.path.document_id.clone()).collect();
    assert_eq!(
        ids,
        vec!["top_level_review".to_string()],
        "AC-CGI-04: an ordinary (all_descendants=false) query must match ONLY the exact \
         collection_path, never a nested one — unchanged by this feature"
    );
}

fn rating(v: f64) -> BTreeMap<String, embyr_core::domain::field_value::FieldValue> {
    let mut m = BTreeMap::new();
    m.insert("rating".to_string(), double_field(v));
    m
}

fn rating_verified(v: f64, verified: bool) -> BTreeMap<String, embyr_core::domain::field_value::FieldValue> {
    let mut m = rating(v);
    m.insert("verified".to_string(), embyr_core::domain::field_value::FieldValue::Boolean(verified));
    m
}
