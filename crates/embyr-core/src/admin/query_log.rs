// SCAFFOLD: true
//! IQueryLogWriter port trait and supporting types.
//!
//! Fire-and-forget append-only write path.
//! Bounded channel (1000) prevents unbounded task accumulation.
//! Zero client latency impact.

/// Type of Firestore operation being logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationType {
    Read,
    Write,
    Delete,
    Listen,
    RunQuery,
}

/// Outcome status of an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpStatus {
    Ok,
    Error,
}

/// A single query log entry.
#[derive(Debug, Clone)]
pub struct QueryLogEntry {
    pub project_id: uuid::Uuid,
    pub timestamp: std::time::SystemTime,
    pub op: OperationType,
    pub status: OpStatus,
    pub collection_path: String,
    pub latency_ms: u64,
    pub docs_returned: u32,
    pub docs_scanned: u32,
}

/// Driven port: fire-and-forget query log writer.
///
/// Implementors: PostgresQueryLogAdapter.
/// If the channel is full, the entry is silently dropped (best-effort).
pub trait IQueryLogWriter: Send + Sync {
    /// Record a log entry. Non-blocking — spawns or enqueues internally.
    fn record(&self, entry: QueryLogEntry);
}
