//! `stripe_webhook_handler` — `POST /admin/v1/webhooks/stripe` (US-203, US-204).
//!
//! `SCAFFOLD: true` — created by DISTILL (card-payments-backend). Dispatch
//! logic panics — DELIVER implements it via Outside-In TDD. Signature
//! verification itself happens in `stripe_signature_middleware`
//! (`admin/middleware/stripe_signature.rs`), layered in front of this
//! handler at the router level — by the time this handler runs, the request
//! is already known-authentic (AC-203-01/02).

use axum::{
    extract::{Extension, State},
    http::StatusCode,
};

use crate::adapters::stripe_gateway::{WebhookEvent, PRO_PLAN_STRIPE_PRICE_ID};
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
        // invoice.* dispatch (AC-204-01/02/03) is a LATER step's scope on
        // this same function — falls through here, deliberately unhandled.
        other => {
            tracing::info!("stripe_webhook_handler: ignoring unhandled event type {other}");
            StatusCode::OK
        }
    }
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
