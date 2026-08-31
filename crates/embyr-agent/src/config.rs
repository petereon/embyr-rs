//! Configuration for embyr-agent.
//!
//! Reads required environment variables on startup and returns `Err` with a
//! descriptive message naming each missing variable.  The caller is
//! responsible for writing the error to stderr and exiting.

/// Full configuration for embyr-agent loaded from environment variables.
pub struct AgentConfig {
    /// PostgreSQL DSN for the customer database.
    pub db_dsn: String,
    /// Project identifier this agent instance serves.
    pub project_id: String,
    /// Path to the PEM-encoded server TLS certificate file.
    pub cert_path: String,
    /// Path to the PEM-encoded server TLS private key file.
    pub key_path: String,
    /// Path to the PEM-encoded CA certificate used to verify client certs (mTLS).
    pub ca_path: String,
    /// Listen address for the gRPC server (default: 0.0.0.0:9191).
    pub listen_addr: String,
    /// Maximum Postgres connection pool size (default: 25).
    pub max_conns: u32,
    /// Tracing subscriber log level (default: "info").
    pub log_level: String,
    /// Graceful-shutdown drain timeout in seconds (default: 30).
    pub shutdown_timeout_secs: u64,
    /// Terminal transaction row retention window in days (default: 30).
    pub transaction_retention_days: i64,
}

impl AgentConfig {
    /// Load configuration from environment variables.
    ///
    /// Returns `Err` containing a newline-separated list of diagnostics
    /// when any required variable is absent or empty.
    pub fn from_env() -> Result<Self, String> {
        let mut errors: Vec<String> = Vec::new();

        let db_dsn = collect_required("EMBYR_AGENT_DB_DSN", &mut errors);
        let project_id = collect_required("EMBYR_AGENT_PROJECT_ID", &mut errors);
        let cert_path = collect_required("EMBYR_AGENT_CERT", &mut errors);
        let key_path = collect_required("EMBYR_AGENT_KEY", &mut errors);
        let ca_path = collect_required("EMBYR_AGENT_CA", &mut errors);

        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }

        let max_conns = parse_optional_u32("EMBYR_AGENT_MAX_CONNS", 25)?;
        let shutdown_timeout_secs =
            parse_optional_u64("EMBYR_AGENT_SHUTDOWN_TIMEOUT_SECS", 30)?;
        let transaction_retention_days =
            parse_optional_i64("EMBYR_AGENT_TRANSACTION_RETENTION_DAYS", 30)?;

        Ok(AgentConfig {
            db_dsn: db_dsn.unwrap(),
            project_id: project_id.unwrap(),
            cert_path: cert_path.unwrap(),
            key_path: key_path.unwrap(),
            ca_path: ca_path.unwrap(),
            listen_addr: std::env::var("EMBYR_AGENT_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:9191".to_string()),
            max_conns,
            log_level: std::env::var("EMBYR_AGENT_LOG_LEVEL")
                .unwrap_or_else(|_| "info".to_string()),
            shutdown_timeout_secs,
            transaction_retention_days,
        })
    }
}

/// Read a required variable; on failure push a diagnostic and return `None`.
fn collect_required(name: &str, errors: &mut Vec<String>) -> Option<String> {
    match std::env::var(name) {
        Ok(val) if !val.is_empty() => Some(val),
        _ => {
            errors.push(format!(
                "missing required environment variable: {name}"
            ));
            None
        }
    }
}

fn parse_optional_u32(name: &str, default: u32) -> Result<u32, String> {
    match std::env::var(name) {
        Err(_) => Ok(default),
        Ok(v) => v
            .parse::<u32>()
            .map_err(|_| format!("invalid value for {name}: {v}")),
    }
}

fn parse_optional_u64(name: &str, default: u64) -> Result<u64, String> {
    match std::env::var(name) {
        Err(_) => Ok(default),
        Ok(v) => v
            .parse::<u64>()
            .map_err(|_| format!("invalid value for {name}: {v}")),
    }
}

fn parse_optional_i64(name: &str, default: i64) -> Result<i64, String> {
    match std::env::var(name) {
        Err(_) => Ok(default),
        Ok(v) => v
            .parse::<i64>()
            .map_err(|_| format!("invalid value for {name}: {v}")),
    }
}
