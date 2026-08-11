//! `StripeGateway` — sole Stripe-calling adapter (ADR-021).
//!
//! `SCAFFOLD: true` — created by DISTILL (card-payments-backend). Concrete
//! struct, not a `trait`-based port (mirrors ADR-015's `RateLimiter`
//! precedent: Stripe has exactly one implementation, D-13 forbids a mocked
//! port).
//!
//! Implementation note (step 01-05, superseding step 01-01): step 01-01
//! discovered the pinned `async-stripe` 1.0.0-rc.8 facade crate does not
//! itself vend per-resource typed request builders — those live in separate
//! companion crates (`async-stripe-core`, `async-stripe-billing`,
//! `async-stripe-webhook`, now added to the workspace) — and used hand-rolled
//! `reqwest` calls as a stopgap. Step 01-05 migrates `get_or_create_customer`
//! and `probe` to the real typed builders: `stripe_core::customer::CreateCustomer`
//! and `stripe_core::balance::RetrieveForMyAccountBalance`, sent through
//! `stripe::Client` (the facade crate's hyper-backed client, re-exported as
//! `Client` because this workspace enables `async-stripe`'s `__hyper`
//! feature transitively via `rustls-tls-webpki-roots`). Still real, unmocked
//! Stripe I/O per D-13 — only the request-construction mechanism changed.
//! `upsert_subscription`/`push_usage_record`/`verify_webhook_signature` (out
//! of this step's scope) are left as RED scaffolds; whichever step
//! implements them should revisit this note.
//!
//! `new()` and `probe()`'s config-validation half are NOT scaffolded (no
//! business logic to TDD — they only need to exist so this struct is
//! constructible by the composition root and other test harnesses that don't
//! exercise billing).

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use stripe::StripeRequest;

// ---------------------------------------------------------------------------
// Value types (real — not scaffolded; plain data carriers)
// ---------------------------------------------------------------------------

/// Errors returned by `StripeGateway` methods.
#[derive(Debug, thiserror::Error)]
pub enum StripeError {
    #[error("stripe API call failed: {0}")]
    ApiError(String),
    #[error("stripe API unreachable: {0}")]
    Unreachable(String),
    #[error("webhook signature verification failed: {0}")]
    InvalidSignature(String),
    #[error("scaffold: {0}")]
    NotYetImplemented(&'static str),
}

/// Fault classification for `StripeGateway::probe()` (Earned Trust, Principle 12).
#[derive(Debug, thiserror::Error)]
pub enum StripeProbeError {
    #[error("stripe API key rejected (401)")]
    Unauthorized,
    #[error("stripe API unreachable: {0}")]
    Unreachable(String),
    #[error("stripe account restricted: {0}")]
    AccountRestricted(String),
}

/// A minimal, project-owned projection of a Stripe Subscription object —
/// decoupled from `async-stripe`'s own `Subscription` type so this adapter's
/// public surface doesn't leak the vendor SDK's shape into callers.
#[derive(Debug, Clone)]
pub struct StripeSubscriptionView {
    pub stripe_subscription_id: String,
    pub stripe_price_id: String,
    pub current_period_start: DateTime<Utc>,
    pub current_period_end: DateTime<Utc>,
}

/// A minimal, project-owned projection of a verified Stripe webhook event —
/// carries only what `webhooks_stripe::stripe_webhook_handler` needs
/// (event id for idempotency, event type for dispatch, raw JSON payload for
/// per-type field extraction).
#[derive(Debug, Clone)]
pub struct WebhookEvent {
    pub id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
}

// ---------------------------------------------------------------------------
// StripeGateway
// ---------------------------------------------------------------------------

/// Real Stripe Price id for the Pro plan (D-4: exactly two tiers).
/// Provisioned once directly against the connected Stripe test-mode account
/// (D-13) — the vendored `async-stripe-core`/`async-stripe-billing`
/// companion crates only expose read-only `Product`/`Price` *data shapes*
/// (`async-stripe-shared`), not request builders to create them, so a Price
/// is provisioned the same way a real deployment would (Stripe
/// Dashboard/CLI, once) and referenced here as static config — never
/// created per-request. `lookup_key = "embyr_pro_monthly"`, $29.00/month USD.
const PRO_PLAN_STRIPE_PRICE_ID: &str = "price_1U3IuADomd8yk6u5rAtzmWPt";

pub struct StripeGateway {
    /// Typed Stripe HTTP client (holds the `sk_test_...` / `sk_live_...`
    /// secret internally as a redacted, sensitive header value — never
    /// logged, mirrors the `EMBYR_ADMIN_KEY` / `EMBYR_AGENT_DB_DSN`
    /// no-plaintext-in-logs precedent, ADR-018 Enforcement section).
    client: stripe::Client,
}

impl StripeGateway {
    /// Construct a gateway holding the given API key. Performs no I/O —
    /// safe to call from composition-root wrapper functions that don't
    /// exercise billing (mirrors `AwsSecretFetcher`/`GcpSecretFetcher`'s
    /// cheap, non-network constructors). `stripe::Client::new()` only builds
    /// connection-pool configuration (hyper client construction), no I/O.
    ///
    /// The workspace links both the `ring` and `aws-lc-rs` rustls crypto
    /// backends transitively (`ring` via `sqlx`/`reqwest`/`tokio-rustls`,
    /// `aws-lc-rs` via the AWS SDK's `hyper-rustls` default feature) —
    /// `stripe::Client`'s hyper-rustls connector resolves its
    /// `CryptoProvider` eagerly at construction via
    /// `rustls::crypto::CryptoProvider::get_default()`, which panics when
    /// both backends are linked and no default has been installed yet.
    /// Installs `ring` as the process default the same way
    /// `embyr-agent::main` and its acceptance-test harnesses already do
    /// (`let _ = ...install_default()` — idempotent, ignores `Err` if
    /// another call site already installed one first).
    pub fn new(api_key: impl Into<String>) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Self {
            client: stripe::Client::new(api_key),
        }
    }

    /// Get-or-create a Stripe Customer for `account_id`. Idempotent in
    /// effect via caller contract — `billing_subscription::get_subscription`
    /// only invokes this when `accounts.stripe_customer_id` IS NULL, so a
    /// single Stripe Customer is created per never-provisioned account
    /// (AC-201-03). Returns the real Stripe Customer id (`cus_...`).
    ///
    /// Sets a synthetic billing email (`account-<id>@billing.embyr.invalid`)
    /// — Stripe requires a Customer email to send Subscription invoices to
    /// under `collection_method=send_invoice` (`upsert_subscription`'s
    /// no-card path, D-3/D-5 card capture out of scope for this feature).
    pub async fn get_or_create_customer(
        &self,
        account_id: uuid::Uuid,
    ) -> Result<String, StripeError> {
        let mut metadata = HashMap::with_capacity(1);
        metadata.insert("embyr_account_id".to_string(), account_id.to_string());

        let customer = stripe_core::customer::CreateCustomer::new()
            .metadata(metadata)
            .email(format!("account-{account_id}@billing.embyr.invalid"))
            .send(&self.client)
            .await
            .map_err(map_stripe_error)?;

        Ok(customer.id.to_string())
    }

    /// Create or update the Stripe Subscription for `customer_id` to `plan`'s
    /// price. Write-through: callers must not update local state until this
    /// returns `Ok` (AC-202-02/03). Lists the customer's active subscriptions
    /// first — updates the existing one's item price if found, else creates a
    /// new Subscription (step 01-02, US-202).
    ///
    /// Free-plan downgrade is out of this slice's scope (see
    /// `slice-02-real-plan-change.md`'s OUT Scope: "downgrade-scheduling
    /// semantics ... DESIGN decision, not locked here") — returns
    /// `NotYetImplemented` rather than fabricating unverified behavior.
    pub async fn upsert_subscription(
        &self,
        customer_id: &str,
        plan: embyr_core::admin::SubscriptionPlan,
    ) -> Result<StripeSubscriptionView, StripeError> {
        let price_id = match plan {
            embyr_core::admin::SubscriptionPlan::Pro => PRO_PLAN_STRIPE_PRICE_ID,
            embyr_core::admin::SubscriptionPlan::Free => {
                return Err(StripeError::NotYetImplemented(
                    "downgrade-to-Free Stripe Subscription scheduling not yet implemented (out of US-202 scope)",
                ))
            }
        };

        let existing = stripe_billing::subscription::ListSubscription::new()
            .customer(customer_id)
            .status(stripe_billing::subscription::ListSubscriptionStatus::Active)
            .limit(1)
            .send(&self.client)
            .await
            .map_err(map_stripe_error)?;

        let subscription = match existing.data.into_iter().next() {
            Some(existing_subscription) => {
                let mut item = stripe_billing::subscription::UpdateSubscriptionItems::new();
                item.id = existing_subscription
                    .items
                    .data
                    .first()
                    .map(|item| item.id.to_string());
                item.price = Some(price_id.to_string());

                stripe_billing::subscription::UpdateSubscription::new(existing_subscription.id)
                    .items(vec![item])
                    // No card capture in this feature's scope (D-3/D-5) — the
                    // account's Stripe Customer has no payment method on
                    // file. `send_invoice` (email an invoice, due in 30
                    // days) creates/updates the real Subscription without
                    // requiring one, unlike the `charge_automatically`
                    // default which 400s with `resource_missing`.
                    .collection_method(stripe_billing::SubscriptionCollectionMethod::SendInvoice)
                    .days_until_due(30u32)
                    .send(&self.client)
                    .await
                    .map_err(map_stripe_error)?
            }
            None => {
                let mut item = stripe_billing::subscription::CreateSubscriptionItems::new();
                item.price = Some(price_id.to_string());

                stripe_billing::subscription::CreateSubscription::new()
                    .customer(customer_id)
                    .items(vec![item])
                    // See comment on the update arm above (D-3/D-5).
                    .collection_method(stripe_billing::SubscriptionCollectionMethod::SendInvoice)
                    .days_until_due(30u32)
                    .send(&self.client)
                    .await
                    .map_err(map_stripe_error)?
            }
        };

        let (period_start, period_end) = subscription
            .items
            .data
            .first()
            .map(|item| (item.current_period_start, item.current_period_end))
            .unwrap_or((0, 0));

        Ok(StripeSubscriptionView {
            stripe_subscription_id: subscription.id.to_string(),
            stripe_price_id: price_id.to_string(),
            current_period_start: DateTime::from_timestamp(period_start, 0).unwrap_or_default(),
            current_period_end: DateTime::from_timestamp(period_end, 0).unwrap_or_default(),
        })
    }

    /// Push one Stripe Usage Record for `subscription_item_id`. `idempotency_key`
    /// (shape `{project_id}:{dimension}:{date}`, AC-205-03) is passed as
    /// `async-stripe`'s native idempotency-key request parameter — reuses
    /// Stripe's own server-side idempotency guarantee (no local ledger for
    /// this call, DDD-7/CPB-AD-06).
    ///
    /// # Panics (RED scaffold)
    pub async fn push_usage_record(
        &self,
        _subscription_item_id: &str,
        _quantity: u64,
        _timestamp: DateTime<Utc>,
        _idempotency_key: &str,
    ) -> Result<(), StripeError> {
        panic!(
            "SCAFFOLD: true -- StripeGateway::push_usage_record not yet implemented -- RED scaffold (DISTILL, US-205)"
        )
    }

    /// Verify a `Stripe-Signature` header against `payload` and
    /// `webhook_secret`. Deterministic HMAC-SHA256 timing-safe comparison
    /// with Stripe's documented replay-window tolerance — no network call
    /// (ADR-021 § Enforcement; delegates to `async-stripe`'s
    /// `Webhook::construct_event` once implemented).
    ///
    /// # Panics (RED scaffold)
    pub fn verify_webhook_signature(
        &self,
        _payload: &[u8],
        _sig_header: &str,
        _webhook_secret: &str,
    ) -> Result<WebhookEvent, StripeError> {
        panic!(
            "SCAFFOLD: true -- StripeGateway::verify_webhook_signature not yet implemented -- RED scaffold (DISTILL, US-203)"
        )
    }

    /// Earned Trust probe (Principle 12): `GET /v1/balance`, 3s timeout.
    /// Soft failure — callers WARN, never refuse to start (billing is not on
    /// the Firestore protocol-serving critical path, ADR-021).
    pub async fn probe(&self) -> Result<(), StripeProbeError> {
        let result = stripe_core::balance::RetrieveForMyAccountBalance::new()
            .customize()
            .timeout(Duration::from_secs(3))
            .send(&self.client)
            .await;

        match result {
            Ok(_balance) => Ok(()),
            Err(stripe::StripeError::Stripe(_, 401)) => Err(StripeProbeError::Unauthorized),
            Err(stripe::StripeError::Stripe(errors, status)) => {
                Err(StripeProbeError::AccountRestricted(format!(
                    "unexpected status {status}: {errors:?}"
                )))
            }
            Err(e) => Err(StripeProbeError::Unreachable(e.to_string())),
        }
    }
}

/// Maps `stripe::StripeError` (the vendor SDK's transport/API error enum) to
/// this adapter's own `StripeError` — `Stripe(_, status)` (a real Stripe API
/// error response, e.g. 401 on an invalid key) becomes `ApiError`; anything
/// else (connection failure, timeout, response deserialization, client
/// misconfiguration) becomes `Unreachable`. Both map to the same
/// `StatusCode::BAD_GATEWAY` at the handler boundary (AC-201-05/AC-202-03),
/// so this classification only affects the error message, not observable
/// behavior. Shared by every Stripe call this adapter makes (customer
/// provisioning, subscription create/update).
fn map_stripe_error(err: stripe::StripeError) -> StripeError {
    match err {
        stripe::StripeError::Stripe(errors, status) => StripeError::ApiError(format!(
            "stripe API call failed, status {status}: {errors:?}"
        )),
        other => StripeError::Unreachable(other.to_string()),
    }
}
