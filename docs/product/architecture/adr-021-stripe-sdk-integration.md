# ADR-021: Stripe Integration via `async-stripe`, Not Hand-Rolled `reqwest`

## Status

Accepted

## Context

`card-payments-backend` requires calling Stripe's API for: Customer creation/lookup (US-201),
Subscription create/update (US-202), Invoice-driven dunning signals delivered via webhooks
(US-203/204), and Usage Record push for metered overage billing (US-205) — plus verifying
`Stripe-Signature` webhook headers (US-203). No `stripe`/`async-stripe` crate exists in this
workspace today (`Cargo.toml` has no Stripe dependency).

This project's established precedent for third-party cloud integrations is **mixed, not
uniformly hand-rolled**:

- `crates/embyr-server/src/adapters/aws_secret_fetcher.rs` uses the **official, full AWS SDK**
  crate (`aws-sdk-secretsmanager`, already a workspace dependency) — a large, typed, actively
  maintained vendor SDK.
- `crates/embyr-server/src/adapters/gcp_secret_fetcher.rs` **hand-rolls** GCP Secret Manager access
  via `reqwest` against the REST API directly — because no comparably mature official/community Rust
  GCP SDK crate was judged worth the dependency for a *single-endpoint* (`AccessSecretVersion`)
  integration.

The distinguishing factor between these two precedents is not "hand-roll always" — it is **API
surface size and the availability of a mature typed SDK**: AWS Secrets Manager's single-endpoint
surface still pulled in the official SDK because it existed and was well-maintained; GCP's surface
was hand-rolled because no equally strong option existed for that single endpoint.

Stripe's required surface for this feature is materially larger than either precedent: Customers,
Subscriptions, Invoices (read via webhook payload), Usage Records, and HMAC-based webhook signature
verification — five distinct API areas, several with retry/idempotency-key semantics, plus a
security-critical cryptographic verification step.

## Decision

**Use `async-stripe` (the community-maintained, de facto standard Rust Stripe client) as the
`embyr-server`-only dependency for all Stripe HTTP interaction, rather than hand-rolling calls via
the existing `reqwest` dependency.**

- License: MIT. OSS-first criteria satisfied (active maintenance, wide adoption, standard choice
  cited across the Rust ecosystem for Stripe integration).
- Version: latest published stable release at implementation time (crafter pins the exact
  semver in `Cargo.toml`, following this project's existing convention of pinning a version
  *series*, e.g. `sqlx = "0.8"`, `tonic = "0.12"` — not a specific patch).
- `async-stripe` is added to `[workspace.dependencies]` in the root `Cargo.toml` (alongside
  `aws-sdk-secretsmanager`, under a new `# Stripe SDK` comment section) and to
  `crates/embyr-server/Cargo.toml` only — `embyr-core` never depends on it (IO-prohibition,
  `deny.toml`, unchanged).
- Webhook signature verification uses `async-stripe`'s built-in
  `Webhook::construct_event(payload, signature_header, webhook_secret)` helper rather than a
  hand-rolled HMAC-SHA256 timing-safe comparison.
- Idempotency keys for Usage Record pushes (US-205, AC-205-03) are passed via `async-stripe`'s
  native idempotency-key request parameter, reusing Stripe's own server-side idempotency guarantee
  rather than building a local `processed_usage_records` ledger table.

## Alternatives Considered

### Alternative 1: Hand-roll via `reqwest`, mirroring the `GcpSecretFetcher` precedent (rejected)

**Rejected because:**
- **API surface size.** `GcpSecretFetcher` hand-rolls exactly one endpoint
  (`AccessSecretVersion`). Stripe requires five distinct resource areas (Customers, Subscriptions,
  Invoices/webhook events, Usage Records, webhook signatures), each with its own request/response
  JSON shape. Hand-rolling all five multiplies the maintenance surface roughly five-fold over the
  precedent this alternative claims to mirror — the analogy does not hold at this scale.
- **Security-critical primitive.** Webhook signature verification is a timing-safe HMAC comparison
  over a signed payload + timestamp, with replay-window tolerance semantics Stripe documents
  precisely. This project's own precedent for exactly this class of risk
  (`docs/product/architecture/brief.md` § Third-party reuse table) already rejected hand-rolling
  comparable security-critical primitives: *"Argon2id hash: Roll-own: rejected (security-critical,
  no benefit to custom impl)."* The same reasoning applies directly here — a subtly wrong hand-rolled
  timestamp-tolerance or byte-comparison check is exactly the class of bug `async-stripe`, exercised
  by a large user base, has already had shaken out.
- **D-13's real-network testing requirement raises the cost of a hand-rolled bug.** Every acceptance
  test in this feature must hit real Stripe test-mode APIs (no mocked port). A hand-rolled JSON
  shape that happens to satisfy today's Stripe test-mode responses but diverges subtly from the
  documented contract is a correctness risk that a maintained typed SDK reduces materially.
- The nearer precedent is `aws-sdk-secretsmanager` (full vendor SDK, adopted because the surface
  and security stakes justified it), not `GcpSecretFetcher` (single simple endpoint).

### Alternative 2: A minimal, purpose-built internal Stripe client (subset of `async-stripe`'s surface, hand-rolled) (rejected)

Write a narrow internal client exposing only the exact five call shapes this feature needs,
avoiding `async-stripe`'s full API surface (hundreds of Stripe resources this feature never
touches).

**Rejected because:** this still requires hand-rolling webhook signature verification (the primary
security concern above applies unchanged), and the "unused surface area" argument is weak — Rust's
dead-code elimination and `async-stripe`'s modular feature-flagging mean unused resource types add
negligible compiled-binary cost, and the crate's typed request builders reduce, not increase,
implementation risk relative to hand-rolled JSON construction for the exact five call shapes this
feature *does* use.

## Consequences

### Positive

- Webhook signature verification is delegated to a widely-exercised, security-focused
  implementation rather than a first-attempt hand-rolled HMAC comparison.
- Typed request/response structs for Customer/Subscription/Invoice/UsageRecord reduce the risk of
  JSON-shape drift silently breaking production while test-mode calls still pass.
- Native idempotency-key support avoids a redundant local ledger table for US-205's
  once-per-project-per-dimension-per-day guarantee.
- Consistent with this project's actual (not idealized) precedent: full vendor SDKs are adopted
  when the API surface and risk profile justify it (AWS), hand-rolling is reserved for genuinely
  narrow single-endpoint integrations (GCP).

### Negative / Trade-offs

- A new, fairly large dependency lands in `embyr-server` (not `embyr-core` — IO-prohibition
  preserved). This is a real increase in the dependency graph this project has otherwise kept
  deliberately small; explicitly weighed, not defaulted to.
- `async-stripe` is community-maintained (not an official Stripe-published crate) — mitigated by
  its wide adoption and active maintenance; if it stagnates, the `StripeGateway` adapter (see
  Component Decomposition) is the sole call site, isolating a future swap to hand-rolled `reqwest`
  or a different client to one file, mirroring the isolation `SecretFetcher`-shaped adapters already
  provide for AWS/GCP.
- The webhook signature helper's exact tolerance/replay-window behavior is inherited from the crate,
  not independently specified by this ADR — acceptable, since Stripe's own documented tolerance
  (5-minute default) is what `async-stripe` implements, and this feature does not need a different
  tolerance.

## Enforcement

- `cargo deny check` (unchanged `deny.toml`) continues to block `async-stripe` (and any transitive
  dependency it pulls) from `embyr-core`'s dependency graph — Stripe HTTP calls exist only in
  `embyr-server::adapters::stripe_gateway`.
- License check (`cargo deny check`, license allowlist) passes `async-stripe`'s MIT license without
  a new exception.
- `StripeGateway` is the sole module permitted to import `async-stripe` types directly — enforced by
  code review convention (mirrors the existing, not-yet-tool-enforced convention that
  `AwsSecretFetcher`/`GcpSecretFetcher` are the sole importers of their respective cloud SDK types).
  No new automated import-boundary tool is introduced for this single-adapter convention — consistent
  with this project's existing practice of enforcing adapter-import isolation by convention plus
  code review at the sub-crate-module level, reserving `cargo-deny`/proc-macro enforcement for the
  crate-boundary (`embyr-core` IO-prohibition) and probe-presence invariants (Principle 12).
