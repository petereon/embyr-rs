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

use embyr_proto::agent::{
    storage_agent_client::StorageAgentClient,
    BeginTransactionRequest, CommitRequest, RollbackRequest,
};

use super::agent_common::{start_test_agent, AgentHandle};

// ---------------------------------------------------------------------------
// Happy path scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a04 @real_io
///
/// Feature: Transaction with no concurrent competition commits successfully
///   Given "orders/ord-2026-001" exists at generation 3 with status="processing"
///   When  a caller opens a transaction, reads the document, and commits with status="complete"
///   Then  the transaction commits; the document is at generation 4 with status="complete"
#[tokio::test]
#[ignore = "requires Docker — unskip in S04A delivery"]
async fn transaction_with_no_competition_commits_successfully() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a04 @real_io
///
/// Feature: Rolling back a transaction discards writes without applying them
///   Given "orders/ord-2026-001" exists at generation 2 with status="processing"
///   And   a transaction opens and prepares a write setting status="complete"
///   When  the caller rolls back the transaction
///   Then  the document still shows status="processing" at generation 2
#[tokio::test]
#[ignore = "requires Docker — unskip in S04A delivery"]
async fn rolling_back_transaction_discards_writes_without_applying_them() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a04 @real_io @error
///
/// Feature: Rolling back a transaction that has already been committed returns not-found
///   Given a transaction that has already committed successfully
///   When  a caller attempts to roll back the same transaction
///   Then  the caller receives a not-found response; committed changes remain intact
#[tokio::test]
#[ignore = "requires Docker — unskip in S04A delivery"]
async fn rolling_back_already_committed_transaction_returns_not_found() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a04 @real_io
///
/// Feature: Expired transactions are removed by the sweep operation
///   Given two transactions exist: one expired, one still active
///   When  the sweep operation runs
///   Then  the expired record is removed; the active record remains
#[tokio::test]
#[ignore = "requires Docker — unskip in S04A delivery"]
async fn expired_transactions_are_removed_by_sweep() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

// ---------------------------------------------------------------------------
// Error scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a04 @real_io @error
///
/// Feature: Concurrent transaction on same generation is rejected with a conflict
///   Given "orders/ord-2026-001" exists at generation 3
///   And   two transactions both read it at generation 3
///   When  both attempt to commit
///   Then  one succeeds; the other receives a conflict-aborted response citing the path
#[tokio::test]
#[ignore = "requires Docker — unskip in S04A delivery"]
async fn concurrent_transaction_on_same_generation_is_rejected_with_conflict() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a04 @real_io @error
///
/// Feature: Transaction reading a document that was later deleted is aborted on commit
///   Given "orders/ord-2026-001" exists at generation 5
///   And   a transaction reads it; another caller deletes it before the transaction commits
///   When  the transaction attempts to commit
///   Then  the commit returns an aborted response indicating the document was removed
#[tokio::test]
#[ignore = "requires Docker — unskip in S04A delivery"]
async fn transaction_on_deleted_document_is_aborted_on_commit() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a04 @real_io @error
///
/// Feature: Committing an expired transaction returns not-found
///   Given a transaction was opened but its time-to-live has elapsed
///   When  a caller attempts to commit it
///   Then  the caller receives a not-found response
#[tokio::test]
#[ignore = "requires Docker — unskip in S04A delivery"]
async fn committing_expired_transaction_returns_not_found() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}
