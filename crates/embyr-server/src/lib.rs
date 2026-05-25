// embyr-server: composition root and gRPC adapter layer.
pub mod adapters;
pub mod encoding;
pub mod grpc;
pub mod middleware;

use std::sync::Arc;

use adapters::{credential_cache::CredentialCache, system_db::SystemDb};
use embyr_proto::firestore::firestore_server::FirestoreServer;
use grpc::handler::FirestoreService;

/// Handle to an in-process test server bound on an ephemeral port.
///
/// Sending a message on the shutdown channel (via `Drop`) causes the
/// tonic server to perform graceful shutdown.
pub struct TestServer {
    pub grpc_addr: std::net::SocketAddr,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Start an in-process tonic server on an ephemeral port.
///
/// Binds a `TcpListener` on `:0`, hands it directly to tonic via
/// `serve_with_incoming_shutdown` to avoid "address already in use" races,
/// and returns a `TestServer` handle.  The server shuts down when the
/// `TestServer` is dropped.
pub async fn start_test_server(system_db: Arc<SystemDb>) -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind to ephemeral port");
    let addr = listener.local_addr().expect("get local addr");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let cache = Arc::new(CredentialCache::new(256));
    let service = FirestoreService {
        system_db,
        credential_cache: cache,
    };

    tokio::spawn(async move {
        let incoming =
            tokio_stream::wrappers::TcpListenerStream::new(listener);
        tonic::transport::Server::builder()
            .add_service(FirestoreServer::new(service))
            .serve_with_incoming_shutdown(incoming, async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("tonic server error");
    });

    // Brief yield to let the server reach its accept loop before returning.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    TestServer {
        grpc_addr: addr,
        shutdown_tx: Some(shutdown_tx),
    }
}
