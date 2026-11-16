//! RW07 (go-live blocker X1) — Pattern Rules Gate Commit, BatchGetDocuments,
//! ListDocuments and RunQuery Too.
//!
//! Before this fix, path-pattern (ADR-063) and recursive-wildcard (ADR-064)
//! rules were resolved only by GetDocument, the unary Create/Update/Delete
//! handlers and Listen's per-event re-check. Commit (what `setDoc()` sends),
//! BatchGetDocuments, ListDocuments and RunQuery looked up EXACT-MATCH rules
//! only, so a collection governed exclusively by a pattern was wide open
//! through them (ADR-063's own named OQ-PM-07, plus its write/batch-read
//! siblings).
//!
//! Every scenario runs against BOTH a fixed-depth wildcard pattern
//! (`journal_entries`) and a recursive-wildcard catch-all (`photos`).
//!
//! Setup uses the REAL admin import endpoint, mirroring RW02/RW03.
//! Driving port: admin HTTP :9090 (import) + gRPC :8080
//! `Commit`/`BatchGetDocuments`/`ListDocuments`/`RunQuery`.

#![allow(unused_imports)]

#[path = "../../security_rules_write_path/common/mod.rs"]
mod common;
use common::{
    commit_writes, mint_client_identity_token, now_unix, string_field, update_write,
    SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use embyr_proto::firestore::{
    batch_get_documents_response::Result as BatchResult,
    firestore_client::FirestoreClient,
    run_aggregation_query_request::QueryType as AggregationQueryType,
    run_query_request::QueryType,
    structured_aggregation_query::{
        aggregation::{Count, Operator as AggregationOperator},
        Aggregation, QueryType as StructuredAggQueryType,
    },
    structured_query::CollectionSelector,
    value::ValueType,
    BatchGetDocumentsRequest, ListDocumentsRequest, RunAggregationQueryRequest, RunQueryRequest,
    StructuredAggregationQuery, StructuredQuery,
};
use rand_core::OsRng;
use tokio_stream::StreamExt;

/// `journal_entries`: fixed-depth wildcard pattern (ADR-063). `photos`:
/// governed ONLY by the recursive-wildcard catch-all (ADR-064). Both use
/// `request.auth != null` — a shape RunQuery's own static compliance check
/// (ADR-031) can decide. `teams/{teamId}/notes` uses a shape it can NOT
/// decide, for the fail-closed scenario.
///
/// `users/{uid}/posts` (fixed-depth) and `vaults/{uid}/{path=**}` (recursive)
/// are owner rules over an ANCESTOR path variable, which the queried
/// collection path fixes. `users/{uid}/drafts` conditions on the document-id
/// wildcard, which it does not.
const RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /expeditions/{expeditionId}/journal_entries/{entryId} {
      allow read, write: if request.auth != null;
    }
    match /expeditions/{expeditionId}/{path=**} {
      allow read, write: if request.auth != null;
    }
    match /teams/{teamId}/notes/{noteId} {
      allow read, write: if request.auth.uid != "";
    }
    match /users/{uid}/posts/{postId} {
      allow read, write: if request.auth.uid == uid;
    }
    match /vaults/{uid}/{path=**} {
      allow read, write: if request.auth.uid == uid;
    }
    match /users/{uid}/drafts/{draftId} {
      allow read, write: if request.auth.uid == draftId;
    }
  }
}
"#;

/// The two pattern-governed collections every scenario covers.
const GOVERNED_COLLECTIONS: [&str; 2] = ["journal_entries", "photos"];

async fn setup(project_id: &str) -> (SecurityRulesFullContext, String) {
    let (ctx, marias_token, _) = setup_two_users(project_id).await;
    (ctx, marias_token)
}

/// `setup`, plus a second signed-in user (Dana).
async fn setup_two_users(project_id: &str) -> (SecurityRulesFullContext, String, String) {
    let ctx = SecurityRulesFullContext::new(project_id).await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{project_id}/access_rules/import")))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": RULES_FILE }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(resp.status().as_u16(), 200, "setup: rules-file import must succeed");

    let token_for =
        |uid| mint_client_identity_token(&signing_key, uid, &ctx.project_id, now_unix() + 3600);
    let (marias_token, danas_token) = (token_for("maria-santos"), token_for("dana-kim"));
    (ctx, marias_token, danas_token)
}

fn resource_name(project_id: &str, collection_path: &str, document_id: &str) -> String {
    format!("projects/{project_id}/databases/(default)/documents/{collection_path}/{document_id}")
}

/// `setDoc()`-style write: a single-`Update` Commit batch.
async fn set_doc(
    ctx: &SecurityRulesFullContext,
    resource: &str,
    token: Option<&str>,
) -> Result<(), tonic::Status> {
    let fields = std::collections::HashMap::from([("caption".to_string(), string_field("summit"))]);
    commit_writes(ctx, vec![update_write(resource, fields)], vec![], token)
        .await
        .map(|_| ())
}

async fn client(ctx: &SecurityRulesFullContext) -> FirestoreClient<tonic::transport::Channel> {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    FirestoreClient::new(channel)
}

fn authorize<T>(
    ctx: &SecurityRulesFullContext,
    message: T,
    token: Option<&str>,
) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", ctx.api_key).parse().unwrap(),
    );
    if let Some(token) = token {
        request.metadata_mut().insert(
            "x-embyr-client-identity",
            format!("Bearer {token}").parse().unwrap(),
        );
    }
    request
}

/// Names of the documents a real `BatchGetDocuments` call returned as FOUND.
async fn batch_get_found(
    ctx: &SecurityRulesFullContext,
    resource: &str,
    token: Option<&str>,
) -> Vec<String> {
    let request = authorize(
        ctx,
        BatchGetDocumentsRequest {
            database: format!("projects/{}/databases/(default)", ctx.project_id),
            documents: vec![resource.to_string()],
            ..Default::default()
        },
        token,
    );
    let mut stream = client(ctx)
        .await
        .batch_get_documents(request)
        .await
        .expect("BatchGetDocuments never aborts on a per-document deny")
        .into_inner();
    let mut found = Vec::new();
    while let Some(item) = stream.next().await {
        if let Some(BatchResult::Found(doc)) = item.expect("stream item").result {
            found.push(doc.name);
        }
    }
    found
}

/// Names of the documents a real `ListDocuments` call returned.
async fn list_names(
    ctx: &SecurityRulesFullContext,
    parent_document: &str,
    collection_id: &str,
    token: Option<&str>,
) -> Vec<String> {
    let request = authorize(
        ctx,
        ListDocumentsRequest {
            parent: format!(
                "projects/{}/databases/(default)/documents/{parent_document}",
                ctx.project_id
            ),
            collection_id: collection_id.to_string(),
            page_size: 0,
            page_token: String::new(),
        },
        token,
    );
    client(ctx)
        .await
        .list_documents(request)
        .await
        .expect("ListDocuments filters denied documents, never errors")
        .into_inner()
        .documents
        .into_iter()
        .map(|d| d.name)
        .collect()
}

/// Names of the documents a real, unfiltered `RunQuery` call returned.
async fn run_query_names(
    ctx: &SecurityRulesFullContext,
    collection_path: &str,
    token: Option<&str>,
) -> Result<Vec<String>, tonic::Status> {
    let request = authorize(
        ctx,
        RunQueryRequest {
            parent: format!("projects/{}/databases/(default)/documents", ctx.project_id),
            query_type: Some(QueryType::StructuredQuery(StructuredQuery {
                from: vec![CollectionSelector {
                    collection_id: collection_path.to_string(),
                    all_descendants: false,
                }],
                ..Default::default()
            })),
            ..Default::default()
        },
        token,
    );
    let mut stream = client(ctx).await.run_query(request).await?.into_inner();
    let mut names = Vec::new();
    while let Some(item) = stream.next().await {
        if let Some(document) = item?.document {
            names.push(document.name);
        }
    }
    Ok(names)
}

/// A single-collection query the way an SDK sends it: the parent DOCUMENT
/// (`users/maria-santos`) plus the bare collection id (`posts`).
fn subcollection_query(collection_id: &str) -> StructuredQuery {
    StructuredQuery {
        from: vec![CollectionSelector {
            collection_id: collection_id.to_string(),
            all_descendants: false,
        }],
        ..Default::default()
    }
}

fn document_parent(ctx: &SecurityRulesFullContext, parent_document: &str) -> String {
    format!(
        "projects/{}/databases/(default)/documents/{parent_document}",
        ctx.project_id
    )
}

/// Names of the documents a real, unfiltered `RunQuery` on
/// `<parent_document>/<collection_id>` returned.
async fn subcollection_query_names(
    ctx: &SecurityRulesFullContext,
    parent_document: &str,
    collection_id: &str,
    token: Option<&str>,
) -> Result<Vec<String>, tonic::Status> {
    let request = authorize(
        ctx,
        RunQueryRequest {
            parent: document_parent(ctx, parent_document),
            query_type: Some(QueryType::StructuredQuery(subcollection_query(collection_id))),
            ..Default::default()
        },
        token,
    );
    let mut stream = client(ctx).await.run_query(request).await?.into_inner();
    let mut names = Vec::new();
    while let Some(item) = stream.next().await {
        if let Some(document) = item?.document {
            names.push(document.name);
        }
    }
    Ok(names)
}

/// The COUNT a real `RunAggregationQuery` on `<parent_document>/<collection_id>`
/// returned.
async fn subcollection_count(
    ctx: &SecurityRulesFullContext,
    parent_document: &str,
    collection_id: &str,
    token: Option<&str>,
) -> Result<Option<ValueType>, tonic::Status> {
    let request = authorize(
        ctx,
        RunAggregationQueryRequest {
            parent: document_parent(ctx, parent_document),
            query_type: Some(AggregationQueryType::StructuredAggregationQuery(
                StructuredAggregationQuery {
                    query_type: Some(StructuredAggQueryType::StructuredQuery(subcollection_query(
                        collection_id,
                    ))),
                    aggregations: vec![Aggregation {
                        operator: Some(AggregationOperator::Count(Count { up_to: None })),
                        alias: String::new(),
                    }],
                },
            )),
            ..Default::default()
        },
        token,
    );
    let mut stream = client(ctx).await.run_aggregation_query(request).await?.into_inner();
    let result = stream
        .next()
        .await
        .expect("one RunAggregationQueryResponse")?
        .result
        .expect("response must carry an AggregationResult");
    Ok(result.aggregate_fields.get("field_1").and_then(|v| v.value_type.clone()))
}

/// Journey:
///   Given: `journal_entries` (wildcard pattern) and `photos` (recursive
///          catch-all) both require a signed-in user to write
///   When:  an anonymous caller calls `setDoc()` (a Commit), then Maria does
///   Then:  the anonymous Commit is denied and leaves no document behind;
///          Maria's Commit succeeds and the document exists
///
/// @error @driving_port @real-io
#[tokio::test]
async fn commit_is_denied_by_a_pattern_write_rule_and_allowed_when_the_rule_allows() {
    let (ctx, marias_token) = setup("trailmark-prod-rw07-commit").await;

    for collection in GOVERNED_COLLECTIONS {
        let resource =
            resource_name(&ctx.project_id, &format!("expeditions/trek-2026/{collection}"), "doc-1");

        let err = set_doc(&ctx, &resource, None)
            .await
            .expect_err("anonymous Commit under a pattern write rule must be denied");
        assert_eq!(err.code(), tonic::Code::PermissionDenied, "{collection}: got {err:?}");
        assert!(
            batch_get_found(&ctx, &resource, Some(&marias_token)).await.is_empty(),
            "{collection}: a denied Commit must not have written the document"
        );

        set_doc(&ctx, &resource, Some(&marias_token))
            .await
            .unwrap_or_else(|e| panic!("{collection}: Maria's Commit must succeed: {e:?}"));
        assert_eq!(
            batch_get_found(&ctx, &resource, Some(&marias_token)).await,
            vec![resource.clone()],
            "{collection}: Maria's committed document must exist"
        );
    }
}

/// Journey:
///   Given: Maria's document in each pattern-governed collection
///   When:  an anonymous caller, then Maria, reads it via BatchGetDocuments
///          and ListDocuments
///   Then:  the anonymous caller sees it as missing / gets an empty list;
///          Maria gets the document from both
///
/// @error @driving_port @real-io
#[tokio::test]
async fn batch_get_and_list_documents_hide_pattern_denied_documents_and_return_allowed_ones() {
    let (ctx, marias_token) = setup("trailmark-prod-rw07-reads").await;

    for collection in GOVERNED_COLLECTIONS {
        let resource =
            resource_name(&ctx.project_id, &format!("expeditions/trek-2026/{collection}"), "doc-1");
        set_doc(&ctx, &resource, Some(&marias_token))
            .await
            .expect("setup: Maria's Commit must succeed");

        assert!(
            batch_get_found(&ctx, &resource, None).await.is_empty(),
            "{collection}: anonymous BatchGetDocuments must report the document as missing"
        );
        assert_eq!(
            batch_get_found(&ctx, &resource, Some(&marias_token)).await,
            vec![resource.clone()],
            "{collection}: Maria's BatchGetDocuments must find the document"
        );

        assert!(
            list_names(&ctx, "expeditions/trek-2026", collection, None).await.is_empty(),
            "{collection}: anonymous ListDocuments must not return the document"
        );
        assert_eq!(
            list_names(&ctx, "expeditions/trek-2026", collection, Some(&marias_token)).await,
            vec![resource.clone()],
            "{collection}: Maria's ListDocuments must return the document"
        );
    }
}

/// Journey:
///   Given: Maria's document in each pattern-governed collection
///   When:  an anonymous caller, then Maria, queries the collection
///   Then:  the anonymous RunQuery is rejected outright, before any row is
///          returned; Maria's RunQuery returns the document
///
/// @error @driving_port @real-io
#[tokio::test]
async fn run_query_is_rejected_by_a_pattern_read_rule_and_admitted_when_the_rule_allows() {
    let (ctx, marias_token) = setup("trailmark-prod-rw07-query").await;

    for collection in GOVERNED_COLLECTIONS {
        let collection_path = format!("expeditions/trek-2026/{collection}");
        let resource = resource_name(&ctx.project_id, &collection_path, "doc-1");
        set_doc(&ctx, &resource, Some(&marias_token))
            .await
            .expect("setup: Maria's Commit must succeed");

        let err = run_query_names(&ctx, &collection_path, None)
            .await
            .expect_err("anonymous RunQuery under a pattern read rule must be rejected");
        assert_eq!(err.code(), tonic::Code::PermissionDenied, "{collection}: got {err:?}");

        assert_eq!(
            run_query_names(&ctx, &collection_path, Some(&marias_token))
                .await
                .unwrap_or_else(|e| panic!("{collection}: Maria's RunQuery must be admitted: {e:?}")),
            vec![resource.clone()],
            "{collection}: Maria's RunQuery must return the document"
        );
    }
}

/// Journey:
///   Given: `teams/{teamId}/notes` is governed by a pattern whose read
///          condition (`request.auth.uid != ""`) per-document evaluation
///          allows for Maria, but RunQuery's static compliance check cannot
///          decide
///   When:  Maria queries the collection
///   Then:  the query fails CLOSED (`PermissionDenied`,
///          `UNSUPPORTED_RULE_SHAPE`) — the same outcome an undecidable
///          exact-match rule already produces (ADR-031) — even though her
///          per-document read of the same data is allowed
///
/// @error @driving_port @real-io
#[tokio::test]
async fn run_query_fails_closed_when_the_governing_pattern_is_not_statically_decidable() {
    let (ctx, marias_token) = setup("trailmark-prod-rw07-undecidable").await;
    let resource = resource_name(&ctx.project_id, "teams/summit-crew/notes", "note-1");
    set_doc(&ctx, &resource, Some(&marias_token))
        .await
        .expect("setup: Maria's Commit must succeed");
    assert_eq!(
        batch_get_found(&ctx, &resource, Some(&marias_token)).await,
        vec![resource.clone()],
        "setup: Maria's per-document read is allowed by the same rule"
    );

    let err = run_query_names(&ctx, "teams/summit-crew/notes", Some(&marias_token))
        .await
        .expect_err("an undecidable pattern rule must reject the query, not run it unrestricted");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {err:?}");
    assert!(
        err.message().contains("UNSUPPORTED_RULE_SHAPE"),
        "rejection must name the reason: {}",
        err.message()
    );
}

/// Journey:
///   Given: owner rules `request.auth.uid == uid` on `users/{uid}/posts`
///          (wildcard pattern) and `vaults/{uid}/{path=**}` (recursive
///          catch-all), and Maria's document in her own subcollection of each
///   When:  Maria, then Dana, then an anonymous caller run a query and a
///          COUNT on MARIA'S subcollection
///   Then:  the collection path itself fixes `uid = "maria-santos"`, so the
///          rule is decided before any row is read: Maria gets her document
///          and a count of 1; Dana and the anonymous caller are rejected
///
/// @driving_port @real-io
#[tokio::test]
async fn owner_rule_on_an_ancestor_path_variable_admits_only_the_owners_query_and_count() {
    let (ctx, marias_token, danas_token) = setup_two_users("trailmark-prod-rw07-owner").await;

    for (root, collection) in [("users", "posts"), ("vaults", "items")] {
        let marias_parent = format!("{root}/maria-santos");
        let resource =
            resource_name(&ctx.project_id, &format!("{marias_parent}/{collection}"), "doc-1");
        set_doc(&ctx, &resource, Some(&marias_token))
            .await
            .expect("setup: Maria's Commit into her own subcollection must succeed");

        assert_eq!(
            subcollection_query_names(&ctx, &marias_parent, collection, Some(&marias_token))
                .await
                .unwrap_or_else(|e| panic!("{root}: the owner's RunQuery must be admitted: {e:?}")),
            vec![resource.clone()],
            "{root}: the owner's RunQuery must return her document"
        );
        assert_eq!(
            subcollection_count(&ctx, &marias_parent, collection, Some(&marias_token))
                .await
                .unwrap_or_else(|e| panic!("{root}: the owner's COUNT must be admitted: {e:?}")),
            Some(ValueType::IntegerValue(1)),
            "{root}: the owner's COUNT must count her document"
        );

        for (who, token) in [("another user", Some(danas_token.as_str())), ("anonymous", None)] {
            let err = subcollection_query_names(&ctx, &marias_parent, collection, token)
                .await
                .expect_err("only the owner may query her subcollection");
            assert_eq!(err.code(), tonic::Code::PermissionDenied, "{root}/{who} RunQuery: {err:?}");

            let err = subcollection_count(&ctx, &marias_parent, collection, token)
                .await
                .expect_err("only the owner may count her subcollection");
            assert_eq!(err.code(), tonic::Code::PermissionDenied, "{root}/{who} COUNT: {err:?}");
        }
    }
}

/// Journey:
///   Given: `users/{uid}/drafts/{draftId}` allows `request.auth.uid ==
///          draftId`, and `vaults/{uid}/{path=**}` allows `request.auth.uid
///          == uid` — both allow Maria's per-document read of her own
///          document
///   When:  Maria queries `users/maria-santos/drafts`, and the root `vaults`
///          collection (where `{uid}` IS the document id)
///   Then:  the wildcard the condition reads differs per returned document,
///          so the collection path cannot fix it: both queries still fail
///          CLOSED with `UNSUPPORTED_RULE_SHAPE`
///
/// @error @driving_port @real-io
#[tokio::test]
async fn run_query_still_fails_closed_when_the_condition_reads_the_document_id_wildcard() {
    let (ctx, marias_token) = setup("trailmark-prod-rw07-doc-id").await;

    for (parent_document, collection) in [("users/maria-santos", "drafts"), ("", "vaults")] {
        let collection_path = format!("{parent_document}/{collection}");
        let resource = resource_name(
            &ctx.project_id,
            collection_path.trim_start_matches('/'),
            "maria-santos",
        );
        set_doc(&ctx, &resource, Some(&marias_token))
            .await
            .expect("setup: Maria's Commit must succeed");
        assert_eq!(
            batch_get_found(&ctx, &resource, Some(&marias_token)).await,
            vec![resource.clone()],
            "{collection}: setup: Maria's per-document read is allowed by the same rule"
        );

        let err = subcollection_query_names(&ctx, parent_document, collection, Some(&marias_token))
            .await
            .expect_err("a condition on the document-id wildcard must reject the query");
        assert_eq!(err.code(), tonic::Code::PermissionDenied, "{collection}: got {err:?}");
        assert!(
            err.message().contains("UNSUPPORTED_RULE_SHAPE"),
            "{collection}: rejection must name the reason: {}",
            err.message()
        );
    }
}
