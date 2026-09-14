//! client-auth-hosted-identity (ADR-036 Decision 7): Customer DB adapter
//! resolution for hosted-identity's own data-plane REST handlers (signup,
//! signin, reset-request, reset-confirm — this slice ships signup only).
//!
//! Composes EXCLUSIVELY pre-existing, already-independently-callable
//! primitives — `SystemDb::get_project_for_auth`, `argon2::verify_api_key`,
//! `ecies::decrypt`, `PostgresBackendAdapter::new`,
//! `AwsSecretFetcher::get_dsn`, `GcpSecretFetcher::get_dsn`,
//! `CredentialCache::get`/`insert` — in the identical order and
//! Argon2id-fast-path discipline `grpc/handler.rs::authenticate` already
//! established. Zero lines of `authenticate()` change.
//!
//! Cache shape note: `CredentialCache` stores `SharedBackendAdapter`
//! (`Arc<dyn BackendAdapter>`), but this function's callers need the
//! CONCRETE `PostgresBackendAdapter` type to run raw SQL against
//! `hosted_identity_accounts` (a table outside the `BackendAdapter` trait's
//! document-CRUD surface) via its own `pool()` accessor. Reconciled by using
//! the cache's existing `dsn` field (already present in `CachedEntry` for
//! the Listen-handler's own reuse) as the fast-path payload: a cache hit
//! skips Argon2id + the System DB round trip + ECIES decryption entirely,
//! then builds a fresh concrete adapter from the cached DSN. A cache MISS
//! populates the SAME shared cache other callers (`authenticate()`) already
//! read from — a second consumer of one already-established cache, not a
//! new caching mechanism.
//!
//! Structurally cannot resolve `backend_mode=agent`: the `agent` branch
//! (folded into the fallback arm below) returns
//! `Err(ProjectAuthError::HostedIdentityUnavailable)` before constructing
//! any adapter at all — `Arc<PostgresBackendAdapter>` is this function's
//! only `Ok` shape, a type-level guarantee, not a runtime convention.

use std::sync::Arc;

use embyr_core::{
    auth::{argon2, blake3, ecies},
    domain::project::{CredentialCacheKey, ProjectId},
};

use crate::adapters::{
    aws_secret_fetcher::AwsSecretFetcher,
    credential_cache::{CachedEntry, CredentialCache, SharedBackendAdapter},
    gcp_secret_fetcher::GcpSecretFetcher,
    postgres_backend::PostgresBackendAdapter,
    system_db::SystemDb,
};

/// Rejection taxonomy for `resolve_customer_db_adapter`. Adapters own
/// presentation (HTTP `reason` enum body) — this type crosses the
/// adapter -> REST-handler boundary as data, mirroring
/// `ClientIdentityVerifyError`'s own discipline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectAuthError {
    /// Project does not exist, or is suspended/deleted.
    ProjectNotFound,
    /// `api_key` failed Argon2id verification against both the current and
    /// (if present) previous hash.
    InvalidApiKey,
    /// `backend_mode == "agent"` (or any mode this function does not know
    /// how to resolve to a `PostgresBackendAdapter`) — hosted identity is
    /// structurally gated to `{direct_pg, aws_secret, gcp_secret}`.
    HostedIdentityUnavailable,
    /// Infrastructure failure (DB unreachable, secret-fetch failure, etc).
    Internal(String),
}

#[allow(clippy::too_many_arguments)]
pub async fn resolve_customer_db_adapter(
    system_db: &SystemDb,
    credential_cache: &CredentialCache,
    aws_secret_fetcher: Option<&AwsSecretFetcher>,
    gcp_secret_fetcher: Option<&GcpSecretFetcher>,
    project_id: &str,
    api_key: &str,
    tenant_db_max_connections: u32,
    tenant_db_acquire_timeout: std::time::Duration,
) -> Result<Arc<PostgresBackendAdapter>, ProjectAuthError> {
    let api_key_blake3 = blake3::derive_cache_key(api_key.as_bytes());
    let domain_project_id =
        ProjectId::new(project_id).map_err(|_| ProjectAuthError::ProjectNotFound)?;
    let cache_key = CredentialCacheKey {
        project_id: domain_project_id,
        api_key_blake3,
    };

    // Cache hit — skip Argon2id + System DB + ECIES entirely (the identical
    // fast-path discipline `authenticate()` already established), then
    // build a fresh concrete adapter from the cached DSN.
    if let Some((_, status, dsn)) = credential_cache.get(&cache_key).await {
        if status == "suspended" || status == "deleted" {
            return Err(ProjectAuthError::ProjectNotFound);
        }
        let adapter = PostgresBackendAdapter::with_pool_config(
            &dsn,
            tenant_db_max_connections,
            tenant_db_acquire_timeout,
        )
            .await
            .map_err(|e| ProjectAuthError::Internal(e.to_string()))?;
        return Ok(Arc::new(adapter));
    }

    let row = system_db
        .get_project_for_auth(project_id)
        .await
        .map_err(|e| ProjectAuthError::Internal(e.to_string()))?
        .ok_or(ProjectAuthError::ProjectNotFound)?;

    if row.status == "suspended" || row.status == "deleted" {
        return Err(ProjectAuthError::ProjectNotFound);
    }

    let verified = argon2::verify_api_key(api_key.as_bytes(), &row.api_key_hash_current)
        .unwrap_or(false)
        || row
            .api_key_hash_previous
            .as_deref()
            .is_some_and(|prev| argon2::verify_api_key(api_key.as_bytes(), prev).unwrap_or(false));
    if !verified {
        return Err(ProjectAuthError::InvalidApiKey);
    }

    let dsn = match row.backend_mode.as_str() {
        "aws_secret" => {
            let arn = row.backend_secret_arn.ok_or_else(|| {
                ProjectAuthError::Internal("aws_secret project missing backend_secret_arn".into())
            })?;
            let fetcher = aws_secret_fetcher.ok_or_else(|| {
                ProjectAuthError::Internal("aws_secret_fetcher not configured on this server".into())
            })?;
            fetcher
                .get_dsn(&arn)
                .await
                .map_err(|e| ProjectAuthError::Internal(format!("aws secret fetch failed: {e}")))?
        }
        "gcp_secret" => {
            let resource_name = row.backend_secret_gcp.ok_or_else(|| {
                ProjectAuthError::Internal("gcp_secret project missing backend_secret_gcp".into())
            })?;
            let fetcher = gcp_secret_fetcher.ok_or_else(|| {
                ProjectAuthError::Internal("gcp_secret_fetcher not configured on this server".into())
            })?;
            fetcher
                .get_dsn(&resource_name)
                .await
                .map_err(|e| ProjectAuthError::Internal(format!("gcp secret fetch failed: {e}")))?
        }
        "direct_pg" => {
            let encrypted_dsn = row.ecies_encrypted_dsn.ok_or_else(|| {
                ProjectAuthError::Internal("project has no stored DSN".into())
            })?;
            let dsn_bytes = ecies::decrypt(api_key.as_bytes(), &encrypted_dsn)
                .map_err(|e| ProjectAuthError::Internal(e.to_string()))?;
            String::from_utf8(dsn_bytes)
                .map_err(|_| ProjectAuthError::Internal("DSN is not valid UTF-8".into()))?
        }
        _ => {
            // "agent" (or any future unrecognized mode): structurally
            // refused — never attempts to build ANY adapter (Decision 7).
            return Err(ProjectAuthError::HostedIdentityUnavailable);
        }
    };

    let concrete = PostgresBackendAdapter::with_pool_config(
        &dsn,
        tenant_db_max_connections,
        tenant_db_acquire_timeout,
    )
        .await
        .map_err(|e| ProjectAuthError::Internal(e.to_string()))?;
    let concrete = Arc::new(concrete);
    let shared: SharedBackendAdapter = concrete.clone();
    credential_cache
        .insert(
            cache_key,
            CachedEntry {
                adapter: shared,
                project_status: row.status.clone(),
                dsn,
            },
        )
        .await;

    Ok(concrete)
}
