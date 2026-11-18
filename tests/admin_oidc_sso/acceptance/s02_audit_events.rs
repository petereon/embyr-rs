// @driving_port @real-io
//! admin-oidc-sso slice 2 (ADR-086 decision 9, O5): the `admin_audit_events` writer and its
//! 12-month retention. CI only (testcontainers Postgres): compile locally with
//! `cargo test -p embyr-server --test admin_oidc_sso_s02_audit_events --no-run`.

use std::net::IpAddr;
use std::sync::Arc;

use embyr_core::admin::audit::{events, AuditEvent, AuditOutcome};
use embyr_server::adapters::admin_audit::{write_audit_event, AuditError};
use embyr_server::adapters::system_db::SystemDb;
use embyr_server::sweepers::signin_rate_limit_sweeper;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};
use uuid::Uuid;

async fn start_system_db() -> (ContainerAsync<Postgres>, Arc<SystemDb>) {
    let container = Postgres::default().with_tag("15-alpine").start().await.expect("start Postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("host port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let db = Arc::new(SystemDb::new(&url).await.expect("SystemDb::new"));
    db.migrate().await.expect("migrations");
    (container, db)
}

async fn rows(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM admin_audit_events").fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn writer_stores_every_field_of_a_labelled_event() {
    let (_c, db) = start_system_db().await;
    let (account, actor, provider) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let ip: IpAddr = "203.0.113.7".parse().unwrap();
    let mut ev = AuditEvent::new(events::SSO_SIGNIN, AuditOutcome::Failed);
    ev.account_id = Some(account);
    ev.actor_user_id = Some(actor);
    ev.provider_id = Some(provider);
    ev.reason = Some("token_expired");
    ev.source_ip = Some(ip);
    write_audit_event(db.pool(), &ev).await.expect("write");

    let row: (String, String, Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT event, outcome, account_id, actor_user_id, provider_id, reason, host(source_ip) FROM admin_audit_events",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            "sso_signin".into(), "failed".into(), Some(account), Some(actor), Some(provider),
            Some("token_expired".into()), Some("203.0.113.7".into())
        )
    );
}

#[tokio::test]
async fn writer_accepts_an_event_with_no_ids_and_an_ipv6_source() {
    let (_c, db) = start_system_db().await;
    let mut ev = AuditEvent::new(events::SSO_PROVIDER_CREATED, AuditOutcome::Ok);
    ev.source_ip = Some("2001:db8::1".parse().unwrap());
    write_audit_event(db.pool(), &ev).await.expect("write");
    let (outcome, account, reason): (String, Option<Uuid>, Option<String>) =
        sqlx::query_as("SELECT outcome, account_id, reason FROM admin_audit_events")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!((outcome.as_str(), account, reason), ("ok", None, None));
}

#[tokio::test]
async fn writer_refuses_free_text_so_no_claim_or_token_can_be_stored() {
    let (_c, db) = start_system_db().await;
    for bad_reason in ["user@example.com mismatched", "eyJhbGciOiJSUzI1NiJ9.e30.sig", "Bearer abc"] {
        let mut ev = AuditEvent::new(events::SSO_SIGNIN, AuditOutcome::Failed);
        ev.reason = Some(bad_reason);
        assert!(matches!(write_audit_event(db.pool(), &ev).await, Err(AuditError::InvalidLabel)), "{bad_reason}");
    }
    let bad_event = AuditEvent::new("Not A Label", AuditOutcome::Ok);
    assert!(matches!(write_audit_event(db.pool(), &bad_event).await, Err(AuditError::InvalidLabel)));
    assert_eq!(rows(db.pool()).await, 0, "nothing written");
}

#[tokio::test]
async fn writer_works_inside_a_transaction_and_rolls_back_with_it() {
    let (_c, db) = start_system_db().await;
    let mut tx = db.pool().begin().await.unwrap();
    write_audit_event(&mut *tx, &AuditEvent::new(events::SSO_LINK_CREATED, AuditOutcome::Ok)).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(rows(db.pool()).await, 0);
}

#[tokio::test]
async fn the_database_rejects_an_unknown_outcome() {
    let (_c, db) = start_system_db().await;
    let bad = sqlx::query("INSERT INTO admin_audit_events (event, outcome) VALUES ('x', 'maybe')")
        .execute(db.pool())
        .await;
    assert!(bad.is_err());
}

#[tokio::test]
async fn sweeper_purges_audit_rows_older_than_twelve_months_and_keeps_the_rest() {
    let (_c, db) = start_system_db().await;
    for (event, age) in [
        ("old_13_months", "13 months"),
        ("old_5_years", "5 years"),
        ("edge_11_months", "11 months"),
        ("fresh", "1 minute"),
    ] {
        sqlx::query("INSERT INTO admin_audit_events (at, event, outcome) VALUES (now() - $1::interval, $2, 'ok')")
            .bind(age)
            .bind(event)
            .execute(db.pool())
            .await
            .unwrap();
    }
    signin_rate_limit_sweeper::run_cycle(&db).await;
    let mut left: Vec<String> = sqlx::query_scalar("SELECT event FROM admin_audit_events")
        .fetch_all(db.pool())
        .await
        .unwrap();
    left.sort();
    assert_eq!(left, vec!["edge_11_months".to_string(), "fresh".to_string()]);
}
