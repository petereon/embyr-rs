//! mTLS gRPC server for embyr-agent.
//!
//! Implements a minimal StorageAgent server that:
//!   1. Probes Postgres via sqlx to verify connectivity
//!   2. Starts a tonic gRPC server with mTLS (requires client certificate)
//!
//! All RPC methods return Unimplemented — real proxying is added in step 09-02.

use embyr_proto::agent::{
    storage_agent_server::{StorageAgent, StorageAgentServer},
    BeginTransactionRequest, BeginTransactionResponse, CommitRequest, CommitResponse,
    CreateDocumentRequest, DeleteDocumentRequest, Document, GetDocumentRequest,
    RollbackRequest, RunQueryRequest, RunQueryResponse, UpdateDocumentRequest,
};
use sqlx::PgPool;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{
    Request, Response, Status,
    transport::{Certificate, Identity, ServerTlsConfig},
};
use tracing::info;

use crate::config::AgentConfig;

/// Stub implementation of StorageAgent — returns Unimplemented for all RPCs.
/// Real proxying is wired in step 09-02.
pub struct StorageAgentService {
    _pool: PgPool,
}

#[tonic::async_trait]
impl StorageAgent for StorageAgentService {
    async fn get_document(
        &self,
        _request: Request<GetDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        Err(Status::unimplemented("not implemented — step 09-02"))
    }

    async fn create_document(
        &self,
        _request: Request<CreateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        Err(Status::unimplemented("not implemented — step 09-02"))
    }

    async fn update_document(
        &self,
        _request: Request<UpdateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        Err(Status::unimplemented("not implemented — step 09-02"))
    }

    async fn delete_document(
        &self,
        _request: Request<DeleteDocumentRequest>,
    ) -> Result<Response<()>, Status> {
        Err(Status::unimplemented("not implemented — step 09-02"))
    }

    type RunQueryStream = ReceiverStream<Result<RunQueryResponse, Status>>;

    async fn run_query(
        &self,
        _request: Request<RunQueryRequest>,
    ) -> Result<Response<Self::RunQueryStream>, Status> {
        Err(Status::unimplemented("not implemented — step 09-02"))
    }

    async fn begin_transaction(
        &self,
        _request: Request<BeginTransactionRequest>,
    ) -> Result<Response<BeginTransactionResponse>, Status> {
        Err(Status::unimplemented("not implemented — step 09-02"))
    }

    async fn commit(
        &self,
        _request: Request<CommitRequest>,
    ) -> Result<Response<CommitResponse>, Status> {
        Err(Status::unimplemented("not implemented — step 09-02"))
    }

    async fn rollback(
        &self,
        _request: Request<RollbackRequest>,
    ) -> Result<Response<()>, Status> {
        Err(Status::unimplemented("not implemented — step 09-02"))
    }
}

/// Start the mTLS gRPC server.
///
/// 1. Connects to Postgres and logs readiness.
/// 2. Reads TLS cert/key/CA from the paths in config.
/// 3. Starts tonic with `ServerTlsConfig` requiring client certificates.
/// 4. Logs the listen address.
pub async fn run(config: AgentConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Probe Postgres
    let pool = PgPool::connect(&config.db_dsn).await?;
    info!("connected to Postgres");

    // Read TLS material from files
    let cert_pem = std::fs::read_to_string(&config.cert_path)?;
    let key_pem = std::fs::read_to_string(&config.key_path)?;
    let ca_pem = std::fs::read_to_string(&config.ca_path)?;

    let identity = Identity::from_pem(cert_pem, key_pem);
    let ca_cert = Certificate::from_pem(ca_pem);

    let tls = ServerTlsConfig::new()
        .identity(identity)
        .client_ca_root(ca_cert);

    let service = StorageAgentService { _pool: pool };

    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    let actual_addr = listener.local_addr()?;

    info!("listening on {}", actual_addr);

    tonic::transport::Server::builder()
        .tls_config(tls)?
        .add_service(StorageAgentServer::new(service))
        .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
        .await?;

    Ok(())
}
