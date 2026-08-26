//! Common test infrastructure — security-rules-realtime acceptance tests
//! (Slice 01, ADR-033, US-01).
//!
//! Reuses `SecurityRulesFullContext`/`create_document`/`string_field` from
//! `security-rules-write-path`'s own fixture module via a path import (this
//! feature's own WS composes access-control machinery
//! `security-rules`/`security-rules-write-path` already established — no
//! new admin-port context class needed). `tests/security_rules/`,
//! `tests/security_rules_write_path/`, and `tests/security_rules_query_path/`
//! files are never touched — only imported, read-only, from here.
//!
//! Slice 01 adds ONE genuinely new driving-port helper: opening a real
//! `Listen` bidirectional stream and collecting delivered events.
//! `tests/acceptance/us_05_listen_realtime.rs` (the base walking-skeleton's
//! own Listen coverage) established the `AddTarget`-building /
//! drain-until-CURRENT pattern reused below — generalized here since this
//! is the first sibling feature to need Listen-stream helpers shared across
//! multiple test files.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_write_path/common/mod.rs"]
mod security_rules_write_path_common;
pub use security_rules_write_path_common::{
    create_document, delete_document, string_field, SecurityRulesFullContext,
};

use std::time::Duration;

use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    listen_request, listen_response,
    structured_query::CollectionSelector,
    target::{self, query_target},
    target_change::TargetChangeType,
    ListenRequest, ListenResponse, StructuredQuery, Target,
};
use tokio_stream::StreamExt;

/// Build an `AddTarget` `ListenRequest` for `collection` — mirrors
/// `us_05_listen_realtime.rs::add_target_request` exactly (Pillar 3: reuse
/// the base WS's own established request shape, not a divergent one).
pub fn add_target_request(project_id: &str, collection: &str) -> ListenRequest {
    ListenRequest {
        database: format!("projects/{project_id}/databases/(default)"),
        target_change: Some(listen_request::TargetChange::AddTarget(Target {
            target_id: 1,
            target_type: Some(target::TargetType::Query(target::QueryTarget {
                parent: format!("projects/{project_id}/databases/(default)/documents"),
                query_type: Some(query_target::QueryType::StructuredQuery(StructuredQuery {
                    from: vec![CollectionSelector {
                        collection_id: collection.to_string(),
                        all_descendants: false,
                    }],
                    ..Default::default()
                })),
            })),
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// Open a real `Listen` gRPC stream subscribed to `collection`,
/// authenticated as `ctx.api_key` — the new driving-port helper this slice
/// needs (no prior sibling feature opened a Listen stream from OUTSIDE
/// `tests/acceptance/us_05_listen_realtime.rs` itself).
pub async fn open_listen_stream(
    ctx: &SecurityRulesFullContext,
    collection: &str,
) -> tonic::Streaming<ListenResponse> {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let req_stream = tokio_stream::once(add_target_request(&ctx.project_id, collection));
    let mut request = tonic::Request::new(req_stream);
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", ctx.api_key).parse().unwrap(),
    );

    client
        .listen(request)
        .await
        .expect("listen should succeed")
        .into_inner()
}

/// Drain a Listen stream until the `TargetChange(CURRENT)` marker arrives —
/// mirrors `us_05_listen_realtime.rs`'s own drain-until-CURRENT loop,
/// generalized as a shared helper.
pub async fn drain_until_current(stream: &mut tonic::Streaming<ListenResponse>) {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("timed out waiting for CURRENT")
            .expect("stream ended before CURRENT")
            .expect("stream error before CURRENT");
        if let Some(listen_response::ResponseType::TargetChange(tc)) = msg.response_type {
            let change_type = TargetChangeType::try_from(tc.target_change_type)
                .unwrap_or(TargetChangeType::NoChange);
            if change_type == TargetChangeType::Current {
                break;
            }
        }
    }
}

/// A delivered live event's own collection-scoping-relevant shape — just
/// enough to assert an event's KIND and named document (Universe
/// discipline: port-exposed observable only, never an internal struct
/// field).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapturedEvent {
    Changed { document_name: String },
    Removed { document_name: String },
}

/// Wait up to `timeout` for the NEXT live `DocumentChange`/`DocumentDelete`
/// event (skipping `TargetChange` keep-alive/`NO_CHANGE` frames). Returns
/// `None` if nothing arrives within `timeout` — the AC-17-105/106 "never
/// receives" case is observed as a bounded wait with no event, not an
/// infinite hang.
pub async fn try_recv_live_event(
    stream: &mut tonic::Streaming<ListenResponse>,
    timeout: Duration,
) -> Option<CapturedEvent> {
    tokio::time::timeout(timeout, async {
        loop {
            let msg = stream.next().await?.ok()?;
            match msg.response_type {
                Some(listen_response::ResponseType::DocumentChange(dc)) => {
                    let name = dc.document.map(|d| d.name).unwrap_or_default();
                    return Some(CapturedEvent::Changed { document_name: name });
                }
                Some(listen_response::ResponseType::DocumentDelete(dd)) => {
                    return Some(CapturedEvent::Removed { document_name: dd.document });
                }
                _ => continue,
            }
        }
    })
    .await
    .unwrap_or(None)
}
