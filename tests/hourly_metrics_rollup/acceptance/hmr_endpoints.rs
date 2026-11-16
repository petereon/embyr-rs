//! hourly-metrics-rollup (ADR-084) slice 4 — admin endpoints.
//!
//! AC-HMR-02/03 (percentiles / null without samples), AC-HMR-04 (zero-filled
//! hourly series), AC-HMR-09 (404 foreign project, 422 bad window, any role),
//! AC-HMR-10 (`GET /admin/v1/metrics/summary`), AC-HMR-11 (counters keep
//! their daily_project_metrics source).
//!
//! Rollup rows are seeded directly as the read model's precondition; the
//! write path that produces them is covered in hmr_write_path.rs.

#[path = "../../admin_api_v2/common/mod.rs"]
mod common;
use common::AdminTestContext;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use embyr_core::metrics_rollup::truncate_to_hour;
use serde_json::Value;

const PROJECT: &str = "test-project-seeded-for-account";

async fn sign_in(ctx: &AdminTestContext, viewer: bool) -> String {
    let (email, password, totp) = if viewer {
        (&ctx.viewer_email, &ctx.viewer_password, ctx.viewer_totp_code_now())
    } else {
        (&ctx.user_email, &ctx.user_password, ctx.totp_code_now())
    };
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({ "email": email, "password": password, "totp_code": totp }))
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

fn hours_ago(n: i64) -> DateTime<Utc> {
    truncate_to_hour(Utc::now()) - Duration::hours(n)
}

/// Seed one rollup row; `buckets` are (index, count) pairs, requests = their sum.
async fn seed(
    ctx: &AdminTestContext,
    project: &str,
    hour: DateTime<Utc>,
    op: i16,
    units: i64,
    errors: i64,
    buckets: &[(usize, i64)],
) {
    let mut b = [0i64; 13];
    for (i, n) in buckets {
        b[*i] = *n;
    }
    let requests: i64 = b.iter().sum();
    sqlx::query(
        "INSERT INTO hourly_project_metrics \
         (project_id, hour, op, requests, units, errors, \
          b00, b01, b02, b03, b04, b05, b06, b07, b08, b09, b10, b11, b12) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)",
    )
    .bind(project)
    .bind(hour)
    .bind(op)
    .bind(requests)
    .bind(units)
    .bind(errors)
    .bind(b[0]).bind(b[1]).bind(b[2]).bind(b[3]).bind(b[4]).bind(b[5]).bind(b[6])
    .bind(b[7]).bind(b[8]).bind(b[9]).bind(b[10]).bind(b[11]).bind(b[12])
    .execute(&ctx.pool)
    .await
    .expect("seed rollup row");
}

async fn get(ctx: &AdminTestContext, cookie: &str, path: &str) -> (u16, Value) {
    let resp = ctx
        .client
        .get(ctx.url(path))
        .header("Cookie", cookie)
        .send()
        .await
        .expect("GET failed");
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap_or(Value::Null))
}

/// Latencies {3, 3, 8, 40, 900} ms land in buckets 2, 2, 3, 5, 9.
async fn seed_known_latencies(ctx: &AdminTestContext, hour: DateTime<Utc>) {
    seed(ctx, PROJECT, hour, 0, 5, 1, &[(2, 2), (3, 1), (5, 1), (9, 1)]).await;
}

/// AC-HMR-02, AC-HMR-03, AC-HMR-04
// @AC-HMR-02 @AC-HMR-03 @AC-HMR-04 @driving_port @real-io
#[tokio::test]
async fn metrics_report_percentiles_null_without_samples_and_zero_filled_series() {
    let ctx = AdminTestContext::new().await;
    let cookie = sign_in(&ctx, false).await;
    seed_known_latencies(&ctx, hours_ago(0)).await;
    seed(&ctx, PROJECT, hours_ago(3), 1, 4, 0, &[(4, 4)]).await;
    seed(&ctx, PROJECT, hours_ago(30), 0, 7, 2, &[(1, 7)]).await; // 7d window only

    let before = truncate_to_hour(Utc::now());
    let (status, body) = get(&ctx, &cookie, &format!("/admin/v1/projects/{PROJECT}/metrics")).await;
    let after = truncate_to_hour(Utc::now());
    assert_eq!(status, 200);

    // AC-HMR-02: percentiles within their buckets, samples counted.
    let read = &body["latency"]["read"];
    assert_eq!(read["samples"], 5);
    let p50 = read["p50_ms"].as_f64().expect("p50 present");
    let p95 = read["p95_ms"].as_f64().expect("p95 present");
    assert!((5.0..=10.0).contains(&p50), "p50 = {p50} (exact median 8 ms)");
    assert!((500.0..=1000.0).contains(&p95), "p95 = {p95}");
    assert_eq!(body["p95_read_ms"].as_f64(), Some(p95));

    // AC-HMR-03: write side has samples from hours_ago(3) -> non-null; a project
    // window without any write sample would be null (never 0.0); read side of an
    // empty project is covered by the summary test below.
    assert_eq!(body["latency"]["write"]["samples"], 4);

    // AC-HMR-04: 24 ascending hourly points ending at the current hour.
    let series = body["series"].as_array().expect("series array");
    assert_eq!(series.len(), 24);
    let hours: Vec<&str> = series.iter().map(|p| p["hour"].as_str().unwrap()).collect();
    assert!(hours.windows(2).all(|w| w[0] < w[1]));
    let last = hours[23].to_string();
    assert!(
        last == before.to_rfc3339_opts(SecondsFormat::Secs, true)
            || last == after.to_rfc3339_opts(SecondsFormat::Secs, true)
    );
    let total_reads: i64 = series.iter().map(|p| p["reads"].as_i64().unwrap()).sum();
    let total_writes: i64 = series.iter().map(|p| p["writes"].as_i64().unwrap()).sum();
    assert_eq!((total_reads, total_writes), (5, 4), "30h-old row is outside 24h");
    assert_eq!(body["sparkline"].as_array().expect("sparkline").len(), 24);

    let (status, body) =
        get(&ctx, &cookie, &format!("/admin/v1/projects/{PROJECT}/metrics?window=7d")).await;
    assert_eq!(status, 200);
    let series = body["series"].as_array().expect("series array");
    assert_eq!(series.len(), 168);
    let total_reads: i64 = series.iter().map(|p| p["reads"].as_i64().unwrap()).sum();
    assert_eq!(total_reads, 5 + 7);
    assert_eq!(body["latency"]["read"]["samples"], 12);
}

/// AC-HMR-03 on the project endpoint: no rollup rows at all.
// @AC-HMR-03 @driving_port @real-io
#[tokio::test]
async fn metrics_without_samples_have_null_percentiles_and_zero_p95() {
    let ctx = AdminTestContext::new().await;
    let cookie = sign_in(&ctx, false).await;

    let (status, body) = get(&ctx, &cookie, &format!("/admin/v1/projects/{PROJECT}/metrics")).await;

    assert_eq!(status, 200);
    for side in ["read", "write"] {
        let l = &body["latency"][side];
        assert!(l["p50_ms"].is_null() && l["p95_ms"].is_null() && l["p99_ms"].is_null());
        assert_eq!(l["samples"], 0);
    }
    assert_eq!(body["p95_read_ms"].as_f64(), Some(0.0));
}

/// AC-HMR-09
// @AC-HMR-09 @driving_port @real-io
#[tokio::test]
async fn foreign_project_is_404_bad_window_is_422_any_role_may_read() {
    let ctx = AdminTestContext::new().await;
    let owner = sign_in(&ctx, false).await;
    let viewer = sign_in(&ctx, true).await;

    let foreign = "/admin/v1/projects/project-belonging-to-other-account/metrics";
    assert_eq!(get(&ctx, &owner, foreign).await.0, 404);
    let missing = "/admin/v1/projects/no-such-project/metrics";
    assert_eq!(get(&ctx, &owner, missing).await.0, 404);

    let own = format!("/admin/v1/projects/{PROJECT}/metrics");
    for bad in ["30d", "1h", ""] {
        assert_eq!(get(&ctx, &owner, &format!("{own}?window={bad}")).await.0, 422, "window={bad}");
    }
    assert_eq!(get(&ctx, &viewer, &own).await.0, 200, "viewer may read");
}

/// AC-HMR-10: one call lists every non-deleted project of the caller's account.
// @AC-HMR-10 @driving_port @real-io
#[tokio::test]
async fn summary_lists_account_projects_with_counters_and_p95() {
    let ctx = AdminTestContext::new().await;
    let cookie = sign_in(&ctx, false).await;
    seed_known_latencies(&ctx, hours_ago(1)).await;
    seed(&ctx, "test-project-deleted", hours_ago(1), 0, 1, 0, &[(0, 1)]).await;
    seed(&ctx, "project-belonging-to-other-account", hours_ago(1), 0, 1, 0, &[(0, 1)]).await;
    sqlx::query(
        "INSERT INTO daily_project_metrics (project_id, date, read_ops, write_ops, delete_ops) \
         VALUES ($1, CURRENT_DATE, 11, 5, 2)",
    )
    .bind(PROJECT)
    .execute(&ctx.pool)
    .await
    .expect("seed daily metrics");
    sqlx::query(
        "INSERT INTO projects (id, account_id, backend_mode, api_key_hash_current, status, name) \
         VALUES ('hmr-quiet', $1, 'direct_pg', 'h', 'active', 'Quiet')",
    )
    .bind(uuid::Uuid::parse_str(&ctx.account_id).expect("account uuid"))
    .execute(&ctx.pool)
    .await
    .expect("seed quiet project");

    let (status, body) = get(&ctx, &cookie, "/admin/v1/metrics/summary").await;

    assert_eq!(status, 200);
    let entries = body.as_array().expect("array");
    let ids: Vec<&str> = entries.iter().map(|e| e["project_id"].as_str().unwrap()).collect();
    assert!(ids.contains(&PROJECT) && ids.contains(&"hmr-quiet"));
    assert!(!ids.contains(&"test-project-deleted"), "deleted projects are excluded");
    assert!(!ids.contains(&"project-belonging-to-other-account"), "foreign projects are excluded");

    let busy = entries.iter().find(|e| e["project_id"] == PROJECT).unwrap();
    assert_eq!(busy["name"], "Test Project");
    assert_eq!(
        (busy["reads_today"].as_i64(), busy["writes_today"].as_i64(), busy["deletes_today"].as_i64()),
        (Some(11), Some(5), Some(2))
    );
    let p95 = busy["p95_read_ms"].as_f64().expect("p95 read present");
    assert!((500.0..=1000.0).contains(&p95));
    assert!(busy["p95_write_ms"].is_null());

    let quiet = entries.iter().find(|e| e["project_id"] == "hmr-quiet").unwrap();
    assert!(quiet["p95_read_ms"].is_null() && quiet["p95_write_ms"].is_null());
    assert_eq!(quiet["reads_today"], 0);
}

/// AC-HMR-11: the `*_today` counters keep coming from daily_project_metrics,
/// whatever the hourly rollup holds.
// @AC-HMR-11 @driving_port @real-io
#[tokio::test]
async fn today_counters_still_come_from_daily_metrics() {
    let ctx = AdminTestContext::new().await;
    let cookie = sign_in(&ctx, false).await;
    sqlx::query(
        "INSERT INTO daily_project_metrics (project_id, date, read_ops, write_ops, delete_ops) \
         VALUES ($1, CURRENT_DATE, 7, 3, 1)",
    )
    .bind(PROJECT)
    .execute(&ctx.pool)
    .await
    .expect("seed daily metrics");
    seed(&ctx, PROJECT, hours_ago(0), 0, 99, 0, &[(2, 99)]).await;

    let (status, body) = get(&ctx, &cookie, &format!("/admin/v1/projects/{PROJECT}/metrics")).await;

    assert_eq!(status, 200);
    assert_eq!(
        (body["reads_today"].as_i64(), body["writes_today"].as_i64(), body["deletes_today"].as_i64()),
        (Some(7), Some(3), Some(1))
    );
}
