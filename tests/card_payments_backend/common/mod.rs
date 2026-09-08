// SCAFFOLD: true
//! Common test infrastructure — card-payments-backend acceptance tests.
//!
//! Infrastructure policy (`docs/architecture/atdd-infrastructure-policy.md`):
//!   Driving port:    reqwest::Client against Axum admin server on ephemeral
//!                    port (mirrors `tests/admin_api_v2/common/mod.rs::AdminTestContext`).
//!   Driven internal: testcontainers-rs Postgres image; `SystemDb::migrate()`;
//!                    fresh per test.
//!   Driven external (Stripe): REAL Stripe test-mode API (`sk_test_...`) over
//!                    the network + Stripe CLI (`stripe trigger`) to synthesize
//!                    webhook events — D-13 (LOCKED): no mocked `IPaymentGateway`
//!                    port for this feature. `stripe_signature_middleware`
//!                    tests use real HMAC verification (deterministic crypto,
//!                    no network needed, still real code — not a mock of the
//!                    Stripe port).
//!
//! `STRIPE_SECRET_KEY` resolution: reads the process environment first; if
//! absent, loads `.env.local` at the workspace root (gitignored, contains the
//! real Stripe test-mode key) via a minimal self-contained parser — no new
//! dependency. A test that cannot find `STRIPE_SECRET_KEY` after this two-step
//! resolution panics with an actionable message (this is a harness
//! responsibility, not an acceptable RED reason — the key IS available).

#![allow(dead_code, unused_imports)]

use std::sync::Arc;

use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

use embyr_server::{
    adapters::{
        cap_status_cache::CapStatusCache, credential_cache::CredentialCache,
        email::NoopEmailSender, stripe_gateway::StripeGateway, system_db::SystemDb,
    },
    admin::{handlers::lifecycle::LifecycleDeps, router::build_admin_router},
};

/// Background `CapUsageRefresher` cycle interval for acceptance tests —
/// short enough that `poll_for_cap_status`'s up-to-90s wait observes at
/// least one real cycle quickly, without hardcoding production's
/// `EMBYR_CAP_CHECK_INTERVAL_SECS` default (30s).
const TEST_CAP_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

// ─── State-delta re-export ────────────────────────────────────────────────────
#[path = "../../common/state_delta.rs"]
pub mod state_delta;
pub use state_delta::{appended_with, assert_state_delta, containing, set_to, unchanged};

// ─── Universe — port-exposed observable names ─────────────────────────────────

/// Port-exposed observable names for card-payments-backend acceptance
/// assertions. All keys are port-exposed (HTTP response fields, DB
/// public-column reads via a dedicated read helper, process exit codes) —
/// never internal struct fields.
pub mod universe {
    pub const SUBSCRIPTION_GET_STATUS: &str = "response.subscription_get.status_code";
    pub const SUBSCRIPTION_POST_STATUS: &str = "response.subscription_post.status_code";
    pub const WEBHOOK_STATUS: &str = "response.webhook.status_code";
    pub const RUN_METERING_STATUS: &str = "response.run_metering.status_code";
    pub const STRIPE_CUSTOMER_ID_PRESENT: &str = "response.subscription.stripe_customer_id_present";
    pub const SUBSCRIPTIONS_ROW_EXISTS: &str = "db.subscriptions.row_exists";
    pub const SUBSCRIPTIONS_PLAN: &str = "db.subscriptions.plan";
    pub const SUBSCRIPTIONS_STATUS: &str = "db.subscriptions.status";
    pub const PROCESSED_WEBHOOK_EVENT_COUNT: &str = "db.processed_webhook_events.count_for_id";
    pub const PROJECT_STATUS: &str = "db.projects.status";
    pub const ACCOUNTS_STRIPE_CUSTOMER_ID: &str = "db.accounts.stripe_customer_id_present";
}

// ─── Stripe test-mode key resolution (D-13) ────────────────────────────────────

/// Minimal `.env.local` parser: sets any `KEY=VALUE` line's env var if not
/// already set in the process environment. No quoting/escaping support —
/// `.env.local` in this workspace contains one simple `STRIPE_SECRET_KEY=...`
/// line (confirmed by DISTILL's own reading of the file).
fn load_dotenv_local_if_present() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = std::path::Path::new(manifest_dir)
        .parent()
        .expect("crates dir")
        .parent()
        .expect("workspace root");
    let dotenv_path = workspace_root.join(".env.local");
    let Ok(contents) = std::fs::read_to_string(&dotenv_path) else {
        return;
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            if std::env::var(key).is_err() {
                std::env::set_var(key, value);
            }
        }
    }
}

/// Resolve the real Stripe test-mode secret key (D-13). Panics with an
/// actionable message if unavailable after checking both the process
/// environment and `.env.local` — a missing key at this point is a
/// test-harness bug, not an acceptable RED reason (per DISTILL dispatch
/// instructions).
pub fn stripe_secret_key() -> String {
    if let Ok(v) = std::env::var("STRIPE_SECRET_KEY") {
        if !v.is_empty() {
            return v;
        }
    }
    load_dotenv_local_if_present();
    std::env::var("STRIPE_SECRET_KEY").unwrap_or_else(|_| {
        panic!(
            "STRIPE_SECRET_KEY not found in process environment or .env.local at workspace root. \
             This is a test-harness bug (D-13 requires real Stripe test-mode API access) — \
             not an acceptable RED reason. Fix: ensure .env.local exists at the workspace root \
             with a STRIPE_SECRET_KEY=sk_test_... line, or `set -a; source .env.local; set +a` \
             before running tests."
        )
    })
}

/// Compute a valid `Stripe-Signature` header value for `payload` under
/// `webhook_secret`, matching Stripe's documented scheme exactly:
/// `t=<unix_timestamp>,v1=<hex(HMAC-SHA256(secret, "{timestamp}.{payload}"))>`.
/// Deterministic crypto, no network call — legitimate real code for
/// signature-verification scenarios (not a mock of the Stripe port itself;
/// see DISTILL dispatch instructions' explicit carve-out for this test shape).
pub fn sign_stripe_payload(payload: &str, webhook_secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_secs();
    let signed_payload = format!("{timestamp}.{payload}");

    let mut mac = Hmac::<Sha256>::new_from_slice(webhook_secret.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(signed_payload.as_bytes());
    let signature_hex = hex::encode(mac.finalize().into_bytes());

    format!("t={timestamp},v1={signature_hex}")
}

/// Absolute path to the Stripe CLI binary, per DISTILL dispatch instructions
/// (`/opt/homebrew/bin/stripe`, verified working).
pub const STRIPE_CLI_PATH: &str = "/opt/homebrew/bin/stripe";

/// Run `stripe trigger <event>` against the real Stripe test-mode account,
/// using `--api-key` explicitly (never `stripe login`, per dispatch
/// instructions — interactive OAuth is unavailable in this environment).
///
/// Returns `true` if the CLI invocation exits 0.
pub fn stripe_trigger(event_name: &str) -> bool {
    let key = stripe_secret_key();
    std::process::Command::new(STRIPE_CLI_PATH)
        .args(["trigger", "--api-key", &key, event_name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Operator Bearer key this test harness's admin routers are built with
/// (`operator_auth_middleware` — guards `POST /admin/v1/projects/*`,
/// `POST /admin/v1/billing/run-metering`, and `GET /metrics`).
pub const ADMIN_KEY: &str = "test-admin-key-from-env";

// ─── Test server ─────────────────────────────────────────────────────────────

/// Ephemeral test context: Postgres container + seeded account + Axum admin
/// HTTP server, wired with a real `StripeGateway` (D-13 — no mocked port).
pub struct CpbTestContext {
    /// Keeps the Postgres container alive for the test's duration.
    _container: ContainerAsync<Postgres>,
    pub base_url: String,
    pub client: reqwest::Client,
    pub pool: sqlx::PgPool,
    pub account_id: uuid::Uuid,
    /// `STRIPE_WEBHOOK_SIGNING_SECRET` this server instance was built with —
    /// tests computing their own `Stripe-Signature` header (or driving
    /// `stripe listen --print-secret`) use this value.
    pub webhook_signing_secret: String,
}

impl CpbTestContext {
    /// Build a `CpbTestContext`:
    ///   1. Start a testcontainers Postgres 15-alpine container.
    ///   2. Run migrations (includes 0019_subscriptions.sql,
    ///      0020_processed_webhook_events.sql).
    ///   3. Seed one account (no `stripe_customer_id` yet — US-201's own
    ///      lazy-provisioning behavior is exactly what the walking skeleton
    ///      proves).
    ///   4. Build the real admin router with a real `StripeGateway` (D-13).
    pub async fn new() -> Self {
        Self::with_webhook_secret("whsec_test_placeholder_not_used_for_signature_math").await
    }

    /// Same as `new()`, but with an explicit webhook signing secret — used by
    /// signature-verification scenarios that need to compute a matching
    /// `Stripe-Signature` header locally.
    pub async fn with_webhook_secret(webhook_signing_secret: &str) -> Self {
        let container = Postgres::default()
            .with_tag("15-alpine")
            .start()
            .await
            .expect("Failed to start Postgres container");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port for Postgres");
        let db_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

        let system_db = Arc::new(SystemDb::new(&db_url).await.expect("SystemDb::new failed"));
        system_db.migrate().await.expect("Migrations failed");
        let pool = system_db.pool().clone();

        let account_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (name) VALUES ('Card Payments Backend Test Account') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("insert account");

        let credential_cache = Arc::new(CredentialCache::new(1024));
        let stripe_gateway = Arc::new(StripeGateway::new(stripe_secret_key()));
        let prometheus_handle = embyr_server::observability::get_or_install_prometheus_handle();
        // card-payments-backend (US-206, step 03-01): ONE shared cache
        // between the router (reader, via `get_subscription`) and
        // `CapUsageRefresher` (writer) — see `TEST_CAP_CHECK_INTERVAL` doc
        // comment above.
        let cap_status_cache = Arc::new(CapStatusCache::new());

        let router = build_admin_router(
            system_db.clone(),
            ADMIN_KEY.to_string(),
            None,
            credential_cache.clone(),
            [0u8; 32],
            None,
            Arc::new(NoopEmailSender),
            None,
            None,
            1000.0,
            prometheus_handle,
            stripe_gateway,
            Some(webhook_signing_secret.to_string()),
            cap_status_cache.clone(),
        );

        let _cap_usage_refresher = embyr_server::sweepers::cap_usage_refresher::spawn(
            system_db.clone(),
            cap_status_cache,
            LifecycleDeps {
                system_db,
                credential_cache: credential_cache.clone(),
            },
            TEST_CAP_CHECK_INTERVAL,
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local_addr");
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("admin server task panicked");
        });
        tokio::task::yield_now().await;

        let client = reqwest::Client::builder()
            .cookie_store(false)
            .build()
            .expect("reqwest client");

        CpbTestContext {
            _container: container,
            base_url,
            client,
            pool,
            account_id,
            webhook_signing_secret: webhook_signing_secret.to_string(),
        }
    }

    /// Same as `new()`, but the `StripeGateway` is constructed with a
    /// deliberately invalid API key — real Stripe rejects it (401), giving
    /// error-path scenarios a genuine (not simulated) Stripe API failure to
    /// assert against, per D-13.
    pub async fn new_with_invalid_stripe_key() -> Self {
        Self::new_with_stripe_key("sk_test_deliberately_invalid_key_00000000000000000000000").await
    }

    async fn new_with_stripe_key(stripe_key: &str) -> Self {
        let container = Postgres::default()
            .with_tag("15-alpine")
            .start()
            .await
            .expect("Failed to start Postgres container");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port for Postgres");
        let db_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

        let system_db = Arc::new(SystemDb::new(&db_url).await.expect("SystemDb::new failed"));
        system_db.migrate().await.expect("Migrations failed");
        let pool = system_db.pool().clone();

        let account_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (name) VALUES ('Card Payments Backend Test Account') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("insert account");

        let credential_cache = Arc::new(CredentialCache::new(1024));
        let stripe_gateway = Arc::new(StripeGateway::new(stripe_key));
        let prometheus_handle = embyr_server::observability::get_or_install_prometheus_handle();
        let webhook_signing_secret = "whsec_test_placeholder_not_used_for_signature_math";
        let cap_status_cache = Arc::new(CapStatusCache::new());

        let router = build_admin_router(
            system_db.clone(),
            ADMIN_KEY.to_string(),
            None,
            credential_cache.clone(),
            [0u8; 32],
            None,
            Arc::new(NoopEmailSender),
            None,
            None,
            1000.0,
            prometheus_handle,
            stripe_gateway,
            Some(webhook_signing_secret.to_string()),
            cap_status_cache.clone(),
        );

        let _cap_usage_refresher = embyr_server::sweepers::cap_usage_refresher::spawn(
            system_db.clone(),
            cap_status_cache,
            LifecycleDeps {
                system_db,
                credential_cache: credential_cache.clone(),
            },
            TEST_CAP_CHECK_INTERVAL,
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local_addr");
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("admin server task panicked");
        });
        tokio::task::yield_now().await;

        let client = reqwest::Client::builder()
            .cookie_store(false)
            .build()
            .expect("reqwest client");

        CpbTestContext {
            _container: container,
            base_url,
            client,
            pool,
            account_id,
            webhook_signing_secret: webhook_signing_secret.to_string(),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Seed a signed-in session for `self.account_id` with the given role.
    /// Mirrors `tests/secrets_management/common/mod.rs::seed_signed_in_session`
    /// — a lighter direct-DB session seed than `admin_api_v2`'s full
    /// Argon2id+TOTP sign-in flow, since this feature does not test
    /// authentication itself, only session-authed billing routes.
    pub async fn seed_session(&self, email: &str, role: &str) -> String {
        let password_hash =
            "$argon2id$v=19$m=65536,t=3,p=4$Y2FyZHBheW1lbnRz$placeholderplaceholderplaceholder";

        let user_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO users (account_id, email, display_name, password_hash) \
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(self.account_id)
        .bind(email)
        .bind("Card Payments Backend Test User")
        .bind(password_hash)
        .fetch_one(&self.pool)
        .await
        .expect("insert user");

        sqlx::query(
            "INSERT INTO account_members (user_id, account_id, role, joined_at) \
             VALUES ($1, $2, $3, now())",
        )
        .bind(user_id)
        .bind(self.account_id)
        .bind(role)
        .execute(&self.pool)
        .await
        .expect("insert account_member");

        let cookie_value = uuid::Uuid::new_v4().to_string();
        let token_hash = blake3::hash(cookie_value.as_bytes()).as_bytes().to_vec();

        sqlx::query(
            "INSERT INTO sessions (user_id, account_id, token_hash, expires_at) \
             VALUES ($1, $2, $3, now() + interval '1 hour')",
        )
        .bind(user_id)
        .bind(self.account_id)
        .bind(&token_hash)
        .execute(&self.pool)
        .await
        .expect("insert session");

        format!("embyr_session={cookie_value}")
    }

    /// Insert a project directly under `self.account_id`, `status = 'active'`.
    /// Used by dunning/cap-exceeded scenarios that assert on
    /// `projects.status` transitions.
    pub async fn insert_project(&self, project_id: &str) {
        sqlx::query(
            "INSERT INTO projects \
             (id, account_id, backend_mode, api_key_hash_current, status, name) \
             VALUES ($1, $2, 'direct_pg', 'placeholder_hash', 'active', $1)",
        )
        .bind(project_id)
        .bind(self.account_id)
        .execute(&self.pool)
        .await
        .expect("insert project");
    }

    /// Read `projects.status` for a given project id.
    pub async fn project_status(&self, project_id: &str) -> Option<String> {
        sqlx::query_scalar("SELECT status FROM projects WHERE id = $1")
            .bind(project_id)
            .fetch_optional(&self.pool)
            .await
            .unwrap_or(None)
    }

    /// Read the `subscriptions` row for `self.account_id`, if any.
    /// Returns `(plan, status, stripe_subscription_id)`.
    pub async fn subscription_row(&self) -> Option<(String, String, Option<String>)> {
        sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT plan, status, stripe_subscription_id FROM subscriptions WHERE account_id = $1",
        )
        .bind(self.account_id)
        .fetch_optional(&self.pool)
        .await
        .unwrap_or(None)
    }

    /// Read `accounts.stripe_customer_id` for `self.account_id`.
    pub async fn account_stripe_customer_id(&self) -> Option<String> {
        sqlx::query_scalar("SELECT stripe_customer_id FROM accounts WHERE id = $1")
            .bind(self.account_id)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(None)
    }

    /// Count rows in `processed_webhook_events` matching `event_id`.
    pub async fn processed_webhook_event_count(&self, event_id: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM processed_webhook_events WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0)
    }

    /// Read the current value of the `embyr_stripe_webhook_dispatch_total`
    /// Prometheus counter (labeled by `event_type`) via the real
    /// `GET /metrics` operator route. Used by the CPB03 concurrent-redelivery
    /// regression test: the idempotency ledger's PRIMARY KEY guarantees
    /// `processed_webhook_events` stays at count 1 regardless of the TOCTOU
    /// bug (Postgres enforces the constraint atomically either way), so
    /// ledger count alone cannot prove business-logic dispatch ran exactly
    /// once. This in-process counter can: it is incremented at the exact
    /// call site dispatch begins (only reached once the INSERT race is won),
    /// and — unlike `pg_stat_user_tables`, whose per-backend flush can sit
    /// pending on an idle pooled connection well past any reasonable test
    /// timeout — is immediately consistent, since `metrics_exporter_prometheus`
    /// updates its atomic in the same process synchronously on `increment()`.
    pub async fn stripe_webhook_dispatch_count(&self, event_type: &str) -> u64 {
        let body = self
            .client
            .get(self.url("/metrics"))
            .header("Authorization", format!("Bearer {ADMIN_KEY}"))
            .send()
            .await
            .expect("GET /metrics failed")
            .text()
            .await
            .expect("GET /metrics body read failed");

        let needle = format!("embyr_stripe_webhook_dispatch_total{{event_type=\"{event_type}\"}}");
        body.lines()
            .find(|line| line.starts_with(&needle))
            .and_then(|line| line.rsplit(' ').next())
            .and_then(|value| value.trim().parse::<f64>().ok())
            .map(|value| value as u64)
            .unwrap_or(0)
    }

    /// Insert a `daily_project_metrics` row for yesterday with the given
    /// counters, used by US-205 metering scenarios (`run-metering` targets
    /// the literal previous calendar day's finalized totals).
    pub async fn insert_yesterday_metrics(
        &self,
        project_id: &str,
        reads: i64,
        writes: i64,
        deletes: i64,
    ) {
        sqlx::query(
            "INSERT INTO daily_project_metrics \
             (project_id, date, read_ops, write_ops, delete_ops) \
             VALUES ($1, CURRENT_DATE - INTERVAL '1 day', $2, $3, $4) \
             ON CONFLICT DO NOTHING",
        )
        .bind(project_id)
        .bind(reads)
        .bind(writes)
        .bind(deletes)
        .execute(&self.pool)
        .await
        .expect("insert daily_project_metrics row");
    }

    /// Insert a `daily_project_metrics` row within the current calendar
    /// month, used by US-206 cumulative cap-check scenarios. The cumulative
    /// cap query sums `m.date >= DATE_TRUNC('month', CURRENT_DATE)`
    /// (ADR-020, month-to-date caps) — clamped here so the row always lands
    /// inside that window regardless of what day of the month the test
    /// runs on (plain `CURRENT_DATE - INTERVAL '1 day'` falls into the
    /// previous month on the 1st and silently drops out of the sum).
    pub async fn insert_current_month_metrics(
        &self,
        project_id: &str,
        reads: i64,
        writes: i64,
        deletes: i64,
    ) {
        sqlx::query(
            "INSERT INTO daily_project_metrics \
             (project_id, date, read_ops, write_ops, delete_ops) \
             VALUES ($1, GREATEST(CURRENT_DATE - INTERVAL '1 day', DATE_TRUNC('month', CURRENT_DATE))::date, $2, $3, $4) \
             ON CONFLICT DO NOTHING",
        )
        .bind(project_id)
        .bind(reads)
        .bind(writes)
        .bind(deletes)
        .execute(&self.pool)
        .await
        .expect("insert daily_project_metrics row");
    }
}
