//! RW08 (go-live blockers X9, X12).
//!
//! X9: `ListCollectionIds` never consulted security rules, so it revealed
//! sub-collection NAMES to callers the rules deny. The parent DOCUMENT is now
//! the read target: the exact-match rule for the parent's collection, else the
//! pattern rules (the same resolution `ListDocuments` uses). No rule = still
//! unrestricted; the database root has no document and stays unrestricted.
//!
//! X12: RunQuery / RunAggregationQuery built their auth context with an empty
//! claims map. Claims now come from the verified identity. The claim-reading
//! rule here is decided by the queried collection's own ancestor path, so a
//! caller's claim is what admits or rejects the query. Anonymous callers and
//! callers without the claim stay rejected.
//!
//! Setup uses the REAL admin import endpoint and real Commit writes, mirroring
//! RW07. Driving port: gRPC :8080 `ListCollectionIds` / `RunQuery` /
//! `RunAggregationQuery`.

#![allow(unused_imports)]

#[path = "../../security_rules_write_path/common/mod.rs"]
mod common;
use common::{
    commit_writes, mint_client_identity_token, now_unix, string_field, update_write,
    SecurityRulesFullContext,
};
#[path = "../../custom_claims/common/mod.rs"]
mod custom_claims_common;
use custom_claims_common::mint_client_identity_token_with_claims;
use ed25519_dalek::SigningKey;
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    run_aggregation_query_request::QueryType as AggregationQueryType,
    run_query_request::QueryType,
    structured_aggregation_query::{
        aggregation::{Count, Operator as AggregationOperator},
        Aggregation, QueryType as StructuredAggQueryType,
    },
    structured_query::CollectionSelector,
    value::ValueType,
    ListCollectionIdsRequest, RunAggregationQueryRequest, RunQueryRequest,
    StructuredAggregationQuery, StructuredQuery,
};
use rand_core::OsRng;
use tokio_stream::StreamExt;

/// `teams/{teamId}/members/{memberId}`: a member document is readable by that
/// member only. `orgs/{orgId}/reports`: readable and writable by callers whose
/// `tenant` claim names the org. `orgs/{orgId}/staff/{staffId}`: same claim
/// gate on a parent document of ListCollectionIds.
const RULES_FILE: &str = r#"
service cloud.firestore {
  match /databases/{database}/documents {
    match /teams/{teamId}/members/{memberId} {
      allow read, write: if request.auth.uid == memberId;
    }
    match /orgs/{orgId}/reports/{reportId} {
      allow read, write: if request.auth.token.tenant == orgId;
    }
    match /orgs/{orgId}/staff/{staffId} {
      allow read, write: if request.auth.token.tenant == orgId;
    }
  }
}
"#;

struct Fixture {
    ctx: SecurityRulesFullContext,
    signing_key: SigningKey,
}

impl Fixture {
    async fn new(project_id: &str) -> Self {
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
        Self { ctx, signing_key }
    }

    fn token(&self, uid: &str, claims: serde_json::Value) -> String {
        mint_client_identity_token_with_claims(
            &self.signing_key,
            uid,
            &self.ctx.project_id,
            now_unix() + 3600,
            claims,
        )
    }

    /// Commit one document with a single string field, as `token`.
    async fn set_doc(&self, relative_path: &str, field: (&str, &str), token: Option<&str>) {
        let resource = format!(
            "projects/{}/databases/(default)/documents/{relative_path}",
            self.ctx.project_id
        );
        let fields = std::collections::HashMap::from([(field.0.to_string(), string_field(field.1))]);
        commit_writes(&self.ctx, vec![update_write(&resource, fields)], vec![], token)
            .await
            .unwrap_or_else(|e| panic!("setup: Commit of {relative_path} must succeed: {e:?}"));
    }

    fn parent(&self, document_path: &str) -> String {
        format!(
            "projects/{}/databases/(default)/documents/{document_path}",
            self.ctx.project_id
        )
    }

    fn authorize<T>(&self, message: T, token: Option<&str>) -> tonic::Request<T> {
        let mut request = tonic::Request::new(message);
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {}", self.ctx.api_key).parse().unwrap(),
        );
        if let Some(token) = token {
            request.metadata_mut().insert(
                "x-embyr-client-identity",
                format!("Bearer {token}").parse().unwrap(),
            );
        }
        request
    }

    async fn client(&self) -> FirestoreClient<tonic::transport::Channel> {
        let channel =
            tonic::transport::Endpoint::new(format!("http://{}", self.ctx.server.grpc_addr))
                .expect("valid endpoint")
                .connect()
                .await
                .expect("connect to gRPC server");
        FirestoreClient::new(channel)
    }

    async fn collection_ids(
        &self,
        parent_document: &str,
        token: Option<&str>,
    ) -> Result<Vec<String>, tonic::Status> {
        let request = self.authorize(
            ListCollectionIdsRequest {
                parent: self.parent(parent_document),
                page_size: 0,
                page_token: String::new(),
            },
            token,
        );
        Ok(self.client().await.list_collection_ids(request).await?.into_inner().collection_ids)
    }

    fn subcollection_query(collection_id: &str) -> StructuredQuery {
        StructuredQuery {
            from: vec![CollectionSelector {
                collection_id: collection_id.to_string(),
                all_descendants: false,
            }],
            ..Default::default()
        }
    }

    async fn query_names(
        &self,
        parent_document: &str,
        collection_id: &str,
        token: Option<&str>,
    ) -> Result<Vec<String>, tonic::Status> {
        let request = self.authorize(
            RunQueryRequest {
                parent: self.parent(parent_document),
                query_type: Some(QueryType::StructuredQuery(Self::subcollection_query(
                    collection_id,
                ))),
                ..Default::default()
            },
            token,
        );
        let mut stream = self.client().await.run_query(request).await?.into_inner();
        let mut names = Vec::new();
        while let Some(item) = stream.next().await {
            if let Some(document) = item?.document {
                names.push(document.name);
            }
        }
        Ok(names)
    }

    async fn count(
        &self,
        parent_document: &str,
        collection_id: &str,
        token: Option<&str>,
    ) -> Result<Option<ValueType>, tonic::Status> {
        let request = self.authorize(
            RunAggregationQueryRequest {
                parent: self.parent(parent_document),
                query_type: Some(AggregationQueryType::StructuredAggregationQuery(
                    StructuredAggregationQuery {
                        query_type: Some(StructuredAggQueryType::StructuredQuery(
                            Self::subcollection_query(collection_id),
                        )),
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
        let mut stream = self.client().await.run_aggregation_query(request).await?.into_inner();
        let result = stream
            .next()
            .await
            .expect("one RunAggregationQueryResponse")?
            .result
            .expect("response must carry an AggregationResult");
        Ok(result.aggregate_fields.get("field_1").and_then(|v| v.value_type.clone()))
    }
}

fn assert_denied<T: std::fmt::Debug>(result: Result<T, tonic::Status>, who: &str) {
    let err = result.expect_err(&format!("{who} must be denied"));
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "{who}: got {err:?}");
}

/// Journey:
///   Given: `teams/{teamId}/members/{memberId}` is readable by that member
///          only, and Maria's member document has a `diary` subcollection
///   When:  Maria, then Dana, then an anonymous caller list the
///          subcollection ids of Maria's member document
///   Then:  Maria gets `["diary"]`; Dana and the anonymous caller get
///          PermissionDenied, never the names
///
/// @error @driving_port @real-io
#[tokio::test]
async fn list_collection_ids_is_gated_by_the_parent_documents_pattern_read_rule() {
    let f = Fixture::new("trailmark-prod-rw08-pattern").await;
    let marias = f.token("maria-santos", serde_json::json!({}));
    let danas = f.token("dana-kim", serde_json::json!({}));
    let parent = "teams/summit/members/maria-santos";
    f.set_doc(&format!("{parent}/diary/d1"), ("entry", "summit"), Some(&marias)).await;

    assert_eq!(f.collection_ids(parent, Some(&marias)).await.unwrap(), vec!["diary".to_string()]);
    assert_denied(f.collection_ids(parent, Some(&danas)).await, "another user");
    assert_denied(f.collection_ids(parent, None).await, "anonymous");
}

/// Journey:
///   Given: an exact-match rule on `profiles` (`request.auth.uid ==
///          resource.data.owner_id`) and Maria's profile with a `notes`
///          subcollection
///   When:  Maria, then Dana, then an anonymous caller list its collection ids
///   Then:  only the owner gets the names; the others get PermissionDenied
///
/// @error @driving_port @real-io
#[tokio::test]
async fn list_collection_ids_is_gated_by_the_parent_documents_exact_read_rule() {
    let f = Fixture::new("trailmark-prod-rw08-exact").await;
    f.ctx.seed_access_rule("profiles", "request.auth.uid == resource.data.owner_id").await;
    let marias = f.token("maria-santos", serde_json::json!({}));
    let danas = f.token("dana-kim", serde_json::json!({}));
    f.set_doc("profiles/maria-santos", ("owner_id", "maria-santos"), None).await;
    f.set_doc("profiles/maria-santos/notes/n1", ("text", "hello"), None).await;

    assert_eq!(
        f.collection_ids("profiles/maria-santos", Some(&marias)).await.unwrap(),
        vec!["notes".to_string()]
    );
    assert_denied(f.collection_ids("profiles/maria-santos", Some(&danas)).await, "another user");
    assert_denied(f.collection_ids("profiles/maria-santos", None).await, "anonymous");
}

/// Journey:
///   Given: `orgs/{orgId}/staff/{staffId}` is readable by callers whose
///          `tenant` claim names the org
///   When:  a caller with `tenant=acme`, one with `tenant=globex` and one with
///          no such claim list the subcollection ids of `orgs/acme/staff/s1`
///   Then:  only the `acme` caller gets the names
///
/// @error @driving_port @real-io
#[tokio::test]
async fn list_collection_ids_evaluates_claims_from_the_verified_identity() {
    let f = Fixture::new("trailmark-prod-rw08-claims-list").await;
    let acme = f.token("alice", serde_json::json!({ "tenant": "acme" }));
    let globex = f.token("bob", serde_json::json!({ "tenant": "globex" }));
    let no_claim = f.token("carol", serde_json::json!({}));
    f.set_doc("orgs/acme/staff/s1/reviews/r1", ("text", "ok"), None).await;

    assert_eq!(
        f.collection_ids("orgs/acme/staff/s1", Some(&acme)).await.unwrap(),
        vec!["reviews".to_string()]
    );
    assert_denied(f.collection_ids("orgs/acme/staff/s1", Some(&globex)).await, "other tenant");
    assert_denied(f.collection_ids("orgs/acme/staff/s1", Some(&no_claim)).await, "no claim");
}

/// Journey:
///   Given: no rule governs `plain/doc1`, which has a `sub` subcollection
///   When:  an anonymous caller lists the ids of `plain/doc1` and of the root
///   Then:  both succeed — no rule stays unrestricted, and the database root
///          (no parent document) is not rule-gated
///
/// @driving_port @real-io
#[tokio::test]
async fn list_collection_ids_without_a_governing_rule_or_at_the_root_stays_unrestricted() {
    let f = Fixture::new("trailmark-prod-rw08-open").await;
    f.set_doc("plain/doc1/sub/s1", ("k", "v"), None).await;

    assert_eq!(f.collection_ids("plain/doc1", None).await.unwrap(), vec!["sub".to_string()]);
    assert!(f.collection_ids("", None).await.unwrap().contains(&"plain".to_string()));
}

/// Journey:
///   Given: `orgs/{orgId}/reports` is readable by callers whose `tenant` claim
///          names the org; Alice (`tenant=acme`) wrote one report
///   When:  Alice, a `tenant=globex` caller, a caller with no claim and an
///          anonymous caller run a query and a COUNT on `orgs/acme/reports`
///   Then:  the verified claim admits only Alice (her document, count 1); every
///          other caller is rejected — an absent identity never matches a claim
///
/// @security-critical @driving_port @real-io
#[tokio::test]
async fn query_and_count_evaluate_claims_from_the_verified_identity_and_never_widen() {
    let f = Fixture::new("trailmark-prod-rw08-claims-query").await;
    let acme = f.token("alice", serde_json::json!({ "tenant": "acme" }));
    let globex = f.token("bob", serde_json::json!({ "tenant": "globex" }));
    let no_claim = f.token("carol", serde_json::json!({}));
    f.set_doc("orgs/acme/reports/r1", ("title", "q3"), Some(&acme)).await;
    let resource = f.parent("orgs/acme/reports/r1");

    assert_eq!(f.query_names("orgs/acme", "reports", Some(&acme)).await.unwrap(), vec![resource]);
    assert_eq!(
        f.count("orgs/acme", "reports", Some(&acme)).await.unwrap(),
        Some(ValueType::IntegerValue(1))
    );
    for (who, token) in
        [("other tenant", Some(globex.as_str())), ("no claim", Some(no_claim.as_str())), ("anonymous", None)]
    {
        assert_denied(f.query_names("orgs/acme", "reports", token).await, &format!("{who} RunQuery"));
        assert_denied(f.count("orgs/acme", "reports", token).await, &format!("{who} COUNT"));
    }
}
