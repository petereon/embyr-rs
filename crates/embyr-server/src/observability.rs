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
            let handle = PrometheusBuilder::new()
                .set_buckets_for_metric(
                    Matcher::Prefix("embyr_grpc_request_duration".to_string()),
                    GRPC_DURATION_BUCKETS,
                )
                .expect("failed to configure gRPC duration histogram buckets")
                .install_recorder()
                .expect("failed to install Prometheus metrics recorder");
            describe_metrics();
            handle
        })
        .clone()
}

/// `# HELP` text for every metric the server emits (audit #69). Must run after
/// the recorder is installed.
fn describe_metrics() {
    use metrics::{describe_counter, describe_gauge, describe_histogram, Unit};
    describe_histogram!("embyr_grpc_request_duration_seconds", Unit::Seconds, "gRPC request latency");
    describe_gauge!("embyr_pg_pool_size", "Connections currently open in the system Postgres pool");
    describe_gauge!("embyr_pg_pool_idle", "Idle connections in the system Postgres pool");
    describe_gauge!(
        "embyr_projects_unconverted_connection_secrets",
        "Projects whose connection secret is not yet converted to the single-store format"
    );
    describe_gauge!(
        "embyr_ttl_last_run_timestamp_seconds",
        "Unix time the TTL sweeper last completed a cycle (max across replicas)"
    );
    for (name, help) in [
        ("embyr_grpc_requests_total", "gRPC requests handled"),
        ("embyr_rate_limit_requests_total", "API requests evaluated by the rate limiter"),
        ("embyr_rate_limit_pg_error_total", "Rate limiter Postgres errors (fell back to in-process bucket)"),
        ("embyr_rate_limit_pg_timeout_total", "Rate limiter Postgres timeouts (fell back to in-process bucket)"),
        ("embyr_signin_rate_limit_requests_total", "Sign-in attempts evaluated by the rate limiter"),
        ("embyr_signin_rate_limit_pg_error_total", "Sign-in rate limiter Postgres errors"),
        ("embyr_signin_rate_limit_pg_timeout_total", "Sign-in rate limiter Postgres timeouts"),
        ("embyr_signin_rate_limit_sweeper_purged_total", "Sign-in rate limit rows purged by the sweeper"),
        ("embyr_pg_notify_listener_reconnect_attempts_total", "Realtime NOTIFY listener reconnect attempts"),
        ("embyr_dsn_convert_divergent_total", "Connection secret conversions whose result diverged"),
        ("embyr_rollup_dropped_total", "Metric rollup events dropped"),
        ("embyr_rollup_flush_total", "Metric rollup flushes"),
        ("embyr_hourly_metrics_retention_purged_total", "Hourly metric rows purged by the retention sweeper"),
        ("embyr_soft_delete_purge_sweeper_purged_total", "Soft-deleted projects purged"),
        ("embyr_account_purge_sweeper_purged_total", "Soft-deleted accounts purged"),
        ("embyr_transaction_sweeper_reclaimed_total", "Abandoned transactions reclaimed"),
        ("embyr_transaction_sweeper_purged_total", "Finished transaction rows purged"),
        ("embyr_sweeper_error_total", "Background sweeper errors"),
        ("embyr_ttl_deleted_total", "Documents deleted by TTL policies"),
        ("embyr_ttl_sweep_errors_total", "TTL sweeper errors (policy load, connect, query or commit)"),
        ("embyr_ttl_skipped_projects_total", "Projects skipped by the TTL sweeper (reason label)"),
        ("embyr_stripe_webhook_dispatch_total", "Stripe webhook events dispatched"),
    ] {
        describe_counter!(name, help);
    }
}
