/// Domain errors for the embyr-core hexagonal layer.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("project not found: {0}")]
    ProjectNotFound(String),

    #[error("document not found: {0}")]
    DocumentNotFound(String),

    #[error("optimistic concurrency conflict")]
    OccConflict,

    #[error("transaction not found or expired")]
    TransactionNotFound,

    #[error("transaction aborted")]
    TransactionAborted,

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("backend unavailable: {0}")]
    BackendUnavailable(String),

    #[error("failed precondition: {0}")]
    FailedPrecondition(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("unauthenticated")]
    Unauthenticated,

    #[error("resource exhausted: {0}")]
    ResourceExhausted(String),
}
