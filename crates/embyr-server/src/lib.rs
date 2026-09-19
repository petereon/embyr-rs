// embyr-server: composition root and gRPC adapter layer.
pub mod adapters;
pub mod admin;
pub mod config;
pub mod encoding;
pub mod grpc;
pub mod middleware;
pub mod observability;
pub mod realtime;
pub mod rest;
pub mod sweepers;

use std::sync::Arc;

use adapters::{
    aws_secret_fetcher::AwsSecretFetcher,
    credential_cache::CredentialCache,
    email::NoopEmailSender,
    gcp_secret_fetcher::GcpSecretFetcher,
    index_manager::IndexManager,
    metrics_adapter::MetricsAdapter,
    postgres_notify_listener::PostgresNotifyListener,
    system_db::SystemDb,
};
use embyr_core::admin::email::IEmailSender;
use middleware::rate_limit::RateLimiter;
use embyr_proto::firestore::firestore_server::FirestoreServer;
use grpc::handler::FirestoreService;
use realtime::listen_registry::ListenRegistry;

// request-body-size-limits (finding #24, ADR-081): explicit, deliberately-
// chosen byte ceilings for the gRPC (native + gRPC-Web) and REST listener
// surfaces, replacing tonic's/axum's implicit library defaults. No env var
// (ADR-081: protocol/payload-shape security ceilings, not operator-tunable
// traffic knobs). `pub(crate)` so `rest::grpc_web` can share the identical
// gRPC constant rather than duplicating the value.
pub(crate) const MAX_GRPC_MESSAGE_BYTES: usize = 10 * 1024 * 1024; // 10 MiB
const MAX_REST_BODY_BYTES: usize = 2 * 1024 * 1024; // 2 MiB

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
    /// Shared rate limiter — exposed for test inspection.
    pub rate_limiter: Arc<RateLimiter>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

// ---------------------------------------------------------------------------
// accounts:<verb> REST bridge dispatch (client-auth + client-auth-hosted-identity)
// ---------------------------------------------------------------------------

/// Combined state for the single `/v1/projects/:project_id/accounts:action`
/// route — see the wiring comment in `spawn_all_servers` for why
/// `signInWithCustomToken` and `signUp` cannot be registered as two separate
/// matchit routes.
#[derive(Clone)]
struct AccountsBridgeState {
    sign_in: rest::sign_in::SignInState,
    hosted_identity: rest::sign_up::HostedIdentityState,
    // oauth-providers (US-02, ADR-037 Decision 6): one new state field for
    // the fifth accounts:<verb> dispatch arm, `signInWithIdp`.
    oauth_provider: rest::sign_in_with_idp::OAuthProviderState,
    // anonymous-sessions (US-02, ADR-043 Decision 5): one new state field
    // for the sixth accounts:<verb> dispatch arm, `signInAnonymously`.
    anonymous_identity: rest::sign_in_anonymously::AnonymousIdentityState,
}

/// Dispatches on the captured `action` param (the literal text after
/// `accounts:` in the request path, e.g. `signInWithCustomToken` or
/// `signUp`) to each feature's own, otherwise-untouched handler function —
/// called directly as a plain async fn, not re-routed through axum.
async fn accounts_bridge_dispatch(
    axum::extract::Path(params): axum::extract::Path<std::collections::HashMap<String, String>>,
    axum::extract::State(state): axum::extract::State<AccountsBridgeState>,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
    body_bytes: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let action = params
        .get("action")
        .map(|s| s.trim_start_matches(':'))
        .unwrap_or_default();

    match action {
        "signInWithCustomToken" => {
            let body: rest::sign_in::SignInWithCustomTokenBody =
                serde_json::from_slice(&body_bytes).unwrap_or(rest::sign_in::SignInWithCustomTokenBody {
                    token: None,
                });
            rest::sign_in::sign_in_with_custom_token(
                axum::extract::Path(params),
                axum::extract::State(state.sign_in),
                axum::extract::Json(body),
            )
            .await
            .into_response()
        }
        "signUp" => {
            let body: rest::sign_up::SignUpBody = serde_json::from_slice(&body_bytes)
                .unwrap_or(rest::sign_up::SignUpBody {
                    email: None,
                    password: None,
                });
            rest::sign_up::sign_up(
                axum::extract::Path(params),
                axum::extract::State(state.hosted_identity),
                axum::extract::Query(rest::sign_up::SignUpQuery {
                    key: query.get("key").cloned(),
                }),
                axum::extract::Json(body),
            )
            .await
        }
        // client-auth-hosted-identity (US-03, ADR-036): shares the SAME
        // `HostedIdentityState` field as `signUp` above — one Customer DB
        // resolution surface for the whole hosted-identity accounts:<verb>
        // family, no new state field needed.
        "signInWithPassword" => {
            let body: rest::sign_in_with_password::SignInWithPasswordBody =
                serde_json::from_slice(&body_bytes).unwrap_or(
                    rest::sign_in_with_password::SignInWithPasswordBody {
                        email: None,
                        password: None,
                    },
                );
            rest::sign_in_with_password::sign_in_with_password(
                axum::extract::Path(params),
                axum::extract::State(state.hosted_identity),
                axum::extract::Query(rest::sign_up::SignUpQuery {
                    key: query.get("key").cloned(),
                }),
                axum::extract::Json(body),
            )
            .await
        }
        // client-auth-hosted-identity (US-04, ADR-036): shares the SAME
        // `HostedIdentityState` field as `signUp`/`signInWithPassword` above.
        "sendOobCode" => {
            let body: rest::reset_password::SendOobCodeBody = serde_json::from_slice(&body_bytes)
                .unwrap_or(rest::reset_password::SendOobCodeBody { email: None });
            rest::reset_password::send_oob_code(
                axum::extract::Path(params),
                axum::extract::State(state.hosted_identity),
                axum::extract::Query(rest::sign_up::SignUpQuery {
                    key: query.get("key").cloned(),
                }),
                axum::extract::Json(body),
            )
            .await
        }
        "resetPassword" => {
            let body: rest::reset_password::ResetPasswordBody =
                serde_json::from_slice(&body_bytes).unwrap_or(
                    rest::reset_password::ResetPasswordBody {
                        oob_code: None,
                        new_password: None,
                    },
                );
            rest::reset_password::reset_password(
                axum::extract::Path(params),
                axum::extract::State(state.hosted_identity),
                axum::extract::Query(rest::sign_up::SignUpQuery {
                    key: query.get("key").cloned(),
                }),
                axum::extract::Json(body),
            )
            .await
        }
        // oauth-providers (US-02, ADR-037 Decision 6): Maria signs in with
        // her Google account — no `?key=` query param (Resolution 3(B)
        // stateless, ADR-037's own positive consequence), mirrors
        // `signInWithCustomToken`'s identical no-query-param shape.
        "signInWithIdp" => {
            let body: rest::sign_in_with_idp::SignInWithIdpBody = serde_json::from_slice(&body_bytes)
                .unwrap_or(rest::sign_in_with_idp::SignInWithIdpBody { id_token: None });
            rest::sign_in_with_idp::sign_in_with_idp(
                axum::extract::Path(params),
                axum::extract::State(state.oauth_provider),
                axum::extract::Json(body),
            )
            .await
        }
        // anonymous-sessions (US-02, ADR-043 Decision 5): Maria gets a real
        // identity with zero prior credential — no request body fields at
        // all (Resolution 3, stateless), `?key=` required (mirrors
        // `signUp`'s query-param shape, unlike `signInWithIdp`'s none).
        "signInAnonymously" => {
            rest::sign_in_anonymously::sign_in_anonymously(
                axum::extract::Path(params),
                axum::extract::State(state.anonymous_identity),
                axum::extract::Query(rest::sign_in_anonymously::SignInAnonymouslyQuery {
                    key: query.get("key").cloned(),
                }),
            )
            .await
        }
        _ => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Read `EMBYR_RATE_LIMIT_RPS` from the environment (default 1000.0).
///
/// Parsed once per function call — callers cache the value in a local variable.
/// Returns the default if the variable is absent, non-numeric, non-finite, or ≤ 0.
fn default_rate_limit_capacity() -> f64 {
    std::env::var("EMBYR_RATE_LIMIT_RPS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(1000.0)
}

/// Shared wiring allocated by every test server variant.
struct TestComponents {
    grpc_listener: tokio::net::TcpListener,
    rest_listener: tokio::net::TcpListener,
    admin_listener: tokio::net::TcpListener,
    grpc_addr: std::net::SocketAddr,
    rest_addr: std::net::SocketAddr,
    admin_addr: std::net::SocketAddr,
    cache: Arc<CredentialCache>,
    cache_for_admin: Arc<CredentialCache>,
    idx_mgr: Arc<IndexManager>,
    metrics: Arc<MetricsAdapter>,
    listen_registry: Arc<ListenRegistry>,
    active_listeners: Arc<tokio::sync::Mutex<std::collections::HashMap<String, PostgresNotifyListener>>>,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
}

/// Bind three ephemeral TCP listeners and allocate shared service components.
async fn alloc_test_components(system_db: &Arc<SystemDb>) -> TestComponents {
    // Install Prometheus recorder before any listener opens (AC-OBS-01-05).
    // OnceLock ensures install_recorder() is called at most once across all
    // test server constructors that share the same process.
    observability::get_or_install_prometheus_handle();

    // OBS-05: spawn pool gauge background task (15-second interval).
    // Runs for the duration of the test process; no shutdown needed since
    // gauge values are informational and the background task is fire-and-forget.
    {
        let pool_for_metrics = system_db.pool().clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(15));
            loop {
                interval.tick().await;
                metrics::gauge!("embyr_pg_pool_size", "pool" => "system")
                    .set(pool_for_metrics.size() as f64);
                metrics::gauge!("embyr_pg_pool_idle", "pool" => "system")
                    .set(pool_for_metrics.num_idle() as f64);
            }
        });
    }

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
    let active_listeners = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

    TestComponents {
        grpc_listener,
        rest_listener,
        admin_listener,
        grpc_addr,
        rest_addr,
        admin_addr,
        cache,
        cache_for_admin,
        idx_mgr,
        metrics,
        listen_registry,
        active_listeners,
        shutdown_tx,
        shutdown_rx,
    }
}

/// Spawn the gRPC, REST hybrid, and admin servers.
///
/// All three share a single `shutdown_rx` oneshot; dropping the returned
/// `TestServer` sends the shutdown signal.
///
/// Returns a `JoinHandle` that resolves once shutdown has been requested AND
/// every gRPC connection that was already in flight (e.g. a still-open
/// `Listen` stream) has been genuinely drained — not merely after a fixed
/// delay. Callers that need to guarantee in-flight requests complete before
/// process exit (D-PR-5) must `.await` this handle after sending the
/// shutdown signal; callers that only need the servers running (test
/// harnesses) may ignore the returned handle.
///
/// The gRPC server runs its own graceful-shutdown future
/// (`serve_with_incoming_shutdown`) as an independent task so that, when the
/// outer signal fires, it stops accepting *new* connections but keeps
/// serving already-open streams/connections until they close naturally.
// oauth-providers (US-02, ADR-037 Decision 6/7): three new trailing
// parameters wire OAuthProviderState's own fields — mirrors this function's
// established precedent of growing by parameter (`email_sender` was added
// the same way) rather than introducing a second wiring mechanism.
#[allow(clippy::too_many_arguments)]
pub fn spawn_all_servers(
    grpc_listener: tokio::net::TcpListener,
    rest_listener: tokio::net::TcpListener,
    admin_listener: tokio::net::TcpListener,
    service: FirestoreService,
    admin_app: axum::Router,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    email_sender: Arc<dyn IEmailSender + Send + Sync>,
    encryption_key: [u8; 32],
    encryption_key_previous: Option<[u8; 32]>,
    google_jwks_cache: Arc<adapters::google_jwks_cache::GoogleJwksCache>,
    tonic_tls_config: Option<tonic::transport::ServerTlsConfig>,
    tls_acceptor: Option<tokio_rustls::TlsAcceptor>,
    cors_allowed_origins: Vec<String>,
) -> tokio::task::JoinHandle<()> {
    let service_for_rest = service.clone();

    // healthz-dependency-checks (ADR-078): `/livez`/`/healthz` need a
    // different state type (`Arc<SystemDb>`) than the rest of `axum_app`
    // (`BrowserChannelState`) — cloned from `service` BEFORE `service` moves
    // into `FirestoreServer::new(service)` below, resolved to `Router<()>`
    // via `.with_state()`, then merged, mirroring `accounts_bridge_app`'s
    // own established pattern in this exact function.
    let readyz_state = Arc::clone(&service.system_db);
    let healthz_app = axum::Router::new()
        .route("/livez", axum::routing::get(grpc::healthz::livez_handler))
        .route("/healthz", axum::routing::get(grpc::healthz::healthz_handler))
        .with_state(readyz_state);

    let bc_state = rest::browser_channel::BrowserChannelState::new();
    let axum_app = axum::Router::new()
        .route(
            "/channel",
            axum::routing::get(rest::browser_channel::browser_channel_get)
                .post(rest::browser_channel::browser_channel_post),
        )
        .with_state(bc_state);
    let axum_app = axum_app.merge(healthz_app);

    // client-auth (US-02, ADR-026) + client-auth-hosted-identity (US-02,
    // ADR-036 Decision 6): both `accounts:signInWithCustomToken` and
    // `accounts:signUp` embed a literal `:` inside the SAME final path
    // segment ("accounts" + verb). `matchit` (axum's router) treats any `:`
    // as starting a named capture regardless of position, and disallows two
    // DIFFERENT capture names diverging from the identical static prefix
    // ("/v1/projects/:project_id/accounts") — registering both as separate
    // routes (each with its own differently-named embedded capture) fails
    // at router-build time with "insertion failed due to conflict with
    // previously registered route". Both verbs are therefore registered as
    // ONE route sharing a single capture name ("action"), dispatched by
    // `accounts_bridge_dispatch` below — which calls each module's own,
    // otherwise-untouched handler function directly (a plain async fn call,
    // not a second trip through axum's routing). Cloned from `service`
    // BEFORE `service` moves into `FirestoreServer::new(service)` below.
    let sign_in_state = rest::sign_in::SignInState {
        system_db: std::sync::Arc::clone(&service.system_db),
    };
    let hosted_identity_state = rest::sign_up::HostedIdentityState {
        system_db: std::sync::Arc::clone(&service.system_db),
        credential_cache: std::sync::Arc::clone(&service.credential_cache),
        aws_secret_fetcher: service.aws_secret_fetcher.clone(),
        gcp_secret_fetcher: service.gcp_secret_fetcher.clone(),
        email_sender,
        tenant_db_max_connections: service.tenant_db_max_connections,
        tenant_db_acquire_timeout: service.tenant_db_acquire_timeout,
    };
    let oauth_provider_state = rest::sign_in_with_idp::OAuthProviderState {
        system_db: std::sync::Arc::clone(&service.system_db),
        encryption_key,
        encryption_key_previous,
        google_jwks_cache,
    };
    // anonymous-sessions (US-02, ADR-043 Decision 5): reuses the SAME
    // encryption_key/encryption_key_previous OAuthProviderState already
    // threads above — no new spawn_all_servers parameter needed (both
    // signing-key tables share the identical AES-256-GCM/EMBYR_ENCRYPTION_KEY
    // custody mechanism, Resolution 2).
    let anonymous_identity_state = rest::sign_in_anonymously::AnonymousIdentityState {
        system_db: std::sync::Arc::clone(&service.system_db),
        encryption_key,
        encryption_key_previous,
    };
    let accounts_bridge_state = AccountsBridgeState {
        sign_in: sign_in_state,
        hosted_identity: hosted_identity_state,
        oauth_provider: oauth_provider_state,
        anonymous_identity: anonymous_identity_state,
    };
    // Rate-limit gate (ADR-043 Decision 7 fix): same shared `Arc<RateLimiter>`
    // the gRPC side uses, applied via `route_layer` so it runs AFTER path
    // matching (i.e. `:project_id` is available to the middleware) but
    // BEFORE `accounts_bridge_dispatch` — mirrors `admin/router.rs`'s own
    // `route_layer(from_fn_with_state(...))` precedent for auth middleware.
    let accounts_bridge_app = axum::Router::new()
        .route(
            "/v1/projects/:project_id/accounts:action",
            axum::routing::post(accounts_bridge_dispatch),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            Arc::clone(&service.rate_limiter),
            middleware::rate_limit::rest_rate_limit_middleware,
        ))
        .with_state(accounts_bridge_state);

    // cors-origin-policy (finding #21): outermost layer on the whole merged
    // :8081 router — tower_http's CorsLayer intercepts and answers preflight
    // OPTIONS itself, before axum routing runs, so it must sit outermost to
    // cover every route including any added later. Empty `cors_allowed_origins`
    // (default) ⇒ `AllowOrigin::list([])` never matches any `Origin` header ⇒
    // no `Access-Control-Allow-Origin` ever emitted — byte-identical to
    // today's no-CORS-layer behavior. `:9090` (admin_app) is untouched.
    let cors_origins: Vec<axum::http::HeaderValue> = cors_allowed_origins
        .iter()
        .map(|o| o.parse())
        .collect::<Result<_, _>>()
        .expect("cors origins validated in ServerConfig::from_env()");
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::list(cors_origins))
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST, axum::http::Method::OPTIONS])
        .allow_headers([axum::http::header::CONTENT_TYPE])
        .allow_credentials(false);
    // request-body-size-limits (finding #24, ADR-081): explicit 2 MiB
    // ceiling on the whole merged :8081 REST router — matches today's
    // implicit axum `Bytes`/`Json` extractor default exactly (zero
    // behavior change, now explicit/tested).
    let axum_app = axum_app
        .merge(accounts_bridge_app)
        .layer(axum::extract::DefaultBodyLimit::max(MAX_REST_BODY_BYTES))
        .layer(cors);

    let rest_task = rest::grpc_web::spawn_hybrid_server(
        rest_listener,
        service_for_rest,
        axum_app,
        tls_acceptor.clone(),
    );

    tokio::spawn(async move {
        let grpc_incoming = tokio_stream::wrappers::TcpListenerStream::new(grpc_listener);

        // Own shutdown signal for the gRPC server: firing it tells tonic to
        // stop accepting new connections while letting already-open streams
        // (e.g. a long-lived Listen subscription) run to completion, instead
        // of aborting them the instant the outer signal below resolves.
        let (grpc_shutdown_tx, grpc_shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let mut grpc_builder = tonic::transport::Server::builder();
        if let Some(tls) = tonic_tls_config {
            // Safe to .expect(): the SAME PEM bytes already passed
            // rustls::ServerConfig::builder()...with_single_cert(...) inside
            // load_tls_material() during from_env() — this cannot fail on
            // content that already-succeeded parse/validate produced.
            grpc_builder = grpc_builder
                .tls_config(tls)
                .expect("tls config already validated in ServerConfig::from_env()");
        }
        let firestore_svc =
            FirestoreServer::new(service).max_decoding_message_size(MAX_GRPC_MESSAGE_BYTES);
        let grpc_fut = grpc_builder
            .add_service(firestore_svc)
            .serve_with_incoming_shutdown(grpc_incoming, async {
                let _ = grpc_shutdown_rx.await;
            });

        // Drive the gRPC server as its own task so it keeps accepting and
        // servicing connections concurrently with rest/admin below, and so
        // it can keep draining after this task moves on to the join below.
        let grpc_handle = tokio::spawn(grpc_fut);

        let admin_fut = spawn_admin_server(admin_listener, admin_app, tls_acceptor);

        tokio::select! {
            _ = rest_task => {},
            _ = admin_fut => {},
            _ = async { let _ = shutdown_rx.await; } => {},
        }

        // Stop accepting new gRPC connections; wait for in-flight requests
        // on already-open connections to finish naturally before returning.
        let _ = grpc_shutdown_tx.send(());
        let _ = grpc_handle.await;
    })
}

/// Serve `admin_app` on `listener`, optionally TLS-wrapped. Mirrors
/// `rest::grpc_web::spawn_hybrid_server`'s own accept-loop shape and
/// reuses its EXACT `accept_maybe_tls` handshake step — the Admin listener
/// previously used `axum::serve`, which owns its own accept loop
/// internally and has no TLS hook.
fn spawn_admin_server(
    listener: tokio::net::TcpListener,
    admin_app: axum::Router,
    tls_acceptor: Option<tokio_rustls::TlsAcceptor>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => break,
            };
            let app = admin_app.clone();
            let tls_acceptor = tls_acceptor.clone();
            tokio::spawn(async move {
                let io = match adapters::tls::accept_maybe_tls(stream, tls_acceptor.as_ref()).await
                {
                    Ok(io) => hyper_util::rt::TokioIo::new(io),
                    Err(_) => return,
                };
                // Reuses grpc_web's own incoming_to_axum_body (now pub(crate))
                // — identical Incoming→axum::body::Body conversion HybridService's
                // non-grpc-web branch already performs.
                let svc = tower::service_fn(move |req: http::Request<hyper::body::Incoming>| {
                    let app = app.clone();
                    async move {
                        // admin-signin-hardening (D-ASH-5, ADR-076): insert the
                        // real peer SocketAddr as a ConnectInfo extension so
                        // signin's own ConnectInfo<SocketAddr> extractor
                        // resolves correctly — this hand-rolled accept loop
                        // has no axum::serve/IntoMakeServiceWithConnectInfo
                        // equivalent to do it automatically.
                        let mut req = req.map(rest::grpc_web::incoming_to_axum_body);
                        req.extensions_mut()
                            .insert(axum::extract::ConnectInfo(peer));
                        Ok::<_, std::convert::Infallible>(
                            tower::ServiceExt::oneshot(app, req).await.unwrap_or_else(
                                |_: std::convert::Infallible| {
                                    http::Response::builder()
                                        .status(500)
                                        .body(axum::body::Body::empty())
                                        .unwrap()
                                },
                            ),
                        )
                    }
                });
                hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                    .serve_connection_with_upgrades(
                        io,
                        hyper_util::service::TowerToHyperService::new(svc),
                    )
                    .await
                    .ok();
            });
        }
    })
}

// ---------------------------------------------------------------------------
// Public test server constructors
// ---------------------------------------------------------------------------

/// Start an in-process server with a configurable Listen keep-alive interval.
///
/// Use this variant in tests that exercise the keep-alive path — pass a short
/// duration (e.g. `Duration::from_millis(500)`) so tests complete quickly.
pub async fn start_test_server_with_keepalive(
    system_db: Arc<SystemDb>,
    keepalive: std::time::Duration,
) -> TestServer {
    let c = alloc_test_components(&system_db).await;
    let listen_registry_ret = Arc::clone(&c.listen_registry);

    let rate_limit_rps = default_rate_limit_capacity();
    let rate_limiter = RateLimiter::new(rate_limit_rps, rate_limit_rps);
    let rate_limiter_ret = Arc::clone(&rate_limiter);

    let service = FirestoreService {
        system_db: Arc::clone(&system_db),
        credential_cache: c.cache,
        index_manager: c.idx_mgr,
        metrics_adapter: c.metrics,
        keepalive_interval: keepalive,
        listen_registry: c.listen_registry,
        active_listeners: c.active_listeners,
        aws_secret_fetcher: None,
        gcp_secret_fetcher: None,
        rate_limiter,
        tenant_db_max_connections: 5,
        tenant_db_acquire_timeout: std::time::Duration::from_secs(5),
        listener_db_max_connections: 2,
        listener_db_acquire_timeout: std::time::Duration::from_secs(5),
    };

    let admin_app = admin::router::build_with_aws(
        system_db,
        "test-admin-key-secret".to_string(),
        c.cache_for_admin,
        None,
    );

    spawn_all_servers(
        c.grpc_listener, c.rest_listener, c.admin_listener,
        service, admin_app, c.shutdown_rx, Arc::new(NoopEmailSender),
        [0u8; 32], None,
        Arc::new(adapters::google_jwks_cache::GoogleJwksCache::production()),
        None, None,
        Vec::new(),
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    TestServer {
        grpc_addr: c.grpc_addr,
        rest_addr: c.rest_addr,
        admin_addr: c.admin_addr,
        listen_registry: listen_registry_ret,
        rate_limiter: rate_limiter_ret,
        shutdown_tx: Some(c.shutdown_tx),
    }
}

/// Start an in-process server with an explicit `EMBYR_CORS_ALLOWED_ORIGINS`
/// allowlist (cors-origin-policy, finding #21). Production keep-alive
/// interval (30s); mirrors `start_test_server`'s shape otherwise.
pub async fn start_test_server_with_cors_origins(
    system_db: Arc<SystemDb>,
    cors_allowed_origins: Vec<String>,
) -> TestServer {
    let c = alloc_test_components(&system_db).await;
    let listen_registry_ret = Arc::clone(&c.listen_registry);

    let rate_limit_rps = default_rate_limit_capacity();
    let rate_limiter = RateLimiter::new(rate_limit_rps, rate_limit_rps);
    let rate_limiter_ret = Arc::clone(&rate_limiter);

    let service = FirestoreService {
        system_db: Arc::clone(&system_db),
        credential_cache: c.cache,
        index_manager: c.idx_mgr,
        metrics_adapter: c.metrics,
        keepalive_interval: std::time::Duration::from_secs(30),
        listen_registry: c.listen_registry,
        active_listeners: c.active_listeners,
        aws_secret_fetcher: None,
        gcp_secret_fetcher: None,
        rate_limiter,
        tenant_db_max_connections: 5,
        tenant_db_acquire_timeout: std::time::Duration::from_secs(5),
        listener_db_max_connections: 2,
        listener_db_acquire_timeout: std::time::Duration::from_secs(5),
    };

    // healthz-dependency-checks (ADR-078): /livez on the admin port too, so
    // the "admin port untouched by CORS" assertion has a zero-I/O endpoint
    // to hit that mirrors the data-port /livez route exactly.
    let readyz_state = Arc::clone(&system_db);
    let healthz_app = axum::Router::new()
        .route("/livez", axum::routing::get(grpc::healthz::livez_handler))
        .route("/healthz", axum::routing::get(grpc::healthz::healthz_handler))
        .with_state(readyz_state);
    let admin_app = admin::router::build_with_aws(
        system_db,
        "test-admin-key-secret".to_string(),
        c.cache_for_admin,
        None,
    )
    .merge(healthz_app);

    spawn_all_servers(
        c.grpc_listener, c.rest_listener, c.admin_listener,
        service, admin_app, c.shutdown_rx, Arc::new(NoopEmailSender),
        [0u8; 32], None,
        Arc::new(adapters::google_jwks_cache::GoogleJwksCache::production()),
        None, None,
        cors_allowed_origins,
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    TestServer {
        grpc_addr: c.grpc_addr,
        rest_addr: c.rest_addr,
        admin_addr: c.admin_addr,
        listen_registry: listen_registry_ret,
        rate_limiter: rate_limiter_ret,
        shutdown_tx: Some(c.shutdown_tx),
    }
}

/// Start an in-process server with the production keep-alive interval (30s).
///
/// Both servers share a single shutdown signal: dropping the returned
/// `TestServer` sends a `()` on the oneshot channel.
pub async fn start_test_server(system_db: Arc<SystemDb>) -> TestServer {
    start_test_server_with_keepalive(system_db, std::time::Duration::from_secs(30)).await
}

/// Start an in-process server with an injected `IEmailSender` (for
/// client-auth-hosted-identity's own hi04 reset-password tests, which need
/// to capture the reset token mailed via `accounts:sendOobCode`). Production
/// keep-alive interval (30s); admin router still uses `NoopEmailSender`
/// (admin routes are not under test here).
pub async fn start_test_server_with_email_sender(
    system_db: Arc<SystemDb>,
    email_sender: Arc<dyn IEmailSender + Send + Sync>,
) -> TestServer {
    let c = alloc_test_components(&system_db).await;
    let listen_registry_ret = Arc::clone(&c.listen_registry);

    let rate_limit_rps = default_rate_limit_capacity();
    let rate_limiter = RateLimiter::new(rate_limit_rps, rate_limit_rps);
    let rate_limiter_ret = Arc::clone(&rate_limiter);

    let service = FirestoreService {
        system_db: Arc::clone(&system_db),
        credential_cache: c.cache,
        index_manager: c.idx_mgr,
        metrics_adapter: c.metrics,
        keepalive_interval: std::time::Duration::from_secs(30),
        listen_registry: c.listen_registry,
        active_listeners: c.active_listeners,
        aws_secret_fetcher: None,
        gcp_secret_fetcher: None,
        rate_limiter,
        tenant_db_max_connections: 5,
        tenant_db_acquire_timeout: std::time::Duration::from_secs(5),
        listener_db_max_connections: 2,
        listener_db_acquire_timeout: std::time::Duration::from_secs(5),
    };

    let admin_app = admin::router::build_with_aws(
        system_db,
        "test-admin-key-secret".to_string(),
        c.cache_for_admin,
        None,
    );

    spawn_all_servers(
        c.grpc_listener, c.rest_listener, c.admin_listener,
        service, admin_app, c.shutdown_rx, email_sender,
        [0u8; 32], None,
        Arc::new(adapters::google_jwks_cache::GoogleJwksCache::production()),
        None, None,
        Vec::new(),
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    TestServer {
        grpc_addr: c.grpc_addr,
        rest_addr: c.rest_addr,
        admin_addr: c.admin_addr,
        listen_registry: listen_registry_ret,
        rate_limiter: rate_limiter_ret,
        shutdown_tx: Some(c.shutdown_tx),
    }
}

/// Start an in-process server with a real `EMBYR_ENCRYPTION_KEY`-equivalent
/// and a `GoogleJwksCache` pointed at a caller-supplied URL (oauth-providers
/// Slice 02 acceptance tests: `google_jwks_base_url` is a local mock JWKS
/// server for AC-19-05/06/07/09, or a closed/unresponsive address to
/// simulate AC-19-11's unreachability). Production keep-alive interval
/// (30s); `NoopEmailSender` (this feature never sends email).
pub async fn start_test_server_with_oauth(
    system_db: Arc<SystemDb>,
    encryption_key: [u8; 32],
    google_jwks_base_url: String,
) -> TestServer {
    let c = alloc_test_components(&system_db).await;
    let listen_registry_ret = Arc::clone(&c.listen_registry);

    let rate_limit_rps = default_rate_limit_capacity();
    let rate_limiter = RateLimiter::new(rate_limit_rps, rate_limit_rps);
    let rate_limiter_ret = Arc::clone(&rate_limiter);

    let service = FirestoreService {
        system_db: Arc::clone(&system_db),
        credential_cache: c.cache,
        index_manager: c.idx_mgr,
        metrics_adapter: c.metrics,
        keepalive_interval: std::time::Duration::from_secs(30),
        listen_registry: c.listen_registry,
        active_listeners: c.active_listeners,
        aws_secret_fetcher: None,
        gcp_secret_fetcher: None,
        rate_limiter,
        tenant_db_max_connections: 5,
        tenant_db_acquire_timeout: std::time::Duration::from_secs(5),
        listener_db_max_connections: 2,
        listener_db_acquire_timeout: std::time::Duration::from_secs(5),
    };

    let admin_app = admin::router::build_with_aws(
        system_db,
        "test-admin-key-secret".to_string(),
        c.cache_for_admin,
        None,
    );

    spawn_all_servers(
        c.grpc_listener, c.rest_listener, c.admin_listener,
        service, admin_app, c.shutdown_rx, Arc::new(NoopEmailSender),
        encryption_key, None,
        Arc::new(adapters::google_jwks_cache::GoogleJwksCache::new(google_jwks_base_url)),
        None, None,
        Vec::new(),
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    TestServer {
        grpc_addr: c.grpc_addr,
        rest_addr: c.rest_addr,
        admin_addr: c.admin_addr,
        listen_registry: listen_registry_ret,
        rate_limiter: rate_limiter_ret,
        shutdown_tx: Some(c.shutdown_tx),
    }
}

/// Start an in-process server with an injected `AwsSecretFetcher` (for US-10 tests).
pub async fn start_test_server_with_aws_fetcher(
    system_db: Arc<SystemDb>,
    keepalive: std::time::Duration,
    aws_fetcher: Arc<AwsSecretFetcher>,
) -> TestServer {
    let c = alloc_test_components(&system_db).await;
    let listen_registry_ret = Arc::clone(&c.listen_registry);

    let rate_limit_rps = default_rate_limit_capacity();
    let rate_limiter = RateLimiter::new(rate_limit_rps, rate_limit_rps);
    let rate_limiter_ret = Arc::clone(&rate_limiter);

    let service = FirestoreService {
        system_db: Arc::clone(&system_db),
        credential_cache: c.cache,
        index_manager: c.idx_mgr,
        metrics_adapter: c.metrics,
        keepalive_interval: keepalive,
        listen_registry: c.listen_registry,
        active_listeners: c.active_listeners,
        aws_secret_fetcher: Some(Arc::clone(&aws_fetcher)),
        gcp_secret_fetcher: None,
        rate_limiter,
        tenant_db_max_connections: 5,
        tenant_db_acquire_timeout: std::time::Duration::from_secs(5),
        listener_db_max_connections: 2,
        listener_db_acquire_timeout: std::time::Duration::from_secs(5),
    };

    let admin_app = admin::router::build_with_aws(
        system_db,
        "test-admin-key-secret".to_string(),
        c.cache_for_admin,
        Some(aws_fetcher),
    );

    spawn_all_servers(
        c.grpc_listener, c.rest_listener, c.admin_listener,
        service, admin_app, c.shutdown_rx, Arc::new(NoopEmailSender),
        [0u8; 32], None,
        Arc::new(adapters::google_jwks_cache::GoogleJwksCache::production()),
        None, None,
        Vec::new(),
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    TestServer {
        grpc_addr: c.grpc_addr,
        rest_addr: c.rest_addr,
        admin_addr: c.admin_addr,
        listen_registry: listen_registry_ret,
        rate_limiter: rate_limiter_ret,
        shutdown_tx: Some(c.shutdown_tx),
    }
}

/// Start an in-process server with an injected `GcpSecretFetcher` (for US-11 tests).
pub async fn start_test_server_with_gcp_fetcher(
    system_db: Arc<SystemDb>,
    keepalive: std::time::Duration,
    gcp_fetcher: Arc<GcpSecretFetcher>,
) -> TestServer {
    let c = alloc_test_components(&system_db).await;
    let listen_registry_ret = Arc::clone(&c.listen_registry);

    let rate_limit_rps = default_rate_limit_capacity();
    let rate_limiter = RateLimiter::new(rate_limit_rps, rate_limit_rps);
    let rate_limiter_ret = Arc::clone(&rate_limiter);

    let service = FirestoreService {
        system_db: Arc::clone(&system_db),
        credential_cache: c.cache,
        index_manager: c.idx_mgr,
        metrics_adapter: c.metrics,
        keepalive_interval: keepalive,
        listen_registry: c.listen_registry,
        active_listeners: c.active_listeners,
        aws_secret_fetcher: None,
        gcp_secret_fetcher: Some(Arc::clone(&gcp_fetcher)),
        rate_limiter,
        tenant_db_max_connections: 5,
        tenant_db_acquire_timeout: std::time::Duration::from_secs(5),
        listener_db_max_connections: 2,
        listener_db_acquire_timeout: std::time::Duration::from_secs(5),
    };

    let admin_app = admin::router::build_with_gcp(
        system_db,
        "test-admin-key-secret".to_string(),
        c.cache_for_admin,
        Some(gcp_fetcher),
    );

    spawn_all_servers(
        c.grpc_listener, c.rest_listener, c.admin_listener,
        service, admin_app, c.shutdown_rx, Arc::new(NoopEmailSender),
        [0u8; 32], None,
        Arc::new(adapters::google_jwks_cache::GoogleJwksCache::production()),
        None, None,
        Vec::new(),
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    TestServer {
        grpc_addr: c.grpc_addr,
        rest_addr: c.rest_addr,
        admin_addr: c.admin_addr,
        listen_registry: listen_registry_ret,
        rate_limiter: rate_limiter_ret,
        shutdown_tx: Some(c.shutdown_tx),
    }
}

/// Start an in-process server with distributed (Postgres-backed) rate limiting.
///
/// Used by DRL acceptance tests (b12, b13, b14).  Wires `RateLimiter::with_pg` so
/// the server enforces rate limits via the shared `rate_buckets` table.
///
/// - `capacity`:  token bucket capacity (burst size, also the refill_rate in tokens/s)
/// - `pg_pool`:   pool connected to the system DB — same DB that holds `rate_buckets`
pub async fn start_test_server_with_distributed_rate_limit(
    system_db: Arc<SystemDb>,
    capacity: f64,
    pg_pool: sqlx::PgPool,
) -> TestServer {
    let c = alloc_test_components(&system_db).await;
    let listen_registry_ret = Arc::clone(&c.listen_registry);

    let rate_limiter = RateLimiter::with_pg(capacity, capacity, pg_pool);
    let rate_limiter_ret = Arc::clone(&rate_limiter);

    let service = FirestoreService {
        system_db: Arc::clone(&system_db),
        credential_cache: c.cache,
        index_manager: c.idx_mgr,
        metrics_adapter: c.metrics,
        keepalive_interval: std::time::Duration::from_secs(30),
        listen_registry: c.listen_registry,
        active_listeners: c.active_listeners,
        aws_secret_fetcher: None,
        gcp_secret_fetcher: None,
        rate_limiter,
        tenant_db_max_connections: 5,
        tenant_db_acquire_timeout: std::time::Duration::from_secs(5),
        listener_db_max_connections: 2,
        listener_db_acquire_timeout: std::time::Duration::from_secs(5),
    };

    let admin_app = admin::router::build_with_aws(
        system_db,
        "test-admin-key-secret".to_string(),
        c.cache_for_admin,
        None,
    );

    spawn_all_servers(
        c.grpc_listener, c.rest_listener, c.admin_listener,
        service, admin_app, c.shutdown_rx, Arc::new(NoopEmailSender),
        [0u8; 32], None,
        Arc::new(adapters::google_jwks_cache::GoogleJwksCache::production()),
        None, None,
        Vec::new(),
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    TestServer {
        grpc_addr: c.grpc_addr,
        rest_addr: c.rest_addr,
        admin_addr: c.admin_addr,
        listen_registry: listen_registry_ret,
        rate_limiter: rate_limiter_ret,
        shutdown_tx: Some(c.shutdown_tx),
    }
}

/// Start an in-process server with a configurable rate limiter (for US-14 tests).
///
/// - `capacity`: token bucket capacity (burst size)
/// - `refill_rate`: tokens per second replenishment rate
/// - `enabled`: if false, rate limiting is disabled entirely
pub async fn start_test_server_with_rate_limit(
    system_db: Arc<SystemDb>,
    capacity: f64,
    refill_rate: f64,
    enabled: bool,
) -> TestServer {
    let c = alloc_test_components(&system_db).await;
    let listen_registry_ret = Arc::clone(&c.listen_registry);

    let rate_limiter = if enabled {
        RateLimiter::new(capacity, refill_rate)
    } else {
        RateLimiter::disabled()
    };
    let rate_limiter_ret = Arc::clone(&rate_limiter);

    let service = FirestoreService {
        system_db: Arc::clone(&system_db),
        credential_cache: c.cache,
        index_manager: c.idx_mgr,
        metrics_adapter: c.metrics,
        keepalive_interval: std::time::Duration::from_secs(30),
        listen_registry: c.listen_registry,
        active_listeners: c.active_listeners,
        aws_secret_fetcher: None,
        gcp_secret_fetcher: None,
        rate_limiter,
        tenant_db_max_connections: 5,
        tenant_db_acquire_timeout: std::time::Duration::from_secs(5),
        listener_db_max_connections: 2,
        listener_db_acquire_timeout: std::time::Duration::from_secs(5),
    };

    let admin_app = admin::router::build_with_aws(
        system_db,
        "test-admin-key-secret".to_string(),
        c.cache_for_admin,
        None,
    );

    spawn_all_servers(
        c.grpc_listener, c.rest_listener, c.admin_listener,
        service, admin_app, c.shutdown_rx, Arc::new(NoopEmailSender),
        [0u8; 32], None,
        Arc::new(adapters::google_jwks_cache::GoogleJwksCache::production()),
        None, None,
        Vec::new(),
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    TestServer {
        grpc_addr: c.grpc_addr,
        rest_addr: c.rest_addr,
        admin_addr: c.admin_addr,
        listen_registry: listen_registry_ret,
        rate_limiter: rate_limiter_ret,
        shutdown_tx: Some(c.shutdown_tx),
    }
}

/// Start an in-process server with all 3 listeners TLS-wrapped using the
/// given `TlsMaterial` (firestore-tls-support AC-TLS-02/03/04). Mirrors
/// `start_test_server_with_keepalive`'s own shape, threading
/// `Some(tonic_tls_config)`/`Some(tls_acceptor)` built from `tls` the same
/// way `main.rs` does from `cfg.tls`.
pub async fn start_test_server_with_tls(
    system_db: Arc<SystemDb>,
    tls: config::TlsMaterial,
) -> TestServer {
    let c = alloc_test_components(&system_db).await;
    let listen_registry_ret = Arc::clone(&c.listen_registry);

    let rate_limit_rps = default_rate_limit_capacity();
    let rate_limiter = RateLimiter::new(rate_limit_rps, rate_limit_rps);
    let rate_limiter_ret = Arc::clone(&rate_limiter);

    let service = FirestoreService {
        system_db: Arc::clone(&system_db),
        credential_cache: c.cache,
        index_manager: c.idx_mgr,
        metrics_adapter: c.metrics,
        keepalive_interval: std::time::Duration::from_secs(30),
        listen_registry: c.listen_registry,
        active_listeners: c.active_listeners,
        aws_secret_fetcher: None,
        gcp_secret_fetcher: None,
        rate_limiter,
        tenant_db_max_connections: 5,
        tenant_db_acquire_timeout: std::time::Duration::from_secs(5),
        listener_db_max_connections: 2,
        listener_db_acquire_timeout: std::time::Duration::from_secs(5),
    };

    // healthz-dependency-checks (ADR-078): clone BEFORE `system_db` moves by
    // value into `build_with_aws` below.
    let readyz_state = Arc::clone(&system_db);
    let healthz_app = axum::Router::new()
        .route("/livez", axum::routing::get(grpc::healthz::livez_handler))
        .route("/healthz", axum::routing::get(grpc::healthz::healthz_handler))
        .with_state(readyz_state);
    let admin_app = admin::router::build_with_aws(
        system_db,
        "test-admin-key-secret".to_string(),
        c.cache_for_admin,
        None,
    )
    .merge(healthz_app);

    let tonic_tls_config = tonic::transport::ServerTlsConfig::new()
        .identity(tonic::transport::Identity::from_pem(&tls.cert_pem, &tls.key_pem));
    let tls_acceptor = tokio_rustls::TlsAcceptor::from(Arc::clone(&tls.rustls_config));

    spawn_all_servers(
        c.grpc_listener, c.rest_listener, c.admin_listener,
        service, admin_app, c.shutdown_rx, Arc::new(NoopEmailSender),
        [0u8; 32], None,
        Arc::new(adapters::google_jwks_cache::GoogleJwksCache::production()),
        Some(tonic_tls_config), Some(tls_acceptor),
        Vec::new(),
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    TestServer {
        grpc_addr: c.grpc_addr,
        rest_addr: c.rest_addr,
        admin_addr: c.admin_addr,
        listen_registry: listen_registry_ret,
        rate_limiter: rate_limiter_ret,
        shutdown_tx: Some(c.shutdown_tx),
    }
}
