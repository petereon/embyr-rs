//! `stripe_webhook_handler` — `POST /admin/v1/webhooks/stripe` (US-203, US-204).
//!
//! Signature verification happens in `stripe_signature_middleware`
//! (`admin/middleware/stripe_signature.rs`), layered in front of this
//! handler at the router level — by the time this handler runs, the request
//! is already known-authentic (AC-203-01/02).

use axum::{
    extract::{Extension, State},
    http::StatusCode,
};

use crate::adapters::stripe_gateway::{WebhookEvent, PRO_PLAN_STRIPE_PRICE_ID};
use crate::admin::handlers::lifecycle::{self, LifecycleDeps};
use crate::admin::state::WebhookState;
use embyr_core::admin::billing::SubscriptionStatus;

/// Dispatches a verified Stripe webhook event (already authenticated by
/// `stripe_signature_middleware`, which stashes the `WebhookEvent` in
/// request extensions) by `event.type`:
///   - `customer.subscription.updated` — sync the local `subscriptions` row's
///     status/current_period_end (and plan, when the payload's Subscription
///     item price is recognized) (AC-203-04).
///   - `customer.subscription.deleted` — mark the local row `Canceled` so it
///     never falsely shows an active paid plan (AC-203-05).
///   - `invoice.*` — a LATER step's scope on this same function; falls
///     through the catch-all below until that step splits it out.
///   - any other validly-signed type — 200, logged, ignored (AC-203-06).
///
/// Idempotent via `processed_webhook_events`: a redelivered `event.id` is a
/// 200 no-op (AC-203-03) — checked BEFORE any state-mutating dispatch,
/// including the dedupe-ledger insert itself.
pub async fn stripe_webhook_handler(
    State(state): State<WebhookState>,
    Extension(event): Extension<WebhookEvent>,
) -> StatusCode {
    let pool = state.system_db.pool();

    let already_processed: Option<i32> =
        match sqlx::query_scalar("SELECT 1 FROM processed_webhook_events WHERE event_id = $1")
            .bind(&event.id)
            .fetch_optional(pool)
            .await
        {
            Ok(row) => row,
            Err(e) => {
                tracing::error!("stripe_webhook_handler: dedupe lookup failed: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
        };

    if already_processed.is_some() {
        return StatusCode::OK;
    }

    if let Err(e) = sqlx::query(
        "INSERT INTO processed_webhook_events (event_id, event_type) VALUES ($1, $2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(&event.id)
    .bind(&event.event_type)
    .execute(pool)
    .await
    {
        tracing::error!("stripe_webhook_handler: failed to record processed event: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    match event.event_type.as_str() {
        "customer.subscription.updated" => sync_subscription_updated(pool, &event).await,
        "customer.subscription.deleted" => sync_subscription_deleted(pool, &event).await,
        "invoice.payment_failed" => handle_invoice_payment_failed(&state, &event).await,
        "invoice.payment_succeeded" => handle_invoice_payment_succeeded(&state, &event).await,
        other => {
            tracing::info!("stripe_webhook_handler: ignoring unhandled event type {other}");
            StatusCode::OK
        }
    }
}

/// AC-204-01/02/05: a final-failure `invoice.payment_failed` (retry schedule
/// exhausted — `data.object.next_payment_attempt` null/absent) suspends
/// every active project under the matched account via
/// `lifecycle::suspend_account_projects` (D-12 literal reuse — same
/// `set_project_status` the operator-initiated `suspend_project` handler
/// uses, so credential-cache eviction happens identically, AC-204-05) and
/// marks the local subscription `past_due`. A non-final (intermediate
/// retry) failure is a 200 no-op (AC-204-02). An unmatched
/// `stripe_subscription_id` is a safe 200 no-op (forward-compatible,
/// mirrors AC-203-06).
async fn handle_invoice_payment_failed(state: &WebhookState, event: &WebhookEvent) -> StatusCode {
    let object = &event.payload["data"]["object"];
    if !is_final_failure(object) {
        return StatusCode::OK;
    }
    let Some(stripe_subscription_id) = object["subscription"].as_str() else {
        tracing::error!(
            "stripe_webhook_handler: invoice.payment_failed payload missing data.object.subscription"
        );
        return StatusCode::OK;
    };
    let Some(account_id) = resolve_account_id(state.system_db.pool(), stripe_subscription_id).await
    else {
        return StatusCode::OK;
    };

    let deps = LifecycleDeps {
        system_db: state.system_db.clone(),
        credential_cache: state.credential_cache.clone(),
    };
    if let Err(status) = lifecycle::suspend_account_projects(account_id, &deps).await {
        return status;
    }

    let result = sqlx::query(
        "UPDATE subscriptions SET status = $1, updated_at = now() WHERE stripe_subscription_id = $2",
    )
    .bind(SubscriptionStatus::PastDue.as_str())
    .bind(stripe_subscription_id)
    .execute(state.system_db.pool())
    .await;

    match result {
        Ok(_) => StatusCode::OK,
        Err(e) => {
            tracing::error!("stripe_webhook_handler: failed to mark subscription past_due: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

/// AC-204-03: reactivates every suspended project under the matched account
/// via `lifecycle::activate_account_projects` — already implemented (step
/// 01-02), reused unchanged here — and marks the local subscription
/// `active`. An unmatched `stripe_subscription_id` is a safe 200 no-op.
async fn handle_invoice_payment_succeeded(
    state: &WebhookState,
    event: &WebhookEvent,
) -> StatusCode {
    let object = &event.payload["data"]["object"];
    let Some(stripe_subscription_id) = object["subscription"].as_str() else {
        tracing::error!(
            "stripe_webhook_handler: invoice.payment_succeeded payload missing data.object.subscription"
        );
        return StatusCode::OK;
    };
    let Some(account_id) = resolve_account_id(state.system_db.pool(), stripe_subscription_id).await
    else {
        return StatusCode::OK;
    };

    let deps = LifecycleDeps {
        system_db: state.system_db.clone(),
        credential_cache: state.credential_cache.clone(),
    };
    if let Err(status) = lifecycle::activate_account_projects(account_id, &deps).await {
        return status;
    }

    let result = sqlx::query(
        "UPDATE subscriptions SET status = $1, updated_at = now() WHERE stripe_subscription_id = $2",
    )
    .bind(SubscriptionStatus::Active.as_str())
    .bind(stripe_subscription_id)
    .execute(state.system_db.pool())
    .await;

    match result {
        Ok(_) => StatusCode::OK,
        Err(e) => {
            tracing::error!("stripe_webhook_handler: failed to mark subscription active: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

/// AC-204-01 vs AC-204-02: retry schedule exhausted iff
/// `next_payment_attempt` is null or absent (serde_json's `Index` impl
/// returns `Value::Null` for a missing key, so one check covers both).
fn is_final_failure(object: &serde_json::Value) -> bool {
    object["next_payment_attempt"].is_null()
}

/// Resolve `subscriptions.account_id` from a Stripe subscription id.
/// `None` when no local row matches — the unmatched-subscription no-op path
/// shared by both `invoice.*` arms.
async fn resolve_account_id(
    pool: &sqlx::PgPool,
    stripe_subscription_id: &str,
) -> Option<uuid::Uuid> {
    sqlx::query_scalar("SELECT account_id FROM subscriptions WHERE stripe_subscription_id = $1")
        .bind(stripe_subscription_id)
        .fetch_optional(pool)
        .await
        .unwrap_or(None)
}

/// AC-203-04: sync status/current_period_end (always present on this event
/// type) and, when derivable, plan — matched by `stripe_subscription_id`.
async fn sync_subscription_updated(pool: &sqlx::PgPool, event: &WebhookEvent) -> StatusCode {
    let object = &event.payload["data"]["object"];
    let Some(stripe_subscription_id) = object["id"].as_str() else {
        tracing::error!(
            "stripe_webhook_handler: subscription.updated payload missing data.object.id"
        );
        return StatusCode::OK;
    };
    let status = object["status"].as_str().unwrap_or("active");
    let current_period_end = object["current_period_end"]
        .as_i64()
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0));
    let plan = subscription_item_price_id(object).and_then(|price_id| {
        (price_id == PRO_PLAN_STRIPE_PRICE_ID)
            .then_some(embyr_core::admin::billing::SubscriptionPlan::Pro.as_str())
    });

    let result = sqlx::query(
        "UPDATE subscriptions SET status = $1, \
         current_period_end = COALESCE($2, current_period_end), \
         plan = COALESCE($3, plan), updated_at = now() \
         WHERE stripe_subscription_id = $4",
    )
    .bind(status)
    .bind(current_period_end)
    .bind(plan)
    .bind(stripe_subscription_id)
    .execute(pool)
    .await;

    match result {
        Ok(_) => StatusCode::OK,
        Err(e) => {
            tracing::error!("stripe_webhook_handler: failed to sync subscription update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

/// AC-203-05: mark the local row `Canceled` — never leave it falsely
/// showing an active paid plan after Stripe reports the Subscription gone.
async fn sync_subscription_deleted(pool: &sqlx::PgPool, event: &WebhookEvent) -> StatusCode {
    let object = &event.payload["data"]["object"];
    let Some(stripe_subscription_id) = object["id"].as_str() else {
        tracing::error!(
            "stripe_webhook_handler: subscription.deleted payload missing data.object.id"
        );
        return StatusCode::OK;
    };

    let result = sqlx::query(
        "UPDATE subscriptions SET status = $1, updated_at = now() WHERE stripe_subscription_id = $2",
    )
    .bind(SubscriptionStatus::Canceled.as_str())
    .bind(stripe_subscription_id)
    .execute(pool)
    .await;

    match result {
        Ok(_) => StatusCode::OK,
        Err(e) => {
            tracing::error!("stripe_webhook_handler: failed to sync subscription deletion: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

/// Real Stripe Subscription-object shape: `items.data[0].price.id`. Absent
/// on the synthetic test payloads used by AC-203-03/06 (no `items` array) —
/// callers treat `None` as "cannot derive plan, leave unchanged".
fn subscription_item_price_id(object: &serde_json::Value) -> Option<&str> {
    object["items"]["data"]
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item["price"]["id"].as_str())
}
