// embyr-agent: statically linked customer-VPC binary (Linux musl target).
// Connects from the customer VPC to the embyr-server.

use embyr_agent::{config, probe, server};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // Install the ring crypto provider for rustls before any TLS operations.
    // rustls 0.23 requires an explicit process-level provider when no default
    // feature flag selects one automatically.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Load config before tracing init so we can use cfg.log_level.
    let cfg = match config::AgentConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("embyr-agent: {e}");
            std::process::exit(1);
        }
    };

    // Default to cfg.log_level; RUST_LOG overrides if set.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&cfg.log_level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    // Hard-gated startup probe: Postgres connectivity + TLS cert validity.
    // This runs BEFORE any port is bound. On failure, exit code 1 is returned
    // and the process terminates without opening the gRPC listener.
    let startup_probe = probe::StartupProbe::new(&cfg.db_dsn, &cfg.cert_path);
    if let Err(e) = startup_probe.run().await {
        eprintln!("embyr-agent: startup probe failed: {e}");
        std::process::exit(1);
    }

    if let Err(err) = server::run(cfg).await {
        eprintln!("embyr-agent: fatal error: {err}");
        std::process::exit(1);
    }
}
