//! Project handlers for session-auth routes.
//!
//! list_projects (GET /admin/v1/projects):
//!   Session auth, any role. WHERE account_id = $session_account AND status != 'deleted'.
//!
//! patch_project (PATCH /admin/v1/projects/:id):
//!   Session auth, Owner or Admin.
//!   Partial update: name, backend_pg_dsn, logging_enabled, log_retention_days, status.
//!   backend_pg_dsn is no longer patchable (422): use PUT /projects/:id/connection (ADR-083).
//!   status 'deleted' not patchable (422).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use embyr_core::admin::account::Role;

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::handlers::get_project::GetProjectDetail;
use crate::admin::handlers::shared::account_on_billing_hold;
use crate::admin::state::UserAdminState;

/// Minimal project summary returned by GET /admin/v1/projects.
#[derive(Serialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: Option<String>,
    pub status: String,
    pub backend_mode: String,
    pub logging_enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub account_id: String,
}

/// GET /admin/v1/projects — account-scoped project list.
///
/// Returns all non-deleted projects owned by the authenticated account.
/// Session auth required (any role).
pub async fn list_projects(
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<Vec<ProjectSummary>>, StatusCode> {
    let pool = state.system_db.pool();

    let rows = sqlx::query(
        "SELECT id, name, status, backend_mode, logging_enabled, created_at, account_id \
         FROM projects \
         WHERE account_id = $1 AND status != 'deleted' \
         ORDER BY created_at DESC",
    )
    .bind(session.account_id)
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("list_projects DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let projects: Vec<ProjectSummary> = rows
        .into_iter()
        .map(|row| ProjectSummary {
            id: row.try_get("id").unwrap_or_default(),
            name: row.try_get("name").unwrap_or(None),
            status: row.try_get("status").unwrap_or_default(),
            backend_mode: row.try_get("backend_mode").unwrap_or_default(),
            logging_enabled: row.try_get("logging_enabled").unwrap_or(false),
            created_at: row
                .try_get("created_at")
                .unwrap_or_else(|_| chrono::Utc::now()),
            account_id: row
                .try_get::<uuid::Uuid, _>("account_id")
                .map(|u| u.to_string())
                .unwrap_or_default(),
        })
        .collect();

    Ok(Json(projects))
}

/// Statuses an Owner/Admin may set via PATCH. `deleted` is operator-only.
const PATCHABLE_STATUSES: &[&str] = &["active", "suspended"];

const MAX_NAME_LEN: usize = 100;
const MAX_LOG_RETENTION_DAYS: i32 = 3650;

/// Audit #66: bound the free-form PATCH fields (name 1..=100 chars, retention 1..=3650 days).
fn patch_fields_valid(body: &PatchProjectBody) -> bool {
    let name_ok = body
        .name
        .as_deref()
        .map_or(true, |n| !n.trim().is_empty() && n.chars().count() <= MAX_NAME_LEN);
    let retention_ok = body
        .log_retention_days
        .map_or(true, |d| (1..=MAX_LOG_RETENTION_DAYS).contains(&d));
    name_ok && retention_ok
}

/// Partial-update body for PATCH /admin/v1/projects/:id.
/// All fields are optional; absent fields are preserved via COALESCE.
#[derive(Deserialize, Default)]
pub struct PatchProjectBody {
    pub name: Option<String>,
    pub status: Option<String>,
    pub logging_enabled: Option<bool>,
    pub log_retention_days: Option<i32>,
    pub backend_pg_dsn: Option<String>,
}

/// PATCH /admin/v1/projects/:project_id — session-auth partial update.
///
/// Rules enforced:
/// - Viewer → 403 (Owner or Admin only).
/// - `status` must be `active` or `suspended`; anything else (incl. `deleted`,
///   which is operator-only) → 422.
/// - `status = "active"` while the account is on a billing hold
///   (`past_due` / `free_cap_exceeded`) → 409.
/// - Account isolation: project must belong to `session.account_id`; otherwise 404.
/// - `name` must be 1..=100 chars, `log_retention_days` 1..=3650; otherwise 422 (#66).
/// - `backend_pg_dsn` → 422 (ADR-083: use PUT /projects/:id/connection).
/// - Credential cache is evicted after a status change.
/// - Absent patch fields are preserved via COALESCE.
pub async fn patch_project(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<PatchProjectBody>,
) -> Response {
    // ADR-083: one write path for connection secrets. Role first so a Viewer still gets 403.
    if session.role >= Role::Admin && body.backend_pg_dsn.is_some() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({
                "error": "backend_pg_dsn_not_patchable",
                "use": "PUT /admin/v1/projects/:project_id/connection",
            })),
        )
            .into_response();
    }
    match patch_project_inner(project_id, state, session, body).await {
        Ok(detail) => detail.into_response(),
        Err(code) => code.into_response(),
    }
}

async fn patch_project_inner(
    project_id: String,
    state: UserAdminState,
    session: SessionContext,
    body: PatchProjectBody,
) -> Result<Json<GetProjectDetail>, StatusCode> {
    // ── Guard: Owner or Admin only (audit finding #59) ────────────────────────
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    if !patch_fields_valid(&body) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }

    // ── Guard: status allow-list, body-only, before any DB access (#59) ───────
    // `deleted` (soft-delete is operator-only) and any unknown string → 422.
    if let Some(status) = body.status.as_deref() {
        if !PATCHABLE_STATUSES.contains(&status) {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
    }

    let pool = state.system_db.pool();

    // ── Pre-check: account isolation ──────────────────────────────────────────
    // Foreign-account projects return 404, same as missing (no existence oracle).
    let pre = sqlx::query(
        "SELECT account_id, status, suspended_by FROM projects WHERE id = $1",
    )
    .bind(&project_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!("patch_project pre-check DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .ok_or(StatusCode::NOT_FOUND)?;

    let existing_status: String = pre
        .try_get("status")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if existing_status == "deleted" {
        return Err(StatusCode::NOT_FOUND);
    }

    let project_account: Option<Uuid> = pre.try_get("account_id").unwrap_or(None);
    if project_account.as_ref() != Some(&session.account_id) {
        return Err(StatusCode::NOT_FOUND);
    }

    // ── Guard: an operator/system suspension cannot be undone via PATCH (#64) ─
    if body.status.as_deref() == Some("active") && existing_status == "suspended" {
        let suspended_by: Option<String> = pre.try_get("suspended_by").unwrap_or(None);
        if suspended_by.as_deref() != Some("account") {
            return Err(StatusCode::CONFLICT);
        }
    }

    // ── Guard: a billing suspension cannot be reversed via PATCH (#59) ────────
    // Billing suspensions (Stripe final payment failure → `past_due`, free-plan
    // cap → `free_cap_exceeded`) are recorded on the account's subscription,
    // and are lifted only by the billing paths (webhook payment_succeeded,
    // plan upgrade). While the account is in either state, refuse reactivation.
    if body.status.as_deref() == Some("active") && existing_status == "suspended" {
        let billing_hold = account_on_billing_hold(pool, session.account_id).await?;
        if billing_hold {
            return Err(StatusCode::CONFLICT);
        }
    }

    // ── UPDATE with COALESCE — only touches columns present in the body ───────
    let row = sqlx::query(
        "UPDATE projects \
         SET \
           name                = COALESCE($2, name), \
           status              = COALESCE($3, status), \
           suspended_by        = CASE WHEN $3::text IS NULL THEN suspended_by \
                                      WHEN $3::text = 'active' THEN NULL \
                                      WHEN status = 'active' THEN 'account' \
                                      ELSE suspended_by END, \
           logging_enabled     = COALESCE($4, logging_enabled), \
           log_retention_days  = COALESCE($5, log_retention_days), \
           updated_at          = now() \
         WHERE id = $1 \
         RETURNING id, name, status, backend_mode, logging_enabled, \
                   log_retention_days, created_at, api_key_rotated_at, \
                   api_key_previous_expires_at",
    )
    .bind(&project_id)
    .bind(&body.name)
    .bind(&body.status)
    .bind(body.logging_enabled)
    .bind(body.log_retention_days)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        tracing::error!("patch_project UPDATE error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // ── Evict credential cache: it holds both the DSN and the project status,
    // so a stale entry would keep serving a just-suspended project (#59) ─────
    if body.status.is_some() {
        state.credential_cache.evict_project(&project_id).await;
    }

    // ── Return updated project (same shape as GET /admin/v1/projects/:id) ─────
    Ok(Json(GetProjectDetail {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or(None),
        status: row.try_get("status").unwrap_or_default(),
        backend_mode: row.try_get("backend_mode").unwrap_or_default(),
        auth_mode: "key".to_string(),
        logging_enabled: row.try_get("logging_enabled").unwrap_or(false),
        log_retention_days: row.try_get("log_retention_days").unwrap_or(None),
        created_at: row
            .try_get("created_at")
            .unwrap_or_else(|_| chrono::Utc::now()),
        api_key_rotated_at: row.try_get("api_key_rotated_at").unwrap_or(None),
        api_key_previous_expires_at: row.try_get("api_key_previous_expires_at").unwrap_or(None),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(name: Option<&str>, days: Option<i32>) -> PatchProjectBody {
        PatchProjectBody {
            name: name.map(String::from),
            log_retention_days: days,
            ..Default::default()
        }
    }

    #[test]
    fn patch_field_bounds() {
        assert!(patch_fields_valid(&body(None, None)));
        assert!(patch_fields_valid(&body(Some("ok"), Some(1))));
        assert!(patch_fields_valid(&body(Some(&"x".repeat(100)), Some(3650))));
        assert!(!patch_fields_valid(&body(Some(""), None)));
        assert!(!patch_fields_valid(&body(Some("   "), None)));
        assert!(!patch_fields_valid(&body(Some(&"x".repeat(101)), None)));
        assert!(!patch_fields_valid(&body(None, Some(0))));
        assert!(!patch_fields_valid(&body(None, Some(-5))));
        assert!(!patch_fields_valid(&body(None, Some(3651))));
    }
}
