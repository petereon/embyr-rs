//! ADR-085: project API key rotation and revocation (session auth, Owner/Admin).
//!
//! POST /admin/v1/projects/:id/api_key/rotate — new key returned once; the old key keeps working
//!   for `EMBYR_API_KEY_ROTATION_WINDOW_SECS` (the "previous" hash, expiry evaluated by Postgres
//!   at read time, no sweeper).
//! POST /admin/v1/projects/:id/api_key/revoke — same operation with a zero window: old key dead.
//!
//! One `UPDATE` compare-and-swaps on the observed current hash (lost race -> 409
//! `rotation_conflict`). Rows whose DSN / agent TLS bundle are still ECIES-encrypted to the old key
//! are refused (409 `connection_secrets_unconverted`, the dsn-single-store predicate): rotating
//! would strand them. A legacy hosted-identity signing key (seed ECIES-encrypted to the old key)
//! is regenerated in the same transaction; end-user tokens minted before then stop verifying
//! (they live 1 h).
//!
//! Cache: only this process's credential cache is evicted. Another replica may keep serving a
//! cached old key for up to `CREDENTIAL_TTL` (plus the window for rotate). Listen streams opened
//! with the old key are not terminated.

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use sqlx::Row;
use uuid::Uuid;

use embyr_core::admin::account::Role;

use super::connection::{err, ApiResult};
use super::provision::generate_api_key;
use crate::adapters::{argon2_blocking, encryption, system_db::UNCONVERTED_CONNECTION_SECRETS_SQL};
use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;

#[derive(Clone, Copy)]
enum Kind {
    Rotate,
    Revoke,
}

impl Kind {
    /// Revoke is rotate with a zero window.
    fn window_secs(self, configured: u32) -> u32 {
        match self {
            Kind::Rotate => configured,
            Kind::Revoke => 0,
        }
    }

    fn event(self) -> &'static str {
        match self {
            Kind::Rotate => "api_key_rotated",
            Kind::Revoke => "api_key_revoked",
        }
    }
}

/// What the project row looks like at one instant.
struct Observed {
    status: String,
    hash_current: String,
    unconverted: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Miss {
    Gone,
    Conflict,
    Unconverted,
}

/// Why a rotate `UPDATE` touched no row, given a fresh re-read.
fn classify_miss(now: Option<&Observed>, observed_hash: &str) -> Miss {
    match now {
        None => Miss::Gone,
        Some(o) if !matches!(o.status.as_str(), "active" | "suspended") => Miss::Gone,
        Some(o) if o.hash_current != observed_hash => Miss::Conflict,
        Some(o) if o.unconverted => Miss::Unconverted,
        Some(_) => Miss::Conflict,
    }
}

fn internal(context: &str, e: impl std::fmt::Display) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!("api_key {context}: {e}");
    err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
}

/// POST /admin/v1/projects/:project_id/api_key/rotate
pub async fn rotate_api_key(
    path: Path<String>,
    state: State<UserAdminState>,
    session: SessionContext,
) -> ApiResult<Response> {
    rotate_or_revoke(Kind::Rotate, path, state, session).await
}

/// POST /admin/v1/projects/:project_id/api_key/revoke
pub async fn revoke_api_key(
    path: Path<String>,
    state: State<UserAdminState>,
    session: SessionContext,
) -> ApiResult<Response> {
    rotate_or_revoke(Kind::Revoke, path, state, session).await
}

async fn read_state(
    pool: &sqlx::PgPool,
    project_id: &str,
) -> Result<Option<(Option<Uuid>, Observed)>, sqlx::Error> {
    let row = sqlx::query(&format!(
        "SELECT account_id, status, api_key_hash_current, {UNCONVERTED_CONNECTION_SECRETS_SQL} AS unconverted \
         FROM projects WHERE id = $1"
    ))
    .bind(project_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| {
        Ok((
            r.try_get("account_id")?,
            Observed {
                status: r.try_get("status")?,
                hash_current: r.try_get("api_key_hash_current")?,
                unconverted: r.try_get("unconverted")?,
            },
        ))
    })
    .transpose()
}

async fn rotate_or_revoke(
    kind: Kind,
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> ApiResult<Response> {
    if session.role < Role::Admin {
        return Err(err(StatusCode::FORBIDDEN, "forbidden"));
    }
    let pool = state.system_db.pool();
    let (account, observed) = read_state(pool, &project_id)
        .await
        .map_err(|e| internal("load", e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "project_not_found"))?;
    if observed.status == "deleted" {
        return Err(err(StatusCode::NOT_FOUND, "project_not_found"));
    }
    if account != Some(session.account_id) {
        return Err(err(StatusCode::NOT_FOUND, "project_not_found"));
    }
    if observed.unconverted {
        return Err(err(StatusCode::CONFLICT, "connection_secrets_unconverted"));
    }

    let window_secs = kind.window_secs(state.api_key_rotation_window_secs);
    let api_key = generate_api_key();
    let new_hash = argon2_blocking::hash_api_key(api_key.as_bytes())
        .await
        .map_err(|e| internal("hash", e))?;
    // Fresh hosted-identity keypair, used only if the project still has a legacy ECIES row.
    let signing_key = SigningKey::generate(&mut OsRng);
    let seed_enc = encryption::encrypt(&state.encryption_key, &signing_key.to_bytes())
        .map_err(|e| internal("encrypt", e))?;

    let mut tx = pool.begin().await.map_err(|e| internal("begin", e))?;
    let updated = sqlx::query(&format!(
        "UPDATE projects SET \
           api_key_hash_previous = CASE WHEN $3 > 0 THEN api_key_hash_current END, \
           api_key_previous_expires_at = CASE WHEN $3 > 0 THEN now() + make_interval(secs => $3) END, \
           api_key_hash_current = $4, api_key_rotated_at = now(), updated_at = now() \
         WHERE id = $1 AND api_key_hash_current = $2 AND status IN ('active', 'suspended') \
           AND NOT {UNCONVERTED_CONNECTION_SECRETS_SQL} \
         RETURNING api_key_rotated_at, api_key_previous_expires_at"
    ))
    .bind(&project_id)
    .bind(&observed.hash_current)
    .bind(f64::from(window_secs))
    .bind(&new_hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| internal("update", e))?;
    let Some(updated) = updated else {
        tx.rollback().await.map_err(|e| internal("rollback", e))?;
        let now = read_state(pool, &project_id).await.map_err(|e| internal("reload", e))?;
        return Err(match classify_miss(now.as_ref().map(|(_, o)| o), &observed.hash_current) {
            Miss::Gone => err(StatusCode::NOT_FOUND, "project_not_found"),
            Miss::Conflict => err(StatusCode::CONFLICT, "rotation_conflict"),
            Miss::Unconverted => err(StatusCode::CONFLICT, "connection_secrets_unconverted"),
        });
    };
    sqlx::query(
        "UPDATE hosted_identity_signing_keys \
         SET public_key = $2, private_key_srv_enc = $3, private_key_enc = NULL \
         WHERE project_id = $1 AND private_key_srv_enc IS NULL",
    )
    .bind(&project_id)
    .bind(&signing_key.verifying_key().to_bytes()[..])
    .bind(&seed_enc)
    .execute(&mut *tx)
    .await
    .map_err(|e| internal("signing key", e))?;
    tx.commit().await.map_err(|e| internal("commit", e))?;

    let rotated_at: DateTime<Utc> =
        updated.try_get("api_key_rotated_at").map_err(|e| internal("read", e))?;
    let previous_expires_at: Option<DateTime<Utc>> =
        updated.try_get("api_key_previous_expires_at").map_err(|e| internal("read", e))?;

    state.credential_cache.evict_project(&project_id).await;
    // Fixed field set so a log alert can key on `event`; never the key or a hash.
    tracing::info!(
        target: "embyr::audit",
        event = kind.event(),
        project_id = %project_id,
        account_id = %session.account_id,
        session_id = %session.session_id,
        window_secs,
        previous_expires_at = ?previous_expires_at,
    );

    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({
            "api_key": api_key,
            "rotated_at": rotated_at,
            "previous_key_expires_at": previous_expires_at,
        })),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed(status: &str, hash: &str, unconverted: bool) -> Observed {
        Observed { status: status.into(), hash_current: hash.into(), unconverted }
    }

    #[test]
    fn revoke_ignores_the_configured_window_and_rotate_uses_it() {
        assert_eq!(Kind::Rotate.window_secs(86_400), 86_400);
        assert_eq!(Kind::Revoke.window_secs(86_400), 0);
        assert_eq!(Kind::Rotate.event(), "api_key_rotated");
        assert_eq!(Kind::Revoke.event(), "api_key_revoked");
    }

    #[test]
    fn a_missed_update_is_classified_from_the_fresh_row() {
        let cases = [
            (None, Miss::Gone),
            (Some(observed("deleted", "h0", false)), Miss::Gone),
            (Some(observed("active", "h1", false)), Miss::Conflict),
            (Some(observed("suspended", "h1", true)), Miss::Conflict), // someone else rotated first
            (Some(observed("active", "h0", true)), Miss::Unconverted),
            (Some(observed("active", "h0", false)), Miss::Conflict),
        ];
        for (i, (now, want)) in cases.iter().enumerate() {
            assert_eq!(&classify_miss(now.as_ref(), "h0"), want, "case {i}");
        }
    }
}
