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
    batch_get_documents_response::Result as BatchResult, firestore_client::FirestoreClient,
    run_query_request::QueryType, structured_query::CollectionSelector, BatchGetDocumentsRequest,
    ListDocumentsRequest, RunQueryRequest, StructuredQuery,
};
use rand_core::OsRng;
use tokio_stream::StreamExt;

/// `journal_entries`: fixed-depth wildcard pattern (ADR-063). `photos`:
/// governed ONLY by the recursive-wildcard catch-all (ADR-064). Both use
/// `request.auth != null` — a shape RunQuery's own static compliance check
/// (ADR-031) can decide. `teams/{teamId}/notes` uses a shape it can NOT
/// decide, for the fail-closed scenario.
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
  }
}
"#;

/// The two pattern-governed collections every scenario covers.
const GOVERNED_COLLECTIONS: [&str; 2] = ["journal_entries", "photos"];

async fn setup(project_id: &str) -> (SecurityRulesFullContext, String) {
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

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);
    (ctx, marias_token)
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
