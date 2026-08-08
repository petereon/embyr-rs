# Slice PR-02: Real Startup Sequence (Migrations + SystemDb Probe + Graceful Shutdown)

## Slice Metadata

| Field | Value |
|-------|-------|
| Slice ID | PR-02 |
| Parent Story | US-PR-01 (Real Server Startup from Environment Variables) |
| job_id | JOB-13 |
| Estimate | 0.5 day |
| Depends on | PR-01 (Config struct + main() skeleton must exist) |
| Blocks | PR-03 (Dockerfile), PR-04 (CI) |

## Deliverable

After this slice, `cargo run -p embyr-server`:

1. Reads config and initializes tracing (from PR-01)
2. Installs Prometheus recorder
3. Runs all 18 migrations via `SystemDb::migrate()` — exits 1 on failure
4. Probes the system DB via `SystemDb::probe()` — exits 1 on failure
5. Binds all three ports
6. Logs "embyr-server ready grpc=... rest=... admin=..."
7. Serves traffic including `GET :9090/healthz → 200` and `GET :9090/readyz → 200`
8. On SIGTERM: stops accepting new connections, drains in-flight requests, exits 0

**Slice boundary rationale:** PR-01 proved the binary can start. PR-02 adds the safety gates
(migrations + probe) that make it safe to accept production traffic. These are logically
coupled (both must succeed before traffic is accepted) and together fit in 0.5 day.

## Decisions

| ID | Decision |
|----|----------|
| D-PR-2 | Init order: tracing → prometheus → migrations → probe SystemDb → bind ports → spawn servers |
| D-PR-5 | Graceful shutdown: `tokio::signal::unix::signal(SIGTERM)` + `tokio::signal::ctrl_c()` race; reuse oneshot pattern from `TestServer.shutdown_tx` |
| D-PR-6 | Startup exits code 1 if migrations fail or `SystemDb::probe()` fails; no partial startup |

## Implementation Notes

### Migration path

`SystemDb::migrate()` already exists and works in tests:

```rust
sqlx::migrate!("../../migrations").run(&self.pool).await
```

The path `../../migrations` is relative to `crates/embyr-server/` and resolves to the
workspace root `migrations/` directory. This has been verified by existing integration tests
in `system_db.rs`. No path change needed.

### Startup sequence in `main.rs`

```
1. ServerConfig::from_env()     -- exits 1 on error (PR-01)
2. init_tracing(&cfg)           -- fmt subscriber with EnvFilter
3. get_or_install_prometheus_handle()   -- ADR-016 ordering
4. SystemDb::new(&cfg.database_url).await  -- exits 1 if connect fails
5. system_db.migrate().await    -- exits 1 if any migration fails
6. system_db.probe().await      -- exits 1 if schema not ready
7. bind three TcpListeners      -- exits 1 if port conflict
8. log "embyr-server ready grpc={} rest={} admin={}"
9. spawn_all_servers(...)       -- wire full server from lib.rs components
10. wait for shutdown signal
11. graceful drain
12. exit 0
```

### Graceful shutdown

Two signal sources race:

```rust
tokio::select! {
    _ = tokio::signal::ctrl_c() => {},
    _ = unix_sigterm() => {},  // tokio::signal::unix::signal(SignalKind::terminate())
}
```

On signal: send on the existing `oneshot` shutdown channel (same pattern as `TestServer`
Drop impl). The server tasks receiving the shutdown signal stop accepting new connections.
In-flight gRPC streams are allowed to complete (no hard kill).

### `GET /readyz` behavior

The architecture brief specifies: "`/readyz` endpoint re-runs the system DB ping on every
call; it does not re-run schema migrations." The existing `/readyz` handler in the REST
router already calls `system_db.probe()` — confirm it is wired from the production server
(not only from test constructors).

### Error messages to stderr

Consistent with the agent pattern:

```
embyr-server: migrations failed: <sqlx error>
embyr-server: startup probe failed: system DB unreachable: connection refused
embyr-server: port conflict: 8080 already in use
```

## Acceptance Criteria for This Slice

- [ ] `SystemDb::migrate()` is called before any port is bound
- [ ] `SystemDb::probe()` is called after migrations succeed, before any port is bound
- [ ] Startup exits code 1 if migrations fail; stderr contains "migrations failed"
- [ ] Startup exits code 1 if DB probe fails; stderr contains "startup probe failed"
- [ ] Startup exits code 1 if any port is already in use; stderr identifies the port
- [ ] On successful startup, all 18 migrations are applied (verified by querying `_sqlx_migrations`)
- [ ] `GET :9090/readyz` returns HTTP 200 when DB is reachable, non-200 when DB is down
- [ ] SIGTERM causes exit 0 after in-flight requests complete
- [ ] CTRL-C causes exit 0 (same drain behavior)

## Out of Scope for This Slice

- Dockerfile (PR-03)
- CI (PR-04)
- Port gauge background task (already wired in `alloc_test_components` — wire same in production)

## Test Guidance

All existing tests in `system_db.rs` already cover `migrate()` and `probe()`. This slice
focuses on integration at the `main()` level:

- Process-level test (using `std::process::Command`):
  - With reachable DB → exit 0 after SIGTERM
  - With unreachable DB → exit 1 + stderr contains "startup probe failed"
  - With migrations already applied → `migrate()` is idempotent (sqlx migration table)

Note: testcontainers-based integration test already exists for `probe_returns_ok_when_db_reachable_and_schema_current` in `system_db.rs`. This slice adds process-level tests.
