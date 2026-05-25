use sqlx::PgPool;

/// Checks whether a READY composite index exists in the system DB for a
/// given (project_id, collection_id) pair.
///
/// The index_manager holds a reference to the system DB pool and is used
/// by the gRPC handler to gate multi-field queries before execution.
pub struct IndexManager {
    pool: PgPool,
}

impl IndexManager {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Return `true` if at least one READY composite index exists for the
    /// given project and collection.
    pub async fn is_index_ready(&self, project_id: &str, collection_id: &str) -> bool {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM composite_indexes \
             WHERE project_id = $1 AND collection_path = $2 AND status = 'ready'",
        )
        .bind(project_id)
        .bind(collection_id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        count > 0
    }
}
