//! Shared admin handler helpers, promoted out of per-resource handler
//! modules when the exact same logic is duplicated across ≥2 resource
//! types (client-auth DELIVER step 01-01 — promoted from `sdk_keys.rs`'s
//! and `client_identity.rs`'s previously-separate, identically-shaped
//! private copies. Mechanical dedup, zero behavior change.).

use axum::http::StatusCode;
use sqlx::Row;
use uuid::Uuid;

/// Verify that `project_id` exists and is owned by `account_id`.
///
/// Returns `Ok(())` on success.
/// Returns `Err(NOT_FOUND)` if the project does not exist or has been deleted.
/// A project owned by a different account also yields `Err(NOT_FOUND)` (404), so the
/// response is not an existence oracle for foreign project IDs (X13).
pub(crate) async fn verify_project_ownership(
    pool: &sqlx::PgPool,
    project_id: &str,
    account_id: Uuid,
) -> Result<(), StatusCode> {
    let row = sqlx::query("SELECT account_id FROM projects WHERE id = $1 AND status != 'deleted'")
        .bind(project_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            tracing::error!("verify_project_ownership DB error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let project_account: Uuid = row
        .try_get("account_id")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if project_account != account_id {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(())
}

/// True while the account's subscription is on a billing hold
/// (`past_due` after a final payment failure, or `free_cap_exceeded`).
/// Shared by `patch_project` (reactivation) and project creation.
pub(crate) async fn account_on_billing_hold(
    pool: &sqlx::PgPool,
    account_id: Uuid,
) -> Result<bool, StatusCode> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM subscriptions \
         WHERE account_id = $1 AND status IN ('past_due', 'free_cap_exceeded'))",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        tracing::error!("account_on_billing_hold DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}
