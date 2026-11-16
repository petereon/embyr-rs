// @driving_port @real-io
//! account-deletion — purge cycle. feature-delta.md AC-ADL-07, AC-ADL-08.
//! Driving port: `soft_delete_purge_sweeper::run_account_cycle` with a backdated
//! `accounts.deleted_at`. CI only (testcontainers Postgres).

use std::sync::Arc;

use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ImageExt},
};
use uuid::Uuid;

use embyr_server::adapters::system_db::SystemDb;
use embyr_server::sweepers::soft_delete_purge_sweeper::run_account_cycle;

const GRACE_DAYS: i64 = 7;

/// Seed an account with one of everything the purge must remove, plus a project
/// (which must survive). `deleted_days_ago = None` leaves the account active.
async fn seed_account(pool: &sqlx::PgPool, name: &str, email: &str, deleted_days_ago: Option<f64>) -> Uuid {
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (name, deleted_at) \
         VALUES ($1, CASE WHEN $2::float8 IS NULL THEN NULL \
                     ELSE now() - make_interval(secs => $2::float8 * 86400) END) \
         RETURNING id",
    )
    .bind(name)
    .bind(deleted_days_ago)
    .fetch_one(pool)
    .await
    .expect("account");
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (account_id, email, display_name, password_hash) \
         VALUES ($1, $2, 'U', 'x') RETURNING id",
    )
    .bind(account_id)
    .bind(email)
    .fetch_one(pool)
    .await
    .expect("user");
    let sa_id: Uuid = sqlx::query_scalar(
        "INSERT INTO service_accounts (account_id, name, role) VALUES ($1, 'sa', 'Viewer') RETURNING id",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .expect("service account");
    // $1 = account id, $2 = user id.
    for sql in [
        "INSERT INTO account_members (user_id, account_id, role) VALUES ($2, $1, 'Owner')",
        "INSERT INTO sessions (user_id, account_id, token_hash, expires_at) \
         VALUES ($2, $1, decode(md5(random()::text), 'hex'), now())",
        "INSERT INTO admin_api_keys (account_id, name, key_hash, prefix, role, member_id) \
         VALUES ($1, 'k', decode(md5(random()::text), 'hex'), 'p', 'Owner', $2)",
        "INSERT INTO invitations (account_id, email, role, expires_at, invited_by) \
         VALUES ($1, 'i-' || $1::text || '@example.com', 'Viewer', now(), $2)",
    ] {
        sqlx::query(sql).bind(account_id).bind(user_id).execute(pool).await.expect(sql);
    }
    for sql in [
        "INSERT INTO oidc_providers (account_id, issuer, client_id, client_secret_enc) \
         VALUES ($1, 'https://issuer', 'c', '\\x01')",
        "INSERT INTO subscriptions (account_id, plan, status) VALUES ($1, 'free', 'active')",
        "INSERT INTO mfa_recovery_codes (user_id, code_hash) \
         SELECT id, '\\x01' FROM users WHERE account_id = $1",
    ] {
        sqlx::query(sql).bind(account_id).execute(pool).await.expect(sql);
    }
    sqlx::query("UPDATE admin_api_keys SET service_account_id = $2 WHERE account_id = $1")
        .bind(account_id)
        .bind(sa_id)
        .execute(pool)
        .await
        .expect("link key to service account");
    sqlx::query(
        "INSERT INTO projects (id, account_id, backend_mode, api_key_hash_current, status, name) \
         VALUES ($1, $2, 'direct_pg', 'h', 'deleted', 'p')",
    )
    .bind(format!("project-of-{account_id}"))
    .bind(account_id)
    .execute(pool)
    .await
    .expect("project");
    account_id
}

async fn dependents(pool: &sqlx::PgPool, account_id: Uuid) -> i64 {
    let mut total = 0;
    for sql in [
        "SELECT count(*) FROM users WHERE account_id = $1",
        "SELECT count(*) FROM account_members WHERE account_id = $1",
        "SELECT count(*) FROM mfa_recovery_codes WHERE user_id IN (SELECT id FROM users WHERE account_id = $1)",
        "SELECT count(*) FROM sessions WHERE account_id = $1",
        "SELECT count(*) FROM service_accounts WHERE account_id = $1",
        "SELECT count(*) FROM admin_api_keys WHERE account_id = $1",
        "SELECT count(*) FROM invitations WHERE account_id = $1",
        "SELECT count(*) FROM oidc_providers WHERE account_id = $1",
        "SELECT count(*) FROM subscriptions WHERE account_id = $1",
    ] {
        let n: i64 = sqlx::query_scalar(sql).bind(account_id).fetch_one(pool).await.expect(sql);
        total += n;
    }
    total
}

async fn name_of(pool: &sqlx::PgPool, account_id: Uuid) -> String {
    sqlx::query_scalar("SELECT name FROM accounts WHERE id = $1")
        .bind(account_id)
        .fetch_one(pool)
        .await
        .expect("account row must still exist (tombstone)")
}

#[tokio::test]
async fn purge_tombstones_accounts_past_grace_and_leaves_the_rest_untouched() {
    let container = Postgres::default().with_tag("15-alpine").start().await.expect("postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let db = Arc::new(
        SystemDb::new(&format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres"))
            .await
            .expect("SystemDb"),
    );
    db.migrate().await.expect("migrate");
    let pool = db.pool();

    let past = seed_account(pool, "Past Grace", "past@example.com", Some(8.0)).await;
    let within = seed_account(pool, "Within Grace", "within@example.com", Some(1.0)).await;
    let live = seed_account(pool, "Live", "live@example.com", None).await;

    run_account_cycle(&db, GRACE_DAYS).await;

    // AC-ADL-07: dependents gone, name blanked, tombstone row + project FK target kept.
    assert_eq!(dependents(pool, past).await, 0);
    assert_eq!(name_of(pool, past).await, "");
    let projects: i64 = sqlx::query_scalar("SELECT count(*) FROM projects WHERE account_id = $1")
        .bind(past)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(projects, 1);
    // ...and the emails are reusable.
    sqlx::query("INSERT INTO users (account_id, email, display_name, password_hash) VALUES ($1, 'past@example.com', 'N', 'x')")
        .bind(live)
        .execute(pool)
        .await
        .expect("email of a purged user must be reusable");

    // AC-ADL-08: inside the window / never deleted: nothing changes.
    assert_eq!(dependents(pool, within).await, 9);
    assert_eq!(name_of(pool, within).await, "Within Grace");
    assert_eq!(dependents(pool, live).await, 9 + 1); // +1: the reused-email user
    assert_eq!(name_of(pool, live).await, "Live");

    // Idempotent: a second cycle is a no-op.
    run_account_cycle(&db, GRACE_DAYS).await;
    assert_eq!(dependents(pool, past).await, 0);
    assert_eq!(name_of(pool, within).await, "Within Grace");
}
