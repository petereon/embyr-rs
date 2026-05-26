// embyr-server: composition root and gRPC adapter layer.
pub mod adapters;
pub mod admin;
pub mod encoding;
pub mod grpc;
pub mod middleware;
pub mod realtime;
pub mod rest;
pub mod transactions;

use std::sync::Arc;

use adapters::{
    aws_secret_fetcher::AwsSecretFetcher,
    credential_cache::CredentialCache,
    index_manager::IndexManager,
    metrics_adapter::MetricsAdapter,
    system_db::SystemDb,
};
use embyr_proto::firestore::firestore_server::FirestoreServer;
use grpc::handler::FirestoreService;
use grpc::healthz::healthz_handler;
use realtime::listen_registry::ListenRegistry;
use rest::browser_channel::{BrowserChannelState, browser_channel_get, browser_channel_post};
use rest::grpc_web::spawn_hybrid_server;

/// Handle to an in-process test server bound on ephemeral ports.
///
/// Holds both the gRPC, REST, and admin addresses.  Sending on the shutdown
/// channel (via `Drop`) causes all servers to stop.
pub struct TestServer {
    pub grpc_addr: std::net::SocketAddr,
    pub rest_addr: std::net::SocketAddr,
    pub admin_addr: std::net::SocketAddr,
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
    let admin_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind admin ephemeral port");

    let grpc_addr = grpc_listener.local_addr().expect("get gRPC local addr");
    let rest_addr = rest_listener.local_addr().expect("get REST local addr");
    let admin_addr = admin_listener.local_addr().expect("get admin local addr");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let cache = Arc::new(CredentialCache::new(256));
    let cache_for_admin = Arc::clone(&cache);
    let idx_mgr = Arc::new(IndexManager::new(system_db.pool().clone()));
    let metrics = Arc::new(MetricsAdapter::new(system_db.pool().clone()));
    let listen_registry = ListenRegistry::new();
    let listen_registry_clone = Arc::clone(&listen_registry);
    let active_listeners = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let service = FirestoreService {
        system_db: Arc::clone(&system_db),
        credential_cache: cache,
        index_manager: idx_mgr,
        metrics_adapter: metrics,
        keepalive_interval: keepalive,
        listen_registry,
        active_listeners,
        aws_secret_fetcher: None,
    };

    // Clone the service for the gRPC-Web port (:8081).
    let service_for_rest = service.clone();

    // Build the axum portion of the REST app (healthz + BrowserChannel).
    // The hybrid server dispatches gRPC-Web requests to tonic and all other
    // requests to this axum router.
    let bc_state = BrowserChannelState::new();
    let axum_app = axum::Router::new()
        .route("/healthz", axum::routing::get(healthz_handler))
        .route(
            "/channel",
            axum::routing::get(browser_channel_get).post(browser_channel_post),
        )
        .with_state(bc_state);

    let rest_task = spawn_hybrid_server(rest_listener, service_for_rest, axum_app);

    let admin_app = admin::router::build_with_aws(
        system_db,
        "test-admin-key-secret".to_string(),
        cache_for_admin,
        None,
    );

    tokio::spawn(async move {
        let grpc_incoming =
            tokio_stream::wrappers::TcpListenerStream::new(grpc_listener);

        let grpc_fut = tonic::transport::Server::builder()
            .add_service(FirestoreServer::new(service))
            .serve_with_incoming(grpc_incoming);

        let admin_fut = axum::serve(admin_listener, admin_app);

        tokio::select! {
            _ = grpc_fut => {},
            _ = rest_task => {},
            _ = admin_fut => {},
            _ = async { let _ = shutdown_rx.await; } => {},
        }
    });

    // Brief yield to let all servers reach their accept loops before returning.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    TestServer {
        grpc_addr,
        rest_addr,
        admin_addr,
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

/// Start an in-process server with an injected `AwsSecretFetcher` (for US-10 tests).
pub async fn start_test_server_with_aws_fetcher(
    system_db: Arc<SystemDb>,
    keepalive: std::time::Duration,
    aws_fetcher: Arc<AwsSecretFetcher>,
) -> TestServer {
    let grpc_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gRPC ephemeral port");
    let rest_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind REST ephemeral port");
    let admin_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind admin ephemeral port");

    let grpc_addr = grpc_listener.local_addr().expect("get gRPC local addr");
    let rest_addr = rest_listener.local_addr().expect("get REST local addr");
    let admin_addr = admin_listener.local_addr().expect("get admin local addr");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let cache = Arc::new(CredentialCache::new(256));
    let cache_for_admin = Arc::clone(&cache);
    let idx_mgr = Arc::new(IndexManager::new(system_db.pool().clone()));
    let metrics = Arc::new(MetricsAdapter::new(system_db.pool().clone()));
    let listen_registry = ListenRegistry::new();
    let listen_registry_clone = Arc::clone(&listen_registry);
    let active_listeners = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let service = FirestoreService {
        system_db: Arc::clone(&system_db),
        credential_cache: cache,
        index_manager: idx_mgr,
        metrics_adapter: metrics,
        keepalive_interval: keepalive,
        listen_registry,
        active_listeners,
        aws_secret_fetcher: Some(Arc::clone(&aws_fetcher)),
    };

    let service_for_rest = service.clone();

    let bc_state = rest::browser_channel::BrowserChannelState::new();
    let axum_app = axum::Router::new()
        .route("/healthz", axum::routing::get(grpc::healthz::healthz_handler))
        .route(
            "/channel",
            axum::routing::get(rest::browser_channel::browser_channel_get)
                .post(rest::browser_channel::browser_channel_post),
        )
        .with_state(bc_state);

    let rest_task = rest::grpc_web::spawn_hybrid_server(rest_listener, service_for_rest, axum_app);

    let admin_app = admin::router::build_with_aws(
        system_db,
        "test-admin-key-secret".to_string(),
        cache_for_admin,
        Some(aws_fetcher),
    );

    tokio::spawn(async move {
        let grpc_incoming =
            tokio_stream::wrappers::TcpListenerStream::new(grpc_listener);

        let grpc_fut = tonic::transport::Server::builder()
            .add_service(embyr_proto::firestore::firestore_server::FirestoreServer::new(service))
            .serve_with_incoming(grpc_incoming);

        let admin_fut = axum::serve(admin_listener, admin_app);

        tokio::select! {
            _ = grpc_fut => {},
            _ = rest_task => {},
            _ = admin_fut => {},
            _ = async { let _ = shutdown_rx.await; } => {},
        }
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    TestServer {
        grpc_addr,
        rest_addr,
        admin_addr,
        listen_registry: listen_registry_clone,
        shutdown_tx: Some(shutdown_tx),
    }
}
