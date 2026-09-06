/// OCC: verify write preconditions within a Postgres transaction.
///
/// Called inside a sqlx transaction before applying writes.
/// Returns `Err(CoreError::TransactionAborted)` if any Write with an
/// update-time precondition finds the document at a different update_time.
use sqlx::{PgConnection, PgPool};

use embyr_core::{
    domain::{document::DocumentPath, transaction::TransactionId},
    error::CoreError,
    storage::backend_adapter::Write,
};

/// Verify all OCC preconditions for the given writes inside an open Postgres transaction.
///
/// `Write::Update` and `Write::Delete` with `version: Some(v)` are OCC-checked:
/// the document's `version` column must equal `v`.
/// A missing document or mismatched version returns `CoreError::TransactionAborted`.
pub async fn verify_versions(
    conn: &mut PgConnection,
    project_id: &str,
    writes: &[Write],
) -> Result<(), CoreError> {
    for write in writes {
        let (path, expected_version) = match write {
            Write::Update { path, version: Some(v), .. } => (path, *v),
            Write::Delete { path, version: Some(v), .. } => (path, *v),
            _ => continue,
        };

        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT version FROM documents \
             WHERE project_id = $1 \
               AND collection_path = $2 \
               AND document_id = $3 \
               AND NOT deleted \
             FOR UPDATE",
        )
        .bind(project_id)
        .bind(&path.collection_path)
        .bind(&path.document_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        match row {
            Some((actual,)) if actual == expected_version => {}
            _ => return Err(CoreError::TransactionAborted),
        }
    }
    Ok(())
}

/// Encode a document path as the flat string key used in a transaction's
/// own `reads` JSONB map (firestore-transaction-read-consistency).
fn read_key(path: &DocumentPath) -> String {
    format!("{}/{}", path.collection_path, path.document_id)
}

/// Record a transactional read (firestore-transaction-read-consistency).
///
/// `version` is `Some(v)` when the document existed at read time, `None`
/// when the read confirmed the document does NOT exist — both are recorded,
/// since a concurrent CREATE of an absent document is also a conflict
/// `verify_reads` must catch at commit time.
///
/// Only an `active`, unexpired transaction (same 60s window
/// `commit_transaction` enforces) accepts a read registration — an unknown,
/// expired, or already-terminal transaction ID returns the identical
/// `CoreError::TransactionNotFound` `commit_transaction`/`rollback_transaction`
/// already return for this case.
pub async fn record_read(
    pool: &PgPool,
    project_id: &str,
    transaction_id: &TransactionId,
    path: &DocumentPath,
    version: Option<i64>,
) -> Result<(), CoreError> {
    let txn_uuid = crate::backend_adapter::uuid_from_bytes(&transaction_id.0)?;
    let key = read_key(path);
    let version_json = match version {
        Some(v) => serde_json::Value::from(v),
        None => serde_json::Value::Null,
    };

    let result = sqlx::query(
        "UPDATE transactions \
         SET reads = reads || jsonb_build_object($3::text, $4::jsonb) \
         WHERE transaction_id = $1 \
           AND project_id = $2 \
           AND status = 'active' \
           AND started_at > NOW() - INTERVAL '60 seconds'",
    )
    .bind(txn_uuid)
    .bind(project_id)
    .bind(&key)
    .bind(&version_json)
    .execute(pool)
    .await
    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(CoreError::TransactionNotFound);
    }
    Ok(())
}

/// Verify all recorded transactional reads inside an open Postgres transaction.
///
/// For each `(path, expected_version)` entry in `reads` (a JSONB object
/// mapping `"{collection_path}/{document_id}"` -> a version number, or JSON
/// `null` for a confirmed-absent read), the document's CURRENT state must
/// still match: a number requires the document to exist with that exact
/// `version`; `null` requires the document to still NOT exist. Any mismatch
/// returns `CoreError::TransactionAborted` — the identical error
/// `verify_versions` already returns for a write-attached version mismatch.
pub async fn verify_reads(
    conn: &mut PgConnection,
    project_id: &str,
    reads: &serde_json::Value,
) -> Result<(), CoreError> {
    let Some(entries) = reads.as_object() else {
        return Ok(());
    };

    for (key, expected) in entries {
        let Some((collection_path, document_id)) = key.rsplit_once('/') else {
            continue;
        };

        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT version FROM documents \
             WHERE project_id = $1 \
               AND collection_path = $2 \
               AND document_id = $3 \
               AND NOT deleted \
             FOR UPDATE",
        )
        .bind(project_id)
        .bind(collection_path)
        .bind(document_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        let satisfied = match (row, expected.as_i64()) {
            (Some((actual,)), Some(expected_version)) => actual == expected_version,
            (None, None) if expected.is_null() => true,
            _ => false,
        };
        if !satisfied {
            return Err(CoreError::TransactionAborted);
        }
    }
    Ok(())
}
