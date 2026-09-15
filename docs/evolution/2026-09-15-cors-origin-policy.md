# cors-origin-policy

Closes Medium finding #21 (Security) from `docs/product/production-readiness-audit-2026-09-08.md`.
DELIVER commit `f54335d00c400525e31a11cced511b4bceaab3de`.

## Business Context

`:8081` (REST/gRPC-Web/BrowserChannel) had no CORS layer and no written origin policy —
fails closed today by construction (no `CorsLayer` in the crate at all), but a landmine for
whoever adds `CorsLayer::permissive()` under deadline pressure to unblock a customer
integration. This adds an explicit, env-var-driven origin allowlist so that decision is made
once, deliberately, instead of ad hoc later.

## Key Decisions

| Decision | Detail |
|---|---|
| Default-deny env-var allowlist | `EMBYR_CORS_ALLOWED_ORIGINS` (comma-separated). Unset/empty ⇒ `AllowOrigin::list([])`, which matches no `Origin` header ⇒ no ACAO header ever. |
| Provably no-op when unset = no regression | Confirmed via `default_empty_allowlist_denies_all_origins` (AC-CORS-01): neither a simple cross-origin GET nor a preflight OPTIONS gets an ACAO header — byte-identical to pre-change (no-`CorsLayer`) behavior. |
| `:9090` (admin) out of scope | Evidence in `feature-delta.md` §2: admin session cookie is `SameSite=Strict` (CSRF closed independently of CORS); no admin-ui wiring to `:9090` today; ADR-007's planned V2 integration is same-origin `leptos_axum` `#[server]` functions, not cross-origin fetch. Verified by `admin_port_has_no_cors_layer` (AC-CORS-04). |
| No new ADR | Medium severity, single middleware layer (`tower_http::cors::CorsLayer` as outermost layer on the merged `:8081` router), token-budget mode per `feature-delta.md`. |
| `allow_credentials(false)` always | No cookie/credentialed cross-origin use case identified; keeps the allowlist simple (no ACAC header ever, even for an allowed origin). |
| Origins validated at startup | `ServerConfig::from_env()` parses each entry as an `http::HeaderValue` and fails fast (`ConfigError::InvalidCorsOrigin`) rather than panicking later inside `spawn_all_servers`. |

## Lessons

- Mutation run scoped to a single real-IO integration test target (`--test cors_origin_policy`,
  per QUALITY_GATE instruction) structurally cannot catch mutants in functions covered only by
  the crate's `--lib` unit tests (`parse_cors_allowed_origins`) — 4/5 "MISSED" mutants were this
  scope artifact, confirmed genuine coverage by manually hand-mutating and re-running the `--lib`
  target directly rather than re-running the full mutants scan.
- A whole-function-stub mutant on `Display for ConfigError` (all 14 variants, not just the new
  one) is cosmetic-only here: the error is still correctly constructed and returned by
  `from_env`'s validation loop regardless of how it renders — only diagnostic text would degrade.
  Not fixed, consistent with the file's pre-existing convention of never testing `Display` output
  for any `ConfigError` variant.
- `CorsLayer` must be the outermost `.layer()` on the merged axum router (after `.merge()`), not
  per-sub-router — `tower_http`'s CORS layer answers preflight `OPTIONS` itself before axum
  routing runs, so it has to sit above every route including ones merged in later.

## Key Files

- `crates/embyr-server/src/config.rs` — `EMBYR_CORS_ALLOWED_ORIGINS` parsing + startup validation
- `crates/embyr-server/src/lib.rs` — `CorsLayer` wiring in `spawn_all_servers`; `start_test_server_with_cors_origins` test harness
- `tests/cors_origin_policy/cors01_origin_allowlist.rs` — AC-CORS-01..04, real Postgres testcontainer + real in-process server
- `docs/feature/cors-origin-policy/feature-delta.md` — combined DISCUSS+DESIGN
- `docs/feature/cors-origin-policy/deliver/mutation/mutation-report.md` — mutation testing report

## Follow-Up

None identified. `:9090` CORS remains explicitly out of scope until a genuine browser
cross-origin need for the admin API appears (see feature-delta.md §2).
