//! embyr-server production entry point.
//!
//! Startup sequence (ADR-017 D-PR-2):
//!   1.  [`config::ServerConfig::from_env()`]   — fail fast on bad config (async, may fetch secrets)
//!   2.  tracing init                             — structured logging from here
//!   3.  Prometheus recorder                      — ADR-016: before any TCP listener
//!   4.  [`adapters::system_db::SystemDb::new()`] — connect to Postgres
//!   5.  `SystemDb::migrate()`                    — apply migrations
//!   6.  `SystemDb::probe()`                      — verify schema (hard gate)
//!   7.  Allocate production components            — cache, rate limiter, registry
//!   8.  `TcpListener::bind` × 3                  — all three ports or none (D-PR-6)
//!   9.  Build [`grpc::handler::FirestoreService`]
//!  10.  Build admin router (+ `/healthz` route)
//!  11.  [`spawn_all_servers()`]                  — start accepting connections
//!  12.  log "embyr-server ready"
//!  13.  `tokio::select!` SIGTERM / Ctrl-C
//!  14.  drain + log "embyr-server stopped"

use std::collections::HashMap;
use std::sync::Arc;

use tracing_subscriber::EnvFilter;

use embyr_server::{
    adapters::{
        credential_cache::CredentialCache,
        email::NoopEmailSender,
        google_jwks_cache::GoogleJwksCache,
        index_manager::IndexManager,
        metrics_adapter::MetricsAdapter,
        postgres_notify_listener::PostgresNotifyListener,
        system_db::SystemDb,
    },
    admin::router::build_admin_router,
    config::ServerConfig,
    grpc::{handler::FirestoreService, healthz::healthz_handler},
    middleware::rate_limit::RateLimiter,
    observability::get_or_install_prometheus_handle,
    realtime::listen_registry::ListenRegistry,
    spawn_all_servers,
};

/// Log `error` under `message` at ERROR level, then exit the process with
/// status 1. Shared by every fail-fast startup step (DB connect, migrate,
/// probe) so the log-then-exit shape is written once.
fn fail_startup(error: impl std::fmt::Display, message: &str) -> ! {
    tracing::error!(error = %error, "{message}");
    std::process::exit(1);
}

/// Bind a TCP listener on `0.0.0.0:{port}`, exiting the process with a
/// structured error log on failure. Shared by the three startup listener
/// binds (gRPC, REST, admin) — all three or none is the desired behavior
/// (D-PR-6), so each bind independently fails fast.
async fn bind_or_exit(port: u16, label: &str) -> tokio::net::TcpListener {
    tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .unwrap_or_else(|e| {
            tracing::error!(port, error = %e, "startup failed: {label} port bind");
            std::process::exit(1);
        })
}

#[tokio::main]
async fn main() {
    // ── Step 1: parse and validate all configuration ──────────────────────
    let cfg = ServerConfig::from_env().await.unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    // ── Step 2: tracing (stderr; honours RUST_LOG or cfg.log_level) ──────
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&cfg.log_level)),
        )
        .with_writer(std::io::stderr)
        .init();

    if cfg.encryption_key_previous.is_some() {
        tracing::warn!(
            rotation_window_open = true,
            var = "EMBYR_ENCRYPTION_KEY_PREVIOUS",
            "decrypt-rotation window is open"
        );
    }

    if cfg.admin_key_previous.is_some() {
        tracing::warn!(
            rotation_window_open = true,
            var = "EMBYR_ADMIN_KEY_PREVIOUS",
            "admin-key rotation window is open"
        );
    }

    // ── Step 3: Prometheus recorder (ADR-016: before any TCP listener) ────
    let prom_handle = get_or_install_prometheus_handle();

    // ── Step 4: connect to system DB ──────────────────────────────────────
    let system_db = Arc::new(
        SystemDb::new(&cfg.db_url)
            .await
            .unwrap_or_else(|e| fail_startup(e, "startup failed: system DB connection")),
    );

    // ── Step 5: run migrations ────────────────────────────────────────────
    system_db
        .migrate()
        .await
        .unwrap_or_else(|e| fail_startup(e, "startup failed: migration"));

    // ── Step 6: probe (schema hard gate — D-PR-6) ─────────────────────────
    system_db
        .probe()
        .await
        .unwrap_or_else(|e| fail_startup(e, "startup probe failed"));

    // ── Step 7: allocate production components ────────────────────────────
    // Pool gauge background task (15-second interval, fire-and-forget).
    {
        let pool = system_db.pool().clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(15));
            loop {
                interval.tick().await;
                metrics::gauge!("embyr_pg_pool_size", "pool" => "system")
                    .set(pool.size() as f64);
                metrics::gauge!("embyr_pg_pool_idle", "pool" => "system")
                    .set(pool.num_idle() as f64);
            }
        });
    }

    let cache = Arc::new(CredentialCache::new(256));
    let cache_for_admin = Arc::clone(&cache);
    let cache_for_cap_refresher = Arc::clone(&cache);
    let idx_mgr = Arc::new(IndexManager::new(system_db.pool().clone()));
    let metrics_adapter = Arc::new(MetricsAdapter::new(system_db.pool().clone()));
    let listen_registry = ListenRegistry::new();
    let active_listeners = Arc::new(
        tokio::sync::Mutex::new(HashMap::<String, PostgresNotifyListener>::new()),
    );
    let rate_limiter =
        RateLimiter::with_pg(cfg.rate_limit_rps, cfg.rate_limit_rps, system_db.pool().clone());

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // ── Step 8: bind all three TCP listeners (all or none — D-PR-6) ──────
    let grpc_listener = bind_or_exit(cfg.grpc_port, "gRPC").await;
    let rest_listener = bind_or_exit(cfg.rest_port, "REST").await;
    let admin_listener = bind_or_exit(cfg.admin_port, "admin").await;

    // ── Step 9: build FirestoreService ────────────────────────────────────
    let service = FirestoreService {
        system_db: Arc::clone(&system_db),
        credential_cache: cache,
        index_manager: idx_mgr,
        metrics_adapter,
        keepalive_interval: std::time::Duration::from_secs(30),
        listen_registry,
        active_listeners,
        aws_secret_fetcher: None,
        gcp_secret_fetcher: None,
        rate_limiter,
    };

    // ── Step 10: build admin router + healthz on admin port ───────────────
    // The admin router (port 9090) does not include /healthz by default;
    // add it here so GET :{admin_port}/healthz returns 200 (US-PR-01 AC).
    // card-payments-backend: `stripe_secret_key` is optional in V1 (config.rs)
    // — absent means billing is unwired; a placeholder key is used so the
    // composition root always constructs successfully (StripeGateway::new
    // performs no I/O). `StripeGateway::probe()` is a soft-failure/WARN, not
    // a startup refusal, consistent with billing not being on the
    // Firestore protocol-serving critical path (ADR-020/021).
    let stripe_gateway = Arc::new(embyr_server::adapters::stripe_gateway::StripeGateway::new(
        cfg.stripe_secret_key
            .clone()
            .unwrap_or_else(|| "stripe-secret-key-not-configured".to_string()),
    ));

    if let Err(e) = stripe_gateway.probe().await {
        tracing::warn!(
            error = %e,
            "startup probe failed: stripe API unreachable or misconfigured (soft failure — billing not on critical path)"
        );
    }

    // card-payments-backend (ADR-020, step 03-01): ONE shared cache instance
    // between the router (reader, via `get_subscription`) and the
    // `CapUsageRefresher` (writer) — a previous version built two
    // independent `CapStatusCache`s here, which would have silently kept
    // `cap_status` permanently absent regardless of `run_cycle` correctness.
    let cap_status_cache = std::sync::Arc::new(
        embyr_server::adapters::cap_status_cache::CapStatusCache::new(),
    );

    let email_sender: Arc<dyn embyr_core::admin::email::IEmailSender + Send + Sync> =
        Arc::new(NoopEmailSender);

    let admin_app = build_admin_router(
        Arc::clone(&system_db),
        cfg.admin_key.clone(),
        cfg.admin_key_previous.clone(),
        cache_for_admin,
        cfg.encryption_key,
        cfg.encryption_key_previous,
        Arc::clone(&email_sender),
        None,
        None,
        cfg.rate_limit_rps,
        prom_handle,
        Arc::clone(&stripe_gateway),
        cfg.stripe_webhook_signing_secret.clone().unwrap_or_default(),
        Arc::clone(&cap_status_cache),
    )
    .route("/healthz", axum::routing::get(healthz_handler));

    // card-payments-backend (ADR-020): background cap-check refresher. Only
    // meaningful once STRIPE_SECRET_KEY is configured (Free-plan accounts
    // need a real `subscriptions` row to exist, seeded via US-201) — spawned
    // unconditionally regardless, mirroring the OBS-05 pool-gauge task's
    // always-on shape.
    let _cap_usage_refresher = embyr_server::sweepers::cap_usage_refresher::spawn(
        Arc::clone(&system_db),
        cap_status_cache,
        embyr_server::admin::handlers::lifecycle::LifecycleDeps {
            system_db: Arc::clone(&system_db),
            credential_cache: cache_for_cap_refresher,
        },
        std::time::Duration::from_secs(cfg.cap_check_interval_secs),
    );

    // ── Step 11: spawn gRPC + REST + admin servers ────────────────────────
    let server_task = spawn_all_servers(
        grpc_listener,
        rest_listener,
        admin_listener,
        service,
        admin_app,
        shutdown_rx,
        email_sender,
        cfg.encryption_key,
        cfg.encryption_key_previous,
        Arc::new(GoogleJwksCache::production()),
    );

    // ── Step 12: log ready ────────────────────────────────────────────────
    tracing::info!(
        grpc = %format!("0.0.0.0:{}", cfg.grpc_port),
        rest = %format!("0.0.0.0:{}", cfg.rest_port),
        admin = %format!("0.0.0.0:{}", cfg.admin_port),
        "embyr-server ready"
    );

    // ── Step 13: await SIGTERM or Ctrl-C ─────────────────────────────────
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received Ctrl-C");
            }
            _ = sigterm.recv() => {
                tracing::info!("received SIGTERM");
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to register Ctrl-C handler");
        tracing::info!("received Ctrl-C");
    }

    // ── Step 14: drain and exit ───────────────────────────────────────────
    let _ = shutdown_tx.send(());
    tracing::info!("shutdown signal received, draining...");
    // Wait for the server task's own graceful shutdown: it stops accepting
    // new connections and waits for genuinely in-flight requests on
    // already-open connections (e.g. a long-lived gRPC Listen stream) to
    // complete naturally, rather than exiting after a fixed delay
    // regardless of open connections (D-PR-5 / AC-PR-04-02).
    let _ = server_task.await;
    tracing::info!("embyr-server stopped");
}
