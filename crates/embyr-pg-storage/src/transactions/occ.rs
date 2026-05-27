/// OCC: verify write preconditions within a Postgres transaction.
///
/// Called inside a sqlx transaction before applying writes.
/// Returns `Err(CoreError::TransactionAborted)` if any Write with an
/// update-time precondition finds the document at a different update_time.
use sqlx::PgConnection;

use embyr_core::{error::CoreError, storage::backend_adapter::Write};

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
