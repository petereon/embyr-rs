//! mTLS gRPC server for embyr-agent.
//!
//! Implements StorageAgent backed by PostgresBackendAdapter.
//! Exposes `serve()` for in-process test starts and `run()` for the binary.

use std::net::SocketAddr;
use std::sync::Arc;

use embyr_core::domain::{document::DocumentPath, project::ProjectId};
use embyr_core::storage::backend_adapter::BackendAdapter;
use embyr_pg_storage::backend_adapter::PostgresBackendAdapter;
use embyr_proto::agent::{
    storage_agent_server::{StorageAgent, StorageAgentServer},
    BeginTransactionRequest, BeginTransactionResponse, CommitRequest, CommitResponse,
    CreateDocumentRequest, DeleteDocumentRequest, Document, GetDocumentRequest,
    PingRequest, PingResponse, RollbackRequest, RunQueryRequest, RunQueryResponse,
    UpdateDocumentRequest,
};
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::{
    Request, Response, Status,
    transport::{Certificate, Identity, ServerTlsConfig},
};
use tracing::info;

use crate::config::AgentConfig;
use crate::encoding::domain_doc_to_proto;

/// StorageAgent gRPC service backed by PostgresBackendAdapter.
pub struct StorageAgentService {
    project_id: String,
    storage: Arc<PostgresBackendAdapter>,
}

impl StorageAgentService {
    /// Construct the service.
    pub fn new(project_id: String, storage: Arc<PostgresBackendAdapter>) -> Self {
        Self { project_id, storage }
    }
}

#[tonic::async_trait]
impl StorageAgent for StorageAgentService {
    async fn get_document(
        &self,
        request: Request<GetDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let start = std::time::Instant::now();
        let name = request.into_inner().name;

        if name.is_empty() {
            return Err(Status::invalid_argument("name is required"));
        }

        let path = parse_document_name(&name, &self.project_id)?;

        match self.storage.get_document(&path).await {
            Ok(Some(doc)) => {
                let duration_ms = start.elapsed().as_millis();
                tracing::info!(
                    project_id = %path.project_id.as_str(),
                    path = %name,
                    duration_ms = duration_ms,
                    "GetDocument"
                );
                Ok(Response::new(domain_doc_to_proto(doc)))
            }
            Ok(None) => Err(Status::not_found(format!("document not found: {name}"))),
            Err(e) => Err(Status::internal(format!("{e}"))),
        }
    }

    async fn create_document(
        &self,
        _request: Request<CreateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        Err(Status::unimplemented("not implemented — step 03-01"))
    }

    async fn update_document(
        &self,
        _request: Request<UpdateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        Err(Status::unimplemented("not implemented — step 03-01"))
    }

    async fn delete_document(
        &self,
        _request: Request<DeleteDocumentRequest>,
    ) -> Result<Response<()>, Status> {
        Err(Status::unimplemented("not implemented — step 03-01"))
    }

    type RunQueryStream = ReceiverStream<Result<RunQueryResponse, Status>>;

    async fn run_query(
        &self,
        _request: Request<RunQueryRequest>,
    ) -> Result<Response<Self::RunQueryStream>, Status> {
        Err(Status::unimplemented("not implemented — step 03-01"))
    }

    async fn begin_transaction(
        &self,
        _request: Request<BeginTransactionRequest>,
    ) -> Result<Response<BeginTransactionResponse>, Status> {
        Err(Status::unimplemented("not implemented — step 03-01"))
    }

    async fn commit(
        &self,
        _request: Request<CommitRequest>,
    ) -> Result<Response<CommitResponse>, Status> {
        Err(Status::unimplemented("not implemented — step 03-01"))
    }

    async fn rollback(
        &self,
        _request: Request<RollbackRequest>,
    ) -> Result<Response<()>, Status> {
        Err(Status::unimplemented("not implemented — step 03-01"))
    }

    async fn ping(
        &self,
        _request: Request<PingRequest>,
    ) -> Result<Response<PingResponse>, Status> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Status::internal(format!("system time error: {e}")))?;
        Ok(Response::new(PingResponse {
            server_time: Some(prost_types::Timestamp {
                seconds: now.as_secs() as i64,
                nanos: now.subsec_nanos() as i32,
            }),
        }))
    }
}

/// Parse a Firestore resource name into a `DocumentPath`.
///
/// Expected format: `projects/{project_id}/databases/(default)/documents/{collection_path}/{document_id}`
fn parse_document_name(name: &str, _expected_project_id: &str) -> Result<DocumentPath, Status> {
    let prefix = "projects/";
    if !name.starts_with(prefix) {
        return Err(Status::invalid_argument(format!("invalid document name: {name}")));
    }
    let rest = &name[prefix.len()..];
    let slash_pos = rest
        .find('/')
        .ok_or_else(|| Status::invalid_argument("missing project_id separator"))?;
    let project_id_str = &rest[..slash_pos];
    let rest = &rest[slash_pos + 1..];

    let doc_prefix = "databases/(default)/documents/";
    if !rest.starts_with(doc_prefix) {
        return Err(Status::invalid_argument("invalid database/documents path"));
    }
    let doc_path = &rest[doc_prefix.len()..];

    let last_slash = doc_path
        .rfind('/')
        .ok_or_else(|| Status::invalid_argument("missing document_id"))?;
    let collection_path = &doc_path[..last_slash];
    let document_id = &doc_path[last_slash + 1..];

    let pid = ProjectId::new(project_id_str)
        .map_err(|e| Status::invalid_argument(format!("invalid project_id: {e}")))?;

    Ok(DocumentPath {
        project_id: pid,
        collection_path: collection_path.to_string(),
        document_id: document_id.to_string(),
    })
}

/// Start the agent server in-process, binding to `listen_addr`.
///
/// Spawns the tonic serve loop in a background task and returns the bound
/// `SocketAddr`. Used by the test harness to start an agent without a separate
/// process.
pub async fn serve(
    project_id: String,
    storage: Arc<PostgresBackendAdapter>,
    tls: ServerTlsConfig,
    listen_addr: &str,
) -> Result<SocketAddr, Box<dyn std::error::Error + Send + Sync>> {
    let service = StorageAgentService::new(project_id, storage);
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .tls_config(tls)
            .unwrap()
            .add_service(StorageAgentServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    Ok(addr)
}

/// Start the mTLS gRPC server from environment config.
///
/// 1. Connects to Postgres (probe logs "connected to Postgres" before this is called).
/// 2. Reads TLS cert/key/CA from the paths in config.
/// 3. Starts tonic with `ServerTlsConfig` requiring client certificates.
/// 4. Listens for SIGTERM; on receipt, drains in-flight RPCs, logs "shutdown complete",
///    and exits with code 0.
pub async fn run(config: AgentConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(config.max_conns)
        .connect(&config.db_dsn)
        .await?;

    let cert_pem = std::fs::read_to_string(&config.cert_path)?;
    let key_pem = std::fs::read_to_string(&config.key_path)?;
    let ca_pem = std::fs::read_to_string(&config.ca_path)?;

    let identity = Identity::from_pem(cert_pem, key_pem);
    let ca_cert = Certificate::from_pem(ca_pem);

    let tls = ServerTlsConfig::new().identity(identity).client_ca_root(ca_cert);

    let storage = Arc::new(PostgresBackendAdapter::new_from_pool(pool));
    let service = StorageAgentService::new(config.project_id, storage);

    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    let addr = listener.local_addr()?;
    info!("listening on {addr}");

    let mut sigterm = tokio::signal::unix::signal(
        tokio::signal::unix::SignalKind::terminate(),
    )?;

    tonic::transport::Server::builder()
        .tls_config(tls)?
        .add_service(StorageAgentServer::new(service))
        .serve_with_incoming_shutdown(
            TcpListenerStream::new(listener),
            async move {
                sigterm.recv().await;
                tracing::info!("received SIGTERM — initiating graceful shutdown");
            },
        )
        .await?;

    info!("shutdown complete");
    Ok(())
}
