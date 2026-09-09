//! Shared DSN-resolution for reaching a customer Postgres database WITHOUT
//! ever holding a live api_key (ADR-072 Decision C).
//!
//! Extracted, behavior-preserving, from `sweepers/transaction_sweeper.rs`'s
//! own `resolve_dsn_without_api_key` + its 3 per-`backend_mode` helpers —
//! that module was the ONLY existing precedent in this codebase for
//! embyr-server reaching a customer DB without a live api_key
//! (`TransactionSweeper`'s own background-task context). This feature
//! (`composite-index-real-creation`) is the second consumer, which is what
//! justifies extracting it now — same log messages, same `None`-on-any
//! -failure contract, byte-identical branch behavior, zero change at the
//! sweeper's own call site beyond building a `PgConnectInfo` from its
//! existing `SweeperProjectRow`.

use crate::adapters::aws_secret_fetcher::AwsSecretFetcher;
use crate::adapters::encryption::decrypt_with_rotation;
use crate::adapters::gcp_secret_fetcher::GcpSecretFetcher;

/// The minimal per-`backend_mode` connection info needed to resolve a
/// customer DSN without an api_key — mirrors `SweeperProjectRow`'s own
/// shape minus `id` (kept as a separate parameter below, only for log
/// context, since this struct is no longer sweeper-specific).
#[derive(Debug, Clone)]
pub struct PgConnectInfo {
    pub backend_mode: String,
    pub backend_secret_arn: Option<String>,
    pub backend_secret_gcp: Option<String>,
    pub backend_pg_dsn_enc: Option<Vec<u8>>,
}

/// Resolve `project_id`'s customer-DB DSN without ever holding a live
/// api_key. `None` on any failure (missing arn/resource-name, fetcher not
/// configured, fetch error, `backend_pg_dsn_enc IS NULL`, decrypt/AEAD
/// failure, malformed UTF-8, or an unreachable `backend_mode` such as
/// `agent`) — every `None` path has already logged a `tracing::warn!`
/// naming the reason, so the caller can skip/fail silently.
pub async fn resolve_dsn_without_api_key(
    project_id: &str,
    info: &PgConnectInfo,
    aws_secret_fetcher: Option<&AwsSecretFetcher>,
    gcp_secret_fetcher: Option<&GcpSecretFetcher>,
    encryption_key: &[u8; 32],
    encryption_key_previous: Option<&[u8; 32]>,
) -> Option<String> {
    match info.backend_mode.as_str() {
        "aws_secret" => resolve_aws_secret_dsn(project_id, info, aws_secret_fetcher).await,
        "gcp_secret" => resolve_gcp_secret_dsn(project_id, info, gcp_secret_fetcher).await,
        "direct_pg" => resolve_direct_pg_dsn(project_id, info, encryption_key, encryption_key_previous),
        other => {
            tracing::warn!(
                project_id = %project_id,
                backend_mode = %other,
                "customer_db_connect: unreachable backend_mode"
            );
            None
        }
    }
}

async fn resolve_aws_secret_dsn(
    project_id: &str,
    info: &PgConnectInfo,
    aws_secret_fetcher: Option<&AwsSecretFetcher>,
) -> Option<String> {
    let Some(arn) = info.backend_secret_arn.as_deref() else {
        tracing::warn!(project_id = %project_id, "customer_db_connect: aws_secret project missing backend_secret_arn, skipping");
        return None;
    };
    let Some(fetcher) = aws_secret_fetcher else {
        tracing::warn!(project_id = %project_id, "customer_db_connect: aws_secret_fetcher not configured, skipping");
        return None;
    };
    match fetcher.get_dsn(arn).await {
        Ok(dsn) => Some(dsn),
        Err(e) => {
            tracing::warn!(project_id = %project_id, error = %e, "customer_db_connect: aws secret fetch failed, skipping");
            None
        }
    }
}

async fn resolve_gcp_secret_dsn(
    project_id: &str,
    info: &PgConnectInfo,
    gcp_secret_fetcher: Option<&GcpSecretFetcher>,
) -> Option<String> {
    let Some(resource_name) = info.backend_secret_gcp.as_deref() else {
        tracing::warn!(project_id = %project_id, "customer_db_connect: gcp_secret project missing backend_secret_gcp, skipping");
        return None;
    };
    let Some(fetcher) = gcp_secret_fetcher else {
        tracing::warn!(project_id = %project_id, "customer_db_connect: gcp_secret_fetcher not configured, skipping");
        return None;
    };
    match fetcher.get_dsn(resource_name).await {
        Ok(dsn) => Some(dsn),
        Err(e) => {
            tracing::warn!(project_id = %project_id, error = %e, "customer_db_connect: gcp secret fetch failed, skipping");
            None
        }
    }
}

fn resolve_direct_pg_dsn(
    project_id: &str,
    info: &PgConnectInfo,
    encryption_key: &[u8; 32],
    encryption_key_previous: Option<&[u8; 32]>,
) -> Option<String> {
    let Some(enc) = info.backend_pg_dsn_enc.as_deref() else {
        // ADR-055: accepted, documented coverage gap — silent skip, not an error.
        tracing::warn!(project_id = %project_id, "customer_db_connect: direct_pg project has backend_pg_dsn_enc IS NULL, skipping (ADR-055)");
        return None;
    };
    let plaintext = match decrypt_with_rotation(encryption_key, encryption_key_previous, enc) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(project_id = %project_id, error = %e, "customer_db_connect: backend_pg_dsn_enc decrypt failed, skipping");
            return None;
        }
    };
    match String::from_utf8(plaintext) {
        Ok(dsn) => Some(dsn),
        Err(_) => {
            tracing::warn!(project_id = %project_id, "customer_db_connect: decrypted DSN is not valid UTF-8, skipping");
            None
        }
    }
}
