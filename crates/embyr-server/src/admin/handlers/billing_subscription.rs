//! `billing_subscription` handlers — `GET`/`POST /admin/v1/billing/subscription`
//! (US-201, US-202, extended with `cap_status` by US-206).
//!
//! Separate file from the existing `billing.rs` (usage-reporting, read-only,
//! zero external calls) per CPB-AD-08 — this file's handlers make real Stripe
//! network calls, a materially different responsibility/failure-mode profile.

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};

use embyr_core::admin::account::Role;
use embyr_core::admin::billing::SubscriptionPlan;

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::handlers::lifecycle::{self, LifecycleDeps};
use crate::admin::state::UserAdminState;

// ---------------------------------------------------------------------------
// Response / request types (real — plain data carriers, no business logic)
// ---------------------------------------------------------------------------

/// One dimension entry in the `cap_status` array (US-206).
#[derive(Debug, Serialize)]
pub struct CapStatusEntry {
    pub dimension: String,
    pub used: u64,
    pub cap: u64,
    pub pct: u64,
}

/// Response body for `GET /admin/v1/billing/subscription` (AC-201-06).
#[derive(Debug, Serialize)]
pub struct SubscriptionResponse {
    pub plan: String,
    pub status: String,
    pub stripe_customer_id: String,
    pub current_period_end: Option<chrono::DateTime<chrono::Utc>>,
    /// `null` until a future card-capture increment populates it (US-201
    /// elevator pitch; D-3/D-5 card capture is out of this feature's scope).
    pub card: Option<serde_json::Value>,
    /// Present only for Free-plan accounts (AC-206-03); `None` serializes as
    /// a JSON field absence via `skip_serializing_if`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap_status: Option<Vec<CapStatusEntry>>,
}

/// Request body for `POST /admin/v1/billing/subscription` (US-202).
#[derive(Debug, Deserialize)]
pub struct PlanChangeRequest {
    pub plan: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /admin/v1/billing/subscription` — session-authed, any role
/// (AC-201-01). Lazily provisions a real Stripe Customer + seeds a Free
/// `subscriptions` row on first call (AC-201-02/03/04), then returns the
/// current subscription state. Extended by US-206 to include `cap_status`
/// for Free-plan accounts (read from `CapStatusCache`, fail-open on a cache
/// miss — AC-206-04).
///
/// AC-201-05: the Stripe call happens strictly before any DB write begins —
/// a `StripeError` returns 502 with neither `accounts.stripe_customer_id`
/// nor a `subscriptions` row written.
pub async fn get_subscription(
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<SubscriptionResponse>, StatusCode> {
    let pool = state.system_db.pool();

    let existing_customer_id: Option<String> =
        sqlx::query_scalar("SELECT stripe_customer_id FROM accounts WHERE id = $1")
            .bind(session.account_id)
            .fetch_one(pool)
            .await
            .map_err(|e| {
                tracing::error!("get_subscription: failed to read accounts row: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

    let stripe_customer_id = match existing_customer_id {
        Some(id) => id,
        None => provision_new_customer(&state, pool, session.account_id).await?,
    };

    let (plan, status, current_period_end) =
        sqlx::query_as::<_, (String, String, Option<chrono::DateTime<chrono::Utc>>)>(
            "SELECT plan, status, current_period_end FROM subscriptions WHERE account_id = $1",
        )
        .bind(session.account_id)
        .fetch_one(pool)
        .await
        .map_err(|e| {
            tracing::error!("get_subscription: failed to read subscriptions row: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let cap_status = state
        .cap_status_cache
        .get(session.account_id)
        .await
        .map(|cs| {
            cs.entries
                .into_iter()
                .map(|entry| CapStatusEntry {
                    dimension: entry.dimension.as_str().to_string(),
                    used: entry.used,
                    cap: entry.cap,
                    pct: entry.pct,
                })
                .collect()
        });

    Ok(Json(SubscriptionResponse {
        plan,
        status,
        stripe_customer_id,
        current_period_end,
        card: None,
        cap_status,
    }))
}

/// First-time provisioning path (AC-201-02/04/05): calls Stripe to create a
/// real Customer, then — only on `Ok` — atomically persists
/// `accounts.stripe_customer_id` and seeds the Free `subscriptions` row in a
/// single transaction. The Stripe call happens strictly before the
/// transaction opens, so a Stripe failure leaves zero partial state
/// (AC-201-05).
async fn provision_new_customer(
    state: &UserAdminState,
    pool: &sqlx::PgPool,
    account_id: uuid::Uuid,
) -> Result<String, StatusCode> {
    let stripe_customer_id = state
        .stripe_gateway
        .get_or_create_customer(account_id)
        .await
        .map_err(|e| {
            tracing::error!("get_subscription: stripe customer provisioning failed: {e}");
            StatusCode::BAD_GATEWAY
        })?;

    let mut tx = pool.begin().await.map_err(|e| {
        tracing::error!("get_subscription: failed to open provisioning transaction: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    sqlx::query("UPDATE accounts SET stripe_customer_id = $1 WHERE id = $2")
        .bind(&stripe_customer_id)
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!("get_subscription: failed to persist stripe_customer_id: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // ON CONFLICT DO NOTHING: a subscriptions row may already exist for this
    // account (e.g. a Pro row seeded directly, simulating a prior US-202
    // upgrade, AC-201-06) even though stripe_customer_id was still NULL.
    // Never clobber an existing row's plan/status — only ensure one exists.
    sqlx::query(
        "INSERT INTO subscriptions (account_id, plan, status) VALUES ($1, 'free', 'active') \
         ON CONFLICT (account_id) DO NOTHING",
    )
    .bind(account_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("get_subscription: failed to seed subscriptions row: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!("get_subscription: failed to commit provisioning transaction: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(stripe_customer_id)
}

/// `POST /admin/v1/billing/subscription` — session-authed, Owner/Admin only
/// (`check_rbac`, AC-202-01). Calls Stripe's Subscription create/update API
/// BEFORE any local write (write-through, AC-202-02/03). Upgrading from
/// `free_cap_exceeded` clears the status and reactivates the account's
/// projects via `lifecycle::activate_account_projects` in the same request
/// (AC-202-04, AC-207-05).
pub async fn post_subscription(
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<PlanChangeRequest>,
) -> Result<Json<SubscriptionResponse>, StatusCode> {
    // AC-202-01: Owner/Admin only (Viewer → 403). Reuses the existing simple
    // role-gate pattern (`members::invite_member`).
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    // AC-202-05: only "free"/"pro" are valid plan values (D-4).
    let plan = SubscriptionPlan::parse(&body.plan).ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;

    let pool = state.system_db.pool();

    let stripe_customer_id: String =
        sqlx::query_scalar("SELECT stripe_customer_id FROM accounts WHERE id = $1")
            .bind(session.account_id)
            .fetch_one(pool)
            .await
            .map_err(|e| {
                tracing::error!("post_subscription: failed to read accounts row: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

    let previous_status: String =
        sqlx::query_scalar("SELECT status FROM subscriptions WHERE account_id = $1")
            .bind(session.account_id)
            .fetch_one(pool)
            .await
            .map_err(|e| {
                tracing::error!("post_subscription: failed to read subscriptions row: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

    // AC-202-02/03: write-through — Stripe call happens BEFORE any local
    // write; a failed call leaves subscriptions.plan untouched.
    let subscription_view = state
        .stripe_gateway
        .upsert_subscription(&stripe_customer_id, plan)
        .await
        .map_err(|e| {
            tracing::error!("post_subscription: stripe upsert_subscription failed: {e}");
            StatusCode::BAD_GATEWAY
        })?;

    // AC-202-04: upgrading out of free_cap_exceeded clears the suspension.
    let new_status = if previous_status == "free_cap_exceeded" {
        "active"
    } else {
        previous_status.as_str()
    };

    sqlx::query(
        "UPDATE subscriptions SET plan = $1, status = $2, stripe_subscription_id = $3, \
         current_period_end = $4, updated_at = now() WHERE account_id = $5",
    )
    .bind(plan.as_str())
    .bind(new_status)
    .bind(&subscription_view.stripe_subscription_id)
    .bind(subscription_view.current_period_end)
    .bind(session.account_id)
    .execute(pool)
    .await
    .map_err(|e| {
        tracing::error!("post_subscription: failed to persist subscription update: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // AC-202-04: reactivate every suspended project under the account in the
    // same request (D-12 literal reuse — same path US-207/US-204 will use).
    if previous_status == "free_cap_exceeded" {
        let deps = LifecycleDeps {
            system_db: state.system_db.clone(),
            credential_cache: state.credential_cache.clone(),
        };
        lifecycle::activate_account_projects(session.account_id, &deps).await?;
    }

    Ok(Json(SubscriptionResponse {
        plan: plan.as_str().to_string(),
        status: new_status.to_string(),
        stripe_customer_id,
        current_period_end: Some(subscription_view.current_period_end),
        card: None,
        cap_status: None,
    }))
}
