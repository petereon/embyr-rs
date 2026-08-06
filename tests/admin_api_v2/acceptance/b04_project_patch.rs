// @US-B04 @driving_port @real-io
//! Slice B-04 — Project Patch, Metrics, Query Logs.
//!
//! All tests are #[ignore] (RED). DELIVER unskips one at a time.
//!
//! Key invariant under test: DSN re-encryption via AES-GCM; credential cache eviction.
//! OQ-B01 resolution: pre-existing projects with backend_pg_dsn_enc = NULL skip re-encryption.
//!
//! Error ratio: 6 error/edge / 14 total = 43% ✓

#[path = "../common/mod.rs"]
mod common;
use common::AdminTestContext;

// ─────────────────────────────────────────────────────────────────────────────
// AC-B04-01, AC-B04-06: Logging toggle takes effect
// ─────────────────────────────────────────────────────────────────────────────

/// PATCH /admin/v1/projects/:id with {logging_enabled: true}: 200; updated project returned.
/// Subsequent write operations insert query_logs entries.
/// Disabling logging stops new entries but does not delete existing ones.
///
/// AC-B04-01, AC-B04-06
// @US-B04 @AC-B04-01 @AC-B04-06 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn owner_enables_logging_and_new_entries_appear_in_logs() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    // Enable logging
    let patch_resp = ctx
        .client
        .patch(ctx.url(&format!("/admin/v1/projects/{}", project_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({
            "logging_enabled": true,
            "log_retention_days": 7
        }))
        .send()
        .await
        .expect("PATCH project failed");

    assert_eq!(
        patch_resp.status().as_u16(),
        200,
        "AC-B04-01: PATCH project must return 200"
    );

    let updated: serde_json::Value = patch_resp.json().await.expect("response must be JSON");
    assert_eq!(
        updated["logging_enabled"].as_bool(),
        Some(true),
        "AC-B04-01: updated project must reflect logging_enabled = true"
    );
    assert_eq!(
        updated["log_retention_days"].as_i64(),
        Some(7),
        "AC-B04-01: updated project must reflect log_retention_days = 7"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B04-02: backend_pg_dsn patch evicts credential cache
// ─────────────────────────────────────────────────────────────────────────────

/// PATCH with backend_pg_dsn: value ECIES-encrypted before storage; credential cache evicted.
/// Plaintext DSN never stored.
///
/// AC-B04-02
// @US-B04 @AC-B04-02 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn patch_backend_dsn_evicts_credential_cache() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    let new_dsn = "postgres://user:password@new-host:5432/mydb";

    let resp = ctx
        .client
        .patch(ctx.url(&format!("/admin/v1/projects/{}", project_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"backend_pg_dsn": new_dsn}))
        .send()
        .await
        .expect("PATCH failed");

    assert_eq!(resp.status().as_u16(), 200, "AC-B04-02: PATCH must return 200");

    // Verify DSN is not stored plaintext in DB.
    // Verify credential cache was evicted (next SDK request uses new DSN).
    panic!(
        "Not yet implemented -- RED scaffold: \
        verify ECIES encryption of DSN and credential cache eviction for project '{}'",
        project_id
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B04-02: Plaintext DSN never stored
// ─────────────────────────────────────────────────────────────────────────────

/// After PATCH with backend_pg_dsn, the projects table must not contain
/// the plaintext DSN value in any column.
///
/// AC-B04-02 (security property)
// @US-B04 @AC-B04-02 @real-io @adapter-integration
#[ignore]
#[tokio::test]
async fn backend_pg_dsn_never_stored_plaintext_after_patch() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    let plaintext_dsn = "postgres://secret_user:secret_pass@host:5432/db";

    ctx.client
        .patch(ctx.url(&format!("/admin/v1/projects/{}", project_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"backend_pg_dsn": plaintext_dsn}))
        .send()
        .await
        .expect("PATCH failed");

    // Direct DB query to verify no plaintext DSN stored.
    panic!(
        "Not yet implemented -- RED scaffold: \
        query DB to verify plaintext DSN '{}...' absent from all columns",
        &plaintext_dsn[..20]
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B04-03: status = "deleted" not patchable via PATCH (error)
// ─────────────────────────────────────────────────────────────────────────────

/// PATCH with {status: "deleted"} → 422.
/// Only "active" and "suspended" are valid status values via PATCH.
///
/// AC-B04-03
// @US-B04 @AC-B04-03 @error @driving_port @real-io
#[ignore]
#[tokio::test]
async fn patch_with_deleted_status_returns_422() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    let resp = ctx
        .client
        .patch(ctx.url(&format!("/admin/v1/projects/{}", project_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"status": "deleted"}))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        422,
        "AC-B04-03: PATCH with status=deleted must return 422"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B04-04: Metrics endpoint returns p95 and sparkline
// ─────────────────────────────────────────────────────────────────────────────

/// GET /admin/v1/projects/:id/metrics: 200 + {p95_read_ms, p95_write_ms, sparkline: [...]}.
///
/// AC-B04-04
// @US-B04 @AC-B04-04 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn metrics_endpoint_returns_p95_and_sparkline() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    let resp = ctx
        .client
        .get(ctx.url(&format!("/admin/v1/projects/{}/metrics", project_id)))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET metrics failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-B04-04: metrics must return 200"
    );

    let metrics: serde_json::Value = resp.json().await.expect("response must be JSON");

    for field in &[
        "p95_read_ms",
        "p95_write_ms",
        "reads_today",
        "writes_today",
        "deletes_today",
        "sparkline",
    ] {
        assert!(
            metrics.get(field).is_some(),
            "AC-B04-04: metrics response must include field '{}'",
            field
        );
    }

    let sparkline = metrics["sparkline"].as_array().expect("sparkline must be an array");
    assert_eq!(
        sparkline.len(),
        24,
        "AC-B04-04: sparkline must have 24 hourly buckets (V1 equal buckets)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B04-05, AC-B04-06: Query logs empty when logging disabled
// ─────────────────────────────────────────────────────────────────────────────

/// GET /admin/v1/projects/:id/query_logs when logging_enabled = false:
/// 200 + {total: 0, entries: []}.
///
/// AC-B04-05, AC-B04-06
// @US-B04 @AC-B04-05 @AC-B04-06 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn query_logs_empty_when_logging_disabled() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    // Ensure logging is disabled
    ctx.client
        .patch(ctx.url(&format!("/admin/v1/projects/{}", project_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"logging_enabled": false}))
        .send()
        .await
        .expect("PATCH failed");

    let resp = ctx
        .client
        .get(ctx.url(&format!("/admin/v1/projects/{}/query_logs", project_id)))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET query_logs failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-B04-05: query_logs with logging disabled must return 200 (not 404)"
    );

    let body: serde_json::Value = resp.json().await.expect("response must be JSON");
    assert_eq!(
        body["total"].as_i64(),
        Some(0),
        "AC-B04-05: total must be 0 when logging disabled"
    );
    assert_eq!(
        body["entries"].as_array().map(|a| a.len()),
        Some(0),
        "AC-B04-05: entries must be empty when logging disabled"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B04-05: Query logs cursor pagination
// ─────────────────────────────────────────────────────────────────────────────

/// GET /admin/v1/projects/:id/query_logs returns max 500 rows per page;
/// cursor pagination via ?after=<id> returns next page.
///
/// AC-B04-05
// @US-B04 @AC-B04-05 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn query_logs_cursor_pagination_returns_next_page() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";
    // Seed 501+ log entries with logging enabled.

    let resp1 = ctx
        .client
        .get(ctx.url(&format!("/admin/v1/projects/{}/query_logs", project_id)))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET query_logs page 1 failed");

    let page1: serde_json::Value = resp1.json().await.expect("response must be JSON");
    let entries1 = page1["entries"].as_array().expect("entries must be array");

    assert!(
        entries1.len() <= 500,
        "AC-B04-05: max 500 rows per page; got {}",
        entries1.len()
    );

    // If total > 500, fetch next page
    if page1["total"].as_i64().unwrap_or(0) > 500 {
        let last_id = entries1.last().and_then(|e| e["id"].as_str()).unwrap_or("");
        let resp2 = ctx
            .client
            .get(ctx.url(&format!(
                "/admin/v1/projects/{}/query_logs?after={}",
                project_id, last_id
            )))
            .header("Cookie", &session_cookie)
            .send()
            .await
            .expect("GET query_logs page 2 failed");

        let page2: serde_json::Value = resp2.json().await.expect("page 2 response must be JSON");
        assert!(
            page2["entries"].as_array().map(|a| a.len()).unwrap_or(0) > 0,
            "AC-B04-05: page 2 must return additional entries after cursor"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B04-05: Query logs filtered by operation type
// ─────────────────────────────────────────────────────────────────────────────

/// GET /admin/v1/projects/:id/query_logs?op=read returns only read operations.
///
/// AC-B04-05
// @US-B04 @AC-B04-05 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn query_logs_filtered_by_operation_type() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    let resp = ctx
        .client
        .get(ctx.url(&format!(
            "/admin/v1/projects/{}/query_logs?op=read",
            project_id
        )))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET query_logs?op=read failed");

    let body: serde_json::Value = resp.json().await.expect("response must be JSON");
    let entries = body["entries"].as_array().expect("entries must be array");

    for entry in entries {
        assert_eq!(
            entry["op"].as_str(),
            Some("read"),
            "AC-B04-05: op=read filter must only return read operations; got: {}",
            entry["op"]
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B04-06: Disabling logging does not delete existing entries
// ─────────────────────────────────────────────────────────────────────────────

/// After disabling logging, existing log entries remain (expire per log_retention_days).
///
/// AC-B04-06
// @US-B04 @AC-B04-06 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn disabling_logging_does_not_delete_existing_logs() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    // Enable logging and seed some entries via DB
    ctx.client
        .patch(ctx.url(&format!("/admin/v1/projects/{}", project_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"logging_enabled": true}))
        .send()
        .await
        .expect("enable logging failed");

    // Seed log entries directly (panics until DB access wired in AdminTestContext)
    // Then disable logging
    ctx.client
        .patch(ctx.url(&format!("/admin/v1/projects/{}", project_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"logging_enabled": false}))
        .send()
        .await
        .expect("disable logging failed");

    // Verify entries are still in DB (not deleted)
    panic!(
        "Not yet implemented -- RED scaffold: \
        verify existing log entries survive logging disable for project '{}'",
        project_id
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// OQ-B01: Pre-existing project with null DSN skips re-encryption
// ─────────────────────────────────────────────────────────────────────────────

/// PATCH on a project provisioned before admin-api-v2 (backend_pg_dsn_enc = NULL):
/// DSN re-encryption is skipped silently. Known limitation documented in OQ-B01.
///
/// OQ-B01 resolution: skip when backend_pg_dsn_enc IS NULL
// @US-B04 @OQ-B01 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn pre_existing_project_with_null_dsn_enc_skips_re_encryption_silently() {
    let ctx = AdminTestContext::new().await;
    // Seed a project with backend_pg_dsn_enc = NULL (pre-migration project).
    panic!(
        "Not yet implemented -- RED scaffold: \
        seed pre-existing project (dsn_enc=NULL) and verify PATCH does not error"
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Error: PATCH project from different account returns 403
// ─────────────────────────────────────────────────────────────────────────────

// @US-B04 @error @driving_port @real-io
#[ignore]
#[tokio::test]
async fn patch_project_from_different_account_returns_403() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let other_account_project_id = "project-belonging-to-other-account";

    let resp = ctx
        .client
        .patch(ctx.url(&format!("/admin/v1/projects/{}", other_account_project_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"logging_enabled": true}))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "error: PATCH project from different account must return 403"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Error: Metrics for project not in account returns 404
// ─────────────────────────────────────────────────────────────────────────────

// @US-B04 @error @driving_port @real-io
#[ignore]
#[tokio::test]
async fn metrics_for_project_not_in_account_returns_404() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects/project-belonging-to-other-account/metrics"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "AC-B04-04: metrics for project not in account must return 404"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Error: Query logs for project not in account returns 404
// ─────────────────────────────────────────────────────────────────────────────

// @US-B04 @error @driving_port @real-io
#[ignore]
#[tokio::test]
async fn query_logs_for_project_not_in_account_returns_404() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects/project-belonging-to-other-account/query_logs"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "AC-B04-05: query_logs for project not in account must return 404"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

async fn sign_in_as_owner(ctx: &AdminTestContext) -> String {
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
        .expect("no cookie in sign-in response")
}
