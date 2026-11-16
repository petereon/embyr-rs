// @driving_port @real-io
//! Security finding X8 — the admin sign-in throttle keys on the TCP peer IP,
//! so behind a load balancer every admin shares one bucket. With
//! `EMBYR_TRUSTED_PROXY_HOPS=1` the client is the last `X-Forwarded-For`
//! entry (the one the trusted LB appended); spoofed left entries are ignored.
//!
//! Burst is 2 with near-zero refill, so no wall-clock window is involved.

#[path = "../common/mod.rs"]
mod common;
use common::AdminTestContext;
use embyr_server::middleware::signin_rate_limit::SigninRateLimiter;

async fn signin_status(ctx: &AdminTestContext, forwarded_for: Option<&str>) -> u16 {
    let mut req = ctx.client.post(ctx.url("/admin/v1/auth/signin")).json(&serde_json::json!({
        "email": "nobody@example.com",
        "password": "wrong-password-xyz",
    }));
    if let Some(xff) = forwarded_for {
        req = req.header("X-Forwarded-For", xff);
    }
    req.send().await.expect("signin request failed").status().as_u16()
}

#[tokio::test]
async fn trusted_proxy_hops_keys_the_throttle_on_the_forwarded_client_not_the_load_balancer() {
    let limiter = SigninRateLimiter::new(2.0, 1.0 / 3600.0).with_trusted_proxy_hops(1);
    let ctx = AdminTestContext::with_signin_limiter(limiter).await;

    // Client A (LB-appended entry 203.0.113.1) spends its 2-token burst; a
    // different spoofed leftmost entry each time must not buy a fresh bucket.
    assert_eq!(signin_status(&ctx, Some("6.6.6.6, 203.0.113.1")).await, 401);
    assert_eq!(signin_status(&ctx, Some("7.7.7.7, 203.0.113.1")).await, 401);
    assert_eq!(
        signin_status(&ctx, Some("8.8.8.8, 203.0.113.1")).await,
        429,
        "spoofing the leftmost X-Forwarded-For entry must not evade the throttle"
    );

    // Client B behind the same LB (same TCP peer) is unaffected by A's lockout.
    assert_eq!(
        signin_status(&ctx, Some("6.6.6.6, 203.0.113.2")).await,
        401,
        "one attacker must not lock out every admin behind the load balancer"
    );

    // No header -> falls back to the peer IP, a bucket of its own.
    assert_eq!(signin_status(&ctx, None).await, 401);
}

#[tokio::test]
async fn default_hops_zero_ignores_forwarded_for_and_keys_on_the_peer() {
    let ctx = AdminTestContext::with_signin_limiter(SigninRateLimiter::new(2.0, 1.0 / 3600.0)).await;

    assert_eq!(signin_status(&ctx, Some("203.0.113.1")).await, 401);
    assert_eq!(signin_status(&ctx, Some("203.0.113.2")).await, 401);
    assert_eq!(
        signin_status(&ctx, Some("203.0.113.3")).await,
        429,
        "with hops=0 the header is untrusted: all requests share the peer's bucket"
    );
}
