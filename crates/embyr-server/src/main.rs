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
//!       8b. Construct cloud secret fetchers          — AWS unconditional, GCP gated on EMBYR_GCP_ACCESS_TOKEN
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
        aws_secret_fetcher::AwsSecretFetcher,
        credential_cache::CredentialCache,
        email::NoopEmailSender,
        gcp_secret_fetcher::GcpSecretFetcher,
        google_jwks_cache::GoogleJwksCache,
        index_manager::IndexManager,
        metrics_adapter::MetricsAdapter,
        postgres_notify_listener::PostgresNotifyListener,
        system_db::SystemDb,
    },
    admin::router::build_admin_router,
    config::{ServerConfig, CLOUD_SECRET_FETCHER_TTL_SECS, GCP_SECRET_MANAGER_BASE_URL},
    grpc::{
        handler::FirestoreService,
        healthz::{healthz_handler, livez_handler},
    },
    middleware::rate_limit::RateLimiter,
    middleware::signin_rate_limit::SigninRateLimiter,
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
    // ANSI color codes are only emitted for an interactive terminal — piped
    // output (subprocess capture, `docker compose logs`, log aggregators)
    // stays plain text so structured fields (e.g. `version="0.1.1"`) remain
    // machine-parseable.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&cfg.log_level)),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
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

    if cfg.tls.is_none() {
        tracing::warn!(
            tls_enabled = false,
            vars = "EMBYR_TLS_CERT_PATH, EMBYR_TLS_KEY_PATH",
            "TLS is disabled — gRPC/REST/admin listeners bind 0.0.0.0 in plaintext; \
             set EMBYR_TLS_CERT_PATH and EMBYR_TLS_KEY_PATH to enable TLS"
        );
    }

    // ── Step 3: Prometheus recorder (ADR-016: before any TCP listener) ────
    let prom_handle = get_or_install_prometheus_handle();

    // ── Step 4: connect to system DB ──────────────────────────────────────
    let system_db = Arc::new(
        SystemDb::with_pool_config(&cfg.db_url, cfg.system_db_max_connections)
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

    // admin-signin-hardening (ADR-076): per-source-IP token bucket gating
    // POST /admin/v1/auth/signin. capacity=150 tokens, refill=10/minute.
    let signin_rate_limiter =
        SigninRateLimiter::with_pg(150.0, 10.0 / 60.0, system_db.pool().clone());

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // ── Step 8: bind all three TCP listeners (all or none — D-PR-6) ──────
    let grpc_listener = bind_or_exit(cfg.grpc_port, "gRPC").await;
    let rest_listener = bind_or_exit(cfg.rest_port, "REST").await;
    let admin_listener = bind_or_exit(cfg.admin_port, "admin").await;

    // ── Step 8b: construct cloud secret fetchers (wire-secret-fetchers,
    // closes production-readiness-audit finding #7) ───────────────────────
    // AWS: unconditional construction. `aws_config::load_defaults` is
    // infallible/lazy — zero I/O, zero risk of blocking or failing startup,
    // regardless of whether real credentials exist. A deployment with no
    // AWS credentials still starts normally; the first aws_secret-mode
    // request or provisioning call fails closed later via the
    // already-implemented `AwsSecretError` path (handler.rs / provision.rs)
    // — no new error-handling code needed.
    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let aws_secret_fetcher = Some(Arc::new(
        AwsSecretFetcher::new(&aws_config, CLOUD_SECRET_FETCHER_TTL_SECS).await,
    ));

    // GCP: gated construction. `GcpSecretFetcher::new` takes the bearer
    // token synchronously — it cannot be constructed without one. `None`
    // here is a deliberate deferral, not an oversight: this feature's own
    // System Constraints lock "no global startup fail-fast for missing
    // AWS/GCP credentials" — a gcp_secret-mode request/provision call fails
    // closed later, exactly as today (handler.rs / provision.rs).
    let gcp_secret_fetcher = std::env::var("EMBYR_GCP_ACCESS_TOKEN")
        .ok()
        .filter(|v| !v.is_empty())
        .map(|token| {
            Arc::new(GcpSecretFetcher::new(
                GCP_SECRET_MANAGER_BASE_URL,
                &token,
                CLOUD_SECRET_FETCHER_TTL_SECS,
            ))
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
        aws_secret_fetcher: aws_secret_fetcher.clone(),
        gcp_secret_fetcher: gcp_secret_fetcher.clone(),
        rate_limiter,
        tenant_db_max_connections: cfg.tenant_db_max_connections,
        tenant_db_acquire_timeout: std::time::Duration::from_secs(cfg.tenant_db_acquire_timeout_secs as u64),
        listener_db_max_connections: cfg.listener_db_max_connections,
        listener_db_acquire_timeout: std::time::Duration::from_secs(cfg.listener_db_acquire_timeout_secs as u64),
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

    // stripe-webhook-secret-required (D1/D2): mount the webhook sub-router
    // only when Stripe billing is genuinely enabled (non-empty
    // STRIPE_SECRET_KEY). `ServerConfig::from_env()` (Step 1, already run)
    // already refused to start the process if STRIPE_SECRET_KEY was set
    // without STRIPE_WEBHOOK_SIGNING_SECRET — the `.expect()` below documents
    // that invariant, it never fires in a process that reached this line.
    let stripe_billing_enabled = cfg
        .stripe_secret_key
        .as_deref()
        .is_some_and(|v| !v.is_empty());
    let webhook_signing_secret: Option<String> = if stripe_billing_enabled {
        Some(
            cfg.stripe_webhook_signing_secret
                .clone()
                .filter(|v| !v.is_empty())
                .expect(
                    "invariant violated: STRIPE_SECRET_KEY is set but \
                     STRIPE_WEBHOOK_SIGNING_SECRET is absent/empty — \
                     ServerConfig::from_env() must already have refused startup",
                ),
        )
    } else {
        None
    };

    // healthz-dependency-checks (ADR-078): `/livez`/`/healthz` need
    // `Arc<SystemDb>` state, resolved to `Router<()>` via `.with_state()`
    // then merged onto the admin router's own return value.
    let healthz_app = axum::Router::new()
        .route("/livez", axum::routing::get(livez_handler))
        .route("/healthz", axum::routing::get(healthz_handler))
        .with_state(Arc::clone(&system_db));
    let admin_app = build_admin_router(
        Arc::clone(&system_db),
        cfg.admin_key.clone(),
        cfg.admin_key_previous.clone(),
        cache_for_admin,
        cfg.encryption_key,
        cfg.encryption_key_previous,
        Arc::clone(&email_sender),
        aws_secret_fetcher.clone(),
        gcp_secret_fetcher.clone(),
        cfg.rate_limit_rps,
        prom_handle,
        Arc::clone(&stripe_gateway),
        webhook_signing_secret,
        Arc::clone(&cap_status_cache),
        Arc::clone(&signin_rate_limiter),
    )
    .merge(healthz_app);

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

    // customer-db-transaction-sweeper (ADR-054): background reclaim +
    // purge of abandoned/terminal transactions rows across every
    // PG-reachable customer database (Slice 01 reclaim, Slice 02 purge).
    // `aws_secret_fetcher`/`gcp_secret_fetcher` (wire-secret-fetchers): the
    // same instances constructed at Step 8b reach this third call site too
    // — a deployment with a cloud's credentials configured now has its
    // aws_secret/gcp_secret projects actually reclaimed here, rather than
    // silently skipped (ADR-054 § D7). Final use in program order, so this
    // moves rather than clones.
    let _transaction_sweeper = embyr_server::sweepers::transaction_sweeper::spawn(
        Arc::clone(&system_db),
        aws_secret_fetcher,
        gcp_secret_fetcher,
        cfg.encryption_key,
        cfg.encryption_key_previous,
        std::time::Duration::from_secs(cfg.transaction_sweep_interval_secs),
        cfg.transaction_retention_days,
    );

    // soft-delete-purge-sweeper (Blocker finding #6, production-readiness-audit-2026-09-08.md):
    // background purge of the 3 sensitive encrypted-credential columns on a
    // `'deleted'` project's own row, once the configured grace window (default
    // 168h/7 days, matching the admin-UI's own unchanged promise) has elapsed.
    // SystemDb-only — no customer-database connection, no DSN resolution
    // (target columns live on the projects row itself, unlike TransactionSweeper).
    let _soft_delete_purge_sweeper = embyr_server::sweepers::soft_delete_purge_sweeper::spawn(
        Arc::clone(&system_db),
        std::time::Duration::from_secs(cfg.soft_delete_sweep_interval_secs),
        cfg.soft_delete_grace_days,
    );

    // admin-signin-hardening (ADR-076): 4th background sweeper — bounds
    // signin_rate_limits table growth against an IP-rotating attacker.
    // Hourly cadence, 24h retention (hardcoded in the sweeper module — no
    // new ServerConfig field, consistent with this feature's narrow scope).
    let _signin_rate_limit_sweeper = embyr_server::sweepers::signin_rate_limit_sweeper::spawn(
        Arc::clone(&system_db),
        std::time::Duration::from_secs(3600),
    );

    // ── Step 11: spawn gRPC + REST + admin servers ────────────────────────
    // firestore-tls-support: derive the 2 listener-facing TLS types from
    // `cfg.tls` (both `None` when TLS is not configured — AC-TLS-01).
    let tonic_tls_config = cfg.tls.as_ref().map(|tls| {
        tonic::transport::ServerTlsConfig::new()
            .identity(tonic::transport::Identity::from_pem(&tls.cert_pem, &tls.key_pem))
    });
    let tls_acceptor = cfg.tls.as_ref().map(|tls| {
        tokio_rustls::TlsAcceptor::from(Arc::clone(&tls.rustls_config))
    });

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
        tonic_tls_config,
        tls_acceptor,
        cfg.cors_allowed_origins.clone(),
    );

    // ── Step 12: log ready ────────────────────────────────────────────────
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        grpc = %format!("0.0.0.0:{}", cfg.grpc_port),
        rest = %format!("0.0.0.0:{}", cfg.rest_port),
        admin = %format!("0.0.0.0:{}", cfg.admin_port),
        "embyr-server v{} ready",
        env!("CARGO_PKG_VERSION")
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
