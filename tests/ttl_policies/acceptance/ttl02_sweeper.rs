//! TTL02 — the TTL sweeper against a real customer database (go-live P5).
//!
//! Driving port: `ttl_sweeper::run_cycle` (same production code the interval
//! loop runs; the interval/lock wrapper is the same shape as the sibling
//! sweepers). Docker-backed; compile-only locally, runs in CI.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{insert_doc, live_doc_ids, put_policy, sweep_config, ts_fields, SecurityRulesFullContext};

use std::time::Duration;

use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    listen_request, listen_response,
    structured_query::CollectionSelector,
    target::{self, query_target},
    target_change::TargetChangeType,
    ListenRequest, ListenResponse, StructuredQuery, Target,
};
use embyr_server::sweepers::ttl_sweeper;
use tokio_stream::StreamExt;

async fn enable_policy(ctx: &SecurityRulesFullContext, group: &str, field: &str, enabled: bool) {
    sqlx::query(
        "INSERT INTO ttl_policies (project_id, collection_group, field_path, enabled) VALUES ($1, $2, $3, $4)",
    )
    .bind(&ctx.project_id)
    .bind(group)
    .bind(field)
    .bind(enabled)
    .execute(&ctx.sys_pool)
    .await
    .expect("insert policy");
}

/// Expired documents go; future timestamps, missing/null/non-timestamp
/// fields and other collections stay.
///
/// @driving_port @real-io
#[tokio::test]
async fn only_documents_with_an_expired_timestamp_are_deleted() {
    let ctx = SecurityRulesFullContext::new("ttl02-eligibility").await;
    enable_policy(&ctx, "sessions", "exp", true).await;

    insert_doc(&ctx, "sessions", "expired", ts_fields("exp", -3600)).await;
    insert_doc(&ctx, "sessions", "future", ts_fields("exp", 3600)).await;
    insert_doc(&ctx, "sessions", "missing", serde_json::json!({})).await;
    insert_doc(&ctx, "sessions", "null", serde_json::json!({"exp": {"t": "N"}})).await;
    insert_doc(&ctx, "sessions", "string", serde_json::json!({"exp": {"t": "S", "v": "2001-01-01"}})).await;
    insert_doc(&ctx, "sessions", "integer", serde_json::json!({"exp": {"t": "I", "v": 1}})).await;
    insert_doc(&ctx, "sessions", "other-field", ts_fields("other", -3600)).await;
    insert_doc(&ctx, "other_collection", "expired", ts_fields("exp", -3600)).await;

    let deleted = ttl_sweeper::run_cycle(&ctx.sys_pool, &sweep_config()).await;

    assert_eq!(deleted, 1);
    assert_eq!(
        live_doc_ids(&ctx, "sessions").await,
        vec!["future", "integer", "missing", "null", "other-field", "string"]
    );
    assert_eq!(live_doc_ids(&ctx, "other_collection").await, vec!["expired"]);
}

/// A disabled policy deletes nothing; so does a suspended project.
///
/// @driving_port @real-io
#[tokio::test]
async fn disabled_policies_and_suspended_projects_are_skipped() {
    let ctx = SecurityRulesFullContext::new("ttl02-skipped").await;
    enable_policy(&ctx, "sessions", "exp", false).await;
    insert_doc(&ctx, "sessions", "expired", ts_fields("exp", -3600)).await;

    assert_eq!(ttl_sweeper::run_cycle(&ctx.sys_pool, &sweep_config()).await, 0);
    assert_eq!(live_doc_ids(&ctx, "sessions").await, vec!["expired"]);

    sqlx::query("UPDATE ttl_policies SET enabled = true WHERE project_id = $1")
        .bind(&ctx.project_id)
        .execute(&ctx.sys_pool)
        .await
        .unwrap();
    sqlx::query("UPDATE projects SET status = 'suspended' WHERE id = $1")
        .bind(&ctx.project_id)
        .execute(&ctx.sys_pool)
        .await
        .unwrap();
    assert_eq!(ttl_sweeper::run_cycle(&ctx.sys_pool, &sweep_config()).await, 0);
    assert_eq!(live_doc_ids(&ctx, "sessions").await, vec!["expired"]);
}

/// Collection-group scope: expired documents in subcollections named like
/// the group are deleted; deleting a parent does NOT delete ITS
/// subcollections (Firestore semantics).
///
/// @driving_port @real-io
#[tokio::test]
async fn subcollection_documents_survive_their_ttl_deleted_parent() {
    let ctx = SecurityRulesFullContext::new("ttl02-subcollection").await;
    enable_policy(&ctx, "rooms", "exp", true).await;

    insert_doc(&ctx, "rooms", "r1", ts_fields("exp", -3600)).await;
    insert_doc(&ctx, "rooms/r1/messages", "m1", serde_json::json!({"text": {"t": "S", "v": "hi"}})).await;
    insert_doc(&ctx, "users/u1/rooms", "r2", ts_fields("exp", -3600)).await;

    assert_eq!(ttl_sweeper::run_cycle(&ctx.sys_pool, &sweep_config()).await, 2);

    assert!(live_doc_ids(&ctx, "rooms").await.is_empty());
    assert!(live_doc_ids(&ctx, "users/u1/rooms").await.is_empty());
    assert_eq!(
        live_doc_ids(&ctx, "rooms/r1/messages").await,
        vec!["m1"],
        "subcollection of a TTL-deleted document must survive"
    );
}

/// The per-run cap bounds a sweep; the remainder is deleted by the next run.
///
/// @driving_port @real-io
#[tokio::test]
async fn per_run_cap_defers_the_remainder_to_the_next_cycle() {
    let ctx = SecurityRulesFullContext::new("ttl02-cap").await;
    enable_policy(&ctx, "sessions", "exp", true).await;
    for i in 0..5 {
        insert_doc(&ctx, "sessions", &format!("e{i}"), ts_fields("exp", -60)).await;
    }
    let mut cfg = sweep_config();
    cfg.max_deletes_per_policy = 3;

    assert_eq!(ttl_sweeper::run_cycle(&ctx.sys_pool, &cfg).await, 3);
    assert_eq!(live_doc_ids(&ctx, "sessions").await.len(), 2);
    assert_eq!(ttl_sweeper::run_cycle(&ctx.sys_pool, &cfg).await, 2);
    assert!(live_doc_ids(&ctx, "sessions").await.is_empty());
}

/// A project whose database cannot be reached does not stop the sweep of
/// the next project.
///
/// @driving_port @real-io @error
#[tokio::test]
async fn an_unreachable_project_does_not_stop_the_others() {
    let ctx = SecurityRulesFullContext::new("ttl02-isolation").await;
    // Sorts before "ttl02-isolation": swept first, and fails (no stored DSN).
    sqlx::query(
        "INSERT INTO projects (id, account_id, backend_mode, api_key_hash_current, status, name) \
         VALUES ('aaa-broken', $1, 'direct_pg', 'unused-hash', 'active', 'aaa-broken')",
    )
    .bind(ctx.account_id)
    .execute(&ctx.sys_pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO ttl_policies (project_id, collection_group, field_path) VALUES ('aaa-broken', 'sessions', 'exp')")
        .execute(&ctx.sys_pool)
        .await
        .unwrap();
    enable_policy(&ctx, "sessions", "exp", true).await;
    insert_doc(&ctx, "sessions", "expired", ts_fields("exp", -60)).await;

    assert_eq!(ttl_sweeper::run_cycle(&ctx.sys_pool, &sweep_config()).await, 1);
    assert!(live_doc_ids(&ctx, "sessions").await.is_empty());
}

/// TTL deletes are real deletes visible to a live Listen subscriber: the
/// stream delivers the removal (same NOTIFY path as a client delete), and
/// the policy is set through the admin API end to end.
///
/// @driving_port @real-io @walking_skeleton
#[tokio::test]
async fn a_live_listen_subscriber_sees_the_ttl_deletion() {
    let ctx = SecurityRulesFullContext::new("ttl02-listen").await;
    let cookie = ctx.seed_session("owner@ttl.example", "Owner").await;
    let put = put_policy(
        &ctx,
        &cookie,
        serde_json::json!({"collection_group": "sessions", "field_path": "exp"}),
    )
    .await;
    assert_eq!(put.status().as_u16(), 200);
    insert_doc(&ctx, "sessions", "expired", ts_fields("exp", -3600)).await;

    let channel = tonic::transport::Channel::from_shared(format!("http://{}", ctx.server.grpc_addr))
        .unwrap()
        .connect_lazy();
    let mut client = FirestoreClient::new(channel);
    let database = format!("projects/{}/databases/(default)", ctx.project_id);
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tx.send(ListenRequest {
        database: database.clone(),
        target_change: Some(listen_request::TargetChange::AddTarget(Target {
            target_id: 1,
            target_type: Some(target::TargetType::Query(target::QueryTarget {
                parent: format!("{database}/documents"),
                query_type: Some(query_target::QueryType::StructuredQuery(StructuredQuery {
                    from: vec![CollectionSelector {
                        collection_id: "sessions".to_string(),
                        all_descendants: false,
                    }],
                    ..Default::default()
                })),
            })),
            ..Default::default()
        })),
        ..Default::default()
    })
    .await
    .unwrap();
    let mut request = tonic::Request::new(tokio_stream::wrappers::ReceiverStream::new(rx));
    request
        .metadata_mut()
        .insert("authorization", format!("bearer {}", ctx.api_key).parse().unwrap());
    let mut stream = client.listen(request).await.expect("listen").into_inner();

    // Initial snapshot: ADD, the document, CURRENT.
    let mut saw_document = false;
    loop {
        match next_message(&mut stream).await {
            listen_response::ResponseType::DocumentChange(dc) => {
                assert!(dc.document.unwrap().name.ends_with("/sessions/expired"));
                saw_document = true;
            }
            listen_response::ResponseType::TargetChange(tc)
                if tc.target_change_type == TargetChangeType::Current as i32 =>
            {
                break
            }
            _ => {}
        }
    }
    assert!(saw_document, "initial snapshot must contain the not-yet-swept document");

    assert_eq!(ttl_sweeper::run_cycle(&ctx.sys_pool, &sweep_config()).await, 1);

    loop {
        match next_message(&mut stream).await {
            listen_response::ResponseType::DocumentDelete(dd) => {
                assert!(dd.document.ends_with("/sessions/expired"));
                return;
            }
            listen_response::ResponseType::DocumentRemove(dr) => {
                assert!(dr.document.ends_with("/sessions/expired"));
                return;
            }
            listen_response::ResponseType::DocumentChange(_) => {
                panic!("TTL deletion must not surface as a document change")
            }
            _ => {}
        }
    }
}

async fn next_message(stream: &mut tonic::Streaming<ListenResponse>) -> listen_response::ResponseType {
    tokio::time::timeout(Duration::from_secs(20), stream.next())
        .await
        .expect("timed out waiting for a Listen message")
        .expect("Listen stream ended")
        .expect("Listen stream error")
        .response_type
        .expect("response_type set")
}
