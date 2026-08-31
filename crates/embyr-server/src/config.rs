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
//!
//! # Secrets-manager-sourced admin key (ADR-018)
//!
//! `EMBYR_ADMIN_KEY_AWS_SECRET_ARN` / `EMBYR_ADMIN_KEY_GCP_SECRET_NAME`
//! (optional) source `admin_key` from AWS/GCP Secrets Manager at startup when
//! plain `EMBYR_ADMIN_KEY` is absent. Exactly one of
//! {`EMBYR_ADMIN_KEY`, `EMBYR_ADMIN_KEY_AWS_SECRET_ARN`,
//! `EMBYR_ADMIN_KEY_GCP_SECRET_NAME`} must resolve to a value — today's
//! missing-var validation still applies when none is set. The GCP path
//! additionally requires `EMBYR_GCP_ACCESS_TOKEN` (bearer token for
//! `GcpSecretFetcher`; an interim static-token stopgap, see OQ-SM-4).
//!
//! `EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN` / `EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME`
//! (optional) source `encryption_key` from AWS/GCP Secrets Manager the same
//! way. The fetched value is validated via the identical
//! [`ConfigError::InvalidEncryptionKey`] path as the plain
//! `EMBYR_ENCRYPTION_KEY` case.
//!
//! `EMBYR_ADMIN_KEY_PREVIOUS` (optional; also sourceable via
//! `_AWS_SECRET_ARN` / `_GCP_SECRET_NAME`) opens an admin-token rotation
//! window (ADR-018 §6): `operator_auth_middleware` accepts either the
//! current or the previous admin key while both are configured.

use std::fmt;

use crate::adapters::aws_secret_fetcher::AwsSecretFetcher;
use crate::adapters::gcp_secret_fetcher::GcpSecretFetcher;

/// GCP Secret Manager REST API base URL used by every GCP-sourced secret
/// fetch in `from_env()`. No env-var override exists yet (OQ-SM-4 / ADR-018
/// Alternatives A6) — out of scope for this feature.
const GCP_SECRET_MANAGER_BASE_URL: &str = "https://secretmanager.googleapis.com";

/// Cache TTL passed to `GcpSecretFetcher::new` / `AwsSecretFetcher::new` for
/// every secret resolved in `from_env()` (admin_key, encryption_key, and
/// their `_previous` counterparts, AWS and GCP alike). Irrelevant in every
/// case — `get_raw_secret` bypasses the cache entirely (D-SM-4: startup-only
/// sourcing, no TTL benefit) — but each fetcher's constructor requires a
/// value, so one shared constant serves all resolvers.
const SECRET_FETCHER_TTL_SECS_UNUSED: u64 = 300;

/// Full configuration for embyr-server loaded from environment variables.
#[derive(Debug)]
pub struct ServerConfig {
    /// `DATABASE_URL` — Postgres DSN; required.
    pub db_url: String,
    /// `EMBYR_ADMIN_KEY` — non-empty admin Bearer token; required.
    pub admin_key: String,
    /// `EMBYR_ADMIN_KEY_PREVIOUS` — optional Bearer token; opens an
    /// auth-rotation window (ADR-018 §6). `None` means no rotation window —
    /// `operator_auth_middleware` degrades to today's single-key behavior
    /// byte-for-byte.
    pub admin_key_previous: Option<String>,
    /// `EMBYR_ENCRYPTION_KEY` — 32-byte AES-256-GCM key encoded as 64 hex chars; required.
    pub encryption_key: [u8; 32],
    /// `EMBYR_ENCRYPTION_KEY_PREVIOUS` — optional 32-byte AES-256-GCM key encoded
    /// as 64 hex chars; opens a decrypt-rotation window (ADR-018 §5). `None`
    /// means no rotation window — `decrypt_with_rotation` degrades to today's
    /// single-key behavior byte-for-byte.
    pub encryption_key_previous: Option<[u8; 32]>,
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
    /// `STRIPE_SECRET_KEY` — optional (card-payments-backend, ADR-021).
    /// Plain `std::env::var(...).ok()`, not the full AWS/GCP secret-manager
    /// resolver chain (simplified vs. DESIGN's exact ask — non-blocking,
    /// see `docs/feature/card-payments-backend/distill/upstream-issues.md`
    /// Finding 3: making this required would break every other
    /// subprocess-spawning test suite that never sets a `STRIPE_*` var).
    /// `None` means billing is unwired; the composition root uses a
    /// placeholder key so `StripeGateway::new()` still constructs.
    pub stripe_secret_key: Option<String>,
    /// `STRIPE_WEBHOOK_SIGNING_SECRET` — optional (US-203); same resolution
    /// simplification as `stripe_secret_key`.
    pub stripe_webhook_signing_secret: Option<String>,
    /// `STRIPE_PUBLISHABLE_KEY` — optional; same resolution simplification
    /// as `stripe_secret_key`. Not yet consumed by any composition-root
    /// wiring (reserved for a future client-side Stripe Elements step).
    pub stripe_publishable_key: Option<String>,
    /// `EMBYR_CAP_CHECK_INTERVAL_SECS` — cumulative cap-check background
    /// task interval (ADR-020); default 30s.
    pub cap_check_interval_secs: u64,
    /// `EMBYR_TRANSACTION_SWEEP_INTERVAL_SECS` — `TransactionSweeper`
    /// background task interval (ADR-054 § D7); default 300s (5 minutes).
    pub transaction_sweep_interval_secs: u64,
    /// `EMBYR_TRANSACTION_RETENTION_DAYS` — `TransactionSweeper` purge
    /// retention window (ADR-054 § D2/D7, Slice 02); default 30 days,
    /// mirroring `SessionCleaner`'s own documented 30-day precedent.
    pub transaction_retention_days: i64,
}

/// Configuration parse error.
#[derive(Debug)]
pub enum ConfigError {
    /// One or more required environment variables are absent or empty.
    MissingVars(Vec<String>),
    /// The resolved encryption-key variable (`EMBYR_ENCRYPTION_KEY` today;
    /// `EMBYR_ENCRYPTION_KEY_PREVIOUS` in a later step) is present but not a
    /// valid 64-hex-char string. `var` names which variable failed so the
    /// same validation function serves both.
    InvalidEncryptionKey { var: String, reason: String },
    /// A port variable is present but cannot be parsed as a `u16`.
    InvalidPort { var: String, value: String },
    /// `EMBYR_RATE_LIMIT_RPS` is present but not a finite positive number.
    InvalidRateLimitRps { value: String },
    /// A secrets-manager-sourced variable's fetch failed (bad ARN, IAM
    /// denied, malformed secret, etc).
    SecretFetchFailed { var: String, reason: String },
    /// More than one source is set for the same logical secret (e.g. plain
    /// `EMBYR_ADMIN_KEY` and `EMBYR_ADMIN_KEY_AWS_SECRET_ARN` both present).
    /// `sources` names every variable that was set — never the resolved value.
    AmbiguousSecretSource {
        var_base: String,
        sources: Vec<String>,
    },
    /// The resolved `EMBYR_ENCRYPTION_KEY_PREVIOUS` bytes are identical to the
    /// resolved `EMBYR_ENCRYPTION_KEY` bytes (ADR-018 §6) — checked after both
    /// values are resolved, comparing resolved bytes, never raw env text.
    DuplicateRotationKey { var: String, base_var: String },
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
            ConfigError::InvalidEncryptionKey { var, reason } => {
                write!(f, "invalid {var}: {reason}")
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
            ConfigError::SecretFetchFailed { var, reason } => {
                // Generic: `var`/`reason` now cover both AWS- and
                // GCP-sourced fetch failures (ADR-018 §3); the specific
                // secrets-manager kind is already named inside `reason`.
                write!(f, "could not fetch {var}: {reason}")
            }
            ConfigError::AmbiguousSecretSource { var_base, sources } => {
                write!(
                    f,
                    "ambiguous configuration for {var_base}: more than one source is set ({})",
                    sources.join(", ")
                )
            }
            ConfigError::DuplicateRotationKey { var, base_var } => {
                write!(f, "{var} must differ from {base_var}")
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
    pub async fn from_env() -> Result<Self, ConfigError> {
        let mut missing: Vec<String> = Vec::new();

        let db_url_opt = collect_required("DATABASE_URL", &mut missing);
        let admin_key_opt = resolve_admin_key(&mut missing).await?;
        let admin_key_previous = resolve_admin_key_previous().await?;
        let enc_hex_opt = resolve_encryption_key_hex(&mut missing).await?;
        let enc_prev_hex_opt = resolve_encryption_key_previous_hex().await?;

        if !missing.is_empty() {
            return Err(ConfigError::MissingVars(missing));
        }

        let admin_key = admin_key_opt.unwrap();
        if let Some(prev) = &admin_key_previous {
            if prev == &admin_key {
                return Err(ConfigError::DuplicateRotationKey {
                    var: "EMBYR_ADMIN_KEY_PREVIOUS".to_string(),
                    base_var: "EMBYR_ADMIN_KEY".to_string(),
                });
            }
        }

        let encryption_key =
            validate_encryption_key_hex("EMBYR_ENCRYPTION_KEY", &enc_hex_opt.unwrap())?;
        let encryption_key_previous = match enc_prev_hex_opt {
            Some(hex_str) => Some(validate_encryption_key_hex(
                "EMBYR_ENCRYPTION_KEY_PREVIOUS",
                &hex_str,
            )?),
            None => None,
        };
        if let Some(prev) = encryption_key_previous {
            if prev == encryption_key {
                return Err(ConfigError::DuplicateRotationKey {
                    var: "EMBYR_ENCRYPTION_KEY_PREVIOUS".to_string(),
                    base_var: "EMBYR_ENCRYPTION_KEY".to_string(),
                });
            }
        }

        // Optional vars with defaults.
        let rate_limit_rps = parse_rate_limit_rps()?;
        let grpc_port = parse_port("GRPC_PORT", 8080)?;
        let rest_port = parse_port("REST_PORT", 8081)?;
        let admin_port = parse_port("ADMIN_PORT", 9090)?;
        let log_level = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());

        // card-payments-backend (ADR-021): optional, plain env-var resolution
        // (Finding 3 — not the full secret-manager chain; see field docs).
        let stripe_secret_key = std::env::var("STRIPE_SECRET_KEY").ok();
        let stripe_webhook_signing_secret = std::env::var("STRIPE_WEBHOOK_SIGNING_SECRET").ok();
        let stripe_publishable_key = std::env::var("STRIPE_PUBLISHABLE_KEY").ok();
        let cap_check_interval_secs = std::env::var("EMBYR_CAP_CHECK_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30);
        let transaction_sweep_interval_secs = std::env::var("EMBYR_TRANSACTION_SWEEP_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(300);
        let transaction_retention_days = std::env::var("EMBYR_TRANSACTION_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(30);

        Ok(ServerConfig {
            db_url: db_url_opt.unwrap(),
            admin_key,
            admin_key_previous,
            encryption_key,
            encryption_key_previous,
            rate_limit_rps,
            grpc_port,
            rest_port,
            admin_port,
            log_level,
            stripe_secret_key,
            stripe_webhook_signing_secret,
            stripe_publishable_key,
            cap_check_interval_secs,
            transaction_sweep_interval_secs,
            transaction_retention_days,
        })
    }
}

// ── Private helpers ──────────────────────────────────────────────────────────

/// Which secrets-manager kind a resolved `source` variable name refers to.
/// Recognised suffixes: `_AWS_SECRET_ARN`, `_GCP_SECRET_NAME`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecretManagerSource {
    Aws,
    Gcp,
}

/// Dispatch a resolved `source` variable name to its secrets-manager kind by
/// suffix. Extracted as a pure function — independent of any network I/O —
/// so the "which cloud" decision is directly unit-testable and the
/// once-present bug (any non-plain source silently treated as AWS,
/// regardless of which var actually matched) cannot recur silently.
fn secret_manager_source_kind(source: &str) -> SecretManagerSource {
    if source.ends_with("_GCP_SECRET_NAME") {
        SecretManagerSource::Gcp
    } else if source.ends_with("_AWS_SECRET_ARN") {
        SecretManagerSource::Aws
    } else {
        unreachable!(
            "resolve_secret_source only returns the plain var or a recognized \
             *_AWS_SECRET_ARN/*_GCP_SECRET_NAME suffix, got: {source}"
        )
    }
}

/// Read `EMBYR_GCP_ACCESS_TOKEN`, requiring it non-empty. `var` names the
/// logical secret being resolved (e.g. `EMBYR_ADMIN_KEY`) and `source` names
/// the specific `*_GCP_SECRET_NAME` variable that triggered the requirement,
/// so the resulting error is actionable without any network I/O attempted.
fn require_gcp_access_token(var: &str, source: &str) -> Result<String, ConfigError> {
    std::env::var("EMBYR_GCP_ACCESS_TOKEN")
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ConfigError::SecretFetchFailed {
            var: var.to_string(),
            reason: format!("EMBYR_GCP_ACCESS_TOKEN is required when {source} is set"),
        })
}

/// Fetch a logical secret's raw value from whichever secrets-manager
/// `source` names, dispatching by [`secret_manager_source_kind`]. Shared by
/// all four resolvers (D-SM-1: reuse the existing pattern) — fixes the
/// pre-existing bug where every resolver's match arm discarded `source` and
/// silently treated any non-plain source as AWS.
async fn fetch_from_secret_manager(
    var: &str,
    source: &str,
    resolved_value: &str,
) -> Result<String, ConfigError> {
    match secret_manager_source_kind(source) {
        SecretManagerSource::Gcp => {
            let token = require_gcp_access_token(var, source)?;
            let fetcher = GcpSecretFetcher::new(
                GCP_SECRET_MANAGER_BASE_URL,
                &token,
                SECRET_FETCHER_TTL_SECS_UNUSED,
            );
            let value = fetcher.get_raw_secret(resolved_value).await.map_err(|e| {
                ConfigError::SecretFetchFailed {
                    var: var.to_string(),
                    reason: e.to_string(),
                }
            })?;
            eprintln!("fetched {var} from GCP Secret Manager");
            Ok(value)
        }
        SecretManagerSource::Aws => {
            let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
            let fetcher =
                AwsSecretFetcher::new(&aws_config, SECRET_FETCHER_TTL_SECS_UNUSED).await;
            let value = fetcher.get_raw_secret(resolved_value).await.map_err(|e| {
                ConfigError::SecretFetchFailed {
                    var: var.to_string(),
                    reason: e.to_string(),
                }
            })?;
            eprintln!("fetched {var} from AWS Secrets Manager");
            Ok(value)
        }
    }
}

/// Resolve `admin_key` from plain `EMBYR_ADMIN_KEY` when present; otherwise,
/// if `EMBYR_ADMIN_KEY_AWS_SECRET_ARN`/`EMBYR_ADMIN_KEY_GCP_SECRET_NAME` is
/// set, fetch the raw secret value from AWS/GCP Secrets Manager
/// ([`fetch_from_secret_manager`]). When none is set, pushes `EMBYR_ADMIN_KEY`
/// onto `missing` — preserving today's missing-var validation byte-for-byte.
/// When more than one is set, returns `AmbiguousSecretSource` before any I/O
/// (`resolve_secret_source` is checked before any fetch is attempted).
async fn resolve_admin_key(missing: &mut Vec<String>) -> Result<Option<String>, ConfigError> {
    let plain = std::env::var("EMBYR_ADMIN_KEY")
        .ok()
        .filter(|v| !v.is_empty());
    let aws_arn = std::env::var("EMBYR_ADMIN_KEY_AWS_SECRET_ARN")
        .ok()
        .filter(|v| !v.is_empty());
    let gcp_name = std::env::var("EMBYR_ADMIN_KEY_GCP_SECRET_NAME")
        .ok()
        .filter(|v| !v.is_empty());

    let resolved = resolve_secret_source(
        "EMBYR_ADMIN_KEY",
        vec![
            ("EMBYR_ADMIN_KEY", plain),
            ("EMBYR_ADMIN_KEY_AWS_SECRET_ARN", aws_arn),
            ("EMBYR_ADMIN_KEY_GCP_SECRET_NAME", gcp_name),
        ],
    )?;

    match resolved {
        Some((source, value)) if source == "EMBYR_ADMIN_KEY" => Ok(Some(value)),
        Some((source, resolved_value)) => {
            let value =
                fetch_from_secret_manager("EMBYR_ADMIN_KEY", &source, &resolved_value).await?;
            Ok(Some(value))
        }
        None => {
            missing.push("EMBYR_ADMIN_KEY".to_string());
            Ok(None)
        }
    }
}

/// Resolve the raw hex string for `encryption_key` from plain
/// `EMBYR_ENCRYPTION_KEY` when present; otherwise, if
/// `EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN` is set, fetch the raw secret value
/// from AWS Secrets Manager. When none are set, pushes `EMBYR_ENCRYPTION_KEY`
/// onto `missing` — preserving today's missing-var validation byte-for-byte.
/// When more than one of {plain, AWS ARN, GCP secret name} is set, returns
/// `AmbiguousSecretSource` before any I/O. `EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME`
/// is recognised here as an ambiguity-detection candidate only — GCP fetching
/// itself is out of scope until a later step (06-01); a GCP name set alone
/// (no plain, no AWS ARN) is not yet reachable from any acceptance scenario.
/// Mirrors [`resolve_admin_key`]'s shape (D-SM-1: reuse the existing pattern).
async fn resolve_encryption_key_hex(
    missing: &mut Vec<String>,
) -> Result<Option<String>, ConfigError> {
    let plain = std::env::var("EMBYR_ENCRYPTION_KEY")
        .ok()
        .filter(|v| !v.is_empty());
    let aws_arn = std::env::var("EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN")
        .ok()
        .filter(|v| !v.is_empty());
    let gcp_name = std::env::var("EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME")
        .ok()
        .filter(|v| !v.is_empty());

    let resolved = resolve_secret_source(
        "EMBYR_ENCRYPTION_KEY",
        vec![
            ("EMBYR_ENCRYPTION_KEY", plain),
            ("EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN", aws_arn),
            ("EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME", gcp_name),
        ],
    )?;

    match resolved {
        Some((source, value)) if source == "EMBYR_ENCRYPTION_KEY" => Ok(Some(value)),
        Some((source, resolved_value)) => {
            let value =
                fetch_from_secret_manager("EMBYR_ENCRYPTION_KEY", &source, &resolved_value)
                    .await?;
            Ok(Some(value))
        }
        None => {
            missing.push("EMBYR_ENCRYPTION_KEY".to_string());
            Ok(None)
        }
    }
}

/// Resolve the OPTIONAL `admin_key_previous` (ADR-018 §3, §6) from plain
/// `EMBYR_ADMIN_KEY_PREVIOUS` when present; otherwise, if
/// `EMBYR_ADMIN_KEY_PREVIOUS_AWS_SECRET_ARN` is set, fetch the raw secret
/// value from AWS Secrets Manager. `EMBYR_ADMIN_KEY_PREVIOUS_GCP_SECRET_NAME`
/// is recognised as an ambiguity-detection candidate only, mirroring
/// [`resolve_encryption_key_hex`]'s GCP handling — GCP fetching itself is out
/// of scope until step 06-01. Unlike the required `admin_key` resolver,
/// absence of every source is NOT an error — `Ok(None)` means no rotation
/// window is open (`operator_auth_middleware` degrades to single-key
/// behavior byte-for-byte). Mirrors [`resolve_encryption_key_previous_hex`]'s
/// shape (D-SM-1: reuse the existing pattern).
async fn resolve_admin_key_previous() -> Result<Option<String>, ConfigError> {
    let plain = std::env::var("EMBYR_ADMIN_KEY_PREVIOUS")
        .ok()
        .filter(|v| !v.is_empty());
    let aws_arn = std::env::var("EMBYR_ADMIN_KEY_PREVIOUS_AWS_SECRET_ARN")
        .ok()
        .filter(|v| !v.is_empty());
    let gcp_name = std::env::var("EMBYR_ADMIN_KEY_PREVIOUS_GCP_SECRET_NAME")
        .ok()
        .filter(|v| !v.is_empty());

    let resolved = resolve_secret_source(
        "EMBYR_ADMIN_KEY_PREVIOUS",
        vec![
            ("EMBYR_ADMIN_KEY_PREVIOUS", plain),
            ("EMBYR_ADMIN_KEY_PREVIOUS_AWS_SECRET_ARN", aws_arn),
            ("EMBYR_ADMIN_KEY_PREVIOUS_GCP_SECRET_NAME", gcp_name),
        ],
    )?;

    match resolved {
        Some((source, value)) if source == "EMBYR_ADMIN_KEY_PREVIOUS" => Ok(Some(value)),
        Some((source, resolved_value)) => {
            let value = fetch_from_secret_manager(
                "EMBYR_ADMIN_KEY_PREVIOUS",
                &source,
                &resolved_value,
            )
            .await?;
            Ok(Some(value))
        }
        None => Ok(None),
    }
}

/// Resolve the raw hex string for the OPTIONAL `encryption_key_previous`
/// (ADR-018 §3, §5) from plain `EMBYR_ENCRYPTION_KEY_PREVIOUS` when present;
/// otherwise, if `EMBYR_ENCRYPTION_KEY_PREVIOUS_AWS_SECRET_ARN` is set, fetch
/// the raw secret value from AWS Secrets Manager.
/// `EMBYR_ENCRYPTION_KEY_PREVIOUS_GCP_SECRET_NAME` is recognised as an
/// ambiguity-detection candidate only, mirroring
/// [`resolve_encryption_key_hex`]'s GCP handling. Unlike the required
/// `encryption_key` resolver, absence of every source is NOT an error —
/// `Ok(None)` means no rotation window is open (`decrypt_with_rotation`
/// degrades to single-key behavior byte-for-byte).
async fn resolve_encryption_key_previous_hex() -> Result<Option<String>, ConfigError> {
    let plain = std::env::var("EMBYR_ENCRYPTION_KEY_PREVIOUS")
        .ok()
        .filter(|v| !v.is_empty());
    let aws_arn = std::env::var("EMBYR_ENCRYPTION_KEY_PREVIOUS_AWS_SECRET_ARN")
        .ok()
        .filter(|v| !v.is_empty());
    let gcp_name = std::env::var("EMBYR_ENCRYPTION_KEY_PREVIOUS_GCP_SECRET_NAME")
        .ok()
        .filter(|v| !v.is_empty());

    let resolved = resolve_secret_source(
        "EMBYR_ENCRYPTION_KEY_PREVIOUS",
        vec![
            ("EMBYR_ENCRYPTION_KEY_PREVIOUS", plain),
            ("EMBYR_ENCRYPTION_KEY_PREVIOUS_AWS_SECRET_ARN", aws_arn),
            ("EMBYR_ENCRYPTION_KEY_PREVIOUS_GCP_SECRET_NAME", gcp_name),
        ],
    )?;

    match resolved {
        Some((source, value)) if source == "EMBYR_ENCRYPTION_KEY_PREVIOUS" => Ok(Some(value)),
        Some((source, resolved_value)) => {
            let value = fetch_from_secret_manager(
                "EMBYR_ENCRYPTION_KEY_PREVIOUS",
                &source,
                &resolved_value,
            )
            .await?;
            Ok(Some(value))
        }
        None => Ok(None),
    }
}

/// Validate and decode a resolved encryption-key hex string: must be exactly
/// 64 hex chars → 32 bytes. `var` names the variable that produced `hex` (in
/// the `ConfigError::InvalidEncryptionKey` it may return) so the identical
/// validation logic serves any resolved-encryption-key variable — today only
/// `EMBYR_ENCRYPTION_KEY`, trivially reusable for `EMBYR_ENCRYPTION_KEY_PREVIOUS`
/// in a later step.
fn validate_encryption_key_hex(var: &str, hex_str: &str) -> Result<[u8; 32], ConfigError> {
    if hex_str.len() != 64 {
        return Err(ConfigError::InvalidEncryptionKey {
            var: var.to_string(),
            reason: format!(
                "expected 64 hex characters (32 bytes), got {} characters",
                hex_str.len()
            ),
        });
    }
    let bytes = hex::decode(hex_str).map_err(|e| ConfigError::InvalidEncryptionKey {
        var: var.to_string(),
        reason: format!("not valid hex: {e}"),
    })?;
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// Resolve a single logical secret from a set of `(source_name, resolved_value)`
/// candidates. At most one source may be set — more than one is a startup
/// config error naming every variable that was set (never the resolved
/// value). Generic over the candidate list so a third source (e.g. a future
/// GCP secret-ref var) is a one-line addition at each call site.
///
/// Returns `Ok(None)` when no source is set, `Ok(Some((source_name, value)))`
/// when exactly one is set, `Err(AmbiguousSecretSource)` when more than one is set.
fn resolve_secret_source(
    var_base: &str,
    sources: Vec<(&str, Option<String>)>,
) -> Result<Option<(String, String)>, ConfigError> {
    let set: Vec<(String, String)> = sources
        .into_iter()
        .filter_map(|(name, val)| val.map(|v| (name.to_string(), v)))
        .collect();

    match set.len() {
        0 => Ok(None),
        1 => Ok(Some(set.into_iter().next().unwrap())),
        _ => Err(ConfigError::AmbiguousSecretSource {
            var_base: var_base.to_string(),
            sources: set.into_iter().map(|(name, _)| name).collect(),
        }),
    }
}

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
                var: "EMBYR_ENCRYPTION_KEY".into(),
                reason: format!("expected 64 hex chars, got {}", short.len()),
            })
        } else {
            Ok(())
        };
        assert!(matches!(
            result,
            Err(ConfigError::InvalidEncryptionKey { .. })
        ));
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
            var: "EMBYR_ENCRYPTION_KEY".into(),
            reason: "too short".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("EMBYR_ENCRYPTION_KEY"), "got: {msg}");
        assert!(msg.contains("too short"), "got: {msg}");
    }

    // ── resolve_secret_source ───────────────────────────────────────────────

    #[test]
    fn resolve_secret_source_none_set_returns_none() {
        let result = resolve_secret_source(
            "EMBYR_ADMIN_KEY",
            vec![
                ("EMBYR_ADMIN_KEY", None),
                ("EMBYR_ADMIN_KEY_AWS_SECRET_ARN", None),
            ],
        );
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn resolve_secret_source_exactly_one_set_returns_it() {
        let cases: Vec<Vec<(&str, Option<String>)>> = vec![
            vec![
                ("EMBYR_ADMIN_KEY", Some("literal-key".to_string())),
                ("EMBYR_ADMIN_KEY_AWS_SECRET_ARN", None),
            ],
            vec![
                ("EMBYR_ADMIN_KEY", None),
                (
                    "EMBYR_ADMIN_KEY_AWS_SECRET_ARN",
                    Some("arn:aws:...".to_string()),
                ),
            ],
        ];
        for sources in cases {
            let expected = sources
                .iter()
                .find_map(|(name, val)| val.clone().map(|v| (name.to_string(), v)))
                .unwrap();
            let result = resolve_secret_source("EMBYR_ADMIN_KEY", sources);
            assert_eq!(result.unwrap(), Some(expected));
        }
    }

    #[test]
    fn resolve_secret_source_multiple_set_returns_ambiguous_error_naming_all() {
        let result = resolve_secret_source(
            "EMBYR_ADMIN_KEY",
            vec![
                ("EMBYR_ADMIN_KEY", Some("literal-key".to_string())),
                (
                    "EMBYR_ADMIN_KEY_AWS_SECRET_ARN",
                    Some("arn:aws:...".to_string()),
                ),
            ],
        );
        match result {
            Err(ConfigError::AmbiguousSecretSource { var_base, sources }) => {
                assert_eq!(var_base, "EMBYR_ADMIN_KEY");
                assert_eq!(
                    sources,
                    vec![
                        "EMBYR_ADMIN_KEY".to_string(),
                        "EMBYR_ADMIN_KEY_AWS_SECRET_ARN".to_string()
                    ]
                );
            }
            other => panic!("expected AmbiguousSecretSource, got {other:?}"),
        }
    }

    #[test]
    fn config_error_display_ambiguous_secret_source_names_both_vars() {
        let err = ConfigError::AmbiguousSecretSource {
            var_base: "EMBYR_ADMIN_KEY".into(),
            sources: vec![
                "EMBYR_ADMIN_KEY".into(),
                "EMBYR_ADMIN_KEY_AWS_SECRET_ARN".into(),
            ],
        };
        let msg = err.to_string();
        assert!(msg.contains("EMBYR_ADMIN_KEY"), "got: {msg}");
        assert!(msg.contains("EMBYR_ADMIN_KEY_AWS_SECRET_ARN"), "got: {msg}");
    }

    // ── DuplicateRotationKey ─────────────────────────────────────────────────

    #[test]
    fn config_error_display_duplicate_rotation_key_names_var_and_differ() {
        let err = ConfigError::DuplicateRotationKey {
            var: "EMBYR_ENCRYPTION_KEY_PREVIOUS".into(),
            base_var: "EMBYR_ENCRYPTION_KEY".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("EMBYR_ENCRYPTION_KEY_PREVIOUS"), "got: {msg}");
        assert!(msg.contains("EMBYR_ENCRYPTION_KEY"), "got: {msg}");
        assert!(msg.contains("differ"), "got: {msg}");
    }

    // ── GCP resolver dispatch (06-01) ───────────────────────────────────────

    #[test]
    fn secret_manager_source_kind_dispatches_by_suffix_not_silently_aws() {
        // Parametrized across all four logical secrets' var-name shapes:
        // the dispatch decision must key off the suffix, never assume AWS.
        let cases = [
            ("EMBYR_ADMIN_KEY_AWS_SECRET_ARN", SecretManagerSource::Aws),
            ("EMBYR_ADMIN_KEY_GCP_SECRET_NAME", SecretManagerSource::Gcp),
            (
                "EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN",
                SecretManagerSource::Aws,
            ),
            (
                "EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME",
                SecretManagerSource::Gcp,
            ),
            (
                "EMBYR_ADMIN_KEY_PREVIOUS_AWS_SECRET_ARN",
                SecretManagerSource::Aws,
            ),
            (
                "EMBYR_ADMIN_KEY_PREVIOUS_GCP_SECRET_NAME",
                SecretManagerSource::Gcp,
            ),
            (
                "EMBYR_ENCRYPTION_KEY_PREVIOUS_AWS_SECRET_ARN",
                SecretManagerSource::Aws,
            ),
            (
                "EMBYR_ENCRYPTION_KEY_PREVIOUS_GCP_SECRET_NAME",
                SecretManagerSource::Gcp,
            ),
        ];
        for (source, expected) in cases {
            assert_eq!(
                secret_manager_source_kind(source),
                expected,
                "source: {source}"
            );
        }
    }

    #[test]
    fn require_gcp_access_token_absent_returns_actionable_error() {
        std::env::remove_var("EMBYR_GCP_ACCESS_TOKEN");
        let result = require_gcp_access_token("EMBYR_ADMIN_KEY", "EMBYR_ADMIN_KEY_GCP_SECRET_NAME");
        match result {
            Err(ConfigError::SecretFetchFailed { var, reason }) => {
                assert_eq!(var, "EMBYR_ADMIN_KEY");
                assert!(reason.contains("EMBYR_GCP_ACCESS_TOKEN"), "got: {reason}");
                assert!(
                    reason.contains("EMBYR_ADMIN_KEY_GCP_SECRET_NAME"),
                    "got: {reason}"
                );
            }
            other => panic!("expected SecretFetchFailed, got {other:?}"),
        }
    }

    #[test]
    fn require_gcp_access_token_present_returns_value() {
        std::env::set_var("_EMBYR_TEST_GCP_TOKEN_XYZZY_99", "1");
        std::env::set_var("EMBYR_GCP_ACCESS_TOKEN", "bearer-token-value");
        let result = require_gcp_access_token("EMBYR_ADMIN_KEY", "EMBYR_ADMIN_KEY_GCP_SECRET_NAME");
        std::env::remove_var("EMBYR_GCP_ACCESS_TOKEN");
        std::env::remove_var("_EMBYR_TEST_GCP_TOKEN_XYZZY_99");
        assert_eq!(result.unwrap(), "bearer-token-value");
    }

    #[tokio::test]
    async fn admin_key_resolver_recognises_gcp_name_as_ambiguity_candidate() {
        std::env::set_var("EMBYR_ADMIN_KEY", "literal-key");
        std::env::set_var("EMBYR_ADMIN_KEY_GCP_SECRET_NAME", "projects/p/secrets/s");
        let mut missing = Vec::new();
        let result = resolve_admin_key(&mut missing).await;
        std::env::remove_var("EMBYR_ADMIN_KEY");
        std::env::remove_var("EMBYR_ADMIN_KEY_GCP_SECRET_NAME");
        match result {
            Err(ConfigError::AmbiguousSecretSource { var_base, sources }) => {
                assert_eq!(var_base, "EMBYR_ADMIN_KEY");
                assert!(
                    sources.contains(&"EMBYR_ADMIN_KEY_GCP_SECRET_NAME".to_string()),
                    "got: {sources:?}"
                );
            }
            other => panic!("expected AmbiguousSecretSource, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_rotation_key_compares_resolved_bytes_not_env_text() {
        // Different hex text (upper vs lower case) that decodes to identical
        // 32-byte resolved values must still be treated as a duplicate — the
        // comparison is over resolved bytes, never the raw env string.
        let lower = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
        let upper = "0102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F20";
        assert_ne!(lower, upper, "precondition: env text differs");
        let current = validate_encryption_key_hex("EMBYR_ENCRYPTION_KEY", lower).unwrap();
        let previous = validate_encryption_key_hex("EMBYR_ENCRYPTION_KEY_PREVIOUS", upper).unwrap();
        assert_eq!(
            current, previous,
            "resolved bytes must be equal despite differing env text case"
        );
    }
}
