//! CPB08 (stripe-checkout-portal) — a customer can start a paid subscription
//! (hosted Stripe Checkout) and manage their payment method (hosted Billing
//! Portal); the webhook turns a completed Checkout into the local
//! subscription state the rest of the system reads.
//!
//!   POST /admin/v1/billing/checkout_session  (Owner only, body {"plan":"pro"})
//!   POST /admin/v1/billing/portal_session    (Owner only)
//!   customer.subscription.created webhook    (links customer -> account)
//!
//! Validation/gating tests use a placeholder Stripe key — those paths return
//! before any Stripe call. Only the last test talks to real Stripe test mode
//! (`STRIPE_SECRET_KEY`, same resolution as cpb01-cpb07).

#[path = "../common/mod.rs"]
mod common;
use common::{sign_stripe_payload, CpbTestContext};
use embyr_server::adapters::stripe_gateway::StripeGateway;

const WEBHOOK_SECRET: &str = "whsec_cpb08_test_signing_secret";
// Real-public-suffix host: Stripe validates return URLs; a `.test` TLD may be rejected.
const BASE_URL: &str = "https://console.example.com";
/// `PRO_PLAN_STRIPE_PRICE_ID` (stripe_gateway.rs) — the price the webhook maps to Pro.
const PRO_PRICE_ID: &str = "price_1U3IuADomd8yk6u5rAtzmWPt";

fn placeholder_gateway(base_url: Option<&str>) -> StripeGateway {
    StripeGateway::new("sk_test_placeholder_never_sent_to_stripe")
        .with_public_base_url(base_url.map(str::to_string))
}

async fn post(
    ctx: &CpbTestContext,
    path: &str,
    cookie: &str,
    body: serde_json::Value,
) -> reqwest::Response {
    ctx.client
        .post(ctx.url(path))
        .header("Cookie", cookie)
        .json(&body)
        .send()
        .await
        .expect("POST failed")
}

/// Seed `accounts.stripe_customer_id` directly (what a prior provisioning
/// GET would have written) so these tests need no Stripe call.
async fn seed_customer(ctx: &CpbTestContext, customer_id: &str) {
    sqlx::query("UPDATE accounts SET stripe_customer_id = $1 WHERE id = $2")
        .bind(customer_id)
        .bind(ctx.account_id)
        .execute(&ctx.pool)
        .await
        .expect("seed customer id");
    sqlx::query(
        "INSERT INTO subscriptions (account_id, plan, status) VALUES ($1, 'free', 'active') \
         ON CONFLICT (account_id) DO NOTHING",
    )
    .bind(ctx.account_id)
    .execute(&ctx.pool)
    .await
    .expect("seed free subscription row");
}

async fn deliver_event(ctx: &CpbTestContext, event_id: &str, event_type: &str, object: serde_json::Value) -> u16 {
    let payload = serde_json::json!({
        "id": event_id,
        "type": event_type,
        "data": { "object": object }
    })
    .to_string();
    let signature = sign_stripe_payload(&payload, WEBHOOK_SECRET);
    ctx.client
        .post(ctx.url("/admin/v1/webhooks/stripe"))
        .header("Stripe-Signature", signature)
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await
        .expect("webhook POST failed")
        .status()
        .as_u16()
}

/// @driving_port @error
#[tokio::test]
async fn non_owner_cannot_start_checkout_or_open_portal() {
    let ctx = CpbTestContext::with_gateway(WEBHOOK_SECRET, placeholder_gateway(Some(BASE_URL))).await;
    let cookie = ctx.seed_session("admin@cpb08.example", "Admin").await;

    let checkout = post(&ctx, "/admin/v1/billing/checkout_session", &cookie, serde_json::json!({"plan": "pro"})).await;
    let portal = post(&ctx, "/admin/v1/billing/portal_session", &cookie, serde_json::json!({})).await;

    assert_eq!(checkout.status().as_u16(), 403, "checkout is Owner-only");
    assert_eq!(portal.status().as_u16(), 403, "portal is Owner-only");
    assert_eq!(ctx.account_stripe_customer_id().await, None, "no Stripe customer provisioned on a rejected request");
}

/// @driving_port @error
#[tokio::test]
async fn checkout_rejects_free_and_unknown_plans() {
    let ctx = CpbTestContext::with_gateway(WEBHOOK_SECRET, placeholder_gateway(Some(BASE_URL))).await;
    let cookie = ctx.seed_session("owner@cpb08.example", "Owner").await;

    for plan in ["free", "enterprise", ""] {
        let resp = post(&ctx, "/admin/v1/billing/checkout_session", &cookie, serde_json::json!({"plan": plan})).await;
        assert_eq!(resp.status().as_u16(), 422, "plan {plan:?} has nothing to check out");
    }
    assert_eq!(ctx.account_stripe_customer_id().await, None);
}

/// @driving_port @error
#[tokio::test]
async fn checkout_and_portal_answer_503_when_public_base_url_unset() {
    let ctx = CpbTestContext::with_gateway(WEBHOOK_SECRET, placeholder_gateway(None)).await;
    let cookie = ctx.seed_session("owner@cpb08.example", "Owner").await;

    let checkout = post(&ctx, "/admin/v1/billing/checkout_session", &cookie, serde_json::json!({"plan": "pro"})).await;
    let portal = post(&ctx, "/admin/v1/billing/portal_session", &cookie, serde_json::json!({})).await;

    assert_eq!(checkout.status().as_u16(), 503);
    assert_eq!(portal.status().as_u16(), 503);
    assert_eq!(ctx.account_stripe_customer_id().await, None, "unconfigured -> no Stripe side effects");
}

/// @driving_port @error
#[tokio::test]
async fn portal_without_stripe_customer_is_409() {
    let ctx = CpbTestContext::with_gateway(WEBHOOK_SECRET, placeholder_gateway(Some(BASE_URL))).await;
    let cookie = ctx.seed_session("owner@cpb08.example", "Owner").await;

    let resp = post(&ctx, "/admin/v1/billing/portal_session", &cookie, serde_json::json!({})).await;

    assert_eq!(resp.status().as_u16(), 409);
}

/// @driving_port @error
#[tokio::test]
async fn checkout_for_account_already_on_paid_plan_is_409() {
    let ctx = CpbTestContext::with_gateway(WEBHOOK_SECRET, placeholder_gateway(Some(BASE_URL))).await;
    let cookie = ctx.seed_session("owner@cpb08.example", "Owner").await;
    seed_customer(&ctx, "cus_cpb08_paid").await;
    sqlx::query("UPDATE subscriptions SET plan = 'pro', stripe_subscription_id = 'sub_cpb08_paid' WHERE account_id = $1")
        .bind(ctx.account_id)
        .execute(&ctx.pool)
        .await
        .expect("seed paid subscription");

    let resp = post(&ctx, "/admin/v1/billing/checkout_session", &cookie, serde_json::json!({"plan": "pro"})).await;

    assert_eq!(resp.status().as_u16(), 409, "already subscribed -> manage via the portal");
}

/// Journey: Checkout completes -> `customer.subscription.created` links the
/// subscription to the account -> a later `customer.subscription.updated`
/// (matched by stripe_subscription_id) now finds the row. Redelivery of the
/// created event is a no-op.
///
/// @driving_port @US-203
#[tokio::test]
async fn subscription_created_webhook_links_account_and_later_events_sync() {
    let ctx = CpbTestContext::with_gateway(WEBHOOK_SECRET, placeholder_gateway(Some(BASE_URL))).await;
    seed_customer(&ctx, "cus_cpb08_link").await;
    let created_id = format!("evt_cpb08_{}", uuid::Uuid::new_v4());
    let created = serde_json::json!({
        "id": "sub_cpb08_link",
        "customer": "cus_cpb08_link",
        "status": "active",
        "current_period_end": 1_799_999_999i64,
        "items": { "data": [ { "price": { "id": PRO_PRICE_ID } } ] }
    });

    assert_eq!(deliver_event(&ctx, &created_id, "customer.subscription.created", created.clone()).await, 200);

    let row = ctx.subscription_row().await.expect("subscriptions row");
    assert_eq!(row, ("pro".to_string(), "active".to_string(), Some("sub_cpb08_link".to_string())));

    // Redelivery: 200 no-op, ledger still holds exactly one row.
    assert_eq!(deliver_event(&ctx, &created_id, "customer.subscription.created", created).await, 200);
    assert_eq!(ctx.processed_webhook_event_count(&created_id).await, 1);

    // Downstream sync now reaches the linked row.
    let updated_id = format!("evt_cpb08_{}", uuid::Uuid::new_v4());
    let updated = serde_json::json!({ "id": "sub_cpb08_link", "status": "past_due" });
    assert_eq!(deliver_event(&ctx, &updated_id, "customer.subscription.updated", updated).await, 200);
    let (plan, status, _) = ctx.subscription_row().await.expect("subscriptions row");
    assert_eq!((plan.as_str(), status.as_str()), ("pro", "past_due"));
}

/// @driving_port @error
#[tokio::test]
async fn subscription_created_for_unknown_customer_is_a_safe_noop() {
    let ctx = CpbTestContext::with_gateway(WEBHOOK_SECRET, placeholder_gateway(Some(BASE_URL))).await;
    seed_customer(&ctx, "cus_cpb08_known").await;
    let object = serde_json::json!({
        "id": "sub_cpb08_stranger",
        "customer": "cus_cpb08_someone_else",
        "status": "active",
        "items": { "data": [ { "price": { "id": PRO_PRICE_ID } } ] }
    });

    let status = deliver_event(&ctx, &format!("evt_cpb08_{}", uuid::Uuid::new_v4()), "customer.subscription.created", object).await;

    assert_eq!(status, 200);
    assert_eq!(
        ctx.subscription_row().await,
        Some(("free".to_string(), "active".to_string(), None)),
        "an unmatched customer must not touch any account's subscription"
    );
}

/// Real Stripe test mode (D-13): the owner gets a hosted Checkout URL, the
/// account's Stripe Customer is persisted, and a second call reuses it.
///
/// @driving_port @real-io
#[tokio::test]
#[ignore = "needs a Stripe test-account setting: Managed Payments is on by default and the Pro product has no tax_code, so Stripe returns 400 'product tax code is missing'. Set the product tax code (or disable Managed Payments by default) in the Stripe dashboard, then run with --ignored"]
async fn owner_checkout_returns_hosted_url_and_reuses_the_stripe_customer() {
    // The handler logs the Stripe error server-side (502 hides it from the
    // client); surface it on stderr so a CI failure is diagnosable.
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let gateway = StripeGateway::new(common::stripe_secret_key()).with_public_base_url(Some(BASE_URL.to_string()));
    let ctx = CpbTestContext::with_gateway(WEBHOOK_SECRET, gateway).await;
    let cookie = ctx.seed_session("owner@cpb08.example", "Owner").await;

    let first = post(&ctx, "/admin/v1/billing/checkout_session", &cookie, serde_json::json!({"plan": "pro"})).await;
    let status = first.status().as_u16();
    let text = first.text().await.expect("body text");
    assert_eq!(status, 200, "checkout_session response body: {text}");
    let body: serde_json::Value = serde_json::from_str(&text).expect("json body");
    let url = body["url"].as_str().expect("url string");
    assert!(url.starts_with("https://checkout.stripe.com/"), "hosted page url, got {url}");

    let customer = ctx.account_stripe_customer_id().await.expect("customer persisted");
    let second = post(&ctx, "/admin/v1/billing/checkout_session", &cookie, serde_json::json!({"plan": "pro"})).await;
    let status = second.status().as_u16();
    assert_eq!(status, 200, "second checkout_session response body: {}", second.text().await.unwrap_or_default());
    assert_eq!(ctx.account_stripe_customer_id().await, Some(customer), "customer reused, not re-created");
}
