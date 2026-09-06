# Evolution: firestore-malformed-filter-shape-validation

**Date:** 2026-09-06
**Feature:** A malformed/adversarial raw gRPC `RunQuery` filter — a non-`Array` value paired with
`In`/`NotIn`/`ArrayContainsAny`, or a `Null` value paired with a range operator — now returns a
clean `INVALID_ARGUMENT` instead of panicking the request task.
**Job:** JOB-11 (`fair-multitenancy`) — **NOT** JOB-01 (`sdk-compat`). See § Lessons Learned.
**ADRs:** none (folded into feature-delta.md's own DESIGN section).

## This is operational-clarity polish, not a crash-risk fix

This feature is a direct follow-up flagged by `firestore-range-operator-value-type-support`'s own
FINALIZE — but DISCUSS-wave investigation there was correct to name it "a DIFFERENT, LOWER-priority
category" from the 3-feature crash-elimination arc that FINALIZE closed, and this feature's own
DISCUSS wave confirmed why in more detail: **no real Firestore SDK client can ever construct the
malformed request shapes this feature guards against.** The `In`/`NotIn`/`ArrayContainsAny`
query-builder methods on every real SDK only ever accept an array; no real SDK exposes a way to
pass `null` to a range-comparison method. Triggering either trigger case requires a caller
deliberately hand-crafting a raw gRPC request no ordinary client would ever produce.

The 3-feature crash-elimination arc (`firestore-query-filter-operator-support` →
`firestore-equal-notequal-value-type-support` → `firestore-range-operator-value-type-support`)
**remains fully closed** as reported at its own FINALIZE. This feature does not reopen, extend, or
add to that arc — it closes a separate, smaller, self-contained gap: turning an
already-harmless-to-other-tenants panic into a clean, named error, purely for the benefit of an
operator (Sam Chen, P2) reading logs/traces, not a caller's own query correctness.

## Business Context

Today, the 4 remaining `panic!` sites reachable only via malformed/adversarial raw gRPC requests
(`crates/embyr-pg-storage/src/encoding/query.rs` lines 75, 96, 138, 251) show up in an operator's
own logs/traces as alarming, uninformative raw Rust panics — indistinguishable at a glance from a
genuine internal server defect. Tokio's own default per-task panic isolation (no custom
`catch_unwind`/panic hook exists in `main.rs`/`lib.rs`, confirmed by direct grep, and repeatedly
observed empirically this session) already confines the blast radius to the SAME caller's own
single request — no other tenant, no process-wide impact. This feature does not change that blast
radius; it changes what the failure LOOKS like in an operator's own operational surface: a clean,
named `INVALID_ARGUMENT` instead of an unexplained panic.

## Key Decisions

| Decision | Verdict |
|---|---|
| Reuse JOB-11 (`fair-multitenancy`, P2 Sam Chen), NOT JOB-01 (`sdk-compat`, P1 Alex) — no real Firestore SDK client can trigger either malformed shape | feature-delta.md § Resolution 1 |
| Priority explicitly recalibrated: a small, quick hardening fix, NOT a reopening of the crash-elimination arc | § Resolution 2 |
| Mechanism: extend `translate_filter`'s existing Array/Map-rejection check with 2 more `if` conditions, identical layering to the immediately-prior feature | § Resolution 3 |

## Steps Completed

1. **Slice 01 (the entire feature, Walking Skeleton)** — `translate_filter` (`embyr-server`) gains
   2 more checks, immediately following the existing Array/Map check: (a) `In`/`NotIn`/
   `ArrayContainsAny` given a non-`Array` value → named rejection; (b) a range operator given `Null`
   → named rejection. Both reuse the SAME `Option<Result<QueryFilter, String>>` →
   `Status::invalid_argument` mechanism already established by the immediately-prior feature — zero
   new validation infrastructure.

**QUALITY_GATE** — `cargo-mutants --in-diff`, `--lib`-scoped to `malformed_filter_shape_tests::`.
First pass found 1 missed mutant; fixed and confirmed closed on a second pass (6 caught, 1
unviable, 0 missed). See § Lessons Learned.

**Full regression**: `cargo test -p embyr-server`, all targets passed (150 tests across the
package's own unit + integration suites, 0 failures attributable to this feature; one unrelated
`admin_api_v2_walking_skeleton` failure during the run was a known cargo-sweep shared-target-dir
race — "never executed" binary error — not a code regression, confirmed clean on rebuild-and-retry).

## Lessons Learned

1. **A follow-up candidate's own suggested framing is a hypothesis, not a given — investigate
   before locking scope.** The originating flag suggested confirming JOB-01 reuse "though note this
   is a hardening/defensive-input concern." Direct investigation (who can actually trigger this,
   what's the actual blast radius) confirmed the suspicion and went further: JOB-01 doesn't merely
   fit poorly, it doesn't apply at all — Alex's real app can never construct the malformed input
   this feature guards against, so this feature does nothing to advance JOB-01's own goal. The
   correct persona was P2 Sam Chen / JOB-11 throughout. Blindly reusing the prior 3 features' own
   JOB-01 pattern would have mis-attributed this feature's own value to the wrong persona.
2. **A wrong string can itself contain the substring a loose `.contains()` assertion checks for.**
   The first `cargo-mutants` pass found a mutant that deleted the `FilterOp::In` match arm inside
   the rejection's own error-message builder, falling through to the `ArrayContainsAny` arm and
   producing the WRONG message text (`"array-contains-any"` instead of `"in"`) while still
   returning `Err`. The test asserted `err.contains("in")` — which `"array-contains-any"` also
   satisfies, since it contains "in" as a substring of "contains". Fixed by tightening all 4 new
   rejection tests to exact `assert_eq!` on the full error-message text. This is the same shape of
   gap as the join-boundary miss found during `firestore-query-filter-operator-support`'s own
   QUALITY_GATE — presence-only assertions are systematically weaker than they look whenever a
   plausible-but-wrong value could itself contain the expected substring.

## Key Files

- `crates/embyr-server/src/grpc/handler.rs` — `translate_filter`'s 2 new rejection checks, 6 new
  unit tests (`malformed_filter_shape_tests`).
- `tests/firestore_malformed_filter_shape_validation/acceptance/mfs01_reject_malformed_shapes.rs` —
  4 real end-to-end tests: non-array + `In`, non-array + `NotIn`/`ArrayContainsAny`, `Null` +
  `LessThan`, and a regression guard proving well-formed `In`/range queries are unaffected.
- `docs/feature/firestore-malformed-filter-shape-validation/feature-delta.md` — full DISCUSS/DESIGN
  narrative, including the persona/job reassignment investigation.
- `docs/feature/firestore-malformed-filter-shape-validation/deliver/mutation/mutation-report.md`
- `docs/product/jobs.yaml`, JOB-11 — new NOTE appended.

## Follow-Up Work

None. This closes the malformed-input hardening gap explicitly named at the end of
`firestore-range-operator-value-type-support`'s own mutation report and evolution doc — the only
2 trigger shapes that flag identified are both fixed here. No further follow-up candidate remains
from this session's Firestore-parity work.

Carried forward, unchanged, from prior features (not in scope for this feature): list-size limit
enforcement, empty-list behavior confirmation, two-simultaneous-`IN`, the
range-filter-orderBy-must-start-same-field query-validity constraint, true segment-wise `Reference`
comparison for cross-depth paths.
