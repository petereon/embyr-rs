//! Trigger coverage — collection-group-query-index (ADR-080 Decision A).
//!
//! ADR-080 § Reading Confirmation names FIVE real `INSERT INTO documents`
//! call sites in `backend_adapter.rs` (347 create_document; 398
//! update_document/precondition=None upsert; 519
//! update_document/precondition=MustNotExist; 1252 commit_transaction's
//! `Write::Update` arm; 1312 commit_transaction's `Write::Transform` arm).
//! ADR-080 Decision A's whole justification for a database trigger over
//! Rust-side population is precisely that all 5 need identical coverage
//! without per-site duplication.
//!
//! Narrowing note (per task instruction: testing all 5 individually is
//! optional if impractical): all 5 sites ARE exercised below, but combined
//! into a single testcontainers-backed test function to spend exactly one
//! container spin-up rather than five — resource-conscious on the 8GB
//! target machine per this feature's own explicit constraint. Coverage is
//! NOT narrowed, only container count.
//!
//! @driving_port @real-io @trigger_coverage

#[path = "common/mod.rs"]
mod common;
use common::*;

use std::collections::BTreeMap;

use embyr_core::domain::document::DocumentPath;
use embyr_core::domain::transaction::TransactionOptions;
use embyr_core::storage::backend_adapter::{
    BackendAdapter, FieldTransform, Write, WritePrecondition,
};

/// Covers all 5 real `INSERT INTO documents` sites: each newly-inserted
/// document's `collection_id` is populated correctly by the trigger,
/// regardless of which write path created it, for both a NESTED collection
/// path (`products/p1/reviews` -> `reviews`) and a TOP-LEVEL one (`logs` ->
/// `logs`, the regex's "no `/` present" case).
#[tokio::test]
async fn every_write_path_reaching_an_insert_populates_collection_id_via_the_trigger() {
    let (_pg, pool, adapter) = migrated_customer_db().await;
    let project_id = "trailmark-prod-cgi-trigger";

    // Site #1 (line 347): create_document — nested collection path.
    let mut fields = BTreeMap::new();
    fields.insert("rating".to_string(), double_field(5.0));
    seed_document(&adapter, project_id, "products/p1/reviews", "r1", fields).await;
    assert_eq!(
        collection_id_of(&pool, project_id, "products/p1/reviews", "r1").await,
        Some("reviews".to_string()),
        "site #1 (create_document, nested path): trigger must extract the last path segment"
    );

    // Site #1 again, TOP-LEVEL path (no `/` — regex must leave it unchanged).
    let mut fields = BTreeMap::new();
    fields.insert("level".to_string(), string_field("info"));
    seed_document(&adapter, project_id, "logs", "log1", fields).await;
    assert_eq!(
        collection_id_of(&pool, project_id, "logs", "log1").await,
        Some("logs".to_string()),
        "site #1 (create_document, top-level path): a bare collection_path has no '/' — \
         regexp_replace('^.*/', '') must return it unchanged"
    );

    // Site #2 (line 398): update_document, precondition = None (upsert,
    // ON CONFLICT DO UPDATE) — first call inserts (the INSERT branch fires).
    let mut fields = BTreeMap::new();
    fields.insert("rating".to_string(), double_field(4.0));
    let path2 = doc_path(project_id, "products/p1/reviews", "r2");
    adapter
        .update_document(&path2, fields, None)
        .await
        .expect("site #2: upsert-insert should succeed");
    assert_eq!(
        collection_id_of(&pool, project_id, "products/p1/reviews", "r2").await,
        Some("reviews".to_string()),
        "site #2 (update_document upsert, precondition=None): trigger must fire on the INSERT arm"
    );

    // Site #3 (line 519): update_document, precondition = MustNotExist.
    let mut fields = BTreeMap::new();
    fields.insert("rating".to_string(), double_field(3.0));
    let path3 = doc_path(project_id, "products/p1/reviews", "r3");
    adapter
        .update_document(&path3, fields, Some(WritePrecondition::MustNotExist))
        .await
        .expect("site #3: MustNotExist insert should succeed");
    assert_eq!(
        collection_id_of(&pool, project_id, "products/p1/reviews", "r3").await,
        Some("reviews".to_string()),
        "site #3 (update_document, precondition=MustNotExist): trigger must fire"
    );

    // Sites #4 and #5: commit_transaction's Write::Update and Write::Transform
    // arms, both inside the SAME transaction (each issues its own INSERT ...
    // ON CONFLICT DO UPDATE inside the pg transaction).
    let txn_id = adapter
        .begin_transaction(&project(project_id), TransactionOptions::ReadWrite)
        .await
        .expect("begin_transaction");

    let mut fields4 = BTreeMap::new();
    fields4.insert("rating".to_string(), double_field(2.0));
    let path4 = doc_path(project_id, "products/p1/reviews", "r4");

    let mut fields5 = BTreeMap::new();
    fields5.insert("note".to_string(), string_field("great product"));
    let path5 = doc_path(project_id, "products/p1/reviews", "r5");

    adapter
        .commit_transaction(
            &project(project_id),
            &txn_id,
            vec![
                Write::Update {
                    path: path4,
                    fields: fields4,
                    version: None,
                    precondition: None,
                    transforms: vec![],
                },
                Write::Transform {
                    path: path5,
                    transforms: vec![FieldTransform::ServerTimestamp("touched_at".to_string())],
                },
            ],
        )
        .await
        .expect("site #4/#5: commit_transaction should succeed");

    assert_eq!(
        collection_id_of(&pool, project_id, "products/p1/reviews", "r4").await,
        Some("reviews".to_string()),
        "site #4 (commit_transaction Write::Update arm): trigger must fire"
    );
    assert_eq!(
        collection_id_of(&pool, project_id, "products/p1/reviews", "r5").await,
        Some("reviews".to_string()),
        "site #5 (commit_transaction Write::Transform arm): trigger must fire"
    );
}

fn doc_path(project_id: &str, collection_path: &str, document_id: &str) -> DocumentPath {
    DocumentPath {
        project_id: project(project_id),
        collection_path: collection_path.to_string(),
        document_id: document_id.to_string(),
    }
}
