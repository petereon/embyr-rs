//! gRPC-Web + axum hybrid server on :8081.
//!
//! # Problem
//! `tonic::transport::Server::into_router()` converts tonic routes into an axum
//! Router but DROPS the tower layers (including `GrpcWebLayer`) from the server
//! stack.  As a result, gRPC-Web requests are handled as native gRPC.
//!
//! # Solution
//! Run a per-connection content-type dispatcher: requests with
//! `Content-Type: application/grpc-web*` are forwarded to the tonic service
//! (wrapped with `tonic_web::enable`); all other requests are forwarded to the
//! axum service (BrowserChannel + healthz).
//!
//! The dispatcher is a `tower::Service<Request<Incoming>>` adapted to hyper 1 via
//! `hyper_util::service::TowerToHyperService`.

use std::{
    convert::Infallible,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use axum::body::Body;
use embyr_proto::firestore::firestore_server::FirestoreServer;
use http::Request;
use http_body_util::BodyExt as _;
use hyper::body::Incoming;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    service::TowerToHyperService,
};
use tonic_web::GrpcWebLayer;
use tower::{Layer as _, Service, ServiceExt as _};

use crate::grpc::handler::FirestoreService;

// ── Body adapters ──────────────────────────────────────────────────────────

fn incoming_to_axum_body(incoming: Incoming) -> Body {
    Body::new(incoming.map_err(axum::Error::new))
}

fn incoming_to_tonic_body(incoming: Incoming) -> tonic::body::BoxBody {
    tonic::body::BoxBody::new(
        incoming.map_err(|e| tonic::Status::internal(format!("body error: {e}"))),
    )
}

// ── HybridService ─────────────────────────────────────────────────────────

type TonicService = <GrpcWebLayer as tower::Layer<
    FirestoreServer<FirestoreService>,
>>::Service;

/// Per-connection dispatcher: gRPC-Web requests → tonic, others → axum.
#[derive(Clone)]
pub struct HybridService {
    grpc_web: TonicService,
    axum: axum::Router,
}

impl HybridService {
    fn new(grpc_service: FirestoreService, axum_app: axum::Router) -> Self {
        let grpc_web = GrpcWebLayer::new().layer(FirestoreServer::new(grpc_service));
        Self {
            grpc_web,
            axum: axum_app,
        }
    }
}

impl Service<Request<Incoming>> for HybridService {
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Incoming>) -> Self::Future {
        let is_grpc_web = req
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|ct| ct.starts_with("application/grpc-web"))
            .unwrap_or(false);

        if is_grpc_web {
            let mut svc = self.grpc_web.clone();
            Box::pin(async move {
                let req = req.map(incoming_to_tonic_body);
                let resp = svc.ready().await.unwrap().call(req).await.unwrap_or_else(|_| {
                    http::Response::builder()
                        .status(500)
                        .body(tonic::body::empty_body())
                        .unwrap()
                });
                Ok(resp.map(Body::new))
            })
        } else {
            let svc = self.axum.clone();
            Box::pin(async move {
                let req = req.map(incoming_to_axum_body);
                let resp = svc.oneshot(req).await.unwrap_or_else(|_| {
                    http::Response::builder()
                        .status(500)
                        .body(Body::empty())
                        .unwrap()
                });
                Ok(resp)
            })
        }
    }
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Spawn the hybrid gRPC-Web + axum server on `listener`.
///
/// Every accepted connection runs a content-type dispatcher:
/// - `application/grpc-web*` → tonic gRPC-Web service (layer preserved)
/// - everything else → axum (BrowserChannel, healthz)
pub fn spawn_hybrid_server(
    listener: tokio::net::TcpListener,
    grpc_service: FirestoreService,
    axum_app: axum::Router,
) -> tokio::task::JoinHandle<()> {
    let hybrid = HybridService::new(grpc_service, axum_app);
    tokio::spawn(async move {
        loop {
            let (stream, _peer) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => break,
            };
            let io = TokioIo::new(stream);
            let svc = TowerToHyperService::new(hybrid.clone());
            let builder =
                hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
            tokio::spawn(async move {
                builder
                    .serve_connection_with_upgrades(io, svc)
                    .await
                    .ok();
            });
        }
    })
}
