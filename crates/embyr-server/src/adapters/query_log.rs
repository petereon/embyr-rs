// SCAFFOLD: true
//! PostgresQueryLogAdapter — implements IQueryLogWriter.
//!
//! Accepts `QueryLogEntry` via a bounded Tokio channel (1000).
//! Spawns a background task that inserts entries into the partitioned
//! `query_logs_<project_id>_<date>` table.
//!
//! Fire-and-forget: if the channel is full, entry is silently dropped.
//! Failure is logged but never fatal.

use embyr_core::admin::query_log::{IQueryLogWriter, QueryLogEntry};

/// Postgres-backed query log writer.
///
/// # RED scaffold
pub struct PostgresQueryLogAdapter;

impl PostgresQueryLogAdapter {
    pub fn new_scaffold() -> Self {
        panic!("Not yet implemented -- RED scaffold: PostgresQueryLogAdapter requires system_db + B-04 migration")
    }
}

impl IQueryLogWriter for PostgresQueryLogAdapter {
    fn record(&self, entry: QueryLogEntry) {
        panic!("Not yet implemented -- RED scaffold: PostgresQueryLogAdapter::record")
    }
}
