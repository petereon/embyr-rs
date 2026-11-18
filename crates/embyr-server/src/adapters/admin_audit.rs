//! Writer for `admin_audit_events` (ADR-086 decision 9). Labels only: an event or reason that is not
//! a short `[a-z0-9_.:-]` label is refused rather than stored, so tokens, claims and e-mail
//! addresses cannot reach the table. Takes any executor, so a caller can write inside its transaction.

use embyr_core::admin::audit::AuditEvent;
use sqlx::PgExecutor;

#[derive(Debug)]
pub enum AuditError {
    /// `event` or `reason` is not an accepted label; nothing was written.
    InvalidLabel,
    Db(sqlx::Error),
}

impl std::fmt::Display for AuditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLabel => write!(f, "audit event or reason is not a label"),
            Self::Db(e) => write!(f, "audit insert failed: {e}"),
        }
    }
}

pub async fn write_audit_event<'e>(
    executor: impl PgExecutor<'e>,
    ev: &AuditEvent<'_>,
) -> Result<(), AuditError> {
    if !ev.is_valid() {
        return Err(AuditError::InvalidLabel);
    }
    // `$7::text::inet`: sqlx has no inet binding without an extra feature; text is enough.
    sqlx::query(
        "INSERT INTO admin_audit_events \
         (event, outcome, account_id, actor_user_id, provider_id, reason, source_ip) \
         VALUES ($1, $2, $3, $4, $5, $6, $7::text::inet)",
    )
    .bind(ev.event)
    .bind(ev.outcome.as_str())
    .bind(ev.account_id)
    .bind(ev.actor_user_id)
    .bind(ev.provider_id)
    .bind(ev.reason)
    .bind(ev.source_ip.map(|ip| ip.to_string()))
    .execute(executor)
    .await
    .map(|_| ())
    .map_err(AuditError::Db)
}
