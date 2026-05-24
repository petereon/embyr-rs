use crate::domain::project::ProjectId;

/// Driven port for recording per-project usage metrics.
pub trait MetricsPort: Send + Sync {
    fn record_request(
        &self,
        project_id: &ProjectId,
        ingress_bytes: u64,
        egress_bytes: u64,
        cpu_ms: u64,
    );
}
