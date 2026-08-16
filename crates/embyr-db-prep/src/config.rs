//! Configuration for embyr-db-prep.
//!
//! Mirrors `embyr-agent`'s `AgentConfig::from_env()` error-accumulation
//! pattern (`crates/embyr-agent/src/config.rs`) — reuse noted in
//! `docs/product/architecture/brief.md` § Reuse Analysis —
//! customer-db-onboarding.

/// Full configuration for embyr-db-prep, loaded from environment variables.
#[derive(Debug, Clone)]
pub struct DbPrepConfig {
    /// `EMBYR_DB_PREP_DSN` (required) — the elevated connection string used
    /// to apply `migrations/customer/` and (if the grant step runs) execute
    /// the `GRANT` statement. NEVER logged.
    pub dsn: String,
    /// `EMBYR_DB_PREP_DML_ROLE_DSN` (optional, ADR-023 revised) — the
    /// DML-only role's own connection string, used only to self-discover
    /// its role name via `SELECT current_user`. NEVER logged, never reused
    /// beyond that one query.
    pub dml_role_dsn: Option<String>,
}

impl DbPrepConfig {
    /// Load configuration from `EMBYR_DB_PREP_DSN` (required) and
    /// `EMBYR_DB_PREP_DML_ROLE_DSN` (optional).
    ///
    /// Returns `Err` naming the missing variable(s) when `EMBYR_DB_PREP_DSN`
    /// is absent or empty — mirrors `AgentConfig::from_env()`'s
    /// error-accumulation pattern (a newline-separated diagnostic list, not
    /// a raw panic or a partial config).
    pub fn from_env() -> Result<Self, String> {
        let mut errors: Vec<String> = Vec::new();

        let dsn = collect_required("EMBYR_DB_PREP_DSN", &mut errors);

        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }

        let dml_role_dsn = std::env::var("EMBYR_DB_PREP_DML_ROLE_DSN")
            .ok()
            .filter(|s| !s.is_empty());

        Ok(DbPrepConfig {
            dsn: dsn.unwrap(),
            dml_role_dsn,
        })
    }
}

/// Read a required variable; on failure push a diagnostic and return `None`.
fn collect_required(name: &str, errors: &mut Vec<String>) -> Option<String> {
    match std::env::var(name) {
        Ok(val) if !val.is_empty() => Some(val),
        _ => {
            errors.push(format!("missing required environment variable: {name}"));
            None
        }
    }
}
