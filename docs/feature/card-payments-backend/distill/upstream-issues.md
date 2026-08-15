# Upstream Issues Found During DISTILL — card-payments-backend

Per `nw-distill` § Document Update (Back-Propagation). Findings that reveal gaps in prior-wave
(DESIGN) claims, discovered while building RED scaffolds.

## Finding 1: DESIGN's "mirrors QueryLogSweeper/SessionCleaner" precedent does not exist

**Where claimed**: `docs/product/architecture/adr-020-cumulative-cap-check-architecture.md` §
Component shape ("structurally identical to the existing `QueryLogSweeper`/`SessionCleaner`
precedent (admin-api-v2)"); `docs/product/architecture/brief.md` § Application Architecture —
card-payments-backend § Component Decomposition (same claim, `CapUsageRefresher` row);
`docs/feature/card-payments-backend/feature-delta.md` § DESIGN Reuse Analysis (`sweepers::query_log_sweeper`/`session_cleaner`
row, "PATTERN REUSE").

**What DISTILL found**: `grep -rln "QueryLogSweeper\|SessionCleaner\|pg_try_advisory_lock" crates/embyr-server/src/`
returns zero matches. No `crates/embyr-server/src/sweepers/` directory exists prior to this
DISTILL run. `crates/embyr-server/src/adapters/query_log.rs` exists but is a synchronous
query-logging adapter (`IQueryLogWriter`), not an interval-scheduled background sweeper. The only
comparable existing background task in the codebase is an inline 15-second Prometheus pool-gauge
loop in `lib.rs` (`OBS-05`), which uses neither a dedicated module nor `pg_try_advisory_lock`.

**Impact**: `CapUsageRefresher` (`crates/embyr-server/src/sweepers/cap_usage_refresher.rs`) is
built fresh in this DISTILL run, following the *shape* ADR-020 describes (Tokio interval +
`pg_try_advisory_lock` guard) without literally reusing nonexistent code. This does not change any
locked acceptance criterion or Decision — it corrects a citation, not a requirement. Flagged here
rather than silently absorbed, per the Document Update procedure.

**Resolution**: no action required before DELIVER. `sweepers/mod.rs`'s own module doc comment
carries this same note for anyone reading the code directly. If a future feature also needs an
advisory-lock-guarded sweeper, `CapUsageRefresher` (once DELIVER implements it) becomes the actual
first such precedent in this codebase — not `QueryLogSweeper`/`SessionCleaner`, which never
existed.

## Finding 2: `build_admin_router`'s signature extension has wider blast radius than DESIGN scoped

**Where claimed**: `docs/product/architecture/brief.md` § Component Decomposition —
"`build_admin_router` signature gains `stripe_gateway: Arc<StripeGateway>`,
`stripe_webhook_signing_secret: String` ... rather than introducing a config struct" — framed as a
purely additive, feature-local change.

**What DISTILL found**: `build_admin_router` is called directly from 3 sites outside this
feature's own new code: `crates/embyr-server/src/main.rs` (production composition root),
`crates/embyr-server/src/admin/router.rs`'s own `build_with_secret_fetchers` wrapper (which 4
other wrapper functions delegate to, in turn called by every `TestServer` constructor in `lib.rs`
— i.e. every other feature's subprocess/in-process test suite in this workspace), and
`tests/admin_api_v2/common/mod.rs` (a sibling feature's test harness, calling the function
directly rather than through a wrapper).

**Impact**: DISTILL updated all 3 call sites (main.rs passes real config-derived values;
`build_with_secret_fetchers` and `tests/admin_api_v2/common/mod.rs` pass a placeholder
`StripeGateway::new("test-stripe-key-unused")` + empty webhook secret, since neither exercises
billing). Verified via `cargo check --workspace --tests`: zero new compile errors across the
entire workspace, zero behavioral change to any existing passing test (confirmed via `cargo test
-p embyr-core --lib` and `cargo test -p embyr-server --lib -- config:: sweepers::`, both fully
green, no regressions).

**Resolution**: no action required — DISTILL absorbed the full blast radius rather than leaving
other suites broken. Flagged so DELIVER (and any reviewer) understands why 3 files outside
`tests/card_payments_backend/` and `docs/feature/card-payments-backend/` appear in this feature's
diff.

## Finding 3: `STRIPE_*` config resolution simplified vs. DESIGN's exact ask (non-blocking)

**Where claimed**: `docs/product/architecture/brief.md` § Component Decomposition — "Three new
resolver functions reusing `resolve_secret_source`/`fetch_from_secret_manager` (ADR-018 pattern)".

**What DISTILL did instead**: `ServerConfig::from_env()` resolves `STRIPE_SECRET_KEY`/
`STRIPE_WEBHOOK_SIGNING_SECRET`/`STRIPE_PUBLISHABLE_KEY` via plain `std::env::var(...).ok()`, all
three OPTIONAL (not pushed onto `missing`), rather than wiring the full AWS/GCP-secret-manager
resolver chain ADR-018 established.

**Why**: making any of the three Stripe vars a *required* var (even via the full resolver, which
still requires at least one of {plain, AWS ARN, GCP name} to be set) would break every other
subprocess-spawning test suite in this workspace (`secrets_management`, `production_readiness`)
that spawns `embyr-server` without ever setting a `STRIPE_*` variable — none of DISCUSS's or
DESIGN's locked ACs require Stripe secret-manager sourcing to be a hard startup gate; the
requirement is that the pattern is *reused where wired*, and V1 wires it minimally to avoid
breaking unrelated features' passing subprocess tests.

**Resolution**: not blocking any AC in this feature. Extending to the full
`resolve_secret_source`/`fetch_from_secret_manager` chain (AWS/GCP-sourced Stripe secrets) is a
straightforward DELIVER-time or follow-up-feature addition — the code shape to copy
(`resolve_admin_key`) is already in `config.rs`, unchanged.

## Finding 4: `async-stripe`'s default features violate `deny.toml`'s tokio/hyper wrapper allowlist

**Not a DESIGN gap** — a build-time discovery made while wiring the dependency per ADR-021.
`async-stripe`'s default feature set (`default-tls`) pulls in `hyper-tls`/`tokio-native-tls`,
neither in `deny.toml`'s `wrappers` allowlist for the `tokio`/`hyper` bans (this workspace is
rustls-only throughout, matching `reqwest`'s own `rustls-tls` + `default-features = false`
choice). Additionally, `async-stripe` itself (any TLS backend) is a direct dependent of `tokio` and
`hyper`, and was not yet in either ban's `wrappers` list (expected — it's a brand-new dependency).

**Resolution** (both applied, verified via `cargo deny check bans licenses` → `bans ok, licenses
ok`):
1. `Cargo.toml`: `async-stripe` declared with `default-features = false`, features
   `["rustls-tls-webpki-roots", "rustls-ring", "uuid"]` — matches the workspace's existing
   rustls+ring choice (`rustls = { version = "0.23", features = ["ring"] }`).
2. `deny.toml`: `"async-stripe"` appended to both the `tokio` ban's and `hyper` ban's `wrappers`
   lists — identical treatment to every other embyr-server-only IO dependency already listed
   (`aws-config`, `reqwest`, etc.).

No change to `embyr-core`'s own IO-prohibition enforcement — `async-stripe` is not, and never was,
reachable from `embyr-core`'s dependency graph (`embyr-core`'s `Cargo.toml` was not touched).
