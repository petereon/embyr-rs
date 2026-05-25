// embyr-server: composition root and gRPC adapter layer.
pub mod adapters;
pub mod encoding;
pub mod grpc;
pub mod middleware;

use std::sync::Arc;

use adapters::{credential_cache::CredentialCache, system_db::SystemDb};
use embyr_proto::firestore::firestore_server::FirestoreServer;
use grpc::handler::FirestoreService;
use grpc::healthz::healthz_handler;

/// Handle to an in-process test server bound on ephemeral ports.
///
/// Holds both the gRPC and REST addresses.  Sending on the shutdown channel
/// (via `Drop`) causes both servers to stop.
pub struct TestServer {
    pub grpc_addr: std::net::SocketAddr,
    pub rest_addr: std::net::SocketAddr,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Start an in-process server exposing both gRPC (tonic) and REST (axum) on
/// ephemeral ports.
///
/// Both servers share a single shutdown signal: dropping the returned
/// `TestServer` sends a `()` on the oneshot channel, which the `tokio::select!`
/// in the spawned task handles.
pub async fn start_test_server(system_db: Arc<SystemDb>) -> TestServer {
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
    let service = FirestoreService {
        system_db,
        credential_cache: cache,
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
        shutdown_tx: Some(shutdown_tx),
    }
}
