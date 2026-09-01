//! Common test infrastructure — agent-mode-write-streaming acceptance tests
//! (ADR-060: zero new proto/adapter/handler code — `write_stream.rs` is
//! reused completely unchanged for `backend_mode=agent`; the only
//! production change is `embyr-agent`'s own `commit()` `write_results` fix).
//!
//! WHY-NEW-FILE: tests/agent_mode_write_streaming/common/mod.rs
//!   CLOSEST-EXISTING: tests/firestore_write_streaming/common/mod.rs
//!   EXTENSION-COST: that file's own `SecurityRulesFullContext` hardcodes
//!     `backend_mode='direct_pg'` at project-insert time (concrete struct,
//!     not generic over backend mode) — cannot provision an agent-mode
//!     project without a second constructor path.
//!   PARALLEL-RATIONALE: an agent-mode context additionally owns a real
//!     `embyr-agent` process (own Postgres, own mTLS server) that a
//!     direct_pg context has no concept of — different lifecycle, not a
//!     stylistic split. Pure `WriteRequest`-building helpers with no `ctx`
//!     dependency (`handshake_request`, `single_write_request`,
//!     `string_field`, `update_write`) ARE reused via path import below.

#![allow(dead_code, unused_imports)]

use std::sync::Arc;

use embyr_proto::firestore::{firestore_client::FirestoreClient, WriteRequest, WriteResponse};
use embyr_server::adapters::system_db::SystemDb;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

// Pure `WriteRequest`/`Write` builders — no `ctx` dependency, safe to reuse
// unchanged across backend modes (per this file's own header note).
#[path = "../../firestore_write_streaming/common/mod.rs"]
mod firestore_write_streaming_common;
pub use firestore_write_streaming_common::{
    handshake_request, single_write_request, string_field, update_write,
    write_requiring_stale_update_time,
};

// Agent-side mTLS cert generation + Postgres bootstrap — reused unchanged
// from embyr-agent's own acceptance fixture (the exact pattern the
// orchestrator pointed at).
#[path = "../../acceptance/embyr_agent/mod.rs"]
mod embyr_agent_common;
pub use embyr_agent_common::{start_test_postgres, test_tls_config};

/// Fixed admin bearer key every `start_test_server`-based context in this
/// codebase uses (see `crates/embyr-server/src/lib.rs`).
const ADMIN_KEY: &str = "test-admin-key-secret";

/// Full production composition root (`embyr-server`, real gRPC/REST/admin)
/// PLUS a real `embyr-agent` instance (real mTLS, real customer-VPC-style
/// Postgres) wired together via the SAME `POST /admin/v1/projects` agent
/// registration endpoint `us_12_agent_backend.rs` already exercises against
/// a mock agent — here the agent side is genuinely real, per DESIGN's own
/// WS Strategy B requirement (no mocked transport).
pub struct AgentModeWriteStreamingContext {
    _sys_container: ContainerAsync<Postgres>,
    _agent_container: ContainerAsync<Postgres>,
    pub server: embyr_server::TestServer,
    pub agent_pool: sqlx::PgPool,
    pub api_key: String,
    pub project_id: String,
}

impl AgentModeWriteStreamingContext {
    pub async fn new(project_id: &str) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();

        // embyr-server's own system DB.
        let sys_container = Postgres::default()
            .with_tag("15-alpine")
            .start()
            .await
            .expect("start system postgres");
        let sys_port = sys_container
            .get_host_port_ipv4(5432)
            .await
            .expect("system pg port");
        let sys_url = format!("postgres://postgres:postgres@127.0.0.1:{sys_port}/postgres");
        let system_db = Arc::new(SystemDb::new(&sys_url).await.expect("SystemDb::new"));
        system_db.migrate().await.expect("system db migrate");

        // The agent's own customer-VPC-style Postgres (real, separate from
        // the system DB — mirrors `start_test_agent`'s own composition).
        let (agent_container, agent_url) = start_test_postgres().await;
        let agent_pool = sqlx::PgPool::connect(&agent_url)
            .await
            .expect("connect agent postgres");
        embyr_pg_storage::backend_adapter::PostgresBackendAdapter::run_migrations(&agent_pool)
            .await
            .expect("run agent customer migrations");
        let storage = Arc::new(
            embyr_pg_storage::backend_adapter::PostgresBackendAdapter::new_from_pool(
                agent_pool.clone(),
            ),
        );

        // Real embyr-agent, in-process, real mTLS.
        let tls = test_tls_config();
        let identity =
            tonic::transport::Identity::from_pem(&tls.server_cert_pem, &tls.server_key_pem);
        let ca_cert = tonic::transport::Certificate::from_pem(&tls.ca_cert_pem);
        let server_tls = tonic::transport::ServerTlsConfig::new()
            .identity(identity)
            .client_ca_root(ca_cert);
        let bridge = Arc::new(embyr_agent::notify_bridge::AgentNotifyBridge::new(
            agent_pool.clone(),
            project_id.to_string(),
        ));
        let agent_addr = embyr_agent::server::serve(
            project_id.to_string(),
            storage,
            bridge,
            server_tls,
            "127.0.0.1:0",
        )
        .await
        .expect("start real embyr-agent server");

        // Real embyr-server composition root.
        let server = embyr_server::start_test_server(system_db).await;

        // Register the agent-mode project through the real admin HTTP
        // endpoint (same endpoint/body shape `us_12_agent_backend.rs`
        // already proves against a mock agent).
        let resp = reqwest::Client::new()
            .post(format!("http://{}/admin/v1/projects", server.admin_addr))
            .header("Authorization", format!("Bearer {ADMIN_KEY}"))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "project_id": project_id,
                "backend_mode": "agent",
                "agent_endpoint": format!("127.0.0.1:{}", agent_addr.port()),
                "agent_ca_pem": tls.ca_cert_pem,
                "agent_client_cert_pem": tls.client_cert_pem,
                "agent_client_key_pem": tls.client_key_pem,
            }))
            .send()
            .await
            .expect("provision agent-mode project");
        assert_eq!(
            resp.status(),
            201,
            "agent-mode project provisioning must succeed"
        );
        let body: serde_json::Value = resp.json().await.expect("parse provision response");
        let api_key = body["api_key"]
            .as_str()
            .expect("api_key in provision response")
            .to_string();

        AgentModeWriteStreamingContext {
            _sys_container: sys_container,
            _agent_container: agent_container,
            server,
            agent_pool,
            api_key,
            project_id: project_id.to_string(),
        }
    }

    /// Real gRPC `GetDocument` call against `embyr-server` — driving port
    /// entry, mirroring `SecurityRulesFullContext::get_document`'s shape.
    pub async fn get_document(
        &self,
        resource_name: &str,
    ) -> Result<tonic::Response<embyr_proto::firestore::Document>, tonic::Status> {
        let channel = tonic::transport::Endpoint::new(format!("http://{}", self.server.grpc_addr))
            .expect("valid endpoint")
            .connect()
            .await
            .expect("connect to gRPC server");
        let mut client = FirestoreClient::new(channel);
        let mut request = tonic::Request::new(embyr_proto::firestore::GetDocumentRequest {
            name: resource_name.to_string(),
            ..Default::default()
        });
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {}", self.api_key).parse().unwrap(),
        );
        client.get_document(request).await
    }
}

/// Open a real gRPC `Write` bidi-stream against `embyr-server` — driving
/// port entry, mirroring `firestore_write_streaming::common::open_write_stream`
/// exactly, just against `AgentModeWriteStreamingContext` instead of
/// `SecurityRulesFullContext` (the two structs are not a shared type, so
/// this ~15-line function cannot itself be reused generically).
pub async fn open_write_stream(
    ctx: &AgentModeWriteStreamingContext,
    first_request: WriteRequest,
) -> (
    mpsc::Sender<WriteRequest>,
    Result<tonic::Streaming<WriteResponse>, tonic::Status>,
) {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let (req_tx, req_rx) = mpsc::channel::<WriteRequest>(4);
    req_tx
        .send(first_request)
        .await
        .expect("send first WriteRequest into outbound channel");

    let outbound = ReceiverStream::new(req_rx);
    let mut request = tonic::Request::new(outbound);
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", ctx.api_key).parse().unwrap(),
    );

    let result = client.write(request).await.map(|r| r.into_inner());
    (req_tx, result)
}
