# Evolution: firestore-tls-support

**Date:** 2026-09-08
**Feature:** Server-side TLS on all 3 listeners (`:8080` gRPC, `:8081` REST/gRPC-Web, `:9090`
Admin), opt-in via 2 new config vars. Default (unset) behavior unchanged, byte-for-byte.
**Job:** JOB-13 (`production-deployment`) — Sam Chen, Service Operator/Platform Engineer.
**ADRs:** none new — decisions embedded in this feature's own `feature-delta.md`, mirroring
`firestore-or-filter-support`'s own precedent of keeping decisions in the narrative document.

## This closes gap #5 from the 2026-09-06 production-readiness scan

## Methodology note: the first feature this session run through the full nWave subagent pipeline

Every prior feature this session (`firestore-or-filter-support`, `firestore-is-null-filter-
support`) had the orchestrator write `feature-delta.md` directly and implement DELIVER itself —
collapsing DISCUSS+DESIGN into a single self-authored document rather than dispatching real
nWave subagents. Partway through this feature's own DISCUSS wave (started the same way), the
user corrected this explicitly: *"you are not using nwave correctly, go through all the phases,
use subagents."* The orchestrator discarded the in-progress ad-hoc attempt (reverted the commit,
discarded uncommitted code) and restarted via 4 real, separately-dispatched subagents:

1. **DISCUSS** — `nw-product-owner`. Wrote the full user story (US-01), 7 acceptance criteria
   (AC-TLS-01 through 07), scope assessment, out-of-scope reasoning, and DoR validation.
2. **DESIGN** — `nw-solution-architect`. Produced the complete code-level design: `TlsMaterial`,
   `ConfigError` variants, the shared `adapters/tls.rs::accept_maybe_tls` helper, the Admin
   listener's migration off `axum::serve`. Found a **stronger existing-code precedent** than the
   lead DISCUSS itself had flagged as unchecked (`tests/acceptance/embyr_agent/mod.rs`'s own
   mTLS test scaffold) — `tests/acceptance/us_12_agent_backend.rs` already used the exact
   `rustls`/`rustls-pemfile`/`rustls::pki_types` API shape this design needed, direct proof the
   API combination compiles in this codebase.
3. **DISTILL** — `nw-acceptance-designer`. Wrote all 7 acceptance tests
   (`pr05_tls_support.rs`), then proactively verified them by temporarily stubbing the two
   not-yet-existing production symbols, confirming a correct RED state, and reverting — before
   ever handing off to DELIVER.
4. **DELIVER** — `nw-software-crafter`. Implemented DESIGN's plan across all 7 named files, made
   every test pass, independently re-verified by the orchestrator (not just trusting the
   subagent's own self-report).

This is recorded as the standing methodology going forward for this project — every future
nWave feature should dispatch real subagents per wave from the start, not just after correction.

## Business Context

An operator (Sam Chen) deploying embyr WITHOUT a TLS-terminating load balancer in front — a
common self-hosted/bare-metal shape — had zero in-process option to encrypt any of the 3
listeners. Client SDK traffic (carrying API keys), admin traffic (carrying the admin key), and
REST/gRPC-Web browser traffic all traveled in plaintext with no mitigation available inside
embyr itself. LB-fronted deployments (today's assumed default) were unaffected by the gap and
had to remain unaffected by the fix — the single highest-priority acceptance criterion
(AC-TLS-01) throughout this feature.

## Key Decisions

| Decision | Verdict |
|---|---|
| D1-D5 (DISCUSS) | 2 new opt-in env vars, both-or-neither; one cert/key pair for all 3 listeners; read once at startup, no rotation; **no mTLS** (gap's severity argument never mentions it, only its title does); missing-file and bad-PEM named differently |
| D6-D11 (DESIGN) | Partial config reuses the existing `MissingVars` accumulator (no new variant); file-not-found/bad-PEM ARE 2 new `ConfigError` variants; `TlsMaterial` carries both raw PEM (tonic) and pre-built `Arc<rustls::ServerConfig>` (axum listeners), built once; **Admin listener migrates off `axum::serve`** (no TLS hook) onto a new `spawn_admin_server`, reusing the same `accept_maybe_tls` helper as the REST/gRPC-Web listener — not a new `axum-server` dependency; `rustls-pemfile` promoted dev→prod dep (mirrors this same `Cargo.toml`'s own `ed25519-dalek` promotion precedent) |

## Steps Completed

Both DISCUSS's single story (US-01) and its own walking skeleton were the entire feature — no
further slicing. All 7 ACs proven with real TLS handshakes (or real subprocess exit-code/stderr
assertions for the fail-fast paths), not mocked. Full regression clean apart from 3 known
pre-existing Docker/testcontainers-contention flakes (`drl_b12_postgres_rate_limit`,
`secrets_management`, `security_rules_cel_parity_cp04`), a 4th instance of the SAME flake class
hitting a different test this run (`firestore_equal_notequal_value_type_support_env01` —
`PortNotExposed` at container setup, confirmed transient via isolated rerun, zero overlap with
this feature's diff), and `pr02_dockerfile`'s own 5 pre-existing failures (no built Docker image
in this dev environment).

**QUALITY_GATE**: 31 mutants, 7 caught, 23 unviable, 1 accepted cosmetic miss. One genuine test
gap found and fixed along the way — see Lessons Learned.

## Lessons Learned

1. **The subagent-pipeline approach itself is this feature's own headline finding.** Real
   value showed up concretely, not just theoretically: DESIGN found a stronger precedent than
   DISCUSS's own flagged lead by doing its own independent codebase investigation; DISTILL
   caught its own potential RED-state ambiguity before handoff by proactively stubbing and
   reverting; each wave's own fresh perspective, unbiased by the prior wave's own framing,
   surfaced things a single self-authoring orchestrator likely would have carried forward
   unquestioned.
2. **`cargo-mutants` skips `#[ignore]`d tests by default — a real, non-obvious interaction for
   any feature with subprocess-based acceptance tests.** This feature's own AC-TLS-05/06/07 are
   `#[ignore]`d (matching this project's own established `pr04_graceful_shutdown.rs` precedent
   for real-subprocess tests), so 5 of the first mutation run's own 6 misses were purely an
   artifact of the tool never running the tests that actually cover that code. Passing bare
   `--ignored` to fix this is itself a trap: it pulls in EVERY ignored test in the binary,
   including unrelated pre-existing failures (`pr02_dockerfile`'s own missing-Docker-image
   issue), which fails the mutation run's own baseline before any mutant is even tested. Fix:
   scope `--ignored` with a name filter matching only the feature's own tests
   (`-- --ignored pr05`).
3. **Mutation testing found a genuine, real acceptance-test gap this time — not just
   environmental noise or cosmetic misses.** DISTILL's own `pr05_tls_support.rs` tested the
   cert-path-only partial-config direction but not the symmetric key-path-only direction. The
   production code was already correct; only the test coverage was asymmetric. This is exactly
   what mutation testing exists to catch, and it did.
4. **The env01 Docker-flake triage reinforced this session's own established discipline** (read
   the actual failure message before dismissing as flaky) — a new-looking failure
   (`firestore_equal_notequal_value_type_support_env01`) turned out to be the same
   `PortNotExposed` root cause already seen this session, just landing on a different test.

## Key Files

- `crates/embyr-server/Cargo.toml` — `rustls-pemfile` promoted to `[dependencies]`.
- `crates/embyr-server/src/config.rs` — `TlsMaterial`, `ConfigError::TlsFileNotFound`/
  `TlsInvalidPem`, TLS var resolution in `from_env()`, `load_tls_material()`.
- `crates/embyr-server/src/adapters/tls.rs` (new) — `TlsOrPlainStream`, `accept_maybe_tls()`.
- `crates/embyr-server/src/adapters/mod.rs` — `pub mod tls;`.
- `crates/embyr-server/src/rest/grpc_web.rs` — `spawn_hybrid_server`'s TLS-wrap step.
- `crates/embyr-server/src/lib.rs` — new `spawn_admin_server`; `spawn_all_servers`'s 2 new
  trailing params; new `start_test_server_with_tls` test constructor.
- `crates/embyr-server/src/main.rs` — TLS material construction wired before `spawn_all_servers`.
- `tests/production_readiness/acceptance/pr05_tls_support.rs` (new) — 8 tests (7 original +
  `exits_nonzero_when_only_key_path_is_set`, added during QUALITY_GATE).
- `docs/feature/firestore-tls-support/feature-delta.md` — full DISCUSS/DESIGN narrative.
- `docs/feature/firestore-tls-support/deliver/mutation/mutation-report.md` — full account of the
  `--ignored`-scoping investigation and the real gap found and fixed.

## Follow-Up Work

- **mTLS (client certificate verification)** — deferred, no evidenced need (see DISCUSS's own
  Out of Scope).
- **Cert rotation/hot-reload** — deferred, matches `EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY`'s own
  existing read-once-at-startup pattern.
- **Per-listener distinct TLS config** — deferred, one cert/key pair for all 3 listeners is this
  feature's own locked scope.
- **ACME/Let's Encrypt automation** — deferred, operator provides pre-issued PEM files.

Carried forward, unchanged, from `docs/product/known-gaps.md`: #7 (`secrets_management`
Docker/LocalStack timing flakiness), #8 (CEL "chaining" construct-detection gap). Gap #6 (no
graceful shutdown) was found stale/already-closed during this feature's own DISCUSS — see
`docs/product/known-gaps.md` row 6's own correction note.
