// @driving_port @real-io
//! admin-oidc-sso slice 3, AC-SSO-06/07: the outbound policy (SSRF) with `EMBYR_ALLOW_PRIVATE_DSN_HOSTS`
//! OFF. The flag is process-wide, so these live in their own test binary where nothing turns it on
//! (`s03_start_and_callback.rs` needs it ON for the loopback mock IdP).
//!
//! CI only (testcontainers Postgres). Compile locally with
//! `cargo test -p embyr-server --test admin_oidc_sso_s03_ssrf_policy --no-run`.

#[path = "../../admin_api_v2/common/mod.rs"]
mod common;
#[path = "../../common/mock_oidc_idp.rs"]
mod mock_oidc_idp;

use common::{totp_code_for_secret, AdminTestContext};
use mock_oidc_idp::MockIdp;
use serde_json::{json, Value};
use uuid::Uuid;

async fn owner_cookie(ctx: &AdminTestContext) -> String {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&json!({"email": ctx.user_email, "password": ctx.user_password,
                      "totp_code": totp_code_for_secret(&ctx.totp_secret_b32)}))
        .send()
        .await
        .unwrap();
    resp.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_string()
}

async fn create(ctx: &AdminTestContext, cookie: &str, issuer: &str) -> (u16, Value) {
    let r = ctx
        .client
        .post(ctx.url("/admin/v1/oidc_providers"))
        .header("Cookie", cookie)
        .json(&json!({"issuer": issuer, "client_id": "c", "client_secret": "s"}))
        .send()
        .await
        .unwrap();
    (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn provider_creation_refuses_private_and_non_https_issuers_without_connecting() {
    std::env::remove_var("EMBYR_ALLOW_PRIVATE_DSN_HOSTS");
    let ctx = AdminTestContext::new().await;
    let idp = MockIdp::start().await;
    let cookie = owner_cookie(&ctx).await;
    let port = idp.base_url.rsplit(':').next().unwrap().to_string();

    for (issuer, reason) in [
        (idp.issuer(), "url_not_https"), // plain http to loopback
        (format!("https://127.0.0.1:{port}"), "url_forbidden_address"),
        ("https://169.254.169.254".to_string(), "url_forbidden_address"), // cloud metadata
        ("https://[::1]".to_string(), "url_forbidden_address"),
        ("https://10.0.0.5".to_string(), "url_forbidden_address"),
        ("https://localhost".to_string(), "url_forbidden_address"), // resolves to loopback
        ("https://user:pw@idp.example.com".to_string(), "url_userinfo"),
        ("ftp://idp.example.com".to_string(), "url_not_https"),
    ] {
        let (status, body) = create(&ctx, &cookie, &issuer).await;
        assert_eq!((status, body["error"].as_str(), body["reason"].as_str()), (422, Some("issuer_invalid"), Some(reason)), "{issuer}");
    }
    assert_eq!(idp.hits("discovery"), 0, "the mock IdP was never contacted");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM oidc_providers").fetch_one(&ctx.pool).await.unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn start_never_connects_to_a_provider_whose_issuer_is_private() {
    std::env::remove_var("EMBYR_ALLOW_PRIVATE_DSN_HOSTS");
    // https origin so `start` reaches discovery; nothing is ever served from it
    let ctx = AdminTestContext::with_public_base_url("https://console.example.test").await;
    let idp = MockIdp::start().await;
    let port = idp.base_url.rsplit(':').next().unwrap().to_string();
    let account = Uuid::parse_str(&ctx.account_id).unwrap();

    for issuer in [format!("https://127.0.0.1:{port}"), idp.issuer(), "https://169.254.169.254".to_string()] {
        let provider: Uuid = sqlx::query_scalar(
            "INSERT INTO oidc_providers (account_id, issuer, client_id, client_secret_enc) \
             VALUES ($1, $2, 'c', '\\x00'::bytea) RETURNING id",
        )
        .bind(account)
        .bind(&issuer)
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
        let r = ctx
            .client
            .post(ctx.url("/admin/v1/auth/oidc/start"))
            .json(&json!({ "provider_id": provider }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "{issuer}");
        assert_eq!(r.json::<Value>().await.unwrap(), json!({"error": "sso_unavailable"}), "{issuer}");
    }
    assert_eq!(idp.hits("discovery"), 0);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM sso_auth_requests").fetch_one(&ctx.pool).await.unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn an_http_public_base_url_is_unavailable_without_the_dev_flag() {
    std::env::remove_var("EMBYR_ALLOW_PRIVATE_DSN_HOSTS");
    let ctx = AdminTestContext::with_public_base_url("http://console.example.test").await;
    let provider: Uuid = sqlx::query_scalar(
        "INSERT INTO oidc_providers (account_id, issuer, client_id, client_secret_enc) \
         VALUES ($1, 'https://idp.example.com', 'c', '\\x00'::bytea) RETURNING id",
    )
    .bind(Uuid::parse_str(&ctx.account_id).unwrap())
    .fetch_one(&ctx.pool)
    .await
    .unwrap();
    let r = ctx
        .client
        .post(ctx.url("/admin/v1/auth/oidc/start"))
        .json(&json!({ "provider_id": provider }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"error": "sso_unavailable"}));
}
