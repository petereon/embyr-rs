# Evolution: sanitize-backend-error-messages

**Date:** 2026-09-12
**Feature:** A backend/database failure (unreachable Postgres, connection pool exhaustion, etc.)
now returns a generic `"internal server error"` message to gRPC clients instead of the raw
driver/schema error text — across 26 call sites in both `embyr-server` and `embyr-agent`.
**Job:** JOB-11 (`fair-multitenancy`) — reused, persona P2 Sam Chen.
**ADRs:** ADR-075 (new) — backend error message sanitization boundary.

## This closes finding #10 from `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

`core_error_to_status`'s catch-all arm (and, it turned out, dozens of direct-bypass call sites)
returned `Status::internal(e.to_string())` — the raw `sqlx`/Postgres error text verbatim to the
client. Confirmed via direct testing that this includes literal column names (e.g.
`"ecies_encrypted_dsn"`, `"api_key_hash_current"`) from `try_get` decode-error messages, and
connection-pool internals ("pool timed out while waiting for an open connection", "attempted to
acquire a connection on a closed pool") — information a client should never see.

## Key Decisions (ADR-075)

| Decision | Verdict |
|---|---|
| Scope | The audit's own "52+ call sites" and the orchestrator's own "177 call sites" both counted `CoreError::BackendUnavailable` *creation* sites (correct as-is — they capture the real error for logging). The actual fix belongs at the CONVERSION boundary (where the error becomes a client-facing `Status`), which DESIGN's own exhaustive re-classification found at exactly **26** sites across both binaries — 17 more than DISCUSS's own initial count |
| Mechanism | One shared `sanitize_backend_error(e, context) -> Status` helper, TWO independent mirrored copies (not a shared crate — `embyr-core` must stay IO-free, `tonic::Status` isn't available there; matches the existing `core_error_to_status` precedent, which is already duplicated per-binary) |
| Message text | A single, fixed `"internal server error"` string, zero variation by RPC — varying it would itself leak which code path failed |
| Untouched | 51 other `Status::internal(` sites confirmed already-safe (JoinError, static strings, caller-input echo, already-a-`tonic::Status` transport decode, etc.) and every OTHER `CoreError` variant (`InvalidArgument`, `DocumentNotFound`, etc.) — a precise, scoped fix, not a blanket rewrite |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with peer
review, DISTILL=`nw-acceptance-designer`, DELIVER=`nw-software-crafter`):

1. **DISCUSS**: found the true scope is much bigger than the audit's own citation — `embyr-agent`
   has its own independent, differently-shaped `core_error_to_status` with an explicit
   `BackendUnavailable => Status::internal(msg)` leak arm, plus a direct bypass at its own
   `subscribe()` handler (later confirmed as currently-dead code, fixed defensively anyway).
   Confirmed via a real triggered constraint violation that raw driver text genuinely does leak
   schema detail. Confirmed zero existing tests assert on `Status::internal`'s own message
   content (no regression risk from changing message text). 11 ACs, DoR 9/9.
2. **DESIGN**: peer-reviewed, 0 critical/high. Re-ran DISCUSS's own grep across ALL 77
   `Status::internal(` sites in both binaries and individually classified every one — found 17
   MORE genuine leak sites DISCUSS's own narrower reading missed (every
   `system_db.get_*_access_rule`/`list_*_access_rule_pattern*` call across `RunQuery`,
   `RunAggregationQuery`, write handlers, `GetDocument`), bringing the total to 26. Locked the
   shared-helper mechanism and produced an exact, exhaustive DELIVER worklist.
3. **DISTILL**: wrote 8 acceptance scenarios spanning both binaries. Empirically confirmed RED
   across 5 distinct real scenarios (authenticate, 2 access-rule-lookup sites, the bare-`sqlx::Error`
   `handle_listen` site, and `embyr-agent`'s own `get_document`) — all currently leak raw driver
   text verbatim, confirmed via actual captured response content, not assumed.
4. **DELIVER**: implemented exactly per DESIGN's own 26-site worklist. All 8 scenarios green;
   regression guards (`us_10_aws_secrets`, `us_11_gcp_secrets`, full `embyr_agent` suite)
   unmodified and green.
5. **Orchestrator's full-workspace regression**: clean (1 unrelated, already-documented
   `cargo-sweep`-class transient interference, confirmed via isolated rerun).
6. **QUALITY_GATE**: two scoped mutation runs (one per package, the now-established pattern for
   this session's own cross-package cargo-mutants limitation). `embyr-agent` side fully clean
   (1/1 caught, 0 missed) once scoped to the full `embyr_agent` suite via `--include-ignored`.
   `embyr-server` side found 7 misses — investigated each individually (not hand-waved), all
   confirmed as whole-function-stub scoping artifacts with real coverage in 87+ pre-existing test
   files (the entire `security_rules*` suite for the two access-rule-evaluation misses,
   write/delete/list-document tests for the three RPC-handler misses, listen tests for the
   `handle_add_target` miss, and multiple existing `FailedPrecondition`-asserting tests for the
   match-arm-deletion miss) — none touched by this run's own narrow 4-test scope.

## Lessons Learned

1. **A finding's own audit citation is a starting point, not the full scope — always re-run the
   grep DISCUSS used, one wave later, with fresh eyes.** DESIGN's own re-classification found 17
   MORE genuine leak sites (nearly triple DISCUSS's own count) by systematically tracing every one
   of 77 candidate sites rather than trusting DISCUSS's own narrower initial reading. This is the
   THIRD finding this session (after #7 and #9) where a later wave's own re-investigation expanded
   a finding's true scope significantly beyond its own citation or the prior wave's own count.
2. **When a mutation run's own test-command scope is deliberately narrow (only the feature's own
   new acceptance tests), whole-function-stub mutants on OTHER, pre-existing functions the diff
   happens to touch are near-guaranteed noise, not real gaps — but still worth individually
   confirming real coverage exists elsewhere, not just assumed.** A quick, targeted grep for each
   miss's own pre-existing test coverage (rather than a blanket "this is probably noise"
   dismissal) took only a few tool calls and gave genuine confidence.
3. **For a cross-package mutation run, scoping the test command to the FULL relevant test suite
   (e.g. `--include-ignored` for a small binary's own aggregated test target) avoids the
   scoping-artifact class entirely when the diff is small enough that this is cheap** — the
   `embyr-agent` side's own clean 1/1 result (vs. `embyr-server`'s 7 misses) came directly from
   this choice, made possible because `embyr-agent`'s own test suite is small enough to run in
   full per-mutant without the runtime cost the much larger `embyr-server` suite would incur.

## Key Files

- `crates/embyr-server/src/grpc/handler.rs` — new `sanitize_backend_error` helper, 21 call-site
  swaps.
- `crates/embyr-server/src/realtime/listen_handler.rs` — 2 call-site swaps (imports the
  server-side helper).
- `crates/embyr-agent/src/server.rs` — its own `sanitize_backend_error` helper, 3 call-site swaps.
- `tests/sanitize_backend_error_messages/acceptance/{sbm01,sbm02,sbm03,sbm04}*.rs`,
  `tests/acceptance/embyr_agent/us_a08_error_sanitization.rs` — 8 new acceptance scenarios.
- `docs/product/architecture/adr-075-backend-error-message-sanitization-boundary.md`.
- `docs/feature/sanitize-backend-error-messages/deliver/mutation/mutation-report.md` — full account.

## Follow-Up Work

Findings #11-#19 (High) remain, plus #20+ (Medium/Low). Next natural target: #11
(`embyr-agent`'s own weaker field-path validator — a real SQL-injection path into the customer's
own Postgres via an mTLS-gated client-cert holder) or the admin-signin pair (#12/#13:
unthrottled Argon2id + timing-oracle email enumeration).
