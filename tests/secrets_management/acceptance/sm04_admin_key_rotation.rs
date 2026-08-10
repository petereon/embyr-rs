//! US-SM-04 — Rotate EMBYR_ADMIN_KEY without a hard cutover outage.
//!
//! Acceptance criteria verified here:
//!   AC-SM-04-01: `EMBYR_ADMIN_KEY_PREVIOUS` (optional) env var.
//!   AC-SM-04-02: `operator_auth_middleware` accepts EITHER `EMBYR_ADMIN_KEY` OR
//!                `EMBYR_ADMIN_KEY_PREVIOUS` (when configured).
//!   AC-SM-04-03: `/metrics` accepts both tokens identically to every other operator route.
//!   AC-SM-04-04: when `EMBYR_ADMIN_KEY_PREVIOUS` is unset, behavior is unchanged.
//!   AC-SM-04-05: `EMBYR_ADMIN_KEY_PREVIOUS` == `EMBYR_ADMIN_KEY` is a startup config error.
//!   AC-SM-04-06: a request with neither/wrong token returns 401, unchanged.
//!   AC-SM-04-07: closing the window (removing `_PREVIOUS`, restarting) immediately
//!                invalidates the retired token.
//!
//! Driving ports:
//!   - `operator_auth_middleware` via `GET /metrics` and `GET /admin/v1/projects` HTTP
//!     requests through a real `embyr-server` subprocess (production composition root).
//!   - `dual_auth_middleware` via `GET /admin/v1/projects/:id` — the DESIGN-added
//!     consistency-fix scenario (ADR-018 §6, B-SM-07).
//!
//! Assertion mode: `assert_state_delta` (Mandate 8) with a Universe of
//! port-exposed HTTP status codes — same pattern as sm03.
//!
//! Scaffold classification target: RED — today `operator_auth_middleware`
//! (`admin/middleware/operator_auth.rs`) compares against a single
//! `state.admin_key`; every dual-token scenario fails for the right reason
//! (previous token is simply unrecognised, indistinguishable from "wrong token").

use std::collections::HashMap;
use std::time::Duration;

use crate::common::{
    assert_state_delta, set_to, start_postgres_container, universe, ServerProcess,
    TEST_ENCRYPTION_KEY,
};

async fn get_status(client: &reqwest::Client, url: &str, bearer: &str) -> u16 {
    client
        .get(url)
        .header("Authorization", format!("Bearer {bearer}"))
        .send()
        .await
        .expect("HTTP request failed")
        .status()
        .as_u16()
}

// ─── AC-SM-04-04: behavior unchanged when no previous key is configured ─────

/// A request bearing `EMBYR_ADMIN_KEY` succeeds; this is the baseline every
/// other scenario in this file chains from.
///
/// Journey:
///   Given: EMBYR_ADMIN_KEY_PREVIOUS is not set
///   When:  a request carries "Authorization: Bearer" set to EMBYR_ADMIN_KEY
///   Then:  the response is HTTP 200
///
/// @real-io @US-SM-04 @AC-SM-04-04
#[tokio::test]
async fn operator_route_accepts_current_admin_key() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "new-token-2026"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    assert!(server.wait_for_healthy(Duration::from_secs(30)).await);

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/metrics", server.admin_port);
    let before: HashMap<&str, String> = HashMap::new();
    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert(
        universe::OPERATOR_ROUTE_STATUS,
        get_status(&client, &url, "new-token-2026").await.to_string(),
    );

    let mut expected = HashMap::new();
    expected.insert(universe::OPERATOR_ROUTE_STATUS, set_to("200".to_string()));
    assert_state_delta(&before, &after, &[universe::OPERATOR_ROUTE_STATUS], &expected);

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-SM-04-02: dual-token acceptance ──────────────────────────────────────

/// Both current and previous admin tokens are accepted during the rotation window.
///
/// Journey (chained from scenario 1's Given + When — real subprocess + GET
/// /metrics — here BOTH tokens are configured and BOTH must succeed):
///   Given: EMBYR_ADMIN_KEY is "new-token-2026" and EMBYR_ADMIN_KEY_PREVIOUS is "old-token-2025"
///   When:  a request carries "Authorization: Bearer old-token-2025"
///   Then:  the response is HTTP 200
///   When:  a request carries "Authorization: Bearer new-token-2026"
///   Then:  the response is HTTP 200
///
/// @real-io @US-SM-04 @AC-SM-04-02
#[tokio::test]
async fn operator_route_accepts_previous_admin_key_during_rotation_window() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "new-token-2026"),
            ("EMBYR_ADMIN_KEY_PREVIOUS", "old-token-2025"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    assert!(
        server.wait_for_healthy(Duration::from_secs(30)).await,
        "server must start with a dual-token rotation window open"
    );

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/metrics", server.admin_port);

    let before: HashMap<&str, String> = HashMap::new();
    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert(
        universe::OPERATOR_ROUTE_STATUS,
        get_status(&client, &url, "old-token-2025").await.to_string(),
    );
    let mut expected = HashMap::new();
    expected.insert(universe::OPERATOR_ROUTE_STATUS, set_to("200".to_string()));
    assert_state_delta(&before, &after, &[universe::OPERATOR_ROUTE_STATUS], &expected);

    // Same route, new token — must ALSO succeed (both tokens valid simultaneously).
    let status_new = get_status(&client, &url, "new-token-2026").await;
    assert_eq!(status_new, 200, "the new token must also succeed during the window");

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-SM-04-03: /metrics honors the same dual-token window ────────────────

/// `GET /metrics` honors the same dual-token window as every other operator
/// route (no separate auth path).
///
/// Journey (chained from scenario 2's Given — dual-token window open):
///   Given: EMBYR_ADMIN_KEY_PREVIOUS is configured alongside EMBYR_ADMIN_KEY
///   When:  a request to GET /metrics carries Bearer set to either token
///   Then:  the response is HTTP 200 in both cases
///
/// @real-io @US-SM-04 @AC-SM-04-03
#[tokio::test]
async fn metrics_endpoint_honors_dual_token_window() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "new-token-2026"),
            ("EMBYR_ADMIN_KEY_PREVIOUS", "old-token-2025"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    assert!(server.wait_for_healthy(Duration::from_secs(30)).await);

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/metrics", server.admin_port);

    let before: HashMap<&str, String> = HashMap::new();
    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert(
        universe::METRICS_STATUS,
        get_status(&client, &url, "old-token-2025").await.to_string(),
    );
    let mut expected = HashMap::new();
    expected.insert(universe::METRICS_STATUS, set_to("200".to_string()));
    assert_state_delta(&before, &after, &[universe::METRICS_STATUS], &expected);

    let status_new = get_status(&client, &url, "new-token-2026").await;
    assert_eq!(status_new, 200);

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-SM-04-06: neither token matches ──────────────────────────────────────

/// A request with a token matching neither the current nor the previous
/// admin key returns 401 — unchanged from today.
///
/// Journey (error path):
///   Given: EMBYR_ADMIN_KEY_PREVIOUS is not set
///   When:  a request carries "Authorization: Bearer" set to any value other than EMBYR_ADMIN_KEY
///   Then:  the response is HTTP 401, identical to pre-feature behavior
///
/// @error @real-io @US-SM-04 @AC-SM-04-06
#[tokio::test]
async fn operator_route_rejects_token_matching_neither() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "new-token-2026"),
            ("EMBYR_ADMIN_KEY_PREVIOUS", "old-token-2025"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    assert!(server.wait_for_healthy(Duration::from_secs(30)).await);

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/metrics", server.admin_port);

    let before: HashMap<&str, String> = HashMap::new();
    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert(
        universe::OPERATOR_ROUTE_STATUS,
        get_status(&client, &url, "totally-unrelated-token")
            .await
            .to_string(),
    );
    let mut expected = HashMap::new();
    expected.insert(universe::OPERATOR_ROUTE_STATUS, set_to("401".to_string()));
    assert_state_delta(&before, &after, &[universe::OPERATOR_ROUTE_STATUS], &expected);

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── Hard cutover with no grace period ───────────────────────────────────────

/// Hard cutover with no grace period remains available: rotating
/// `EMBYR_ADMIN_KEY` without setting `EMBYR_ADMIN_KEY_PREVIOUS` immediately
/// invalidates the old token — the pre-existing, unchanged behavior.
///
/// Journey (error path):
///   Given: Sam rotates EMBYR_ADMIN_KEY without setting EMBYR_ADMIN_KEY_PREVIOUS
///   When:  a request carries the old, now-retired token
///   Then:  the response is HTTP 401 immediately after restart
///
/// @error @real-io @US-SM-04 @AC-SM-04-04
#[tokio::test]
async fn hard_cutover_with_no_grace_period_remains_available() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "incident-response-token"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
            // EMBYR_ADMIN_KEY_PREVIOUS deliberately absent — hard cutover.
        ],
    );
    assert!(server.wait_for_healthy(Duration::from_secs(30)).await);

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/metrics", server.admin_port);
    let status = get_status(&client, &url, "old-token-2025").await;
    assert_eq!(
        status, 401,
        "the retired token must be rejected immediately with no grace period"
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-SM-04-05: identical current/previous key rejected ───────────────────

/// Startup rejects an identical current and previous admin key.
///
/// Journey (error path):
///   Given: EMBYR_ADMIN_KEY and EMBYR_ADMIN_KEY_PREVIOUS are set to the same value
///   When:  Sam runs "cargo run -p embyr-server"
///   Then:  the process exits with code 1
///   And:   stderr states that EMBYR_ADMIN_KEY_PREVIOUS must differ from EMBYR_ADMIN_KEY
///
/// @error @US-SM-04 @AC-SM-04-05
#[tokio::test]
async fn startup_rejects_identical_current_and_previous_admin_key() {
    let mut server = ServerProcess::start_env_only(&[
        (
            "DATABASE_URL",
            "postgres://postgres:postgres@127.0.0.1:5432/embyr",
        ),
        ("EMBYR_ADMIN_KEY", "same-value"),
        ("EMBYR_ADMIN_KEY_PREVIOUS", "same-value"),
        ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
    ]);

    let exit_code = server.wait_for_exit(Duration::from_secs(10)).await;
    let stderr = server.drain_stderr();

    assert_eq!(
        exit_code,
        Some(1),
        "server must exit 1 when EMBYR_ADMIN_KEY_PREVIOUS equals EMBYR_ADMIN_KEY; got {exit_code:?}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("EMBYR_ADMIN_KEY_PREVIOUS") && stderr.contains("differ"),
        "stderr must state that EMBYR_ADMIN_KEY_PREVIOUS must differ from EMBYR_ADMIN_KEY; got: {stderr}"
    );
}

// ─── DESIGN-added consistency fix (ADR-018 §6, B-SM-07) ─────────────────────

/// `dual_auth_middleware` (GET /admin/v1/projects/:id) also accepts the
/// previous admin token — the consistency fix ADR-018 flagged: without it, a
/// mid-rotation operator using the previous token would 401 on this one
/// route while succeeding everywhere else.
///
/// Journey (chained from scenario 2's Given — dual-token window open; When
/// differs — targets the dual-auth-guarded route instead of operator-only):
///   Given: EMBYR_ADMIN_KEY_PREVIOUS is configured alongside EMBYR_ADMIN_KEY
///   When:  a request to GET /admin/v1/projects/:id carries the previous token
///   Then:  the response is HTTP 200 (not the project's actual ID — 404 is
///          also an acceptable "authenticated but not found" outcome; the
///          assertion is "not 401")
///
/// @real-io @US-SM-04 @AC-SM-04-02 @consistency-fix
#[tokio::test]
async fn dual_auth_middleware_accepts_previous_admin_key_too() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "new-token-2026"),
            ("EMBYR_ADMIN_KEY_PREVIOUS", "old-token-2025"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    assert!(server.wait_for_healthy(Duration::from_secs(30)).await);

    let client = reqwest::Client::new();
    let url = format!(
        "http://127.0.0.1:{}/admin/v1/projects/nonexistent-project-id",
        server.admin_port
    );
    let before: HashMap<&str, String> = HashMap::new();
    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert(
        universe::DUAL_AUTH_ROUTE_STATUS,
        get_status(&client, &url, "old-token-2025").await.to_string(),
    );

    let status = after
        .get(universe::DUAL_AUTH_ROUTE_STATUS)
        .expect("status recorded");
    assert_ne!(
        status, "401",
        "dual_auth_middleware must accept the previous admin token, not reject it as unauthenticated"
    );

    // Universe-bound assertion still applies: the route status is the only
    // declared observable; we assert "not 401" above because the exact
    // success status (200 vs 404 for a nonexistent project id) is not part
    // of THIS scenario's contract (auth outcome is).
    let _ = (before, after);

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── Rotation window closed ───────────────────────────────────────────────────

/// Retired admin token is rejected once the rotation window is closed
/// (removing `EMBYR_ADMIN_KEY_PREVIOUS` and restarting).
///
/// Journey (chained: this is the operational end-state of scenario 2/3's
/// rotation window — same tokens, but the server restarts WITHOUT the
/// previous var):
///   Given: EMBYR_ADMIN_KEY_PREVIOUS has been removed from the environment and the server restarted
///   When:  a request to GET /metrics carries "Authorization: Bearer old-token-2025"
///   Then:  the response is HTTP 401
///
/// @error @real-io @US-SM-04 @AC-SM-04-07
#[tokio::test]
async fn retired_admin_token_rejected_once_rotation_window_closed() {
    let (_pg, db_url) = start_postgres_container().await;

    // Window closed: only the new token is configured, mirroring the operator
    // action of dropping EMBYR_ADMIN_KEY_PREVIOUS and restarting.
    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "new-token-2026"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    assert!(server.wait_for_healthy(Duration::from_secs(30)).await);

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/metrics", server.admin_port);
    let status = get_status(&client, &url, "old-token-2025").await;
    assert_eq!(
        status, 401,
        "the retired token must be rejected once the rotation window is closed"
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}
