//! Prometheus metrics recorder — installed exactly once per process via `OnceLock`.
//!
//! Call [`get_or_install_prometheus_handle`] at server startup (before any TCP
//! listener opens) to ensure the global recorder is active before any metric
//! is emitted.  Subsequent calls return a clone of the already-installed handle.
//!
//! The `OnceLock` guarantees idempotency: multiple test server constructors
//! in the same process each call this function, but `install_recorder()` is
//! invoked only once.  `PrometheusHandle` is `Clone` — cloning shares the
//! underlying recorder arc without creating a new global recorder.

use std::sync::OnceLock;

use metrics_exporter_prometheus::PrometheusHandle;

static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Histogram bucket boundaries for `embyr_grpc_request_duration_seconds`.
///
/// The `2.0` bucket aligns to the Firestore SLA threshold (p99 ≤ 2 s, AC-05c),
/// enabling `histogram_quantile(0.99, ...) > 2.0` as the SLO alert expression.
const GRPC_DURATION_BUCKETS: &[f64] =
    &[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0];

/// Return the installed [`PrometheusHandle`], installing it on first call.
///
/// Safe to call from multiple server constructors in the same process: the
/// [`OnceLock`] ensures [`install_recorder`] is called at most once.
///
/// # Panics
///
/// Panics if the histogram bucket boundary configuration fails or if
/// `install_recorder()` fails.  Both are programming errors — not runtime
/// recoverable states (e.g., a second incompatible recorder already installed).
pub fn get_or_install_prometheus_handle() -> PrometheusHandle {
    PROMETHEUS_HANDLE
        .get_or_init(|| {
            use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
            PrometheusBuilder::new()
                .set_buckets_for_metric(
                    Matcher::Prefix("embyr_grpc_request_duration".to_string()),
                    GRPC_DURATION_BUCKETS,
                )
                .expect("failed to configure gRPC duration histogram buckets")
                .install_recorder()
                .expect("failed to install Prometheus metrics recorder")
        })
        .clone()
}
