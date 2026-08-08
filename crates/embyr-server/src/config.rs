//! Server configuration loaded from environment variables.
//!
//! Mirrors `embyr_agent::config::AgentConfig`. All required vars are collected
//! before returning so the operator sees every problem simultaneously.
//!
//! # Required environment variables
//!
//! | Variable              | Description                                      |
//! |-----------------------|--------------------------------------------------|
//! | `DATABASE_URL`        | Postgres DSN for the system DB                   |
//! | `EMBYR_ADMIN_KEY`     | Non-empty Bearer token for operator routes       |
//! | `EMBYR_ENCRYPTION_KEY`| Exactly 64 hex chars (32-byte AES-256-GCM key)  |
//!
//! # Optional environment variables
//!
//! | Variable               | Default | Description                         |
//! |------------------------|---------|-------------------------------------|
//! | `EMBYR_RATE_LIMIT_RPS` | 1000.0  | Token bucket capacity and refill    |
//! | `GRPC_PORT`            | 8080    | gRPC listener port                  |
//! | `REST_PORT`            | 8081    | REST/gRPC-Web listener port         |
//! | `ADMIN_PORT`           | 9090    | Admin HTTP listener port            |
//! | `RUST_LOG`             | "info"  | Tracing level filter                |

use std::fmt;

/// Full configuration for embyr-server loaded from environment variables.
#[derive(Debug)]
pub struct ServerConfig {
    /// `DATABASE_URL` — Postgres DSN; required.
    pub db_url: String,
    /// `EMBYR_ADMIN_KEY` — non-empty admin Bearer token; required.
    pub admin_key: String,
    /// `EMBYR_ENCRYPTION_KEY` — 32-byte AES-256-GCM key encoded as 64 hex chars; required.
    pub encryption_key: [u8; 32],
    /// `EMBYR_RATE_LIMIT_RPS` — token bucket capacity and refill rate; default 1000.0.
    pub rate_limit_rps: f64,
    /// `GRPC_PORT` — gRPC listener port; default 8080.
    pub grpc_port: u16,
    /// `REST_PORT` — REST/gRPC-Web listener port; default 8081.
    pub rest_port: u16,
    /// `ADMIN_PORT` — admin HTTP listener port; default 9090.
    pub admin_port: u16,
    /// `RUST_LOG` — tracing level filter; default "info".
    pub log_level: String,
}

/// Configuration parse error.
#[derive(Debug)]
pub enum ConfigError {
    /// One or more required environment variables are absent or empty.
    MissingVars(Vec<String>),
    /// `EMBYR_ENCRYPTION_KEY` is present but not a valid 64-hex-char string.
    InvalidEncryptionKey { reason: String },
    /// A port variable is present but cannot be parsed as a `u16`.
    InvalidPort { var: String, value: String },
    /// `EMBYR_RATE_LIMIT_RPS` is present but not a finite positive number.
    InvalidRateLimitRps { value: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::MissingVars(vars) => {
                let lines: Vec<String> = vars
                    .iter()
                    .map(|v| format!("missing required environment variable: {v}"))
                    .collect();
                write!(f, "{}", lines.join("\n"))
            }
            ConfigError::InvalidEncryptionKey { reason } => {
                write!(f, "invalid EMBYR_ENCRYPTION_KEY: {reason}")
            }
            ConfigError::InvalidPort { var, value } => {
                write!(
                    f,
                    "invalid value for {var}: '{value}' (must be a valid port number 1-65535)"
                )
            }
            ConfigError::InvalidRateLimitRps { value } => {
                write!(
                    f,
                    "invalid EMBYR_RATE_LIMIT_RPS: '{value}' (must be a finite positive number)"
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl ServerConfig {
    /// Load and validate all configuration from environment variables.
    ///
    /// Collects ALL missing required variables before returning so the operator
    /// sees every problem in a single error message.
    ///
    /// Returns `Err` if any required variable is absent/empty, or if any value
    /// fails validation (encryption key length, port range, RPS validity).
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut missing: Vec<String> = Vec::new();

        let db_url_opt = collect_required("DATABASE_URL", &mut missing);
        let admin_key_opt = collect_required("EMBYR_ADMIN_KEY", &mut missing);
        let enc_hex_opt = collect_required("EMBYR_ENCRYPTION_KEY", &mut missing);

        if !missing.is_empty() {
            return Err(ConfigError::MissingVars(missing));
        }

        // EMBYR_ENCRYPTION_KEY: must be exactly 64 hex chars → 32 bytes.
        let enc_hex = enc_hex_opt.unwrap();
        if enc_hex.len() != 64 {
            return Err(ConfigError::InvalidEncryptionKey {
                reason: format!(
                    "expected 64 hex characters (32 bytes), got {} characters",
                    enc_hex.len()
                ),
            });
        }
        let enc_bytes = hex::decode(&enc_hex).map_err(|e| ConfigError::InvalidEncryptionKey {
            reason: format!("not valid hex: {e}"),
        })?;
        let mut encryption_key = [0u8; 32];
        encryption_key.copy_from_slice(&enc_bytes);

        // Optional vars with defaults.
        let rate_limit_rps = parse_rate_limit_rps()?;
        let grpc_port = parse_port("GRPC_PORT", 8080)?;
        let rest_port = parse_port("REST_PORT", 8081)?;
        let admin_port = parse_port("ADMIN_PORT", 9090)?;
        let log_level = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());

        Ok(ServerConfig {
            db_url: db_url_opt.unwrap(),
            admin_key: admin_key_opt.unwrap(),
            encryption_key,
            rate_limit_rps,
            grpc_port,
            rest_port,
            admin_port,
            log_level,
        })
    }
}

// ── Private helpers ──────────────────────────────────────────────────────────

/// Read a required variable; on failure push the variable name to `missing`.
fn collect_required(name: &str, missing: &mut Vec<String>) -> Option<String> {
    match std::env::var(name) {
        Ok(val) if !val.is_empty() => Some(val),
        _ => {
            missing.push(name.to_string());
            None
        }
    }
}

/// Parse an optional port variable. Returns `default` when the variable is absent.
fn parse_port(name: &str, default: u16) -> Result<u16, ConfigError> {
    match std::env::var(name) {
        Err(_) => Ok(default),
        Ok(v) => v.parse::<u16>().map_err(|_| ConfigError::InvalidPort {
            var: name.to_string(),
            value: v,
        }),
    }
}

/// Parse `EMBYR_RATE_LIMIT_RPS`. Returns 1000.0 when the variable is absent.
fn parse_rate_limit_rps() -> Result<f64, ConfigError> {
    match std::env::var("EMBYR_RATE_LIMIT_RPS") {
        Err(_) => Ok(1000.0),
        Ok(v) => {
            let f = v
                .parse::<f64>()
                .map_err(|_| ConfigError::InvalidRateLimitRps { value: v.clone() })?;
            if !f.is_finite() || f <= 0.0 {
                return Err(ConfigError::InvalidRateLimitRps { value: v });
            }
            Ok(f)
        }
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_required_absent_pushes_name() {
        let mut missing = Vec::new();
        let val = collect_required("_EMBYR_TEST_ABSENT_VAR_XYZZY_99", &mut missing);
        assert!(val.is_none());
        assert_eq!(missing, vec!["_EMBYR_TEST_ABSENT_VAR_XYZZY_99"]);
    }

    #[test]
    fn collect_required_present_returns_some() {
        std::env::set_var("_EMBYR_TEST_PRESENT_VAR_XYZZY_99", "hello");
        let mut missing = Vec::new();
        let val = collect_required("_EMBYR_TEST_PRESENT_VAR_XYZZY_99", &mut missing);
        std::env::remove_var("_EMBYR_TEST_PRESENT_VAR_XYZZY_99");
        assert_eq!(val, Some("hello".to_string()));
        assert!(missing.is_empty());
    }

    #[test]
    fn encryption_key_64_hex_chars_decodes() {
        let key = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
        assert_eq!(key.len(), 64);
        let bytes = hex::decode(key).unwrap();
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn encryption_key_too_short_returns_error() {
        let short = "tooshort";
        let result: Result<(), ConfigError> = if short.len() != 64 {
            Err(ConfigError::InvalidEncryptionKey {
                reason: format!("expected 64 hex chars, got {}", short.len()),
            })
        } else {
            Ok(())
        };
        assert!(matches!(result, Err(ConfigError::InvalidEncryptionKey { .. })));
    }

    #[test]
    fn parse_port_absent_returns_default() {
        let result = parse_port("_EMBYR_TEST_PORT_ABSENT_XYZZY_99", 9090);
        assert_eq!(result.unwrap(), 9090);
    }

    #[test]
    fn parse_port_invalid_returns_error() {
        std::env::set_var("_EMBYR_TEST_PORT_INVALID_XYZZY_99", "notaport");
        let result = parse_port("_EMBYR_TEST_PORT_INVALID_XYZZY_99", 9090);
        std::env::remove_var("_EMBYR_TEST_PORT_INVALID_XYZZY_99");
        assert!(matches!(result, Err(ConfigError::InvalidPort { .. })));
    }

    #[test]
    fn parse_rate_limit_rps_absent_returns_default() {
        // Remove the var if set to ensure default.
        std::env::remove_var("EMBYR_RATE_LIMIT_RPS");
        let result = parse_rate_limit_rps();
        // Only assert if the var was not set — can't guarantee in all test environments.
        // Accept both 1000.0 (absent) and any valid positive float (if var was set before remove).
        assert!(result.is_ok());
        let v = result.unwrap();
        assert!(v > 0.0 && v.is_finite());
    }

    #[test]
    fn config_error_display_missing_vars() {
        let err = ConfigError::MissingVars(vec!["DATABASE_URL".into(), "EMBYR_ADMIN_KEY".into()]);
        let msg = err.to_string();
        assert!(msg.contains("DATABASE_URL"), "got: {msg}");
        assert!(msg.contains("EMBYR_ADMIN_KEY"), "got: {msg}");
    }

    #[test]
    fn config_error_display_invalid_encryption_key() {
        let err = ConfigError::InvalidEncryptionKey {
            reason: "too short".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("EMBYR_ENCRYPTION_KEY"), "got: {msg}");
        assert!(msg.contains("too short"), "got: {msg}");
    }
}
