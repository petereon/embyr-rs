// SCAFFOLD: true
//! Common test infrastructure — admin-api-v2 acceptance tests.
//!
//! Provides:
//! - `AdminTestContext`: ephemeral Postgres + Axum admin server on a random port.
//! - `seed_account`: inserts one account + one user with known TOTP secret + one Owner member.
//! - `totp_code_for_seed`: generates a valid TOTP code for the seeded user.
//! - `state_delta`: re-export of the project-level state-delta port.
//!
//! Infrastructure policy (from docs/architecture/atdd-infrastructure-policy.md):
//!   Driving port:    reqwest::Client against Axum admin server on ephemeral port.
//!   Driven internal: testcontainers-rs Postgres image; sqlx::migrate!; fresh per test module.
//!   Driven external: NoopEmailSender (in-process); FakeEmailSender (capture variant).
//!   Clock:           tokio::time::advance (paused clock) per #[tokio::test(start_paused = true)].

// ─── State-delta re-export ────────────────────────────────────────────────────
#[path = "../../common/state_delta.rs"]
pub mod state_delta;
pub use state_delta::{assert_state_delta, set_to, unchanged, appended_with, containing};

// ─── Domain snapshot types used in universe assertions ────────────────────────

/// Port-exposed observable names used in state-delta universe assertions.
/// These map to public API response fields, not internal struct fields.
pub mod universe {
    pub const SESSION_COOKIE_SET: &str = "response.set_cookie.embyr_session";
    pub const RESPONSE_STATUS: &str = "response.status_code";
    pub const ACCOUNT_LOCKED: &str = "auth.account_locked";
    pub const SESSION_ROW_EXPIRES: &str = "db.sessions.expires_at";
    pub const PROJECT_LIST_COUNT: &str = "response.projects.len";
    pub const SDK_KEY_PLAINTEXT_IN_RESPONSE: &str = "response.sdk_key.key_present";
    pub const SDK_KEY_HASH_IN_DB: &str = "db.sdk_api_keys.key_hash_stored";
    pub const MEMBER_COUNT: &str = "response.members.len";
    pub const INVITATION_ROW_CREATED: &str = "db.invitations.row_exists";
    pub const EMAIL_SENT_COUNT: &str = "fake_email_sender.sent_count";
    pub const OIDC_SECRET_IN_RESPONSE: &str = "response.oidc_provider.client_secret_present";
    pub const SESSION_COOKIE_CLEARED: &str = "response.set_cookie.max_age_zero";
}

// ─── Test server ─────────────────────────────────────────────────────────────

/// Ephemeral test context: Postgres container + seeded data + admin HTTP server.
///
/// # RED scaffold
/// This struct panics on construction because the production router
/// (`embyr_server::admin::build_admin_router`) does not yet exist.
/// When B-01 handler stubs are implemented, replace the panic with real wiring.
pub struct AdminTestContext {
    /// Base URL of the ephemeral admin HTTP server, e.g. "http://127.0.0.1:54321"
    pub base_url: String,
    /// reqwest client with no default headers (tests set Auth headers per request)
    pub client: reqwest::Client,
    /// TOTP seed secret (base32) for the seeded test user — use `totp_code_for_seed` to generate
    pub totp_secret_b32: String,
    /// UUID of the seeded account
    pub account_id: String,
    /// UUID of the seeded user (Owner)
    pub user_id: String,
    /// Email of the seeded user
    pub user_email: String,
    /// Plain-text password of the seeded user
    pub user_password: String,
}

impl AdminTestContext {
    /// Build an AdminTestContext by:
    ///   1. Starting a testcontainers Postgres container.
    ///   2. Running sqlx migrations.
    ///   3. Seeding one account + user + TOTP secret.
    ///   4. Starting the Axum admin router on an ephemeral port.
    ///
    /// # Panics (RED scaffold)
    /// Panics with "Not yet implemented -- RED scaffold" because the production
    /// `build_admin_router` function does not exist yet. This is the correct RED state.
    pub async fn new() -> Self {
        panic!("Not yet implemented -- RED scaffold: AdminTestContext::new requires embyr_server::admin::build_admin_router")
    }

    /// Generate a valid TOTP code for the seeded user's secret at the current time.
    ///
    /// # Panics (RED scaffold)
    pub fn totp_code_now(&self) -> String {
        panic!("Not yet implemented -- RED scaffold: totp_code_now requires totp-rs")
    }

    /// Generate an intentionally wrong TOTP code (off by one time step).
    pub fn totp_code_wrong(&self) -> String {
        "000000".to_string() // always wrong (real implementation will verify)
    }

    /// Returns the URL for the given admin API path, e.g. "/admin/v1/auth/signin"
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

// ─── FakeEmailSender ─────────────────────────────────────────────────────────

/// Captures emails sent during a test for assertion.
///
/// # RED scaffold
/// This is a placeholder. The real implementation uses the IEmailSender port
/// from embyr-core::admin::email (not yet scaffolded).
#[derive(Debug, Default)]
pub struct FakeEmailSender {
    pub sent: std::sync::Mutex<Vec<CapturedEmail>>,
}

#[derive(Debug, Clone)]
pub struct CapturedEmail {
    pub to: String,
    pub subject: String,
    pub body: String,
}

impl FakeEmailSender {
    pub fn sent_count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }

    pub fn last_email(&self) -> Option<CapturedEmail> {
        self.sent.lock().unwrap().last().cloned()
    }
}
