//! AC-DSS-15: `embyr_projects_unconverted_connection_secrets` counts non-deleted direct_pg/agent
//! rows still lacking a server-store secret. Own test binary: the gauge is process-global, so a
//! sibling test scraping concurrently would race it.

#[path = "../common/mod.rs"]
mod common;
use common::*;

fn gauge_value(body: &str) -> f64 {
    body.lines()
        .find(|l| l.starts_with("embyr_projects_unconverted_connection_secrets"))
        .and_then(|l| l.split_whitespace().last())
        .and_then(|v| v.parse().ok())
        .expect("gauge line present in /metrics")
}

#[tokio::test]
async fn gauge_counts_only_live_rows_missing_a_server_store_secret() {
    let env = start_system_db().await;
    let ins = |id: &'static str, status: &'static str, mode: &'static str, dsn: bool, bundle: bool| {
        let pool = env.sys_pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO projects (id, status, backend_mode, api_key_hash_current, \
                 backend_pg_dsn_enc, agent_tls_bundle_srv_enc) VALUES ($1, $2, $3, 'h', $4, $5)",
            )
            .bind(id)
            .bind(status)
            .bind(mode)
            .bind(dsn.then(|| vec![1u8; 20]))
            .bind(bundle.then(|| vec![1u8; 20]))
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    ins("g-direct-unconverted", "active", "direct_pg", false, false).await; // counts
    ins("g-direct-suspended", "suspended", "direct_pg", false, false).await; // counts (not deleted)
    ins("g-agent-unconverted", "active", "agent", false, false).await; // counts
    ins("g-direct-converted", "active", "direct_pg", true, false).await;
    ins("g-agent-converted", "active", "agent", false, true).await;
    ins("g-direct-deleted", "deleted", "direct_pg", false, false).await;
    ins("g-aws", "active", "aws_secret", false, false).await;

    let server = server_for(&env).await;
    assert_eq!(gauge_value(&scrape_metrics(&server).await), 3.0);

    sqlx::query("UPDATE projects SET backend_pg_dsn_enc = $1 WHERE id = 'g-direct-unconverted'")
        .bind(vec![2u8; 20])
        .execute(&env.sys_pool)
        .await
        .unwrap();
    assert_eq!(gauge_value(&scrape_metrics(&server).await), 2.0);
}
