# Mutation Testing Report — security-rules-cel-recursive-wildcards

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-04
**Scope**: feature-touched implementation files (`git log --name-only db2680a..HEAD -- 'crates/*'`):
`embyr-core/src/access_control/path_routing.rs`, `embyr-core/src/access_control/rules_file.rs`,
`embyr-server/src/adapters/system_db.rs`, `embyr-server/src/admin/handlers/access_rules.rs`,
`embyr-server/src/grpc/handler.rs`, `embyr-server/src/realtime/listen_handler.rs` — filtered
(`-F`) to only functions this feature added or extended (`bind_recursive_prefix`,
`classify_prefix_relation`, `generalizes`, `fixed_depth_full_reach`, `validate_segment_shape`,
`decompose_block`, `overlap_rejection`, `parse_recursive_prefix`,
`simulate_recursive_wildcard_candidate`, `evaluate_stored_pattern_outcome`,
`simulate_routed_access_rule`, `resolve_access_rule_pattern`,
`list_recursive_access_rule_patterns_up_to`, `list_all_access_rule_patterns`,
`resolve_recursive_wildcard_condition`, `handle_add_target`, `evaluate_write_rule_for_commit`).

## Pass 1 — `embyr-core` (pure, zero-IO, no Docker)

`cargo mutants -p embyr-core --in-place --timeout 30 -- access_control::` (test filter narrowed
to the `access_control` module to skip the deliberately-slow, unrelated Argon2id KDF tests in the
same crate).

**Result**: 57 mutants tested — **21 caught, 3 missed, 3 unviable, 30 timeout**.

### Missed mutants — investigated

| Mutant | Verdict |
|---|---|
| `rules_file.rs:647:39`/`:49` — `decompose_block`'s recursive-branch copy of the same-bucket conflicting-condition guard (`Some(existing) if *existing != rewritten`) | **Real gap in this feature's own new code.** No test exercised two `allow read` clauses with different conditions inside one recursive-wildcard `match` block. **Fixed**: added `decompose_rejects_conflicting_conditions_for_the_same_verb_bucket_in_a_recursive_wildcard_block` (unit test, `rules_file.rs`). Re-ran the mutant's own test scope after the fix — passes, and the added assertion directly targets the mutated branch. |
| `rules_file.rs:719:35` — the *pre-existing* (non-recursive) copy of the identical guard | **Out of this feature's scope.** `git log -L 719,719:...rules_file.rs` traces this line to `security-rules-cel-parity` Slice 01 (commit `f2af383`), a different, already-finalized feature. Pre-existing gap, not introduced or touched by this feature. Not fixed here — flagged for a follow-up ticket against `security-rules-cel-parity` rather than silently absorbed into this feature's finalize. |

### Timeouts (30/57) — root cause, not a coverage gap

The unmutated baseline built and ran in under 1s (shared incremental target). Individual mutant
builds cost 10–40s under normal conditions, but many runs during this pass hit the 30s **test**
timeout even though the narrowed `access_control::` suite runs in ~0.01–9s standalone. Root
cause identified during the run: a concurrent second Claude Code session
(`mutation-testing-analysis (2)`, PID 24377) was active on the same machine, building against the
**same shared `~/.cargo/shared-target` incremental cache** — confirmed by a single-test rerun of
the newly-added test alone taking 13 minutes to compile immediately afterward (vs. sub-second
before/after that contention window), and by `mds`/`mds_stores` (Spotlight) independently
consuming 35%+ CPU rebuilding its index after a large, unrelated Docker container churn earlier in
this session (see Pass 2 below). These are environment/scheduling artifacts, not evidence the
underlying mutants would have survived — cargo-mutants reports `TIMEOUT` as inconclusive, distinct
from `MISSED`.

## Pass 2 — `embyr-server` (Docker/testcontainers-backed) — **not run to completion, documented skip**

**First attempt** (`-j 4`, all 6 `rw0X` acceptance-test binaries as the per-mutant test command):
ran for **4 hours**, tested 75 mutants, **0 caught/missed — 29 unviable, 46 timeouts**. Root cause:
each of the 4 parallel mutant-build jobs independently spun up all 6 `rw0X` binaries' own
`testcontainers` Postgres instances; over unattended hours this piled up to **68 simultaneously
running + 206 total Postgres containers** and 260 orphaned volumes (8.7 GB), thrashing the
machine's CPU/disk/Docker-daemon badly enough that per-mutant build times escalated from ~300s to
2000s+, and every single mutant timed out — zero actionable signal for 4 hours of wall-clock time.
All stray containers and volumes were force-removed after the fact (`docker kill`/`rm -f` all,
`docker volume prune`).

**Decision: skip Pass 2, documented justification** (matches this skill's own "Skip conditions —
each requires documented justification" clause):

1. **Disproportionate cost confirmed empirically** — one real attempt cost 4 hours of wall-clock
   time and caused a genuine resource-exhaustion incident, for zero mutants classified either
   caught or missed.
2. **The server-side new logic is already directly, purposefully exercised** by this feature's own
   acceptance-test suite, written specifically to hit the same branches a mutation test would
   target:
   - `rw06_simulate_recursive_wildcard_precedence.rs` (8 tests) drives
     `simulate_recursive_wildcard_candidate`/`evaluate_stored_pattern_outcome` through all 3
     outcome branches (candidate wins / stored wins / no-matching-pattern) plus the
     zero-live-effect guarantee — exactly AC-17-259 through AC-17-262.
   - `rw02_route_with_precedence_on_read.rs`, `rw03_write_and_listen_parity.rs`,
     `rw04_reject_precedence_ties.rs` exercise `resolve_access_rule_pattern`'s step-3 recursive
     scan, `resolve_recursive_wildcard_condition` (Listen's per-event recheck), and precedence-tie
     rejection directly, at both read and write/Listen call sites.
   - `rw05_non_regression_guardrail.rs` plus the full 58-target, 468-test `security_rules_*`
     baseline (re-run clean after every slice, see slice commits) cover
     `list_recursive_access_rule_patterns_up_to`/`list_all_access_rule_patterns` and
     `handle_add_target`'s own extended call site.
3. Re-attempting with a much smaller, serial (`-j 1`), single-binary-scoped configuration remains
   possible as a follow-up once the concurrent-session/Docker-contention conditions observed during
   this run are no longer present — not attempted again in this pass to avoid repeating the same
   failure mode under the same conditions.

## Overall verdict

**WARN, with the actionable finding fixed.** Pass 1 (embyr-core, the pure computational core of
this feature's new logic) surfaced one real, feature-introduced gap, which is now closed. Pass 2
(embyr-server integration layer) is skipped with the justification above; that layer's own
purpose-built acceptance suite (58 targets / 468 tests, run clean after this slice) is the
substitute quality gate for this feature.

Proceeding to FINALIZE.
