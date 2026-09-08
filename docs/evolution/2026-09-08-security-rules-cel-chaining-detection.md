# Evolution: security-rules-cel-chaining-detection

**Date:** 2026-09-08
**Feature:** A chaining `get()` call (a `get()` whose own path argument contains another `get()`)
is now reliably named as an offending construct at rule-import time, alone or alongside other
offending blocks in the same file.
**Job:** JOB-17 (`document-access-control`) — reused, persona P1 Alex.
**ADRs:** none new — decisions embedded in this feature's own `feature-delta.md`, mirroring
`firestore-tls-support`'s own established precedent.

## This closes gap #8 — the LAST remaining item in the 2026-09-06 production-readiness/parity scan

## Business Context

`tests/security_rules_cel_parity/acceptance/cp04_reject_out_of_scope_imports.rs::multiple_
offending_blocks_are_all_named_in_a_single_rejection_response` imports a rules file with 3
deliberately-offending `match` blocks and expects all 3 to be named in the rejection — but only
1 was ever named. A real rule author writing a chaining `get()` in a real security-rules file
got no warning that the construct is unsupported, an import that could silently succeed (or, in
a multi-block file, silently obscure the chaining problem behind whichever other error happened
to surface first) and fail unpredictably later at RUNTIME evaluation instead of being cleanly
rejected at IMPORT time.

## Key Decisions

| Decision | Verdict |
|---|---|
| D1 | Full JTBD path (Decision 4 = Yes), NOT infrastructure-only, unlike gap #7 — a real rule author is affected |
| D2 | Reuse JOB-17, same persona (Alex) as every prior `security-rules-cel-*` feature this session |
| D3 | Chaining itself remains a deliberate non-goal (ADR-066 Resolution 2) — scoped strictly to detection/naming reliability, never evaluation |
| D4 | Root-cause diagnosis explicitly deferred to DESIGN, not guessed in DISCUSS |
| D5-D7 (DESIGN) | Two-stage pipeline aggregation gap confirmed root cause (not the narrower per-block-classifier hypothesis, refuted with evidence); fix is strictly additive by construction; no Earned Trust probe needed (zero new I/O) |

## Steps Completed — the first fully-clean run of the corrected nWave subagent pipeline

Run through all 4 waves as real, separately-dispatched subagents — DISCUSS (`nw-product-owner`),
DESIGN (`nw-solution-architect`, peer-reviewed, 0 critical/high issues), DISTILL
(`nw-acceptance-designer`), DELIVER (`nw-software-crafter`) — with zero orchestrator-written
implementation. (The DISCUSS dispatch itself hit a session rate limit partway through; per the
user's own explicit "always resume automatically" instruction, it was retried once the limit
cleared and completed normally — the only interruption across the whole pipeline.)

1. **DISCUSS**: wrote US-01 under the full JTBD path (not infrastructure-only), reusing JOB-17.
2. **DESIGN**: found the ACTUAL root cause by direct investigation — confirmed with a real test
   run (`assert_eq!` failure `left: 1, right: 3`), not hand-tracing. The rule-import pipeline is
   two-stage (Stage 1 `parse_rules_file`, Stage 2 `decompose`); only Stage 2 aggregates across
   blocks. Stage 1 fails fast via `?` on the FIRST per-block content error and discards
   already-successfully-parsed blocks — so Stage 2, where the chaining detector's own findings
   would surface, never runs once any earlier block hits a Stage-1-only problem. DISCUSS's own
   narrower alternative hypothesis (a per-block classifier that simply never checks for
   chaining) was refuted: the actual chaining detector (`parse_path_template`) was already
   correct, proven by an existing, already-passing isolated unit test using the identical
   condition shape.
3. **DISTILL**: corrected a stale test assertion (`CUSTOM_FUNCTION` → `UNDEFINED_FUNCTION`,
   superseded by an already-FINALIZED prior feature, unnoticed only because the test never got
   past its first failing assertion) and added a new unit test proving multi-block aggregation
   across a Stage-1-only error and a Stage-2-only error in the same file — confirmed both in the
   correct RED state before handoff.
4. **DELIVER**: implemented DESIGN's fully-specified fix. While wiring up DISTILL's own new unit
   test, found that its `Err` arm (as DISTILL wrote it) only read `offending_blocks` and
   discarded the new `partial_blocks` field — meaning it could never reach GREEN even after the
   production fix landed. Completed the test's own `Err` arm to match its own documented intent
   (mirroring `import_rules_file`'s own Stage 1→Stage 2 wiring) — a genuine bug in DISTILL's own
   test, caught by DELIVER's own independent read of the code, not silently propagated.

**Full workspace regression**: clean apart from `drl_b12_postgres_rate_limit` (known
pre-existing flake) and `security_rules_write_path_delete_gated_by_resource` — a NEW test hit by
the SAME `PortNotExposed` Docker-contention root cause already confirmed transient multiple
times this session (now confirmed on at least its 4th distinct test victim), verified via
isolated rerun. `security_rules_cel_parity_cp04` — this feature's own fix target — was
completely clean.

**QUALITY_GATE**: 15 mutants, 8 caught, 4 unviable, 3 missed — all 3 fully explained (2 confirmed
scoping artifacts on an unrelated admin route, 1 verified genuine equivalent mutant traced
end-to-end through the actual code, not assumed). Zero real gaps.

## Lessons Learned

1. **Any multi-stage validation pipeline should be audited for whether EVERY stage that can find
   multiple independent problems actually accumulates them, not just the last one.** This bug's
   entire root cause was that Stage 2 already had the right "loop, don't fail fast, accumulate"
   shape — Stage 1 simply hadn't been built to match it. This is a genuinely reusable
   architectural lesson for any future multi-stage validation/parsing logic in this codebase:
   check every stage, not just the one that happens to run last.
2. **The subagent pipeline's own layered verification caught something a single self-writing
   orchestrator might well have missed.** DELIVER independently reading DISTILL's own test
   (rather than blindly trusting it was already correct) found a real bug in that test relative
   to its own documented intent — exactly the kind of cross-check this session's own methodology
   correction (see `[[feedback_nwave_use_subagents]]`) was meant to produce.
3. **The `PortNotExposed` Docker-contention flake class continues to recur on new, previously-
   unaffected tests** — this session's own 4th distinct victim of the same root cause. Not a new
   finding, but worth reinforcing: read the actual failure message and verify via isolated rerun
   every time, per this session's own established triage discipline, rather than pattern-
   matching on the test NAME alone.
4. **Verifying a "genuine equivalent mutant" claim by tracing actual code (not just asserting
   it) is worth the effort for security-adjacent logic.** The match-guard-forced-to-`true` miss
   could have been waved away as "probably fine" — instead, tracing `decompose`'s own empty-
   input behavior and `rules_file_rejection_response`'s own field usage produced a genuine,
   checkable proof that the mutation is mathematically undetectable, not a hand-wave.

## Key Files

- `crates/embyr-core/src/access_control/rules_file.rs` — `MatchBlock` gains `Eq`;
  `RulesFileError` gains `partial_blocks`; `parse_nested_match_blocks` restructured to
  accumulate-and-continue; `decompose`'s own `Err` construction updated; 14 mechanical test-site
  fixes; new unit test proving multi-block, multi-stage aggregation.
- `crates/embyr-server/src/admin/handlers/access_rules.rs` — `import_rules_file` merges Stage
  1/Stage 2 offending lists; 3 mechanical `partial_blocks: Vec::new()` additions.
- `tests/security_rules_cel_parity/acceptance/cp04_reject_out_of_scope_imports.rs` — stale
  assertion corrected.
- `docs/feature/security-rules-cel-chaining-detection/feature-delta.md` — full DISCUSS/DESIGN
  narrative, including the evidenced root-cause investigation and DESIGN's own peer review.
- `docs/feature/security-rules-cel-chaining-detection/deliver/mutation/mutation-report.md` —
  full account of all 3 misses' own resolutions.

## Follow-Up Work

None specific to this feature. **This closes the LAST item in `docs/product/known-gaps.md`** —
the entire 8-item production-readiness/parity sweep from the 2026-09-06 scan is now FULLY
RESOLVED:

- 6 gaps closed via real fixes: transaction-read-consistency, end-cursor-support, or-filter-
  support, is-null-filter-support, tls-support, this chaining-detection fix.
- 1 gap found stale/already-implemented during another feature's own DISCUSS (graceful
  shutdown).
- 1 gap closed via a test-reliability fix (secrets-management LocalStack warm-up).
