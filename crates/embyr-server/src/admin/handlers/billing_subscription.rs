//! `billing_subscription` handlers — `GET`/`POST /admin/v1/billing/subscription`
//! (US-201, US-202, extended with `cap_status` by US-206).
//!
//! `SCAFFOLD: true` — created by DISTILL (card-payments-backend). Both
//! handlers panic — DELIVER implements them via Outside-In TDD, unskipping
//! one acceptance scenario at a time.
//!
//! Separate file from the existing `billing.rs` (usage-reporting, read-only,
//! zero external calls) per CPB-AD-08 — this file's handlers make real Stripe
//! network calls, a materially different responsibility/failure-mode profile.

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};

use crate::admin::extractors::session_context::SessionContext;
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

    let (plan, status, current_period_end) = sqlx::query_as::<
        _,
        (String, String, Option<chrono::DateTime<chrono::Utc>>),
    >("SELECT plan, status, current_period_end FROM subscriptions WHERE account_id = $1")
    .bind(session.account_id)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        tracing::error!("get_subscription: failed to read subscriptions row: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let cap_status = state.cap_status_cache.get(session.account_id).await.map(|cs| {
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
    let stripe_customer_id = state.stripe_gateway.get_or_create_customer(account_id).await.map_err(
        |e| {
            tracing::error!("get_subscription: stripe customer provisioning failed: {e}");
            StatusCode::BAD_GATEWAY
        },
    )?;

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
///
/// # Panics (RED scaffold)
/// Always panics. DELIVER implements the real write-through plan-change path.
pub async fn post_subscription(
    State(_state): State<UserAdminState>,
    _session: SessionContext,
    Json(_body): Json<PlanChangeRequest>,
) -> Result<Json<SubscriptionResponse>, StatusCode> {
    panic!(
        "SCAFFOLD: true -- billing_subscription::post_subscription not yet implemented -- RED scaffold (DISTILL, card-payments-backend US-202)"
    )
}
