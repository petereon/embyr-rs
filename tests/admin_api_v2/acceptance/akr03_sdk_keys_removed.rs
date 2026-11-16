// @driving_port @real-io
//! api-key-rotation slice 3 (ADR-085): the decorative `sdk_keys` admin routes
//! are gone. AC-AKR-16.

#[path = "../common/mod.rs"]
mod common;
use common::AdminTestContext;

const PROJECT: &str = "test-project-seeded-for-account";

async fn owner_cookie(ctx: &AdminTestContext) -> String {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email": ctx.user_email,
            "password": ctx.user_password,
            "totp_code": ctx.totp_code_now(),
        }))
        .send()
        .await
        .expect("signin request failed");
    resp.headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(|s| s.trim().to_string())
        .expect("no cookie in signin response")
}

/// AC-AKR-16: GET/POST/DELETE on the removed routes answer 404, authenticated or not.
#[tokio::test]
async fn removed_sdk_keys_routes_return_404() {
    let ctx = AdminTestContext::new().await;
    let cookie = owner_cookie(&ctx).await;
    let list = format!("/admin/v1/projects/{PROJECT}/sdk_keys");
    let one = format!("{list}/00000000-0000-0000-0000-000000000000");

    for cookie in [Some(cookie.as_str()), None] {
        for (method, path) in [
            (reqwest::Method::GET, &list),
            (reqwest::Method::POST, &list),
            (reqwest::Method::DELETE, &one),
        ] {
            let mut req = ctx.client.request(method.clone(), ctx.url(path));
            if let Some(c) = cookie {
                req = req.header("Cookie", c);
            }
            let resp = req
                .json(&serde_json::json!({"name": "k"}))
                .send()
                .await
                .expect("request failed");
            assert_eq!(
                resp.status().as_u16(),
                404,
                "AC-AKR-16: {method} {path} (cookie={}) must be 404",
                cookie.is_some()
            );
        }
    }
}

/// AC-AKR-16: `delete_project` no longer touches `sdk_api_keys`; the table and
/// its rows are retained (unchanged) for rollback.
#[tokio::test]
async fn project_delete_leaves_sdk_api_keys_rows_untouched() {
    let ctx = AdminTestContext::new().await;
    sqlx::query(
        "INSERT INTO sdk_api_keys (project_id, name, key_hash, prefix) \
         VALUES ($1, 'legacy', 'legacy-hash-bytes', 'embyr_sdk_')",
    )
    .bind(PROJECT)
    .execute(&ctx.pool)
    .await
    .expect("seed sdk_api_keys row");

    let del = ctx
        .client
        .delete(ctx.url(&format!("/admin/v1/projects/{PROJECT}")))
        .header("Authorization", "Bearer test-admin-key-from-env")
        .send()
        .await
        .expect("DELETE project failed");
    assert_eq!(del.status().as_u16(), 200, "operator delete must succeed");

    let revoked: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL FROM sdk_api_keys WHERE project_id = $1",
    )
    .bind(PROJECT)
    .fetch_one(&ctx.pool)
    .await
    .expect("sdk_api_keys row must survive project deletion");
    assert!(!revoked, "AC-AKR-16: delete_project must not cascade-revoke sdk_api_keys");
}
