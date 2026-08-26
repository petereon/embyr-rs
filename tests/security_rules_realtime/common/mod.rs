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
    create_document, delete_document, mint_client_identity_token, now_unix,
    seed_write_access_rule_full, string_field, update_document, SecurityRulesAdminContext,
    SecurityRulesFullContext,
};

use std::time::Duration;

use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    listen_request, listen_response,
    run_query_request::QueryType as RunQueryType,
    structured_query::{
        composite_filter::Operator as CompositeOp, field_filter::Operator as FieldOp,
        filter::FilterType, CollectionSelector, CompositeFilter, FieldFilter, FieldReference,
        Filter,
    },
    target::{self, query_target},
    target_change::TargetChangeType,
    value::ValueType,
    Document, ListenRequest, ListenResponse, RunQueryRequest, StructuredQuery, Target, Value,
};
use tokio_stream::StreamExt;

/// Build an `AddTarget` `ListenRequest` for `collection` — mirrors
/// `us_05_listen_realtime.rs::add_target_request` exactly (Pillar 3: reuse
/// the base WS's own established request shape, not a divergent one).
pub fn add_target_request(project_id: &str, collection: &str) -> ListenRequest {
    add_target_request_filtered(project_id, collection, None)
}

/// Build an `AddTarget` `ListenRequest` for `collection` carrying an
/// OPTIONAL `where_` filter (Slice 02, US-02) — generalizes
/// `add_target_request` above, which now delegates here with `filter: None`.
pub fn add_target_request_filtered(
    project_id: &str,
    collection: &str,
    filter: Option<Filter>,
) -> ListenRequest {
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
                    r#where: filter,
                    ..Default::default()
                })),
            })),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn string_value(s: &str) -> Value {
    Value { value_type: Some(ValueType::StringValue(s.to_string())) }
}

fn equality_filter(field_path: &str, value: &str) -> Filter {
    Filter {
        filter_type: Some(FilterType::FieldFilter(FieldFilter {
            field: Some(FieldReference { field_path: field_path.to_string() }),
            op: FieldOp::Equal as i32,
            value: Some(string_value(value)),
        })),
    }
}

/// Build a `where_` filter from `(field, value)` equality pairs, AND-composed
/// via `CompositeFilter` when more than one — mirrors
/// `security_rules_query_path::common::run_query`'s own filter-building
/// shape exactly (Slice 02's own reuse discipline extends to the shared test
/// fixture pattern, not just production code).
pub fn equality_where_filter(equality_filters: &[(&str, &str)]) -> Option<Filter> {
    match equality_filters {
        [] => None,
        [(field, value)] => Some(equality_filter(field, value)),
        many => Some(Filter {
            filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
                op: CompositeOp::And as i32,
                filters: many.iter().map(|(f, v)| equality_filter(f, v)).collect(),
            })),
        }),
    }
}

/// Real gRPC `RunQuery` call — the oracle Slice 02's own initial-snapshot
/// tests cross-check Listen's filtered snapshot against ("an identical
/// filter shape and identical seeded documents must produce an identical
/// result set" — AC-17-109/110's own design). A LOCAL helper, not a
/// cross-`#[path]`-tree import of
/// `security_rules_query_path::common::run_query`: Rust does not
/// deduplicate types across separate `#[path]` inclusion points, so a
/// `SecurityRulesFullContext` reached via that OTHER path tree is a
/// structurally distinct type from this file's own
/// `security_rules_write_path_common::SecurityRulesFullContext`, even
/// though both ultimately come from the identical source file. Keeping this
/// helper local, reusing `equality_where_filter`/`StructuredQuery`/
/// `CollectionSelector` already defined above, keeps both sides of the
/// comparison on the SAME type.
pub async fn run_query(
    ctx: &SecurityRulesFullContext,
    collection_id: &str,
    equality_filters: &[(&str, &str)],
    client_identity_token: Option<&str>,
) -> Result<Vec<Document>, tonic::Status> {
    run_query_raw(
        ctx,
        collection_id,
        false,
        equality_where_filter(equality_filters),
        client_identity_token,
    )
    .await
}

/// Real gRPC `RunQuery` call with an explicit `all_descendants` flag —
/// Slice 06 (US-06)'s own AC-17-128 needs to prove BOTH the non-group
/// (`access_rules`) and group (`group_access_rules`) `RunQuery` arms are
/// unaffected by this feature, mirroring
/// `security_rules_collection_group_rules::common::run_query_raw` exactly,
/// kept LOCAL for the same reason `run_query` above is local (no type-safe
/// cross-`#[path]`-tree reuse of `SecurityRulesFullContext`). `run_query`
/// above now delegates here with `all_descendants: false`.
pub async fn run_query_raw(
    ctx: &SecurityRulesFullContext,
    collection_id: &str,
    all_descendants: bool,
    filter: Option<Filter>,
    client_identity_token: Option<&str>,
) -> Result<Vec<Document>, tonic::Status> {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let sq = StructuredQuery {
        from: vec![CollectionSelector {
            collection_id: collection_id.to_string(),
            all_descendants,
        }],
        r#where: filter,
        ..Default::default()
    };

    let mut request = tonic::Request::new(RunQueryRequest {
        parent: format!(
            "projects/{}/databases/(default)/documents",
            ctx.project_id
        ),
        query_type: Some(RunQueryType::StructuredQuery(sq)),
        ..Default::default()
    });
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", ctx.api_key).parse().unwrap(),
    );
    if let Some(token) = client_identity_token {
        request.metadata_mut().insert(
            "x-embyr-client-identity",
            format!("Bearer {token}").parse().unwrap(),
        );
    }

    let mut stream = client.run_query(request).await?.into_inner();
    let mut docs = Vec::new();
    while let Some(item) = stream.next().await {
        let response = item?;
        if let Some(document) = response.document {
            docs.push(document);
        }
    }
    Ok(docs)
}

/// Directly seed a `group_access_rules` row against a
/// `SecurityRulesFullContext` — Slice 06 (US-06)'s own AC-17-128 needs a
/// group rule active to prove `RunQuery`'s group arm is unaffected by this
/// feature. Mirrors
/// `security_rules_collection_group_rules::common::seed_group_access_rule_full`
/// exactly, kept LOCAL for the same type-identity reason `run_query_raw`
/// above is local — this feature's own common module never imports across
/// the `security_rules_collection_group_rules` path tree.
pub async fn seed_group_access_rule_full(
    ctx: &SecurityRulesFullContext,
    collection_id: &str,
    condition_source: &str,
) {
    sqlx::query(
        "INSERT INTO group_access_rules (project_id, collection_id, condition_source) \
         VALUES ($1, $2, $3)",
    )
    .bind(&ctx.project_id)
    .bind(collection_id)
    .bind(condition_source)
    .execute(&ctx.sys_pool)
    .await
    .expect("insert group_access_rules row");
}

/// Open a real `Listen` gRPC stream subscribed to `collection`,
/// authenticated as `ctx.api_key` — the new driving-port helper this slice
/// needs (no prior sibling feature opened a Listen stream from OUTSIDE
/// `tests/acceptance/us_05_listen_realtime.rs` itself).
pub async fn open_listen_stream(
    ctx: &SecurityRulesFullContext,
    collection: &str,
) -> tonic::Streaming<ListenResponse> {
    open_listen_stream_filtered(ctx, collection, None).await
}

/// Open a real `Listen` gRPC stream subscribed to `collection`, carrying an
/// OPTIONAL `where_` filter on the initial `AddTarget` (Slice 02, US-02) —
/// generalizes `open_listen_stream` above, which now delegates here with
/// `filter: None`.
pub async fn open_listen_stream_filtered(
    ctx: &SecurityRulesFullContext,
    collection: &str,
    filter: Option<Filter>,
) -> tonic::Streaming<ListenResponse> {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let req_stream = tokio_stream::once(add_target_request_filtered(
        &ctx.project_id,
        collection,
        filter,
    ));
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

/// Open a real `Listen` gRPC stream subscribed to `collection`, carrying an
/// OPTIONAL `where_` filter AND an OPTIONAL `x-embyr-client-identity` token
/// (Slice 03, US-03) — generalizes `open_listen_stream_filtered` above,
/// which now delegates here with `client_identity_token: None`. Needed
/// because `check_query_compliance()`'s subscribe-time gate (ADR-033 §
/// Decision — Subscribe-Time Composition) reads `request.auth` via
/// `attach_client_identity_if_present`, mirroring
/// `security_rules_query_path::common::run_query`'s own identical
/// identity-header wiring.
pub async fn open_listen_stream_filtered_as(
    ctx: &SecurityRulesFullContext,
    collection: &str,
    filter: Option<Filter>,
    client_identity_token: Option<&str>,
) -> tonic::Streaming<ListenResponse> {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let req_stream = tokio_stream::once(add_target_request_filtered(
        &ctx.project_id,
        collection,
        filter,
    ));
    let mut request = tonic::Request::new(req_stream);
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", ctx.api_key).parse().unwrap(),
    );
    if let Some(token) = client_identity_token {
        request.metadata_mut().insert(
            "x-embyr-client-identity",
            format!("Bearer {token}").parse().unwrap(),
        );
    }

    client
        .listen(request)
        .await
        .expect("listen should succeed")
        .into_inner()
}

/// Collect every `DocumentChange` document name delivered as part of the
/// INITIAL SNAPSHOT — i.e. everything received up to (not including) the
/// `TargetChange(CURRENT)` marker. Generalizes `drain_until_current` below
/// (which discards the snapshot contents) for Slice 02's own "does the
/// snapshot honor the filter" assertions.
pub async fn collect_initial_snapshot(stream: &mut tonic::Streaming<ListenResponse>) -> Vec<String> {
    let mut names = Vec::new();
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("timed out waiting for CURRENT")
            .expect("stream ended before CURRENT")
            .expect("stream error before CURRENT");
        match msg.response_type {
            Some(listen_response::ResponseType::DocumentChange(dc)) => {
                if let Some(doc) = dc.document {
                    names.push(doc.name);
                }
            }
            Some(listen_response::ResponseType::TargetChange(tc)) => {
                let change_type = TargetChangeType::try_from(tc.target_change_type)
                    .unwrap_or(TargetChangeType::NoChange);
                if change_type == TargetChangeType::Current {
                    break;
                }
            }
            _ => {}
        }
    }
    names
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
