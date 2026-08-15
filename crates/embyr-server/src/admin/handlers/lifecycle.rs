use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
};

use crate::adapters::credential_cache::CredentialCache;
use crate::adapters::system_db::SystemDb;
use crate::admin::state::OperatorState;

/// Dependencies for lifecycle-status transitions invoked outside the HTTP
/// router — used by `CapUsageRefresher` (ADR-020) to call
/// `set_project_status`-shaped suspension logic from the background cap-check
/// task, which has no `OperatorState` (no HTTP request in flight).
pub struct LifecycleDeps {
    pub system_db: Arc<SystemDb>,
    pub credential_cache: Arc<CredentialCache>,
}

/// Apply a lifecycle status transition to a project and evict the credential cache.
///
/// Updates `status` and `updated_at` where the current status is in
/// `('active', 'suspended')`. Returns `NOT_FOUND` when the project does not
/// exist or has already been deleted; `INTERNAL_SERVER_ERROR` on DB failure.
///
/// Takes `system_db`/`credential_cache` directly (rather than a whole state
/// struct) so both `OperatorState`-backed HTTP handlers (below) and
/// `LifecycleDeps`-backed callers with no HTTP request in flight
/// (`activate_account_projects`, the background `CapUsageRefresher`) can
/// share this exact transition — D-12 literal reuse.
async fn set_project_status(
    project_id: &str,
    new_status: &str,
    system_db: &SystemDb,
    credential_cache: &CredentialCache,
) -> StatusCode {
    let result = sqlx::query(
        "UPDATE projects SET status = $2, updated_at = now() \
         WHERE id = $1 AND status IN ('active', 'suspended')",
    )
    .bind(project_id)
    .bind(new_status)
    .execute(system_db.pool())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            credential_cache.evict_project(project_id).await;
            StatusCode::OK
        }
        Ok(_) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub async fn suspend_project(
    Path(project_id): Path<String>,
    State(state): State<OperatorState>,
) -> StatusCode {
    // Auth is enforced by operator_auth_middleware applied at the router layer.
    set_project_status(
        &project_id,
        "suspended",
        &state.system_db,
        &state.credential_cache,
    )
    .await
}

pub async fn activate_project(
    Path(project_id): Path<String>,
    State(state): State<OperatorState>,
) -> StatusCode {
    // Auth is enforced by operator_auth_middleware applied at the router layer.
    set_project_status(
        &project_id,
        "active",
        &state.system_db,
        &state.credential_cache,
    )
    .await
}

/// Reactivate every currently-suspended project under `account_id` — the
/// SAME `set_project_status` transition the operator-initiated
/// `activate_project` handler above uses (D-12 literal reuse). Used by the
/// plan-change upgrade path when a subscription was `free_cap_exceeded`
/// (AC-202-04, `billing_subscription::post_subscription`, step 01-02 — THIS
/// is the first implementation) and — unchanged — by a future step's
/// `invoice.payment_succeeded` webhook arm (US-204). Returns the count of
/// projects transitioned.
pub async fn activate_account_projects(
    account_id: uuid::Uuid,
    deps: &LifecycleDeps,
) -> Result<u64, StatusCode> {
    let project_ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM projects WHERE account_id = $1 AND status = 'suspended'",
    )
    .bind(account_id)
    .fetch_all(deps.system_db.pool())
    .await
    .map_err(|e| {
        tracing::error!("activate_account_projects: failed to list suspended projects: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mut activated_count = 0u64;
    for project_id in &project_ids {
        let status = set_project_status(
            project_id,
            "active",
            &deps.system_db,
            &deps.credential_cache,
        )
        .await;
        if status == StatusCode::OK {
            activated_count += 1;
        }
    }

    Ok(activated_count)
}

/// Suspend every currently-active project under `account_id` — the SAME
/// `set_project_status` transition the operator-initiated `suspend_project`
/// handler above uses (D-12 literal reuse). Used by the dunning
/// final-failure webhook arm (`webhooks_stripe::stripe_webhook_handler`,
/// AC-204-01, THIS is the first implementation) — the credential-cache
/// eviction claim (AC-204-05) holds "for free" because `set_project_status`
/// already evicts on every call, unchanged — and, unchanged, by a future
/// step's cap-exceeded enforcement (US-207). Returns the count of projects
/// transitioned.
pub async fn suspend_account_projects(
    account_id: uuid::Uuid,
    deps: &LifecycleDeps,
) -> Result<u64, StatusCode> {
    let project_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM projects WHERE account_id = $1 AND status = 'active'")
            .bind(account_id)
            .fetch_all(deps.system_db.pool())
            .await
            .map_err(|e| {
                tracing::error!("suspend_account_projects: failed to list active projects: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

    let mut suspended_count = 0u64;
    for project_id in &project_ids {
        let status = set_project_status(
            project_id,
            "suspended",
            &deps.system_db,
            &deps.credential_cache,
        )
        .await;
        if status == StatusCode::OK {
            suspended_count += 1;
        }
    }

    Ok(suspended_count)
}

pub async fn delete_project(
    Path(project_id): Path<String>,
    State(state): State<OperatorState>,
) -> StatusCode {
    // Auth is enforced by operator_auth_middleware applied at the router layer.

    // AC-B03-06: cascade-revoke all active SDK keys before soft-deleting the project.
    let _ = sqlx::query(
        "UPDATE sdk_api_keys SET revoked_at = now() \
         WHERE project_id = $1 AND revoked_at IS NULL",
    )
    .bind(&project_id)
    .execute(state.system_db.pool())
    .await;

    let result = sqlx::query(
        "UPDATE projects SET status = 'deleted', deleted_at = now(), updated_at = now() \
         WHERE id = $1 AND status != 'deleted'",
    )
    .bind(&project_id)
    .execute(state.system_db.pool())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            state.credential_cache.evict_project(&project_id).await;
            StatusCode::OK
        }
        Ok(_) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
