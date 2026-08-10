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
        SystemDb::new(&cfg.db_url).await.unwrap_or_else(|e| {
            tracing::error!(error = %e, "startup failed: system DB connection");
            std::process::exit(1);
        }),
    );

    // ── Step 5: run migrations ────────────────────────────────────────────
    system_db.migrate().await.unwrap_or_else(|e| {
        tracing::error!(error = %e, "startup failed: migration");
        std::process::exit(1);
    });

    // ── Step 6: probe (schema hard gate — D-PR-6) ─────────────────────────
    system_db.probe().await.unwrap_or_else(|e| {
        tracing::error!(error = %e, "startup probe failed");
        std::process::exit(1);
    });

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
    let grpc_listener =
        tokio::net::TcpListener::bind(format!("0.0.0.0:{}", cfg.grpc_port))
            .await
            .unwrap_or_else(|e| {
                tracing::error!(
                    port = cfg.grpc_port,
                    error = %e,
                    "startup failed: gRPC port bind"
                );
                std::process::exit(1);
            });

    let rest_listener =
        tokio::net::TcpListener::bind(format!("0.0.0.0:{}", cfg.rest_port))
            .await
            .unwrap_or_else(|e| {
                tracing::error!(
                    port = cfg.rest_port,
                    error = %e,
                    "startup failed: REST port bind"
                );
                std::process::exit(1);
            });

    let admin_listener =
        tokio::net::TcpListener::bind(format!("0.0.0.0:{}", cfg.admin_port))
            .await
            .unwrap_or_else(|e| {
                tracing::error!(
                    port = cfg.admin_port,
                    error = %e,
                    "startup failed: admin port bind"
                );
                std::process::exit(1);
            });

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
    let admin_app = build_admin_router(
        Arc::clone(&system_db),
        cfg.admin_key.clone(),
        cfg.admin_key_previous.clone(),
        cache_for_admin,
        cfg.encryption_key,
        cfg.encryption_key_previous,
        Arc::new(NoopEmailSender),
        None,
        None,
        cfg.rate_limit_rps,
        prom_handle,
    )
    .route("/healthz", axum::routing::get(healthz_handler));

    // ── Step 11: spawn gRPC + REST + admin servers ────────────────────────
    let server_task = spawn_all_servers(
        grpc_listener,
        rest_listener,
        admin_listener,
        service,
        admin_app,
        shutdown_rx,
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
