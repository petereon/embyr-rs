# Evolution: occ-precondition-validation

**Date:** 2026-09-10
**Feature:** A malformed OCC `Precondition.update_time` (out-of-range seconds/nanos) on a write
RPC now returns a clean `INVALID_ARGUMENT` instead of panicking the handling task.
**Job:** JOB-11 (`fair-multitenancy`) — reused, persona P2 Sam Chen.
**ADRs:** none new — narrow validation fix, no new architectural pattern.

## This closes finding #9 from `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

`to_datetime(seconds, nanos)` in `crates/embyr-pg-storage/src/backend_adapter.rs` used
`.expect("valid timestamp")`, panicking whenever a client-supplied `Precondition.update_time`
(the "only write if the document's `update_time` still matches" OCC precondition any Firestore
write RPC can attach) fell outside chrono's representable range. No real Firestore SDK can
construct such a value client-side, but a raw gRPC caller can trivially trigger it. Tokio's
per-task isolation means the panic only kills the one offending request, not the whole process —
but the client sees a broken connection instead of a clean, documented error.

## Key Decisions

| Decision | Verdict |
|---|---|
| Fix location | Make `to_datetime` itself fallible (`Result<DateTime<Utc>, CoreError>`) in the shared `embyr-pg-storage` crate — closes BOTH `embyr-server`'s and `embyr-agent`'s own independent, structurally-identical zero-validation call paths with one change, discovered by DISCUSS reading both binaries |
| Validation order | `nanos` checked first, before the `as u32` cast (a negative nanos would otherwise wrap to a huge positive number and get misattributed to the seconds check) |
| No early validation | `convert_precondition`/`parse_precondition` (the two call sites building the `WritePrecondition` enum) stay untouched — both already run before any SQL executes, so the only savings from earlier validation would be a single pooled-connection acquire, not worth the ripple to their own return types |
| No `embyr-core` extraction | Single call site (2 uses within one function's own module), no duplication to justify pulling the range checks into a shared crate |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with peer
review, DISTILL=`nw-acceptance-designer`, DELIVER=`nw-software-crafter`):

1. **DISCUSS**: found the second, independent vulnerable call path in `embyr-agent`'s own
   `parse_precondition` — the audit finding's own citation only named the server-side location.
   Confirmed via direct read that Tokio's per-task isolation contains the panic to one request.
   Confirmed no existing reusable timestamp-validation helper exists anywhere in the workspace.
2. **DESIGN**: peer-reviewed, 0 critical/high. Locked the exact signature, validation order, error
   messages, and confirmed the complete call-site blast radius (exactly 2, both already inside
   `Result<_, CoreError>`-returning functions — a clean `?` propagation).
3. **DISTILL**: extended 3 existing test files (no new files/registrations needed) with 4 new
   scenarios. Empirically confirmed RED — all 3 call sites currently panic, contained by Tokio, the
   client observing `tonic::Code::Cancelled` rather than a crash or the target `INVALID_ARGUMENT`.
4. **DELIVER**: implemented exactly as designed — a genuinely minimal diff (add the two range
   checks, `?` at both call sites). All 4 new scenarios green; all pre-existing regression guards
   for the valid-match, valid-stale-mismatch, and `Exists`-precondition cases unaffected.
5. **Orchestrator's full-workspace regression**: clean (1 unrelated, already-documented
   `drl_b12_postgres_rate_limit` flake, confirmed transient via isolated rerun).
6. **QUALITY_GATE**: hit a real, previously-documented cargo-mutants cross-package limitation
   (auto-detects the mutated file's OWN package for its test invocation; neither
   `--test-workspace true` nor `--test-package` overrides this, confirmed empirically both ways,
   refining the earlier `firestore-transaction-read-consistency` finding — even a genuinely
   multi-package `--in-diff` didn't help here). Resolved by adding a small, direct `#[cfg(test)]`
   unit-test module for `to_datetime` in the same file — gives real, fast, single-package mutation
   coverage with zero further workaround needed. Result: 7/8 caught, 1 scoping-artifact miss (a
   whole-function-stub mutant on the unrelated `commit_transaction` method, by-construction
   unreachable from a `--lib`-only scope, with real coverage confirmed at the integration level).

## Lessons Learned

1. **When a mutation-testing cross-package limitation can't be worked around via cargo-mutants'
   own flags, adding a small, direct unit test in the SAME package as the mutated code is a
   cleaner, more durable fix than continuing to fight the tool's own package-detection logic.**
   This sidesteps the entire class of problem rather than searching for the "right" incantation of
   `-p`/`--test-workspace`/`--test-package` (none of which worked here, extending this session's
   own prior finding on this exact tool limitation). A useful default going forward: if a
   mutated function is a good candidate for direct unit testing (pure-ish, small, branching logic),
   prefer adding that test over chasing cross-package mutation-testing workarounds.
2. **A single-cause bug (one `.expect()` call) can have multiple independent trigger paths across
   different binaries sharing a crate** — this is now the SECOND finding this session (after
   `wire-secret-fetchers`) where DISCUSS's own careful reading of BOTH `embyr-server` and
   `embyr-agent` surfaced a second vulnerable call path the audit's own citation missed. Worth
   treating as a standing checklist item: any finding touching shared-crate code should always be
   checked against BOTH binaries that consume it, not just the one the finding happened to cite.

## Key Files

- `crates/embyr-pg-storage/src/backend_adapter.rs` — the only production file changed; `to_datetime`
  signature change, 2 call-site `?` additions, new `to_datetime_tests` unit-test module.
- `tests/acceptance/us_02_write_document.rs`, `us_06_transactions.rs`,
  `tests/acceptance/embyr_agent/us_a02_write_operations.rs` — 4 new acceptance scenarios.
- `docs/feature/occ-precondition-validation/deliver/mutation/mutation-report.md` — full account.

## Follow-Up Work

Findings #10-#19 (High) remain, plus #20+ (Medium/Low). Next natural target: #10 (raw
Postgres/sqlx driver error text leaked verbatim to the client on 52+ call sites) or #11
(`embyr-agent`'s own weaker field-path validator — a real SQL-injection path into the customer's
own Postgres via an mTLS-gated client-cert holder).
