//! Observability helpers — metric label mappings for gRPC handlers.
//!
//! Centralises the `tonic::Code → label string` mapping (AC-OBS-02-03) and
//! the gRPC method name constants used by `embyr_grpc_requests_total` and
//! `embyr_grpc_request_duration_seconds`.

// ── gRPC method name constants ─────────────────────────────────────────────

pub const METHOD_GET_DOCUMENT: &str = "GetDocument";
pub const METHOD_CREATE_DOCUMENT: &str = "CreateDocument";
pub const METHOD_UPDATE_DOCUMENT: &str = "UpdateDocument";
pub const METHOD_DELETE_DOCUMENT: &str = "DeleteDocument";
pub const METHOD_LIST_DOCUMENTS: &str = "ListDocuments";
pub const METHOD_BATCH_GET_DOCUMENTS: &str = "BatchGetDocuments";
pub const METHOD_BEGIN_TRANSACTION: &str = "BeginTransaction";
pub const METHOD_COMMIT: &str = "Commit";
pub const METHOD_BATCH_WRITE: &str = "BatchWrite";
pub const METHOD_ROLLBACK: &str = "Rollback";
pub const METHOD_WRITE: &str = "Write";
pub const METHOD_RUN_QUERY: &str = "RunQuery";
pub const METHOD_RUN_AGGREGATION_QUERY: &str = "RunAggregationQuery";
pub const METHOD_LISTEN: &str = "Listen";

// ── Status label mapping (AC-OBS-02-03) ───────────────────────────────────

/// Map a [`tonic::Code`] to the lowercase Prometheus label string used in
/// `embyr_grpc_requests_total{status=...}`.
pub fn grpc_status_label(code: tonic::Code) -> &'static str {
    match code {
        tonic::Code::Ok => "ok",
        tonic::Code::NotFound => "not_found",
        tonic::Code::Unauthenticated => "unauthenticated",
        tonic::Code::PermissionDenied => "permission_denied",
        tonic::Code::ResourceExhausted => "resource_exhausted",
        tonic::Code::Internal => "internal",
        tonic::Code::Unavailable => "unavailable",
        tonic::Code::Aborted => "aborted",
        tonic::Code::AlreadyExists => "already_exists",
        tonic::Code::InvalidArgument => "invalid_argument",
        tonic::Code::FailedPrecondition => "failed_precondition",
        tonic::Code::Unimplemented => "unimplemented",
        tonic::Code::DeadlineExceeded => "deadline_exceeded",
        tonic::Code::Cancelled => "cancelled",
        tonic::Code::DataLoss => "data_loss",
        tonic::Code::OutOfRange => "out_of_range",
        _ => "unknown",
    }
}

// ── Metric recording helpers ───────────────────────────────────────────────

/// Increment `embyr_grpc_requests_total{method, status}`.
pub fn record_grpc_request(method: &'static str, status: &'static str) {
    metrics::counter!(
        "embyr_grpc_requests_total",
        "method" => method,
        "status" => status
    )
    .increment(1);
}

/// Record a wall-clock duration sample for `embyr_grpc_request_duration_seconds`.
pub fn record_grpc_duration(method: &'static str, elapsed_secs: f64) {
    metrics::histogram!(
        "embyr_grpc_request_duration_seconds",
        "method" => method
    )
    .record(elapsed_secs);
}

/// Combined helper: record both counter and histogram for a completed gRPC call.
///
/// Generic over the `Ok` response type so it works for all 10 gRPC handlers.
/// Called from each thin wrapper in the [`Firestore`] trait impl.
pub fn record_grpc_call<T>(
    method: &'static str,
    result: &Result<tonic::Response<T>, tonic::Status>,
    start: std::time::Instant,
) {
    let code = match result {
        Ok(_) => tonic::Code::Ok,
        Err(s) => s.code(),
    };
    record_grpc_request(method, grpc_status_label(code));
    record_grpc_duration(method, start.elapsed().as_secs_f64());
}
