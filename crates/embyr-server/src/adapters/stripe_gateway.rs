//! `StripeGateway` — sole Stripe-calling adapter (ADR-021).
//!
//! `SCAFFOLD: true` — created by DISTILL (card-payments-backend). Concrete
//! struct, not a `trait`-based port (mirrors ADR-015's `RateLimiter`
//! precedent: Stripe has exactly one implementation, D-13 forbids a mocked
//! port).
//!
//! Implementation note (step 01-01): the workspace's pinned `async-stripe`
//! 1.0.0-rc.8 release only vendors the HTTP client scaffolding
//! (`async-stripe-client-core`) and generated value types
//! (`async-stripe-shared`) as transitive dependencies of the `async-stripe`
//! facade crate — it does NOT re-export a `Customer::create`-style request
//! builder (those live in separate, not-yet-added per-resource crates
//! upstream, e.g. `async-stripe-core`). Rather than widen the dependency
//! surface outside this step's `files_to_modify` boundary, `get_or_create_customer`
//! and `probe` call the real Stripe test-mode REST API directly via
//! `reqwest` (already a workspace runtime dependency) — still real,
//! unmocked Stripe I/O per D-13. `upsert_subscription`/`push_usage_record`/
//! `verify_webhook_signature` (out of this step's scope) are left as RED
//! scaffolds; whichever step implements them should revisit this note.
//!
//! `new()` and `probe()`'s config-validation half are NOT scaffolded (no
//! business logic to TDD — they only need to exist so this struct is
//! constructible by the composition root and other test harnesses that don't
//! exercise billing).

use chrono::{DateTime, Utc};

/// Stripe REST API base URL — real test-mode/live-mode endpoint (D-13: no
/// mock server for this feature).
const STRIPE_API_BASE: &str = "https://api.stripe.com/v1";

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

pub struct StripeGateway {
    /// `sk_test_...` / `sk_live_...`. Never logged (mirrors the
    /// `EMBYR_ADMIN_KEY` / `EMBYR_AGENT_DB_DSN` no-plaintext-in-logs
    /// precedent, ADR-018 Enforcement section).
    api_key: String,
    /// Pooled HTTP client for real Stripe REST API calls (see module doc for
    /// why this adapter calls the REST API directly rather than through
    /// generated `async-stripe` request builders).
    http: reqwest::Client,
}

impl StripeGateway {
    /// Construct a gateway holding the given API key. Performs no I/O —
    /// safe to call from composition-root wrapper functions that don't
    /// exercise billing (mirrors `AwsSecretFetcher`/`GcpSecretFetcher`'s
    /// cheap, non-network constructors). `reqwest::Client::new()` only
    /// allocates connection-pool configuration, no I/O.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            http: reqwest::Client::new(),
        }
    }

    /// Get-or-create a Stripe Customer for `account_id`. Idempotent in
    /// effect via caller contract — `billing_subscription::get_subscription`
    /// only invokes this when `accounts.stripe_customer_id` IS NULL, so a
    /// single Stripe Customer is created per never-provisioned account
    /// (AC-201-03). Returns the real Stripe Customer id (`cus_...`).
    pub async fn get_or_create_customer(
        &self,
        account_id: uuid::Uuid,
    ) -> Result<String, StripeError> {
        let resp = self
            .http
            .post(format!("{STRIPE_API_BASE}/customers"))
            .bearer_auth(&self.api_key)
            .form(&[("metadata[embyr_account_id]", account_id.to_string())])
            .send()
            .await
            .map_err(|e| StripeError::Unreachable(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(StripeError::ApiError(format!(
                "customer create failed, status {status}: {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| StripeError::ApiError(format!("invalid JSON response: {e}")))?;
        body.get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                StripeError::ApiError(format!("response missing 'id' field: {body}"))
            })
    }

    /// Create or update the Stripe Subscription for `customer_id` to `plan`'s
    /// price. Write-through: callers must not update local state until this
    /// returns `Ok` (AC-202-02/03).
    ///
    /// # Panics (RED scaffold)
    pub async fn upsert_subscription(
        &self,
        _customer_id: &str,
        _plan: embyr_core::admin::SubscriptionPlan,
    ) -> Result<StripeSubscriptionView, StripeError> {
        panic!(
            "SCAFFOLD: true -- StripeGateway::upsert_subscription not yet implemented -- RED scaffold (DISTILL, US-202)"
        )
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
        let result = self
            .http
            .get(format!("{STRIPE_API_BASE}/balance"))
            .bearer_auth(&self.api_key)
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await;

        match result {
            Ok(resp) if resp.status().is_success() => Ok(()),
            Ok(resp) if resp.status() == reqwest::StatusCode::UNAUTHORIZED => {
                Err(StripeProbeError::Unauthorized)
            }
            Ok(resp) => Err(StripeProbeError::AccountRestricted(format!(
                "unexpected status: {}",
                resp.status()
            ))),
            Err(e) => Err(StripeProbeError::Unreachable(e.to_string())),
        }
    }
}
