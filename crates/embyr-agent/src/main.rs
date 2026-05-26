// embyr-agent: statically linked customer-VPC binary (Linux musl target).
// Connects from the customer VPC to the embyr-server.

mod config;
mod server;

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // Install the ring crypto provider for rustls before any TLS operations.
    // rustls 0.23 requires an explicit process-level provider when no default
    // feature flag selects one automatically.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Default to INFO level if RUST_LOG is not set.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let cfg = config::AgentConfig::from_env();

    if let Err(err) = server::run(cfg).await {
        eprintln!("embyr-agent: fatal error: {}", err);
        std::process::exit(1);
    }
}
