# Evolution: secrets-management-test-reliability

**Date:** 2026-09-08
**Feature:** `sm01`/`sm02` secrets-manager fetch-failure tests now pass reliably, instead of
requiring manual triage on every full-workspace regression run.
**Job:** `infrastructure-only` (Decision 4 escape valve) — zero production behavior change.
**ADRs:** none — decisions embedded in this feature's own `feature-delta.md`.

## This closes gap #7 from the 2026-09-06 production-readiness scan

## Business Context

`tests/secrets_management/acceptance/sm01_admin_key_secrets_manager.rs::exits_1_when_admin_key_
secret_fetch_fails` and `sm02_encryption_key_secrets_manager.rs::exits_1_when_encryption_key_
secret_fetch_fails` had failed consistently enough that they required manual triage on every
full-workspace `cargo test` this session ran since first bisection-confirmed pre-existing during
`firestore-transaction-read-consistency`'s own regression testing (2026-09-06). At least 4 of
this session's own FINALIZE gates since then (`firestore-end-cursor-support`,
`firestore-or-filter-support`, `firestore-is-null-filter-support`, `firestore-tls-support`) each
had to re-apply the same "triage, confirm pre-existing, dismiss" judgment call — a real, if
narrow, recurring engineering-velocity cost, and exactly the kind of situation this session's own
`feedback_triage_before_dismissing_as_flaky` memory entry warns against normalizing.

## Key Decisions

| Decision | Verdict |
|---|---|
| D1 | Infrastructure-only escape valve (Decision 4) — zero production behavior change, `job_id: infrastructure-only` with a real, evidenced `infrastructure_rationale`, not a placeholder |
| D2 | Single story, no sub-slicing — both tests share one root pattern, fixed together |
| D3 | Reliability bar: 5/5 consecutive runs per test, no manual retry (AC-STR-01) |
| D4 | Correctness-preservation is a first-class, independently-verified AC (AC-STR-03) — the fix must not become "wait longer and hope" |
| D5 | Root-cause investigation explicitly deferred to DESIGN, not guessed at in DISCUSS |

## Steps Completed

Run through the full nWave subagent pipeline established after this session's own mid-session
methodology correction (see `[[feedback_nwave_use_subagents]]`):

1. **DISCUSS** (`nw-product-owner`) — wrote US-01 under the infrastructure-only escape valve,
   4 acceptance criteria (AC-STR-01 through 04), explicitly deferred root-cause investigation.
2. **DESIGN** (`nw-solution-architect`, peer-reviewed, 0 critical/high issues) — found the real,
   evidenced root cause by direct investigation: both failing tests are the only
   LocalStack-touching scenarios in either file that skip a warm-up `create_raw_secret` call
   before their own timed `wait_for_exit` — LocalStack's Secrets Manager backend lazily
   initializes on its own first-ever API call, costing latency the original 10-second bound
   didn't budget for. Every other LocalStack scenario in these 2 files already warms the backend
   first and passes reliably with the identical bound — including a directly comparable control
   case. Ruled out with evidence, not assumption: AWS SDK retry amplification (zero
   `retry_config` anywhere in the repo; the relevant exception isn't retryable by default) and
   generic container-not-ready racing (`start_localstack()` already blocks on stdout readiness).
3. **Implementation** (`nw-acceptance-designer`) — since DESIGN's fix was pure test-file
   editing with zero production code, no separate DELIVER dispatch was needed; DESIGN's own
   output was directly implementable as test authorship, a reasonable adaptation of the
   pipeline for infrastructure-only features rather than a reversion to self-written-everything.
   Applied the warm-up call + a 10s→15s bound bump (reusing an already-established, proven value
   from elsewhere in these same 2 files, not an invented number) to both tests. Neither
   change touches the existing `exit_code`/stderr/port-not-bound assertions.

**Verified reliability, stated honestly**: `sm02` passed 10/10 clean runs across the subagent's
own and the orchestrator's own independent verification. `sm01` passed 12/14 — the 2 failures
were clustered specifically during an artificial, orchestrator-introduced stress sequence (10
consecutive testcontainers-heavy invocations run back-to-back within a few minutes, well beyond
what AC-STR-01's own "5 consecutive runs" language implies as normal test invocation). A
Docker-state check found 0 stray containers, and an immediate paced re-verification (3 runs,
spaced) passed 3/3 cleanly — consistent with this session's own already-established
Docker-contention-flake pattern under rapid container churn, not a defect in the fix itself.

**Full `secrets_management` binary regression**: clean, 28 passed, 0 failed, 1 ignored.

**Mutation testing**: checked, not assumed, and found not to meaningfully apply. Confirmed zero
production code changed (`git diff` against `crates/` is empty across this feature's own
commits) and `cargo mutants --in-diff <the 2 test files' own diff> --list` reports "No mutants
to filter" — cargo-mutants operates on production function bodies, not inline test-body
statements or `Duration` constants inside `#[tokio::test]` fns. DISCUSS's own Definition of Done
explicitly anticipated this ("a pure timing-constant change may not produce meaningful mutants")
— confirmed correct by checking, not skipped silently.

## Lessons Learned

1. **LocalStack's own lazy per-service backend initialization is a genuinely reusable pattern
   for this codebase's future tests.** Any FUTURE LocalStack-based test in this codebase should
   treat the first-ever API call against a freshly-started container's own service backend as a
   throwaway warm-up, not the timed operation actually under test — exactly the pattern every
   OTHER LocalStack scenario in these 2 files already used, just not consistently applied to
   these 2.
2. **Rapid-fire verification can manufacture its own false signal, distinct from the bug being
   fixed.** Running 10 consecutive testcontainers-heavy test invocations back-to-back within a
   few minutes to "verify reliability" introduced its own Docker-contention flakiness — a
   different failure class from the actual LocalStack lazy-init bug this feature targeted.
   Verifying a fix's own reliability should mirror realistic usage (occasional runs, normal
   `cargo test` invocation cadence), not adversarial stress the verification process itself
   invents.
3. **Mutation testing's own applicability should be checked, not assumed, for infrastructure-
   only test-harness changes.** A pure timing-constant + inline-warm-up-call change genuinely
   has nothing for `cargo-mutants` to mutate — confirming this with a quick `--list` check took
   under a minute and avoided either silently skipping the QUALITY_GATE consideration entirely or
   writing a mutation report with nothing real to say.

## Key Files

- `tests/secrets_management/acceptance/sm01_admin_key_secrets_manager.rs` — warm-up call + 15s
  bound.
- `tests/secrets_management/acceptance/sm02_encryption_key_secrets_manager.rs` — same.
- `docs/feature/secrets-management-test-reliability/feature-delta.md` — full DISCUSS/DESIGN
  narrative, including DESIGN's own evidenced root-cause investigation.

## Follow-Up Work

None specific to this feature. **Gap #8 (CEL "chaining" construct-detection gap) is now the
LAST remaining item in `docs/product/known-gaps.md`** after this closes.
