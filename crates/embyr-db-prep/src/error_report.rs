//! Error classification for embyr-db-prep.
//!
//! Classifies a `migrate()`-phase `CoreError` into one of 3 named message
//! shapes UAT requires (AC-01-01, AC-01-04, AC-01-05):
//!   1. Insufficient privilege (`CoreError::PermissionDenied` or a
//!      `BackendUnavailable` whose wrapped sqlx/Postgres error text carries
//!      the `insufficient_privilege` (SQLSTATE 42501) condition).
//!   2. Generic backend-unavailable fallback (any other migration failure).
//!
//! Connect-phase failures (unreachable host, auth failure before any query
//! reaches Postgres) never reach this function -- `main.rs` short-circuits
//! those to a distinct hardcoded message before `migrate()` is even
//! attempted (AC-01-05). Only a `migrate()`-phase failure (pool connected,
//! `probe()` succeeded, but `migrate()` itself errored) is classified here.
//!
//! Role and database names are named from the DSN's own components, never
//! from server error-message content (`docs/product/architecture/brief.md`
//! § Driven Ports + Adapters — customer-db-onboarding, Earned Trust probe
//! design table convention of not trusting substrate-supplied text for
//! identity).

use embyr_core::error::CoreError;

/// Classify a `migrate()`-phase error into a presentable message.
///
/// `dsn` is the same connection string `main.rs` used to connect -- its own
/// role/database components (never server error text) supply the identity
/// named in the insufficient-privilege message.
pub fn classify(err: &CoreError, dsn: &str) -> String {
    match err {
        CoreError::PermissionDenied(_) => insufficient_privilege_message(dsn),
        CoreError::BackendUnavailable(msg) if is_insufficient_privilege(msg) => {
            insufficient_privilege_message(dsn)
        }
        other => format!("database preparation failed: {other}"),
    }
}

/// Postgres reports `insufficient_privilege` (SQLSTATE 42501) failures with
/// a "permission denied ..." message text -- the only signal available once
/// the error has been flattened to a `String` by the adapter layer.
fn is_insufficient_privilege(msg: &str) -> bool {
    msg.to_lowercase().contains("permission denied")
}

fn insufficient_privilege_message(dsn: &str) -> String {
    let (role, database) = role_and_database_from_dsn(dsn);
    format!("insufficient privilege: role {role} lacks CREATE on database {database}")
}

/// Parse `role` and `database` out of a `postgres://role:pass@host:port/db`
/// connection string. Never inspects server-supplied error text -- identity
/// comes only from the DSN the caller itself constructed the connection
/// with.
fn role_and_database_from_dsn(dsn: &str) -> (String, String) {
    let after_scheme = dsn.split_once("://").map_or(dsn, |(_, rest)| rest);
    let (userinfo, host_and_path) = after_scheme
        .split_once('@')
        .unwrap_or(("", after_scheme));

    let role = userinfo.split(':').next().unwrap_or("").to_string();
    let database = host_and_path
        .split_once('/')
        .map(|(_, path)| path.split(['?', '#']).next().unwrap_or(path))
        .unwrap_or("")
        .to_string();

    (role, database)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_and_database_from_dsn_extracts_both_components() {
        let dsn = "postgres://elena_dba:secret@pg-prod.example.internal:5432/meridian_embyr";
        assert_eq!(
            role_and_database_from_dsn(dsn),
            ("elena_dba".to_string(), "meridian_embyr".to_string())
        );
    }

    #[test]
    fn role_and_database_from_dsn_strips_query_string() {
        let dsn = "postgres://embyr_app:pw@host/meridian_embyr?sslmode=require";
        let (_, database) = role_and_database_from_dsn(dsn);
        assert_eq!(database, "meridian_embyr");
    }

    #[test]
    fn is_insufficient_privilege_true_for_permission_denied() {
        assert!(is_insufficient_privilege(
            "permission denied for table _sqlx_migrations"
        ));
    }

    #[test]
    fn is_insufficient_privilege_false_for_other_errors() {
        assert!(!is_insufficient_privilege("connection refused"));
    }

    #[test]
    fn classify_backend_unavailable_with_permission_text_names_role_and_database() {
        let dsn = "postgres://elena_dba:pw@host/meridian_embyr";
        let err = CoreError::BackendUnavailable("permission denied for schema public".to_string());
        let msg = classify(&err, dsn);
        assert!(msg.contains("elena_dba"), "got: {msg}");
        assert!(msg.contains("meridian_embyr"), "got: {msg}");
    }

    #[test]
    fn classify_generic_backend_unavailable_is_the_fallback_message() {
        let dsn = "postgres://elena_dba:pw@host/meridian_embyr";
        let err = CoreError::BackendUnavailable("connection reset by peer".to_string());
        let msg = classify(&err, dsn);
        assert!(msg.contains("database preparation failed"), "got: {msg}");
        assert!(!msg.contains("insufficient privilege"), "got: {msg}");
    }
}
