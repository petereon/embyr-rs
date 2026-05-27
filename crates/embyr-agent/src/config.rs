//! Configuration for embyr-agent.
//!
//! Reads required environment variables on startup and exits with a
//! non-zero code and a descriptive stderr message if any are missing.

/// Required environment variables for embyr-agent.
pub struct AgentConfig {
    /// PostgreSQL DSN for the customer database.
    pub db_dsn: String,
    /// Path to the PEM-encoded server TLS certificate file.
    pub cert_path: String,
    /// Path to the PEM-encoded server TLS private key file.
    pub key_path: String,
    /// Path to the PEM-encoded CA certificate used to verify client certs (mTLS).
    pub ca_path: String,
    /// Listen address for the gRPC server (default: 0.0.0.0:9191).
    pub listen_addr: String,
}

impl AgentConfig {
    /// Load configuration from environment variables.
    ///
    /// Writes a diagnostic to stderr and exits with code 1 if any required
    /// variable is absent or empty.
    pub fn from_env() -> Self {
        let db_dsn = require_env("EMBYR_AGENT_DB_DSN");
        let cert_path = require_env("EMBYR_AGENT_CERT");
        let key_path = require_env("EMBYR_AGENT_KEY");
        let ca_path = require_env("EMBYR_AGENT_CA");
        let listen_addr = std::env::var("EMBYR_AGENT_LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:9191".to_string());

        AgentConfig {
            db_dsn,
            cert_path,
            key_path,
            ca_path,
            listen_addr,
        }
    }
}

/// Read an environment variable or exit non-zero naming the missing variable.
fn require_env(name: &str) -> String {
    match std::env::var(name) {
        Ok(val) if !val.is_empty() => val,
        _ => {
            eprintln!("embyr-agent: missing required environment variable: {name}");
            std::process::exit(1);
        }
    }
}
