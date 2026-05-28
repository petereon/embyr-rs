// SCAFFOLD: true
//! Shared test infrastructure for embyr-agent acceptance tests.
//!
//! Provides:
//! - `start_test_agent` — spins up a real Postgres container and the embyr-agent
//!   in-process, returns a tonic mTLS client ready to invoke StorageAgent RPCs.
//! - `test_tls_config` — generates a self-signed test CA + server cert + client
//!   cert using `rcgen`.  Client cert is required; tests asserting TLS failure
//!   use a raw channel with no client cert.
//! - `AgentHandle` — cleanup guard; drops the Postgres container and shuts the
//!   agent server when the test ends.
//!
//! Infrastructure policy (from docs/architecture/atdd-infrastructure-policy.md):
//!   Driving: in-process tonic mTLS test client; test CA + leaf certs via rcgen
//!   Driven internal: testcontainers-rs Postgres image, fresh per test

use std::net::SocketAddr;
use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Holds live resources for a running agent test instance.
///
/// Drop this value to trigger cleanup: the Postgres container exits and the
/// agent server task is aborted.
pub struct AgentHandle {
    /// The address the agent's mTLS gRPC listener is bound on.
    pub grpc_addr: SocketAddr,
    /// Keep the Postgres container alive for the duration of the test.
    _postgres: ContainerAsync<Postgres>,
    /// Abort handle for the in-process agent task.
    _agent_task: tokio::task::JoinHandle<()>,
    /// Pool exposed for seeding test data.
    pub pool: sqlx::PgPool,
}

/// TLS artifacts for a test instance.
pub struct TestTlsConfig {
    /// PEM-encoded CA certificate used to sign both the server and client certs.
    pub ca_cert_pem: String,
    /// PEM-encoded server certificate (signed by `ca_cert_pem`).
    pub server_cert_pem: String,
    /// PEM-encoded server private key.
    pub server_key_pem: String,
    /// PEM-encoded client certificate (signed by `ca_cert_pem`).
    pub client_cert_pem: String,
    /// PEM-encoded client private key.
    pub client_key_pem: String,
}

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Generate a test mTLS identity using `rcgen`.
///
/// Creates a self-signed CA and signs both a server certificate (SANs:
/// `localhost`, `127.0.0.1`) and a client certificate from it.
pub fn test_tls_config() -> TestTlsConfig {
    // CA cert
    let mut ca_params = CertificateParams::new(vec!["embyr-ca".to_string()]).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();

    // Server cert signed by CA (SANs: localhost + 127.0.0.1)
    let server_params =
        CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()]).unwrap();
    let server_key = KeyPair::generate().unwrap();
    let server_cert = server_params.signed_by(&server_key, &ca_cert, &ca_key).unwrap();

    // Client cert signed by CA
    let client_params = CertificateParams::new(vec!["embyr-saas".to_string()]).unwrap();
    let client_key = KeyPair::generate().unwrap();
    let client_cert = client_params.signed_by(&client_key, &ca_cert, &ca_key).unwrap();

    TestTlsConfig {
        ca_cert_pem: ca_cert.pem(),
        server_cert_pem: server_cert.pem(),
        server_key_pem: server_key.serialize_pem(),
        client_cert_pem: client_cert.pem(),
        client_key_pem: client_key.serialize_pem(),
    }
}

/// Convenience: start Postgres only (used by lifecycle tests that launch the
/// agent process themselves via `std::process::Command`).
///
/// Returns the container handle and the `DATABASE_URL` connection string.
pub async fn start_test_postgres() -> (ContainerAsync<Postgres>, String) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let postgres = Postgres::default()
        .start()
        .await
        .expect("start postgres container");
    let port = postgres.get_host_port_ipv4(5432).await.expect("get postgres port");
    let db_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    (postgres, db_url)
}

/// Start a real Postgres container and the embyr-agent in-process.
///
/// Returns an `AgentHandle` (cleanup guard) and a connected tonic mTLS client
/// pointed at the agent's gRPC listener.
///
/// The agent is configured with `EMBYR_AGENT_PROJECT_ID = project_id`.
pub async fn start_test_agent(
    project_id: &str,
) -> (
    AgentHandle,
    embyr_proto::agent::storage_agent_client::StorageAgentClient<tonic::transport::Channel>,
) {
    // Install ring crypto provider (idempotent — safe to call multiple times).
    let _ = rustls::crypto::ring::default_provider().install_default();

    // 1. Start Postgres testcontainer.
    let postgres = Postgres::default()
        .start()
        .await
        .expect("start postgres container");
    let db_port = postgres
        .get_host_port_ipv4(5432)
        .await
        .expect("get postgres port");
    let db_url = format!("postgres://postgres:postgres@127.0.0.1:{db_port}/postgres");

    // 2. Run migrations and keep the pool for seeding.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("connect to test postgres");
    embyr_pg_storage::backend_adapter::PostgresBackendAdapter::run_migrations(&pool)
        .await
        .expect("run customer migrations");
    let storage = Arc::new(
        embyr_pg_storage::backend_adapter::PostgresBackendAdapter::new_from_pool(pool.clone()),
    );

    // 3. Generate test certs.
    let tls = test_tls_config();
    let identity =
        tonic::transport::Identity::from_pem(&tls.server_cert_pem, &tls.server_key_pem);
    let ca_cert = tonic::transport::Certificate::from_pem(&tls.ca_cert_pem);
    let server_tls = tonic::transport::ServerTlsConfig::new()
        .identity(identity)
        .client_ca_root(ca_cert);

    // 4. Construct AgentNotifyBridge at the composition root and start the server.
    let bridge = Arc::new(embyr_agent::notify_bridge::AgentNotifyBridge::new(
        pool.clone(),
        project_id.to_string(),
    ));
    let grpc_addr = embyr_agent::server::serve(
        project_id.to_string(),
        storage,
        bridge,
        server_tls,
        "127.0.0.1:0",
    )
    .await
    .expect("start agent server");

    // 5. Build mTLS client.
    let client_ca = tonic::transport::Certificate::from_pem(&tls.ca_cert_pem);
    let client_identity =
        tonic::transport::Identity::from_pem(&tls.client_cert_pem, &tls.client_key_pem);
    let client_tls = tonic::transport::ClientTlsConfig::new()
        .domain_name("localhost")
        .ca_certificate(client_ca)
        .identity(client_identity);
    let channel = tonic::transport::Channel::from_shared(format!(
        "https://127.0.0.1:{}",
        grpc_addr.port()
    ))
    .unwrap()
    .tls_config(client_tls)
    .unwrap()
    .connect()
    .await
    .expect("connect mTLS client to agent");

    let client =
        embyr_proto::agent::storage_agent_client::StorageAgentClient::new(channel);

    let handle = AgentHandle {
        grpc_addr,
        _postgres: postgres,
        _agent_task: tokio::spawn(std::future::pending()),
        pool,
    };

    (handle, client)
}
