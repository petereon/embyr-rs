# tls-startup-warning

Closes finding #22 (Medium, Security/Ops), `docs/product/production-readiness-audit-2026-09-08.md`.

## Problem

TLS wiring already correct (rustls 0.23 safe defaults, TLS1.2+/AEAD only, verified). Gap: no startup nudge when TLS is off while all 3 listeners (`:8080` gRPC, `:8081` REST, `:9090` admin) bind `0.0.0.0`. Binding to `0.0.0.0` is unconditional today (not a variable to check) — the only real condition is TLS on/off.

## Decision

One `tracing::warn!` line, gated on `cfg.tls.is_none()`. No hard failure — plaintext is a legitimate choice (local dev, behind a TLS-terminating LB; this repo's own `docker-compose.yml` from deployment-release-process runs without TLS deliberately). No `/healthz`/metrics surface — log line only, no strong reason found for more.

## Placement

`crates/embyr-server/src/main.rs`, Step 2 block, immediately after the existing `admin_key_previous` rotation-window warning (currently ends line 107), before Step 3 (Prometheus recorder init, line 109-110). Matches the existing pattern exactly: same block, same `tracing::warn!` macro, same structured-field style as the two rotation-window warnings already there.

```rust
if cfg.tls.is_none() {
    tracing::warn!(
        tls_enabled = false,
        vars = "EMBYR_TLS_CERT_PATH, EMBYR_TLS_KEY_PATH",
        "TLS is disabled — gRPC/REST/admin listeners bind 0.0.0.0 in plaintext; \
         set EMBYR_TLS_CERT_PATH and EMBYR_TLS_KEY_PATH to enable TLS"
    );
}
```

Trigger condition: `cfg.tls.is_none()` (from `ServerConfig::from_env()`, `config.rs:141` — `None` = both `EMBYR_TLS_CERT_PATH`/`EMBYR_TLS_KEY_PATH` absent, the existing both-or-neither contract). Fires once, in `main()` before any listener bind/spawn — not per-request, not per-connection.

## ADR

None. Single log line reusing an established pattern (see `admin_key_previous`/`encryption_key_previous` warnings, same file, same block) — no new architectural decision to record.

## Self-review

- Actionable — names the exact two env vars to set, not just "TLS is off."
- Fires once — lives in `main()` startup sequence, not a request/connection path.
- Correctly silent when configured — gated on `cfg.tls.is_none()`; `Some(TlsMaterial)` skips it.

Verdict: PASS.
