// embyr-server: composition root and gRPC adapter layer.
pub mod adapters;
pub mod encoding;
pub mod grpc;
pub mod middleware;
pub mod realtime;

use std::sync::Arc;

use adapters::{credential_cache::CredentialCache, index_manager::IndexManager, system_db::SystemDb};
use embyr_proto::firestore::firestore_server::FirestoreServer;
use grpc::handler::FirestoreService;
use grpc::healthz::healthz_handler;
use realtime::listen_registry::ListenRegistry;

/// Handle to an in-process test server bound on ephemeral ports.
///
/// Holds both the gRPC and REST addresses.  Sending on the shutdown channel
/// (via `Drop`) causes both servers to stop.
pub struct TestServer {
    pub grpc_addr: std::net::SocketAddr,
    pub rest_addr: std::net::SocketAddr,
    /// Shared listen registry — exposed for test-only overflow simulation.
    pub listen_registry: Arc<realtime::listen_registry::ListenRegistry>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Start an in-process server with a configurable Listen keep-alive interval.
///
/// Use this variant in tests that exercise the keep-alive path — pass a short
/// duration (e.g. `Duration::from_millis(500)`) so tests complete quickly.
pub async fn start_test_server_with_keepalive(
    system_db: Arc<SystemDb>,
    keepalive: std::time::Duration,
) -> TestServer {
    let grpc_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gRPC ephemeral port");
    let rest_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind REST ephemeral port");

    let grpc_addr = grpc_listener.local_addr().expect("get gRPC local addr");
    let rest_addr = rest_listener.local_addr().expect("get REST local addr");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let cache = Arc::new(CredentialCache::new(256));
    let idx_mgr = Arc::new(IndexManager::new(system_db.pool().clone()));
    let listen_registry = ListenRegistry::new();
    let listen_registry_clone = Arc::clone(&listen_registry);
    let active_listeners = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let service = FirestoreService {
        system_db,
        credential_cache: cache,
        index_manager: idx_mgr,
        keepalive_interval: keepalive,
        listen_registry,
        active_listeners,
    };

    let rest_app = axum::Router::new()
        .route("/healthz", axum::routing::get(healthz_handler));

    tokio::spawn(async move {
        let grpc_incoming =
            tokio_stream::wrappers::TcpListenerStream::new(grpc_listener);

        let grpc_fut = tonic::transport::Server::builder()
            .add_service(FirestoreServer::new(service))
            .serve_with_incoming(grpc_incoming);

        let rest_fut = axum::serve(rest_listener, rest_app);

        tokio::select! {
            _ = grpc_fut => {},
            _ = rest_fut => {},
            _ = async { let _ = shutdown_rx.await; } => {},
        }
    });

    // Brief yield to let both servers reach their accept loops before returning.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    TestServer {
        grpc_addr,
        rest_addr,
        listen_registry: listen_registry_clone,
        shutdown_tx: Some(shutdown_tx),
    }
}

/// Start an in-process server with the production keep-alive interval (30s).
///
/// Both servers share a single shutdown signal: dropping the returned
/// `TestServer` sends a `()` on the oneshot channel.
pub async fn start_test_server(system_db: Arc<SystemDb>) -> TestServer {
    start_test_server_with_keepalive(system_db, std::time::Duration::from_secs(30)).await
}
