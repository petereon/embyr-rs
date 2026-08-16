# ADR-022: Customer DB Prep — New `embyr-db-prep` Crate + Single-Sourced Migration Adapter

## Status

Accepted

## Context

The `customer-db-onboarding` feature (JOB-15) lets a customer's DBA apply `migrations/customer/`
to their own Postgres under their own elevated, one-time credentials, then hand embyr's operator
a lower-privilege, DML-only connection string — instead of granting embyr's SaaS provisioning flow
DDL rights on a stored connection string.

DISCUSS's Handoff Package names the single highest-consequence design risk in this feature
explicitly: `migrations/customer/` must stay a single source of truth between the new standalone
tool and `embyr-server`'s own existing migration behavior. "DESIGN must ensure both consumers read
the identical migration set ... not two independently-maintained copies."

Codebase inspection during DESIGN found this risk is not hypothetical — it already exists in a
smaller form today. Two independent `sqlx::migrate!("../../migrations/customer")` macro
invocations exist in the workspace before this feature:

1. `crates/embyr-server/src/admin/handlers/provision.rs` — three inline call sites (the
   `aws_secret`, `gcp_secret`, and `direct_pg` branches each embed the macro independently).
2. `crates/embyr-pg-storage/src/backend_adapter.rs` — `PostgresBackendAdapter::migrate()` and the
   static `PostgresBackendAdapter::run_migrations(pool)` convenience, both wrapping the identical
   macro call. Per its own doc comment, this method is currently used only by test harnesses.

Adding a third, independent embed for the new customer-run prep binary would triple the drift
surface DISCUSS is explicitly worried about, rather than closing it.

Separately, `crates/embyr-agent` is the workspace's existing precedent for a customer-run
standalone binary: its own `[[bin]]` target, `AgentConfig::from_env()` error-accumulating
configuration pattern, and named, non-cryptic startup errors (`crates/embyr-agent/src/main.rs`,
`config.rs`, `probe.rs`). It was considered as a home for the new prep tool.

## Decision

**1. `PostgresBackendAdapter::migrate()` becomes the sole embed point for
`sqlx::migrate!("../../migrations/customer")` in the entire workspace.**

`provision.rs`'s three independent inline macro calls are replaced with calls to
`PostgresBackendAdapter::migrate()` against the already-open `customer_pool` (a ~3-line change per
branch — the pool is already constructed by the existing `probe_customer_db()` helper).

**2. A new workspace crate, `embyr-db-prep`, is the customer-run standalone prep binary.**

```toml
# Cargo.toml (root workspace) — new member
[workspace]
members = [
    "crates/embyr-proto",
    "crates/embyr-core",
    "crates/embyr-pg-storage",
    "crates/embyr-server",
    "crates/embyr-admin",
    "crates/embyr-agent",
    "crates/embyr-admin-ui",
    "crates/embyr-db-prep",   # new
]
```

`embyr-db-prep` depends only on `embyr-pg-storage`, `embyr-core`, `sqlx`, and `tokio` — no tonic,
no rustls-server, no aws-sdk, no axum. It contains **no independent migration embed**; it calls the
exact same `PostgresBackendAdapter::migrate()` function `embyr-server` itself now calls.

Because both consumers (the new prep binary and `embyr-server`'s provisioning path) call the
identical function in the identical crate, migration-set drift between them is structurally
impossible within a single deployed commit. The only remaining drift vector is version skew across
*time* — a DBA runs an older build of `embyr-db-prep` against a database while a newer
`embyr-server` is deployed — which is addressed separately by ADR-023's verification mechanism, not
by single-sourcing alone.

## Alternatives Considered

### Alternative 1: New `[[bin]]` target inside the existing `embyr-agent` crate

Reuse `embyr-agent`'s `[[bin]]`+`[lib]` shape and `AgentConfig::from_env()` pattern directly by
adding a second binary target to the same crate.

**Rejected because:** `embyr-agent` is a long-running mTLS gRPC daemon — `tonic`, `rustls` server
TLS configuration, a background transaction sweeper, a NOTIFY bridge. A one-shot CLI migration tool
sharing this crate would pull all of these unrelated dependencies into a binary that needs none of
them, working directly against the same supply-chain-minimization rationale the architecture brief
already established for `embyr-agent` itself (SD-09: "Agent runs in customer VPC where embyr SaaS
is untrusted. Static binary minimizes supply-chain surface"). A prep tool run by a customer DBA in
a security-scanned/restricted environment deserves the same minimization discipline, not an
exemption from it.

### Alternative 2: New `[[bin]]` target inside `embyr-server`

Add `crates/embyr-server/src/bin/db_prep.rs` and ship it as a second binary from the existing
`embyr-server` crate.

**Rejected because:** `embyr-server`'s `Cargo.toml` pulls `axum`, `tonic`, `tonic-web`,
`aws-sdk-secretsmanager`, `async-stripe` (+ 3 companion crates), and a dozen other SaaS-side
dependencies entirely irrelevant to a one-shot DB migration tool a customer's DBA runs in their own
environment. Cargo's per-crate dependency graph does not let a `[[bin]]` target opt out of the
crate's full `[dependencies]` table — shipping this binary to a customer VPC would carry the
server's entire dependency surface (and its associated CVE/audit surface) for no functional
benefit.

### Alternative 3: Independent migration embed in the new prep binary (no consolidation)

Leave `provision.rs`'s three inline `sqlx::migrate!` calls as-is, and give the new prep binary its
own independent `sqlx::migrate!("../../migrations/customer")` call.

**Rejected because:** this is the literal drift risk DISCUSS's Handoff Package names as the single
highest-consequence design risk in the feature. Three independent embeds instead of one is a
regression, not a mitigation — even though all three point at byte-identical source files at any
single commit, the risk DISCUSS is naming is exactly this kind of accidental divergence over time
(someone edits one embed's path, or adds migration-loading logic to only one of the three call
sites, or one binary is built from a stale checkout). Consolidating to one canonical function call,
reused by every consumer, removes the divergence opportunity by construction rather than by
convention.

## Consequences

### Positive

- Single embed point for the entire workspace's customer-schema migration logic. `provision.rs`'s
  diff is ~3 lines per branch (replace macro invocation with a method call on the pool it already
  has). The new prep binary needs zero new migration-application code — it calls an existing,
  already-tested function.
- Matches an existing, underused precedent: `PostgresBackendAdapter::migrate()` already existed in
  `embyr-pg-storage` before this feature, previously reserved for test harnesses. This feature
  promotes it to production use on both the SaaS and customer-run sides.
- `embyr-db-prep`'s dependency footprint (`sqlx`, `tokio`, `embyr-core`, `embyr-pg-storage`) is
  materially smaller than either alternative crate would have produced — supply-chain review of a
  customer-run binary stays tractable.

### Negative / Trade-offs

- `embyr-pg-storage` becomes a shared dependency of three binaries now (`embyr-server`,
  `embyr-agent`, `embyr-db-prep`) rather than two. A change to `embyr-pg-storage`'s public surface
  now potentially affects all three binaries' release cadence. Acceptable: the crate's public
  migration surface (`migrate()`) is a single, narrow, versioned function; `embyr-pg-storage`
  already had this exact shared-dependency shape with `embyr-server` and `embyr-agent` for its
  storage-adapter functionality before this feature.
- A new workspace crate is a small but real addition to build-graph complexity (a 9th workspace
  member after `embyr-admin-ui`). Mitigated by the crate's minimal dependency list — it does not
  meaningfully lengthen `cargo build --workspace` compared to `embyr-agent` or `embyr-server`.

## Enforcement

- Code-review convention plus a DISTILL-wave regression test asserting the literal string
  `sqlx::migrate!("../../migrations/customer")` (or the macro's resolved path) appears exactly once
  in the workspace source tree (`crates/embyr-pg-storage/src/backend_adapter.rs`) — a grep-based
  architecture test, mirroring the project's existing "sole importer" conventions (e.g.,
  `StripeGateway` as the sole `async-stripe` import site, ADR-021).
- `deny.toml` is extended to register `embyr-db-prep` with the same IO-permissive scope as
  `embyr-agent` and `embyr-server` (only `embyr-core` is IO-prohibited).
- `cargo deny check` (unchanged `deny.toml` rules) covers `embyr-db-prep`'s dependency tree
  automatically once registered.
