// @US-B02 @driving_port @real-io
//! Slice B-02 — Project List + Expanded Detail.
//!
//! All tests are #[ignore] (RED). DELIVER unskips one at a time.
//! Chained from B-01: assumes session auth middleware is available.
//!
//! Key invariant under test: account_id FK scoping.
//! No cross-account leakage (AC-B02-03): the most critical security property.
//!
//! Error ratio: 4 error/edge / 10 total = 40%

#[path = "../common/mod.rs"]
mod common;
use common::AdminTestContext;

// ─────────────────────────────────────────────────────────────────────────────
// AC-B02-01: Authenticated user sees only their own databases
// ─────────────────────────────────────────────────────────────────────────────

/// Account-scoped project list: returns only projects where account_id matches session.
/// Excludes deleted projects.
///
/// AC-B02-01
// @US-B02 @AC-B02-01 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn authenticated_user_sees_only_their_account_databases() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_and_get_cookie(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET /admin/v1/projects failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-B02-01: GET /admin/v1/projects must return 200"
    );

    let projects: serde_json::Value = resp.json().await.expect("response must be JSON");
    assert!(projects.is_array(), "AC-B02-01: response must be a JSON array");

    // Each item must have required fields.
    if let Some(first) = projects.as_array().and_then(|a| a.first()) {
        for field in &["id", "name", "status", "backend_mode", "logging_enabled", "created_at"] {
            assert!(
                first.get(field).is_some(),
                "AC-B02-01: project object must include field '{}'",
                field
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B02-02: Empty account returns empty array, not 404
// ─────────────────────────────────────────────────────────────────────────────

/// Account with zero projects: GET /admin/v1/projects returns 200 + [].
///
/// AC-B02-02
// @US-B02 @AC-B02-02 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn empty_account_returns_empty_array_not_404() {
    let ctx = AdminTestContext::new().await;
    // Context with zero projects seeded for this account.
    let session_cookie = sign_in_and_get_cookie(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-B02-02: empty account must return 200, not 404"
    );

    let projects: serde_json::Value = resp.json().await.expect("response must be JSON");
    assert_eq!(
        projects.as_array().map(|a| a.len()).unwrap_or(1),
        0,
        "AC-B02-02: empty account must return []"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B02-01 (exclusion): Deleted projects excluded from list
// ─────────────────────────────────────────────────────────────────────────────

/// GET /admin/v1/projects excludes projects with status = 'deleted'.
///
/// AC-B02-01 (deleted exclusion)
// @US-B02 @AC-B02-01 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn deleted_projects_excluded_from_account_list() {
    let ctx = AdminTestContext::new().await;
    // Seed: one active project + one deleted project in the same account.
    let session_cookie = sign_in_and_get_cookie(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("request failed");

    let projects: Vec<serde_json::Value> = resp.json().await.expect("response must be JSON");

    assert!(
        projects.iter().all(|p| p["status"].as_str() != Some("deleted")),
        "AC-B02-01: deleted projects must not appear in the project list"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B02-03 (cross-account): Session from account A cannot see account B projects
// ─────────────────────────────────────────────────────────────────────────────

/// Critical security property: project detail for a project owned by account B
/// returns 403 when requested by an authenticated user from account A.
///
/// AC-B02-03
// @US-B02 @AC-B02-03 @error @driving_port @real-io
#[ignore]
#[tokio::test]
async fn cross_account_project_detail_returns_403() {
    let ctx = AdminTestContext::new().await;
    // Seed: project owned by a DIFFERENT account than ctx.account_id.
    let session_cookie = sign_in_and_get_cookie(&ctx).await;
    let other_account_project_id = "project-belonging-to-other-account"; // seeded by test context

    let resp = ctx
        .client
        .get(ctx.url(&format!("/admin/v1/projects/{}", other_account_project_id)))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "AC-B02-03: project detail for another account must return 403 (not 404)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B02-03 (expansion): Expanded project detail includes logging fields
// ─────────────────────────────────────────────────────────────────────────────

/// GET /admin/v1/projects/:id (session auth) returns expanded shape with
/// name, logging_enabled, log_retention_days, auth_mode, created_at.
///
/// AC-B02-03
// @US-B02 @AC-B02-03 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn expanded_project_detail_includes_logging_fields() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_and_get_cookie(&ctx).await;
    let project_id = "test-project-seeded-for-account"; // seeded by test context

    let resp = ctx
        .client
        .get(ctx.url(&format!("/admin/v1/projects/{}", project_id)))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-B02-03: project detail must return 200"
    );

    let project: serde_json::Value = resp.json().await.expect("response must be JSON");

    for field in &[
        "id",
        "name",
        "status",
        "backend_mode",
        "auth_mode",
        "logging_enabled",
        "log_retention_days",
        "created_at",
    ] {
        assert!(
            project.get(field).is_some(),
            "AC-B02-03: expanded detail must include field '{}'; got: {}",
            field,
            project
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B02-04: Operator Bearer can get project detail unchanged
// ─────────────────────────────────────────────────────────────────────────────

/// Existing operator route GET /admin/v1/projects/:id continues to work with
/// Bearer EMBYR_ADMIN_KEY. The dual-auth middleware allows either principal.
///
/// AC-B02-04
// @US-B02 @AC-B02-04 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn operator_bearer_can_get_project_detail_via_dual_auth_route() {
    let ctx = AdminTestContext::new().await;
    let project_id = "test-project-seeded-for-account";

    let resp = ctx
        .client
        .get(ctx.url(&format!("/admin/v1/projects/{}", project_id)))
        .header("Authorization", "Bearer test-admin-key-from-env")
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-B02-04: operator Bearer must work on dual-auth project detail route"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Error: Nonexistent project ID returns 404
// ─────────────────────────────────────────────────────────────────────────────

/// GET /admin/v1/projects/:id for a project that does not exist in any account: 404.
///
// @US-B02 @error @driving_port @real-io
#[ignore]
#[tokio::test]
async fn nonexistent_project_id_returns_404() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_and_get_cookie(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects/this-project-does-not-exist"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "error: GET project that does not exist must return 404"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Error: Session from account A cannot list account B's projects
// ─────────────────────────────────────────────────────────────────────────────

/// GET /admin/v1/projects with session from account A returns only account A's
/// projects — never account B's projects (list endpoint scoping).
///
// @US-B02 @error @driving_port @real-io
#[ignore]
#[tokio::test]
async fn project_list_scoped_to_session_account_only() {
    let ctx = AdminTestContext::new().await;
    // Seed: two accounts; ctx user is in account A; account B has a project.
    let session_cookie = sign_in_and_get_cookie(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("request failed");

    let projects: Vec<serde_json::Value> = resp.json().await.expect("response must be JSON");

    // All returned projects must belong to ctx.account_id
    for project in &projects {
        assert_eq!(
            project["account_id"].as_str().unwrap_or(""),
            ctx.account_id,
            "AC-B02-03: list must only include projects from session account; found cross-account project: {}",
            project["id"]
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Dual-auth middleware collision safety (Forge critical finding, ADR-009 AA-01)
// ─────────────────────────────────────────────────────────────────────────────

/// Operator-only routes must REJECT a session cookie — the sub-router merge
/// must not allow session-auth middleware to run on bearer-only routes.
/// Sends a valid session cookie to GET /admin/operator/projects (operator-only route).
/// Expects 401 (session cookie not accepted on operator routes).
///
// @dual-auth-safety @error @driving_port @real-io
#[ignore]
#[tokio::test]
async fn operator_bearer_route_must_reject_session_cookie_auth() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_and_get_cookie(&ctx).await;

    // Send session cookie to the operator-only project list route.
    // The operator route must not accept session cookies — only Bearer EMBYR_ADMIN_KEY.
    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/operator/projects"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "dual-auth-safety: operator route must reject session cookie; \
         expected 401, got {}. Sub-router merge is leaking session middleware onto operator routes.",
        resp.status()
    );
}

/// Session-auth routes must REJECT a Bearer EMBYR_ADMIN_KEY token when no session cookie
/// is present — the user-facing routes must not accept operator credentials.
/// Sends Bearer token to GET /admin/v1/projects (session-auth route).
/// Expects 401 (Bearer not accepted on session-auth routes without dual-auth annotation).
///
// @dual-auth-safety @error @driving_port @real-io
#[ignore]
#[tokio::test]
async fn session_auth_route_must_reject_bare_bearer_token() {
    let ctx = AdminTestContext::new().await;

    // POST /admin/v1/auth/signin is a session-auth route. Send a Bearer token with no cookie.
    // It should return 401 (not 403, not 200) — bearer alone is not a valid session credential.
    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Authorization", "Bearer test-admin-key-from-env")
        .send()
        .await
        .expect("request failed");

    // A session-only route must return 401 when only a bearer token is sent.
    // Note: dual-auth routes (AC-B02-04) DO accept Bearer; this tests a session-ONLY route.
    assert_eq!(
        resp.status().as_u16(),
        401,
        "dual-auth-safety: session-only route must reject bare Bearer token; \
         expected 401, got {}. Sub-router merge is leaking operator middleware onto session routes.",
        resp.status()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

async fn sign_in_and_get_cookie(ctx: &AdminTestContext) -> String {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email":     ctx.user_email,
            "password":  ctx.user_password,
            "totp_code": ctx.totp_code_now(),
        }))
        .send()
        .await
        .expect("sign-in failed");

    resp.headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(|s| s.trim().to_string())
        .expect("no Set-Cookie in sign-in response")
}
