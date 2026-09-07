# Evolution: firestore-or-filter-support

**Date:** 2026-09-07
**Feature:** `Filter.or()` composite OR queries — previously rejected outright with a clean
error — now execute correctly, AND are checked for security-rule ownership compliance
correctly, not just permissively.
**Job:** JOB-01 (`sdk-compat`) — ordinary SDK query composition, no reassignment needed.
**ADRs:** none new; ADR-031 (`docs/product/architecture/adr-031-query-shape-compliance-check.md`)
read closely and its own AND-only assumption extended, not overturned.

## This closes gap #3 from the 2026-09-06 production-readiness scan — but the investigation found a WORSE risk than the gap itself

Gaps #1 (`firestore-transaction-read-consistency`) and #2 (`firestore-end-cursor-support`) were
both "client silently gets wrong results" bugs. Gap #3 started the same way — `Filter.or()`
returns a clean rejection error today, an availability problem, not a correctness one. DISCUSS's
own investigation into fixing it surfaced something more serious: the naive fix (mirroring
`Composite`'s existing AND semantics) would have reopened a genuine **access-control bypass** —
worse than the gap it was meant to close. This is the most notable finding across all three
production-readiness gaps closed this session.

## Business Context

`crates/embyr-core/src/access_control/mod.rs`'s `filter_binds_field_to_uid` decides whether a
query's filter tree structurally GUARANTEES an ownership-equality binding
(`field == caller_uid`) for every document it could ever return — the mechanism security rules
compliance-checking depends on to admit or reject a query without reading any documents. ADR-031's
own Handoff Package explicitly documents: "confirmed `QueryFilter::Composite` is AND-only
('Composite AND filter' doc comment), the structural fact this whole ADR's tractability rests
on." Naively adding OR support by reusing `Composite`'s `.any()` semantics — "the binding holds if
ANY child enforces it" — is correct for AND, but wrong for OR: a document matching an OR filter
need only satisfy ONE branch, so `WHERE ownerId == callerUid OR true` would be wrongly treated as
ownership-compliant, while its real result set includes every tenant's own documents matching the
permissive `true` branch. Surfaced via `AskUserQuestion`, the user chose the full, security-correct
fix over a narrower "reject OR on protected collections only" alternative.

## Key Decisions

| Decision | Verdict |
|---|---|
| Full security-correct fix, not a narrower "reject OR on protected collections" workaround | User's own explicit choice via `AskUserQuestion` |
| `QueryFilter::CompositeOr(Vec<QueryFilter>)` as a strictly ADDITIVE new domain variant, not a parameterized `Composite(Vec<QueryFilter>, LogicalOp)` | feature-delta.md — avoids touching any of 15 pre-existing AND-only `Composite` call sites across 6 files; lets the compiler's own exhaustiveness checking find every REQUIRED touch point instead |
| `filter_binds_field_to_uid`'s `CompositeOr` arm uses `.all()`, not `.any()` | The security-critical fix — see Business Context above |
| `backend_mode=agent` cleanly rejects OR filters rather than silently mistranslating to AND | The agent's own internal `embyr.agent.v1.CompositeFilterOp` proto has no `Or` variant at all — a silent AND-mistranslation would return the intersection instead of the union, the same class of bug `domain_filter_to_agent_filter`'s own doc comment already records a prior incident for |

## Steps Completed

1. **Slice 01 (US-01, OR query execution)**: `translate_filter`'s `CompositeFilter` arm gains a
   full `CompositeOp::Or` branch mirroring the existing `And` branch, producing
   `QueryFilter::CompositeOr`. `append_filter` OR-joins children and parenthesizes the whole
   expression — required now that OR can nest inside AND (or vice versa), where the pre-existing
   AND-only code never needed parens since AND-of-AND is associative. Proven via 2 new tests in
   `tests/acceptance/us_04_query_collection.rs`: a plain OR union, and an OR nested inside AND
   with seed data specifically designed to catch a missing-parens precedence bug.
2. **Slice 02 (US-02, OR query security compliance)**: `filter_binds_field_to_uid`'s new
   `CompositeOr` arm uses `.all()`. `collect_filter_fields` (shared by
   `requires_composite_index`/`missing_index_fields`) gains a `CompositeOr` arm reusing the
   identical recursive-flatten logic already used for `Composite` — zero new index-requirement
   heuristic; OR-specific composite-index widening explicitly deferred. `backend_mode=agent`'s
   `domain_filter_to_agent_filter` gains a clean rejection arm plus 2 new unit tests. Proven via 4
   new tests in `tests/security_rules_query_path/acceptance/or_composed_query_compliance.rs`:
   OR with only one branch proving ownership (rejected), OR with every branch proving ownership
   (admitted), and an OR branch that itself AND-composes with ownership (still admitted).

**Full regression**: `cargo test -p embyr-server --no-fail-fast`. Took 3 attempts before a
genuinely clean read — see Lessons Learned. The clean run showed only known pre-existing
Docker/testcontainers-contention flakes (`distributed_rate_limiting`, `secrets_management`
sm01/sm02, `security_rules_cel_parity_cp04`), plus a new flavor of the same root cause
(`PortNotExposed` in `admin_api_v2_b04_project_patch`) — same class, different test this run,
zero code overlap with this feature's own changed files.

**QUALITY_GATE**: 12 mutants across `embyr-core`, `embyr-pg-storage`, `embyr-server`. Final
result: 9 caught, 3 correctly unviable (`FilterOp`/`QueryFilter` don't derive `Default`, making
cargo-mutants' own substitution mutant structurally impossible to compile), 0 missed, no fixes
needed. Both `filter_binds_field_to_uid` mutants (forcing `.all()`'s arm to always return `true`
or always `false`) were caught directly — computational confirmation that the security fix is
actually exercised by the test suite, not merely reasoned about. See the mutation report for the
two environmental detours this run hit before reaching that result.

## Lessons Learned

1. **`.any()` is correct for AND-composed structural guarantees; `.all()` is required for
   OR-composed ones — a broadly reusable principle, not specific to this feature.** Any check of
   the shape "does this structural condition hold for EVERY possible match of a filter tree"
   needs `.any()` under AND (if one branch enforces it, the whole conjunction does) and `.all()`
   under OR (a match need only satisfy one branch, so the guarantee only holds if every branch
   independently enforces it). Getting this backwards for OR is not a subtle edge case — it is a
   full access-control bypass, silent and undetectable without reading the actual query
   semantics against the compliance-check's own contract.
2. **An additive enum variant, paired with the compiler's own exhaustiveness checking, is a
   safer way to extend a closed set of behaviors than parameterizing an existing variant.**
   Confirming `QueryFilter::Composite` had exactly 15 call sites across 6 files up front
   justified `CompositeOr` as a wholly new variant: every one of those 15 sites is untouched, and
   the compiler found the 5 sites that DID need a new arm (domain enum, `translate_filter`,
   `append_filter`, `filter_binds_field_to_uid`, `collect_filter_fields`,
   `domain_filter_to_agent_filter`) rather than requiring a manual audit to find them.
3. **Reading the ADR that a feature's own DISCUSS wave will touch, closely, before writing any
   code, can surface a hidden security dimension a superficial read would miss.** ADR-031's own
   Handoff Package spelled out its AND-only assumption explicitly; treating that line as
   load-bearing (rather than skimming past it) is what surfaced the `.any()`-vs-`.all()` risk
   before any code was written, not after.
4. **The periodic `cargo-sweep-shared-target.sh` background process corrupted BOTH this
   feature's DELIVER verification and its QUALITY_GATE mutation testing** — the same
   `extern location does not exist` / `No such file or directory` signature seen repeatedly this
   session, now confirmed as a genuinely recurring environmental hazard rather than a one-off.
   DELIVER's full regression needed 3 attempts (2 corrupted, 1 clean) before a trustworthy read;
   QUALITY_GATE's full-workspace mutation run had 5 of its first 12 results corrupted the same
   way. Recovery pattern each time: confirm via `pgrep -fl "cargo-sweep sweep"` that the sweep
   has cleared, do a clean warm-up build, then retry immediately before another sweep cycle
   starts.
5. **`cargo-mutants` invocation gotchas, worth remembering for future features:**
   - A bare trailing token after mixed `--lib`/`--test` flags in the `--` arg list is consumed
     as `cargo test`'s own GLOBAL positional filter, applied across every selected target — not
     scoped to just the target it appears to follow. Symptom: some scoped targets report
     "0 tests... N filtered out" instead of their real counts; catch it by reading the per-mutant
     log, not just the summary.
   - Postgres-testcontainers-backed acceptance tests need a generous `--timeout` (240s was
     sufficient here; 90s caused the unmutated *baseline* itself to time out before any mutant
     ran).
   - `--test-workspace true` forces a full-workspace test-binary REBUILD per mutant when the
     diff touches a foundational crate — genuinely necessary for correctness across a
     multi-package diff, but far more expensive than a same-package diff would be (~2 hours for
     12 mutants here, versus ~11 minutes for a same-package retry of 5 of them). When a mutant's
     file and all its relevant tests live in the SAME package, skip `--workspace`/
     `--test-workspace` entirely for a much faster, equally correct result.
6. **Discovered dozens of leaked testcontainers Postgres containers (2-23 hours old) accumulated
   across this session's earlier features** — almost certainly the real root cause of the
   repeated "system running low on memory" kills that hit background wait-loop shells throughout
   this feature's DELIVER and QUALITY_GATE stages. Flagged to the user rather than force-cleaned
   (a bulk `docker rm -f` was blocked by the permission classifier as a destructive bulk action);
   cleanup remains pending explicit user authorization.

## Key Files

- `crates/embyr-core/src/domain/query.rs` — new `QueryFilter::CompositeOr(Vec<QueryFilter>)`
  variant.
- `crates/embyr-core/src/access_control/mod.rs` — `filter_binds_field_to_uid`'s
  SECURITY-CRITICAL `.all()` arm for `CompositeOr`.
- `crates/embyr-pg-storage/src/encoding/query.rs` — `append_filter`'s OR-join and
  parenthesization.
- `crates/embyr-server/src/grpc/handler.rs` — `translate_filter`'s new `Or` arm;
  `collect_filter_fields`'s `CompositeOr` reuse.
- `crates/embyr-server/src/adapters/agent_backend.rs` — `domain_filter_to_agent_filter`'s clean
  rejection; 2 new unit tests.
- `crates/embyr-server/Cargo.toml` — new `[[test]]` entry for the OR security-compliance
  acceptance suite.
- `tests/acceptance/us_04_query_collection.rs` — 2 new tests (OR union, OR-nested-in-AND
  parenthesization).
- `tests/security_rules_query_path/acceptance/or_composed_query_compliance.rs` (new) — 4 tests
  covering the `.all()` semantics.
- `docs/feature/firestore-or-filter-support/feature-delta.md` — full DISCUSS/DESIGN narrative,
  including the ADR-031 finding and the `AskUserQuestion` decision point.
- `docs/feature/firestore-or-filter-support/deliver/mutation/mutation-report.md` — full account
  of the invocation tuning and cargo-sweep corruption during QUALITY_GATE.

## Follow-Up Work

- **`backend_mode=agent` OR-filter execution** — deferred as its own future feature
  (`agent-mode-or-filter-support`); requires a proto change to the agent's own internal
  `embyr.agent.v1.CompositeFilterOp` enum, which currently has no `Or` variant at all.
- **OR-specific composite-index widening** — `collect_filter_fields`'s `CompositeOr` arm reuses
  the existing AND-shaped field-collection walk with zero new index-requirement heuristic;
  whether real Firestore's OR queries need a structurally different composite-index shape is
  explicitly unresolved and deferred.
- **Automated testcontainers cleanup verification** — an operational follow-up prompted by this
  feature's own leaked-container discovery; worth checking whether the Ryuk reaper is actually
  running for this workspace's testcontainers setup, given dozens of containers accumulated
  silently across a single session.

Carried forward, unchanged, from `docs/product/known-gaps.md`: #4 (`IS_NULL`/`IS_NOT_NULL` unary
filter rejected — unconfirmed, needs live verification), #5 (no TLS/mTLS on any listener), #6 (no
graceful shutdown), #7 (`secrets_management` Docker/LocalStack timing flakiness — reinforced by
today's new `PortNotExposed` flavor in `admin_api_v2_b04_project_patch`, same root cause), #8 (CEL
"chaining" construct-detection gap).
