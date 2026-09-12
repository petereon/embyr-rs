# Evolution: agent-field-path-validation

**Date:** 2026-09-12
**Feature:** `embyr-agent`'s own weak field-path validator (only checked for consecutive dots) is
replaced with the real, spec-mandated charset guard already used by `embyr-server` — closing a
genuine SQL-injection path into the customer's own Postgres.
**Job:** JOB-04 (`credential-isolation`) — reused, persona P4 Riley Nakamura.
**ADRs:** none new — narrow validator-swap fix, no new pattern.

## This closes finding #11 from `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

`embyr-agent`'s own `validate_field_path` independently duplicated (and weakened) the real
validation logic already established in `embyr_core::domain::query::validate_field_path` — only
rejecting consecutive dots, letting every other character (quotes, semicolons, SQL operators)
through to `proto_filter_to_domain`, which feeds a raw-interpolating SQL builder
(`fields->'{field_path}'` — structurally necessary for a JSONB path expression, not
parameterizable). This is a genuine SQL injection into the CUSTOMER's own Postgres database,
reachable via any client holding a valid mTLS certificate for that agent deployment — directly
defeating the reason `embyr-agent` exists (the vendor should never have code paths that can
compromise data it was never meant to access).

## Key Decisions

| Decision | Verdict |
|---|---|
| Fix | Delete `embyr-agent`'s own weak duplicate validator, call `embyr_core::domain::query::validate_field_path` directly — a 12-line deletion plus a 1-line call-site change |
| Error conversion | Reuse the ALREADY-EXISTING `core_error_to_status` function (which already had a correct `InvalidArgument` arm) rather than writing a new wrapper |
| Blast radius | Exactly ONE call site (`proto_filter_to_domain`) in the entire `embyr-agent` crate, independently re-verified twice (DISCUSS, then DESIGN) — write paths are structurally unaffected (field names become JSON keys inside a bound parameter, never raw-interpolated) |
| Behavioral delta | Consecutive-dot field paths (`"a..b"`) are now INTENTIONALLY accepted (the stricter charset's own regex doesn't forbid dot sequences) — matches `embyr-server`'s own already-live behavior, not new permissiveness. Explicitly tested and documented, not silently absorbed |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with peer
review, DISTILL=`nw-acceptance-designer`, DELIVER=`nw-software-crafter`):

1. **DISCUSS**: confirmed `embyr-agent` already depends on `embyr-core`, traced the exact single
   call site, confirmed the stricter charset is SPEC-mandated (`docs/SPEC.md:538`, not just server
   preference — every finalized query feature this session already relies on it). Swept the rest
   of `embyr-agent/src` for other duplicated-validator patterns — found none, an isolated
   incident. DoR 9/9.
2. **DESIGN**: peer-reviewed, 0 critical/high. Found the elegant simplification DISCUSS's own
   illustrative sketch missed — reuse `core_error_to_status` (already correct) instead of writing
   a new wrapper. Independently re-confirmed the single-call-site blast radius via a
   whole-crate grep.
3. **DISTILL**: wrote 11 acceptance scenarios. Empirically confirmed RED with two genuinely
   distinct, real failure modes: a quote-breaking payload surfaces as `Status::internal` (a raw
   Postgres syntax error), while a non-quote-breaking payload (a bare semicolon, or even just a
   space) SILENTLY SUCCEEDS with zero rows and zero error — worse than a crash, giving no signal a
   spec-violating field path was ever submitted. Also caught and fixed a stale pre-existing test
   (`us_a03_query_operations.rs`) that would have silently broken post-fix, since it asserted
   rejection of the now-intentionally-accepted consecutive-dot shape.
4. **DELIVER**: implemented the exact 3-edit fix — net -12 LOC, zero new functions/types. All 11
   scenarios green (63/63 across both test targets, independently reconfirmed by the
   orchestrator).
5. **Orchestrator's full-workspace regression**: clean (1 unrelated, already-documented
   `drl_b12_postgres_rate_limit` transient flake, confirmed via isolated rerun).
6. **QUALITY_GATE**: mutation testing confirmed, empirically, that this diff has essentially zero
   new mutable logic surface (a pure deletion + a call-site substitution reusing already-tested
   code) — the single candidate mutant found was unviable. Correctness is proven by the 11
   acceptance scenarios instead, not by mutation coverage, and this was verified rather than
   assumed.

## Lessons Learned

1. **A "swap the weak validator for the real one" fix is often smaller and cleaner than DISCUSS's
   own illustrative sketch anticipates — DESIGN found an existing function
   (`core_error_to_status`) that made a whole wrapper unnecessary.** Worth a habit: before writing
   a new small conversion/wrapper function, check whether the SAME file already has an equivalent
   one nearby that already does the right thing.
2. **When a fix is a pure deletion plus reuse of pre-existing, already-tested code, a mutation run
   confirming near-zero mutable surface is itself a valid, honest QUALITY_GATE result** — not
   every feature needs "N mutants caught" to demonstrate real correctness; sometimes the honest
   finding is "there was nothing new to mutate, and here's why."
3. **This is the smallest, cleanest fix of the High-severity findings closed so far this session** —
   a useful contrast against the much larger `sanitize-backend-error-messages` (26 sites) and
   `occ-precondition-validation` (2 independent binaries) fixes, showing the full nWave pipeline
   scales down cleanly to a genuinely small, well-scoped change without unnecessary ceremony.

## Key Files

- `crates/embyr-agent/src/server.rs` — the only production file changed; net -12 LOC.
- `tests/acceptance/embyr_agent/us_a09_field_path_validation.rs` (new, 10 scenarios).
- `tests/agent_field_path_validation/acceptance/afp06_cross_binary_parity.rs` (new, 1 scenario).
- `tests/acceptance/embyr_agent/us_a03_query_operations.rs` — stale test payload fixed.
- `docs/feature/agent-field-path-validation/deliver/mutation/mutation-report.md` — full account.

## Follow-Up Work

Findings #12-#19 (High) remain, plus #20+ (Medium/Low). Also flagged (out of scope for this
feature): finding #26, defense-in-depth at the SQL-builder layer itself (validating field paths
again at the point of SQL construction, not just at the RPC boundary) — a good candidate for a
future hardening pass if the audit list continues that far.
