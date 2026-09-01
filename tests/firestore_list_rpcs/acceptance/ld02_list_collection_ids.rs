//! LD02 (Slice 02, LAST slice, US-02) — Alex Discovers What Collections
//! Exist Under a Path.
//!
//! Acceptance criteria verified here (slice-02-list-collection-ids.md):
//!   AC-02-01/AC-02-02: an explicit `page_size` returns at most that many
//!             distinct collection IDs per page, `next_page_token` present
//!             iff more remain; presenting a previously-returned token
//!             returns the remaining IDs, never repeating or skipping any.
//!   AC-02-03: a root-level `parent` returns only top-level collections,
//!             never a nested subcollection.
//!   AC-02-04: a document with no subcollections returns an empty
//!             `collection_ids` array and an empty `next_page_token`, no
//!             error.
//!   AC-02-05: each distinct collection name is returned exactly once,
//!             regardless of how many documents it contains.
//!   AC-02-06: an empty `parent` is rejected with `InvalidArgument`.
//!   Cross-version graceful degradation (agent-mode-list-collection-ids,
//!             ADR-059 § Consequences, "Residual" — supersedes this file's
//!             former "Agent-mode deferral" scenario, obsolete since
//!             `AgentBackendAdapter` now has a real `list_collection_ids`
//!             override): an old `embyr-agent` binary predating this RPC
//!             fails with a clean `Internal`, never a panic or a
//!             silently-wrong empty success.
//!
//! Driving port: gRPC :8080 `ListCollectionIds` (via `SecurityRulesFullContext`
//! — the same `embyr_server::start_test_server` composition root the
//! existing regression suite uses, Pillar 3).
//!
//! Test Budget: 7 behaviors (paginated listing; root-vs-nested scoping;
//! empty-subcollections no-error; distinct-name-exactly-once;
//! empty-parent rejection; old-agent graceful degradation) x 2 = 14 max. 6
//! written — one per behavior, no variation-inflation (pagination pair
//! AC-02-01/02 counted as one behavior per ld01's own identical precedent).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{create_document, list_collection_ids, nested_parent, root_parent, string_field, SecurityRulesFullContext};
use std::collections::HashMap;

fn owner_fields(owner: &str) -> HashMap<String, embyr_proto::firestore::Value> {
    HashMap::from([("owner_id".to_string(), string_field(owner))])
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-02-01 / AC-02-02: distinct child collection IDs are listed across
// pages, in order, no repeats or skips.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-02-01, AC-02-02
///
/// @driving_port @real-io @US-02 @AC-02-01 @AC-02-02
#[tokio::test]
async fn distinct_child_collection_ids_are_listed_across_pages_with_no_repeats_or_skips() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-lc02-paginated").await;

    for (collection, doc_id) in [
        ("users/maria-santos-a1b2/trip_entries", "yosemite-2024"),
        ("users/maria-santos-a1b2/payment_methods", "visa-ending-1234"),
        ("users/maria-santos-a1b2/support_notes", "ticket-001"),
    ] {
        create_document(&ctx, collection, doc_id, owner_fields("maria-santos"), None)
            .await
            .expect("create seed document");
    }

    let parent = nested_parent(&ctx, "users/maria-santos-a1b2");

    let page1 = list_collection_ids(&ctx, &parent, 2, "")
        .await
        .expect("AC-02-01: page 1 must succeed");
    assert_eq!(page1.collection_ids.len(), 2, "AC-02-01: page 1 must contain exactly page_size collection IDs");
    assert!(
        !page1.next_page_token.is_empty(),
        "AC-02-01: next_page_token must be present when more collection IDs remain"
    );

    let page2 = list_collection_ids(&ctx, &parent, 2, &page1.next_page_token)
        .await
        .expect("AC-02-02: page 2 must succeed");
    assert_eq!(page2.collection_ids.len(), 1, "AC-02-02: page 2 must contain the remaining collection ID");
    assert!(
        page2.next_page_token.is_empty(),
        "AC-02-02: next_page_token must be empty on the last page"
    );

    let mut seen: Vec<String> = page1
        .collection_ids
        .iter()
        .chain(page2.collection_ids.iter())
        .cloned()
        .collect();
    seen.sort();
    let mut expected = vec!["trip_entries".to_string(), "payment_methods".to_string(), "support_notes".to_string()];
    expected.sort();
    assert_eq!(seen, expected, "AC-02-02: every collection ID must appear exactly once across pages");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-02-03: a root-level parent returns only top-level collections, never a
// nested one.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-02-03
///
/// @driving_port @real-io @US-02 @AC-02-03
#[tokio::test]
async fn a_root_level_parent_returns_only_top_level_collections_never_a_nested_one() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-lc02-root").await;

    create_document(&ctx, "users", "maria-santos-a1b2", owner_fields("maria-santos"), None)
        .await
        .expect("create top-level users document");
    create_document(
        &ctx,
        "users/maria-santos-a1b2/trip_entries",
        "yosemite-2024",
        owner_fields("maria-santos"),
        None,
    )
    .await
    .expect("create nested trip entry");

    let parent = root_parent(&ctx);
    let response = list_collection_ids(&ctx, &parent, 100, "")
        .await
        .expect("AC-02-03: root-level call must succeed");

    assert!(
        response.collection_ids.iter().any(|c| c == "users"),
        "AC-02-03: must include the top-level 'users' collection"
    );
    assert!(
        !response.collection_ids.iter().any(|c| c == "trip_entries"),
        "AC-02-03: must NOT include the nested 'trip_entries' collection"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-02-04: a document with no subcollections returns an empty result, no
// error.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-02-04
///
/// @driving_port @real-io @US-02 @AC-02-04
#[tokio::test]
async fn a_document_with_no_subcollections_returns_an_empty_result_and_no_error() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-lc02-empty").await;

    let parent = nested_parent(&ctx, "users/brand-new-user");
    let response = list_collection_ids(&ctx, &parent, 10, "")
        .await
        .expect("AC-02-04: a document with no subcollections must not error");

    assert!(response.collection_ids.is_empty(), "AC-02-04: collection_ids must be empty");
    assert!(response.next_page_token.is_empty(), "AC-02-04: next_page_token must be empty");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-02-05: each distinct collection name is returned exactly once,
// regardless of how many documents it contains.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-02-05
///
/// @driving_port @real-io @US-02 @AC-02-05
#[tokio::test]
async fn each_distinct_collection_name_is_returned_exactly_once_regardless_of_document_count() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-lc02-distinct").await;

    for doc_id in ["yosemite-2024", "banff-2024", "patagonia-2025"] {
        create_document(
            &ctx,
            "users/maria-santos-a1b2/trip_entries",
            doc_id,
            owner_fields("maria-santos"),
            None,
        )
        .await
        .expect("create trip entry");
    }

    let parent = nested_parent(&ctx, "users/maria-santos-a1b2");
    let response = list_collection_ids(&ctx, &parent, 100, "")
        .await
        .expect("AC-02-05: call must succeed");

    let occurrences = response.collection_ids.iter().filter(|c| c.as_str() == "trip_entries").count();
    assert_eq!(occurrences, 1, "AC-02-05: 'trip_entries' must appear exactly once, not once per document");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-02-06: an empty parent is rejected with InvalidArgument.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-02-06
///
/// @error @driving_port @real-io @US-02 @AC-02-06
#[tokio::test]
async fn an_empty_parent_is_rejected_with_invalid_argument() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-lc02-empty-parent").await;

    let result = list_collection_ids(&ctx, "", 10, "").await;

    let err = result.expect_err("AC-02-06: an empty parent must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

// ─────────────────────────────────────────────────────────────────────────────
// Cross-version graceful degradation (superseded by agent-mode-list-collection-ids,
// ADR-059): ADR-051 § Decision 3 originally deferred agent-mode
// `ListCollectionIds` entirely (`AgentBackendAdapter` had no override, so the
// trait's default-provided body rejected with `FailedPrecondition` before any
// network call). agent-mode-list-collection-ids (ADR-059) gives
// `AgentBackendAdapter::list_collection_ids` a real override that DOES call
// the agent over mTLS — so this scenario now documents a DIFFERENT, still-real
// case: an `embyr-agent` binary that predates this RPC (here, `StubAgentServer`
// stands in for exactly that — it implements `StorageAgent` but not this RPC's
// real logic, mirroring an old deployed agent) returns a clean
// `Status::unimplemented`, which `AgentBackendAdapter`'s own `grpc_err`
// collapses to `CoreError::BackendUnavailable` -> `Status::internal`
// (ADR-059 § Consequences, "Residual" — the named cross-version failure mode,
// not a panic or a silently-wrong empty success).
// ─────────────────────────────────────────────────────────────────────────────

// A minimal mTLS `StorageAgent` stub — exists so `AgentBackendAdapter::new`'s
// real mTLS `connect()` succeeds, AND (post agent-mode-list-collection-ids,
// ADR-059) so `list_collection_ids` specifically is now reached over the
// wire and returns a deliberate `unimplemented`, standing in for an
// old-version agent binary that predates this RPC. Every OTHER method below
// remains unreachable by this test; each returns `unimplemented` to fail
// loudly if that assumption is ever wrong.
mod stub_agent {
    use embyr_proto::agent::{
        storage_agent_server::StorageAgent, BeginTransactionRequest, BeginTransactionResponse,
        CommitRequest, CommitResponse, CreateDocumentRequest, DeleteDocumentRequest, DocChange,
        Document as AgentDocument, GetDocumentRequest, ListCollectionIdsRequest,
        ListCollectionIdsResponse, ListDocumentsRequest,
        ListDocumentsResponse, PingRequest, PingResponse, RollbackRequest,
        RunAggregationQueryRequest, RunAggregationQueryResponse, RunQueryRequest,
        RunQueryResponse, SubscribeRequest, UpdateDocumentRequest,
    };
    use tokio_stream::wrappers::ReceiverStream;
    use tonic::{Request, Response, Status};

    pub struct StubAgentServer;

    #[tonic::async_trait]
    impl StorageAgent for StubAgentServer {
        async fn get_document(&self, _: Request<GetDocumentRequest>) -> Result<Response<AgentDocument>, Status> {
            Err(Status::unimplemented("stub agent — unreachable by the ListCollectionIds deferral test"))
        }
        async fn create_document(&self, _: Request<CreateDocumentRequest>) -> Result<Response<AgentDocument>, Status> {
            Err(Status::unimplemented("stub"))
        }
        async fn update_document(&self, _: Request<UpdateDocumentRequest>) -> Result<Response<AgentDocument>, Status> {
            Err(Status::unimplemented("stub"))
        }
        async fn delete_document(&self, _: Request<DeleteDocumentRequest>) -> Result<Response<()>, Status> {
            Err(Status::unimplemented("stub"))
        }
        type RunQueryStream = ReceiverStream<Result<RunQueryResponse, Status>>;
        async fn run_query(&self, _: Request<RunQueryRequest>) -> Result<Response<Self::RunQueryStream>, Status> {
            Err(Status::unimplemented("stub"))
        }
        async fn begin_transaction(&self, _: Request<BeginTransactionRequest>) -> Result<Response<BeginTransactionResponse>, Status> {
            Err(Status::unimplemented("stub"))
        }
        async fn commit(&self, _: Request<CommitRequest>) -> Result<Response<CommitResponse>, Status> {
            Err(Status::unimplemented("stub"))
        }
        async fn rollback(&self, _: Request<RollbackRequest>) -> Result<Response<()>, Status> {
            Err(Status::unimplemented("stub"))
        }
        async fn ping(&self, _: Request<PingRequest>) -> Result<Response<PingResponse>, Status> {
            Err(Status::unimplemented("stub"))
        }
        async fn run_aggregation_query(&self, _: Request<RunAggregationQueryRequest>) -> Result<Response<RunAggregationQueryResponse>, Status> {
            Err(Status::unimplemented("stub"))
        }
        async fn list_documents(&self, _: Request<ListDocumentsRequest>) -> Result<Response<ListDocumentsResponse>, Status> {
            Err(Status::unimplemented("stub"))
        }
        async fn list_collection_ids(&self, _: Request<ListCollectionIdsRequest>) -> Result<Response<ListCollectionIdsResponse>, Status> {
            // Deliberately unimplemented — stands in for an embyr-agent
            // binary that predates the ListCollectionIds RPC (ADR-059 §
            // Consequences, "Residual"). Reached for real now that
            // AgentBackendAdapter has an override that calls this RPC.
            Err(Status::unimplemented("stub agent predates ListCollectionIds"))
        }
        type SubscribeStream = ReceiverStream<Result<DocChange, Status>>;
        async fn subscribe(&self, _: Request<SubscribeRequest>) -> Result<Response<Self::SubscribeStream>, Status> {
            Err(Status::unimplemented("stub"))
        }
    }
}

/// A full mTLS cert set (CA + server + client) as PEM bytes, matching
/// `AgentBackendAdapter`'s own expected TLS bundle shape.
struct MtlsCertSet {
    ca_pem: Vec<u8>,
    server_cert_pem: Vec<u8>,
    server_key_pem: Vec<u8>,
    client_cert_pem: Vec<u8>,
    client_key_pem: Vec<u8>,
}

fn generate_mtls_cert_set() -> MtlsCertSet {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};

    let mut ca_params = CertificateParams::new(vec!["embyr-ca".to_string()]).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();

    let server_params = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    let server_key = KeyPair::generate().unwrap();
    let server_cert = server_params.signed_by(&server_key, &ca_cert, &ca_key).unwrap();

    let client_params = CertificateParams::new(vec!["embyr-saas".to_string()]).unwrap();
    let client_key = KeyPair::generate().unwrap();
    let client_cert = client_params.signed_by(&client_key, &ca_cert, &ca_key).unwrap();

    MtlsCertSet {
        ca_pem: ca_cert.pem().into_bytes(),
        server_cert_pem: server_cert.pem().into_bytes(),
        server_key_pem: server_key.serialize_pem().into_bytes(),
        client_cert_pem: client_cert.pem().into_bytes(),
        client_key_pem: client_key.serialize_pem().into_bytes(),
    }
}

/// Start the stub mTLS `StorageAgent` server on a free port. Returns the
/// port and a shutdown sender.
async fn start_stub_agent_server(
    ca_pem: &[u8],
    server_cert_pem: &[u8],
    server_key_pem: &[u8],
) -> (u16, tokio::sync::oneshot::Sender<()>) {
    use tonic::transport::{Certificate, Identity, ServerTlsConfig};

    let _ = rustls::crypto::ring::default_provider().install_default();

    let identity = Identity::from_pem(server_cert_pem, server_key_pem);
    let ca_cert = Certificate::from_pem(ca_pem);
    let tls = ServerTlsConfig::new().identity(identity).client_ca_root(ca_cert);

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind free port");
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .tls_config(tls)
            .expect("tls config")
            .add_service(embyr_proto::agent::storage_agent_server::StorageAgentServer::new(
                stub_agent::StubAgentServer,
            ))
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async {
                    let _ = shutdown_rx.await;
                },
            )
            .await
            .ok();
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    (port, shutdown_tx)
}

/// Directly insert a `backend_mode='agent'` project row (bypassing the
/// admin HTTP provisioning endpoint — this test only needs the system DB
/// row + a live mTLS stub agent for `AgentBackendAdapter::new` to connect
/// to, mirroring `SecurityRulesFullContext::new`'s own direct-SQL project
/// insert pattern for `direct_pg` projects).
async fn provision_agent_mode_project(
    ctx: &SecurityRulesFullContext,
    project_id: &str,
) -> (String, tokio::sync::oneshot::Sender<()>) {
    let certs = generate_mtls_cert_set();
    let (port, shutdown_tx) =
        start_stub_agent_server(&certs.ca_pem, &certs.server_cert_pem, &certs.server_key_pem).await;
    let endpoint = format!("127.0.0.1:{port}");

    let api_key = format!("test-sk-lc02-agent-{project_id}");
    let api_key_hash =
        embyr_core::auth::argon2::hash_api_key(api_key.as_bytes()).expect("hash api key");
    let pub_key = embyr_core::auth::ecies::derive_public_key(api_key.as_bytes());
    let bundle = serde_json::json!({
        "ca_pem": String::from_utf8_lossy(&certs.ca_pem),
        "client_cert_pem": String::from_utf8_lossy(&certs.client_cert_pem),
        "client_key_pem": String::from_utf8_lossy(&certs.client_key_pem),
    })
    .to_string();
    let encrypted_bundle =
        embyr_core::auth::ecies::encrypt(&pub_key, bundle.as_bytes()).expect("ecies encrypt bundle");

    sqlx::query(
        "INSERT INTO projects \
         (id, account_id, status, backend_mode, api_key_hash_current, backend_agent_endpoint, agent_tls_bundle_enc) \
         VALUES ($1, $2, 'active', 'agent', $3, $4, $5)",
    )
    .bind(project_id)
    .bind(ctx.account_id)
    .bind(&api_key_hash)
    .bind(&endpoint)
    .bind(&encrypted_bundle)
    .execute(&ctx.sys_pool)
    .await
    .expect("insert agent-mode project");

    (api_key, shutdown_tx)
}

/// Cross-version graceful degradation: an old embyr-agent binary predating
/// ListCollectionIds (agent-mode-list-collection-ids, ADR-059 § Consequences,
/// "Residual") fails cleanly, never a panic or a silently-wrong empty
/// success. Supersedes this file's own former "agent-mode deferral" test
/// (ADR-051 § Decision 3), obsolete since AgentBackendAdapter now has a real
/// list_collection_ids override (ADR-059).
///
/// @error @driving_port @real-io @US-02
#[tokio::test]
async fn an_old_agents_list_collection_ids_call_returns_a_clean_internal_error() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-lc02-agent-mode").await;

    let project_id = "lc02-agent-proj";
    let (agent_api_key, _shutdown_tx) = provision_agent_mode_project(&ctx, project_id).await;

    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = embyr_proto::firestore::firestore_client::FirestoreClient::new(channel);
    let mut request = tonic::Request::new(embyr_proto::firestore::ListCollectionIdsRequest {
        parent: format!("projects/{project_id}/databases/(default)/documents"),
        page_size: 10,
        page_token: String::new(),
    });
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {agent_api_key}").parse().unwrap(),
    );

    let result = client.list_collection_ids(request).await;

    let err = result.expect_err(
        "an old agent's ListCollectionIds call must be rejected, not silently succeed with wrong data",
    );
    assert_eq!(
        err.code(),
        tonic::Code::Internal,
        "an old agent's Unimplemented must cross grpc_err's own BackendUnavailable collapse into a \
         clean Internal, not a panic or a silently-wrong empty success; got: {err:?}"
    );
}
