// SCAFFOLD: true
//! US-A04 — Transaction lifecycle via agent
//!
//! As Alex (SDK Developer), I want Firestore transactions (runTransaction)
//! to work through the agent with OCC semantics, so that my app's
//! concurrent-write scenarios are safe.
//!
//! Driving port: StorageAgent gRPC service (mTLS :9191)
//!   RPCs: BeginTransaction, Commit, Rollback (sweep via internal trigger)
//! Red classification: MISSING_FUNCTIONALITY
//!
//! Feature file: tests/features/agent/us_a04_transactions.feature
//! Execution order: S04A (after S02A writes, before S03A queries)

use std::collections::HashMap;

use embyr_proto::agent::{
    precondition::ConditionType,
    storage_agent_client::StorageAgentClient,
    value::ValueType,
    write::Operation,
    BeginTransactionRequest, CommitRequest, CreateDocumentRequest, DeleteDocumentRequest,
    Document, GetDocumentRequest, Precondition, RollbackRequest, UpdateDocumentRequest, Value,
    Write,
};
use tonic::transport::Channel;

use super::agent_common::{start_test_agent, AgentHandle};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn str_val(s: &str) -> Value {
    Value { value_type: Some(ValueType::StringValue(s.to_string())) }
}

/// Create a document and return it.
async fn create_doc(
    client: &mut StorageAgentClient<Channel>,
    project_id: &str,
    collection: &str,
    doc_id: &str,
    fields: HashMap<String, Value>,
) -> Document {
    let req = CreateDocumentRequest {
        parent: format!("projects/{project_id}/databases/(default)/documents"),
        collection_id: collection.to_string(),
        document_id: doc_id.to_string(),
        document: Some(Document { name: String::new(), fields, ..Default::default() }),
        ..Default::default()
    };
    client.create_document(req).await.expect("create_document").into_inner()
}

/// Update a document (full replace) and return the updated doc.
async fn update_doc(
    client: &mut StorageAgentClient<Channel>,
    name: &str,
    fields: HashMap<String, Value>,
) -> Document {
    let req = UpdateDocumentRequest {
        document: Some(Document { name: name.to_string(), fields, ..Default::default() }),
        ..Default::default()
    };
    client.update_document(req).await.expect("update_document").into_inner()
}

/// Get a document by full resource name.
async fn get_doc(client: &mut StorageAgentClient<Channel>, name: &str) -> Document {
    let req = GetDocumentRequest { name: name.to_string(), ..Default::default() };
    client.get_document(req).await.expect("get_document").into_inner()
}

// ---------------------------------------------------------------------------
// Happy path scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a04 @real_io
///
/// Feature: Transaction with no concurrent competition commits successfully
///   Given "orders/ord-txn-01" exists at generation 3 with status="processing"
///   When  a caller opens a transaction, reads the document, and commits with status="complete"
///   Then  the transaction commits; the document is at generation 4 with status="complete"
#[tokio::test]
async fn transaction_with_no_competition_commits_successfully() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    let project_id = "finops-prod";
    let doc_name =
        "projects/finops-prod/databases/(default)/documents/orders/ord-txn-01";

    // Seed: create + update twice to reach generation 3.
    let mut fields = HashMap::new();
    fields.insert("status".to_string(), str_val("processing"));
    let doc = create_doc(&mut client, project_id, "orders", "ord-txn-01", fields.clone()).await;
    let doc = update_doc(&mut client, &doc.name, fields.clone()).await;
    let doc = update_doc(&mut client, &doc.name, fields.clone()).await;
    assert_eq!(doc.generation, 3, "seeded document must be at generation 3");

    let update_time = doc.update_time.expect("update_time present");

    // Begin transaction.
    let txn_resp = client
        .begin_transaction(BeginTransactionRequest { ..Default::default() })
        .await
        .expect("begin_transaction")
        .into_inner();
    assert!(!txn_resp.transaction.is_empty(), "transaction token must be non-empty");

    // Commit: update status to "complete" with UpdateTime precondition.
    let mut commit_fields = HashMap::new();
    commit_fields.insert("status".to_string(), str_val("complete"));
    let write = Write {
        operation: Some(Operation::Update(Document {
            name: doc_name.to_string(),
            fields: commit_fields,
            ..Default::default()
        })),
        current_document: Some(Precondition {
            condition_type: Some(ConditionType::UpdateTime(update_time)),
        }),
        ..Default::default()
    };
    client
        .commit(CommitRequest {
            transaction: txn_resp.transaction,
            writes: vec![write],
            ..Default::default()
        })
        .await
        .expect("commit must succeed");

    // Verify: generation advances and field is updated.
    let updated = get_doc(&mut client, doc_name).await;
    assert_eq!(updated.generation, 4, "generation must advance to 4 after commit");
    let status = updated.fields["status"].value_type.as_ref().expect("status field present");
    assert!(
        matches!(status, ValueType::StringValue(s) if s == "complete"),
        "status must be 'complete' after commit"
    );
}

/// @driving_port @us_a04 @real_io
///
/// Feature: Rolling back a transaction discards writes without applying them
///   Given "orders/ord-txn-02" exists at generation 1 with status="processing"
///   And   a transaction opens and prepares a write setting status="complete"
///   When  the caller rolls back the transaction
///   Then  the document still shows status="processing" at generation 1
#[tokio::test]
async fn rolling_back_transaction_discards_writes_without_applying_them() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    let project_id = "finops-prod";
    let doc_name =
        "projects/finops-prod/databases/(default)/documents/orders/ord-txn-02";

    // Seed at generation 1.
    let mut fields = HashMap::new();
    fields.insert("status".to_string(), str_val("processing"));
    let doc =
        create_doc(&mut client, project_id, "orders", "ord-txn-02", fields).await;
    assert_eq!(doc.generation, 1, "seeded document must be at generation 1");

    // Begin transaction.
    let txn_resp = client
        .begin_transaction(BeginTransactionRequest { ..Default::default() })
        .await
        .expect("begin_transaction")
        .into_inner();

    // Rollback — do NOT commit.
    client
        .rollback(RollbackRequest {
            transaction: txn_resp.transaction,
            ..Default::default()
        })
        .await
        .expect("rollback must succeed");

    // Document must remain unchanged at generation 1 with status="processing".
    let unchanged = get_doc(&mut client, doc_name).await;
    assert_eq!(unchanged.generation, 1, "generation must stay at 1 after rollback");
    let status =
        unchanged.fields["status"].value_type.as_ref().expect("status field present");
    assert!(
        matches!(status, ValueType::StringValue(s) if s == "processing"),
        "status must still be 'processing' after rollback"
    );
}

/// @driving_port @us_a04 @real_io @error
///
/// Feature: Rolling back a transaction that has already been committed returns not-found
///   Given a transaction that has already committed successfully
///   When  a caller attempts to roll back the same transaction
///   Then  the caller receives a not-found response; committed changes remain intact
#[tokio::test]
async fn rolling_back_already_committed_transaction_returns_not_found() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    let project_id = "finops-prod";
    let doc_name =
        "projects/finops-prod/databases/(default)/documents/orders/ord-txn-03";

    // Seed doc.
    let mut fields = HashMap::new();
    fields.insert("status".to_string(), str_val("processing"));
    let doc =
        create_doc(&mut client, project_id, "orders", "ord-txn-03", fields).await;
    let update_time = doc.update_time.expect("update_time");

    // Begin transaction.
    let txn_resp = client
        .begin_transaction(BeginTransactionRequest { ..Default::default() })
        .await
        .expect("begin_transaction")
        .into_inner();
    let txn_token = txn_resp.transaction.clone();

    // Commit.
    let mut commit_fields = HashMap::new();
    commit_fields.insert("status".to_string(), str_val("complete"));
    let write = Write {
        operation: Some(Operation::Update(Document {
            name: doc_name.to_string(),
            fields: commit_fields,
            ..Default::default()
        })),
        current_document: Some(Precondition {
            condition_type: Some(ConditionType::UpdateTime(update_time)),
        }),
        ..Default::default()
    };
    client
        .commit(CommitRequest {
            transaction: txn_token.clone(),
            writes: vec![write],
            ..Default::default()
        })
        .await
        .expect("commit must succeed");

    // Now try to rollback the already-committed transaction.
    let err = client
        .rollback(RollbackRequest { transaction: txn_token, ..Default::default() })
        .await
        .expect_err("rollback of committed txn must fail");
    assert_eq!(
        err.code(),
        tonic::Code::NotFound,
        "rolling back a committed transaction must return NOT_FOUND"
    );
}

/// @driving_port @us_a04 @real_io
///
/// Feature: Expired transactions are removed by the sweep operation
///   Given two transactions exist: one expired, one still active
///   When  the sweep operation runs
///   Then  the expired record is removed; the active record remains
///   And   committing the swept transaction returns not-found
#[tokio::test]
async fn expired_transactions_are_removed_by_sweep() {
    let (handle, mut client) = start_test_agent("finops-prod").await;

    // Begin an "expired" transaction.
    let txn_resp = client
        .begin_transaction(BeginTransactionRequest { ..Default::default() })
        .await
        .expect("begin_transaction")
        .into_inner();
    let txn_uuid = uuid::Uuid::from_slice(&txn_resp.transaction)
        .expect("transaction token must be a valid UUID");

    // Age it past the 60-second TTL.
    sqlx::query(
        "UPDATE transactions SET started_at = NOW() - INTERVAL '65 seconds' \
         WHERE transaction_id = $1",
    )
    .bind(txn_uuid)
    .execute(&handle.pool)
    .await
    .expect("age transaction in DB");

    // Begin a second transaction that should NOT be swept (it is still active).
    let txn_active_resp = client
        .begin_transaction(BeginTransactionRequest { ..Default::default() })
        .await
        .expect("begin second transaction")
        .into_inner();
    let txn_active_uuid = uuid::Uuid::from_slice(&txn_active_resp.transaction)
        .expect("active transaction token must be a valid UUID");

    // Criterion 2 + 3: run one sweep with 60-second TTL.
    let sweeper = embyr_agent::sweeper::AgentTransactionSweeper::new(
        handle.pool.clone(),
        60,
        std::time::Duration::from_millis(100),
    );
    sweeper.sweep_once().await.expect("sweep_once must not error");

    // Criterion 3a: expired transaction record is absent.
    let count: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM transactions WHERE transaction_id = $1",
    )
    .bind(txn_uuid)
    .fetch_one(&handle.pool)
    .await
    .expect("count expired");
    assert_eq!(count.0, 0, "expired transaction must be absent after sweep");

    // Criterion 3b: active transaction record is still present.
    let count2: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM transactions WHERE transaction_id = $1",
    )
    .bind(txn_active_uuid)
    .fetch_one(&handle.pool)
    .await
    .expect("count active");
    assert_eq!(count2.0, 1, "active transaction must remain after sweep");

    // Criterion 4: committing the swept transaction returns NOT_FOUND.
    let result = client
        .commit(CommitRequest {
            transaction: txn_resp.transaction.clone(),
            writes: vec![],
            ..Default::default()
        })
        .await;
    assert!(result.is_err(), "commit of swept transaction must fail");
    assert_eq!(
        result.unwrap_err().code(),
        tonic::Code::NotFound,
        "commit of swept transaction must return NOT_FOUND"
    );
}

// ---------------------------------------------------------------------------
// Error scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a04 @real_io @error
///
/// Feature: Concurrent transaction on same generation is rejected with a conflict
///   Given "orders/ord-txn-04" exists at generation 1
///   And   two transactions both read it at generation 1
///   When  both attempt to commit
///   Then  one succeeds; the other receives a conflict-aborted response
#[tokio::test]
async fn concurrent_transaction_on_same_generation_is_rejected_with_conflict() {
    let (_handle, mut client1) = start_test_agent("finops-prod").await;

    let project_id = "finops-prod";
    let doc_name =
        "projects/finops-prod/databases/(default)/documents/orders/ord-txn-04";

    // Seed doc at generation 1.
    let mut fields = HashMap::new();
    fields.insert("status".to_string(), str_val("init"));
    let doc = create_doc(&mut client1, project_id, "orders", "ord-txn-04", fields).await;
    assert_eq!(doc.generation, 1);
    let update_time = doc.update_time.expect("update_time");

    // Clone the first client — tonic channels are Arc-backed and cheap to clone.
    let mut client2 = client1.clone();

    // Both transactions begin (separate tokens).
    let txn1 = client1
        .begin_transaction(BeginTransactionRequest { ..Default::default() })
        .await
        .expect("begin txn1")
        .into_inner()
        .transaction;
    let txn2 = client2
        .begin_transaction(BeginTransactionRequest { ..Default::default() })
        .await
        .expect("begin txn2")
        .into_inner()
        .transaction;

    // Build identical writes — same precondition (update_time at generation 1).
    let make_write = |new_status: &str| {
        let mut f = HashMap::new();
        f.insert("status".to_string(), str_val(new_status));
        Write {
            operation: Some(Operation::Update(Document {
                name: doc_name.to_string(),
                fields: f,
                ..Default::default()
            })),
            current_document: Some(Precondition {
                condition_type: Some(ConditionType::UpdateTime(update_time.clone())),
            }),
            ..Default::default()
        }
    };

    let (r1, r2) = tokio::join!(
        client1.commit(CommitRequest {
            transaction: txn1,
            writes: vec![make_write("winner")],
            ..Default::default()
        }),
        client2.commit(CommitRequest {
            transaction: txn2,
            writes: vec![make_write("winner")],
            ..Default::default()
        }),
    );

    let success_count = [r1.is_ok(), r2.is_ok()].iter().filter(|&&b| b).count();
    assert_eq!(success_count, 1, "exactly one of the two concurrent commits must succeed");

    let failed = if r1.is_err() { r1 } else { r2 };
    assert_eq!(
        failed.unwrap_err().code(),
        tonic::Code::Aborted,
        "the losing commit must return ABORTED"
    );
}

/// @driving_port @us_a04 @real_io @error
///
/// Feature: Transaction reading a document that was later deleted is aborted on commit
///   Given "orders/ord-txn-05" exists at generation 1
///   And   a transaction reads it; another caller deletes it before the transaction commits
///   When  the transaction attempts to commit
///   Then  the commit returns an aborted response indicating the document was removed
#[tokio::test]
async fn transaction_on_deleted_document_is_aborted_on_commit() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    let project_id = "finops-prod";
    let doc_name =
        "projects/finops-prod/databases/(default)/documents/orders/ord-txn-05";

    // Seed doc at generation 1.
    let mut fields = HashMap::new();
    fields.insert("status".to_string(), str_val("active"));
    let doc = create_doc(&mut client, project_id, "orders", "ord-txn-05", fields).await;
    assert_eq!(doc.generation, 1);
    let update_time = doc.update_time.expect("update_time");

    // Begin transaction.
    let txn_resp = client
        .begin_transaction(BeginTransactionRequest { ..Default::default() })
        .await
        .expect("begin_transaction")
        .into_inner();

    // Another caller deletes the document before the transaction commits.
    client
        .delete_document(DeleteDocumentRequest {
            name: doc_name.to_string(),
            ..Default::default()
        })
        .await
        .expect("delete by outsider must succeed");

    // Try to commit the transaction with an UpdateTime precondition pointing to
    // the now-deleted document.
    let mut commit_fields = HashMap::new();
    commit_fields.insert("status".to_string(), str_val("complete"));
    let write = Write {
        operation: Some(Operation::Update(Document {
            name: doc_name.to_string(),
            fields: commit_fields,
            ..Default::default()
        })),
        current_document: Some(Precondition {
            condition_type: Some(ConditionType::UpdateTime(update_time)),
        }),
        ..Default::default()
    };
    let err = client
        .commit(CommitRequest {
            transaction: txn_resp.transaction,
            writes: vec![write],
            ..Default::default()
        })
        .await
        .expect_err("commit on deleted document must fail");
    assert_eq!(
        err.code(),
        tonic::Code::Aborted,
        "committing after external delete must return ABORTED"
    );
}

/// @driving_port @us_a04 @real_io @error
///
/// Feature: Committing an expired transaction returns not-found
///   Given a transaction was opened but its time-to-live has elapsed
///   When  a caller attempts to commit it
///   Then  the caller receives a not-found response
#[tokio::test]
async fn committing_expired_transaction_returns_not_found() {
    let (handle, mut client) = start_test_agent("finops-prod").await;

    // Begin transaction.
    let txn_resp = client
        .begin_transaction(BeginTransactionRequest { ..Default::default() })
        .await
        .expect("begin_transaction")
        .into_inner();

    // Extract the UUID from the token bytes.
    let txn_uuid = uuid::Uuid::from_slice(&txn_resp.transaction)
        .expect("transaction token must be a valid UUID");

    // Artificially age the transaction past the 60-second TTL.
    sqlx::query(
        "UPDATE transactions SET started_at = NOW() - INTERVAL '65 seconds' \
         WHERE transaction_id = $1",
    )
    .bind(txn_uuid)
    .execute(&handle.pool)
    .await
    .expect("age transaction in DB");

    // Attempting to commit must return NOT_FOUND.
    let err = client
        .commit(CommitRequest {
            transaction: txn_resp.transaction,
            writes: vec![],
            ..Default::default()
        })
        .await
        .expect_err("commit of expired transaction must fail");
    assert_eq!(
        err.code(),
        tonic::Code::NotFound,
        "committing an expired transaction must return NOT_FOUND"
    );
}
