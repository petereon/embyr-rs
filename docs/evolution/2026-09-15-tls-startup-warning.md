# tls-startup-warning

Closes Medium finding #22 (Security/Ops) from `docs/product/production-readiness-audit-2026-09-08.md`.
DELIVER commit `562e9a3025e605f22b1cb54c488fc0ab5f6ad40a`.

## Business Context

TLS was entirely opt-in with nothing nudging an operator to turn it on — all 3 listeners bind
`0.0.0.0` in plaintext when unconfigured. This adds one startup log warning so the gap is visible,
not silent.

## Key Decision

Log-line nudge only: `tracing::warn!` in `main.rs` Step 2 when `cfg.tls.is_none()`, naming
`EMBYR_TLS_CERT_PATH`/`EMBYR_TLS_KEY_PATH`. No hard failure, no ADR — matches the existing
rotation-window warning pattern in the same block.

## Key Files

- `crates/embyr-server/src/main.rs` — the warning (Step 2, 9 lines)
- `tests/tls_startup_warning/` — acceptance tests (warns when disabled / silent when configured)
- `docs/feature/tls-startup-warning/deliver/mutation/mutation-report.md` — mutation report (1/1 caught)

## Follow-Up

None.
