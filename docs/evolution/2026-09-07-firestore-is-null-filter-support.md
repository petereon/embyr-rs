# Evolution: firestore-is-null-filter-support

**Date:** 2026-09-07
**Feature:** `IS_NULL`/`IS_NOT_NULL` unary filter ops — previously rejected outright alongside the
already-supported `IS_NAN`/`IS_NOT_NAN` siblings — now execute with real Firestore's own
documented semantics.
**Job:** JOB-01 (`sdk-compat`) — ordinary SDK query composition, no reassignment needed.
**ADRs:** none new.

## This closes gap #4 from the 2026-09-06 production-readiness scan

Unlike gap #3 (`firestore-or-filter-support`), whose own DISCUSS wave surfaced a genuine
access-control bypass risk in the naive fix, this gap had no security dimension at all —
confirmed up front, not discovered mid-implementation. It is the smallest, lowest-stakes gap
closed this session: a mechanical extension of an already-proven pattern.

## Business Context

`translate_filter`'s `UnaryFilter` arm already handled `IS_NAN`/`IS_NOT_NAN`; `IS_NULL`/
`IS_NOT_NULL` fell into its own catch-all rejection. The original scan flagged this gap as
"likely reachable... unconfirmed, needs live-SDK verification." DISCUSS resolved that
uncertainty by direct proto inspection: `UnaryFilter.Operator` is a real, 4-member GA proto enum
(`IS_NAN`, `IS_NULL`, `IS_NOT_NAN`, `IS_NOT_NULL`), and every official Firebase SDK lowers
`.where(field, '==', null)`/`.where(field, '!=', null)` to this exact shape — for the same reason
`NaN` comparisons are unary ops rather than ordinary `FieldFilter.EQUAL`: Firestore has no total
ordering that includes `null` (or `NaN`), so equality-to-null is special-cased at the SDK layer.
This is confirmed reachable, not a hypothetical.

## Key Decisions

| Decision | Verdict |
|---|---|
| Mirror `IS_NAN`/`IS_NOT_NAN`'s already-proven shape exactly, across all 4 existing call sites | feature-delta.md — a same-shape extension, not new architecture |
| `IS_NULL`/`IS_NOT_NULL` both require the field to be PRESENT to match (a missing field matches neither) | Matches real Firestore's own documented semantics independently for each operator — a deliberate divergence from the pre-existing `IS_NOT_NAN` arm's own permissive treatment of missing fields, not an inconsistency to reconcile |
| `backend_mode=agent` rejects cleanly | The agent's own internal proto has no equivalent unary op at all, mirroring the existing `IS_NAN`/`IS_NOT_NAN` rejection |

## Steps Completed

1. **US-01 (single slice, Walking Skeleton = the whole feature)**: `FilterOp` gains `IsNull`/
   `IsNotNull`. `translate_filter`'s `UnaryFilter` arm gains matching branches. `append_field_filter`
   gains 2 new SQL predicates — `FieldValue::Null` encodes as `{"t": "N"}` (no `v` key), so
   `IS_NULL` checks `fields->'{fp}'->>'t' = 'N'` and `IS_NOT_NULL` checks the field is present AND
   its type tag is not `N`. `domain_filter_op_to_agent` rejects both cleanly. Blast-radius
   confirmed up front via direct grep of every existing `IsNan`/`IsNotNan` reference (exactly 4
   call sites in 3 files) — the compiler's own exhaustiveness checking then confirmed nothing else
   needed touching once the enum variants were added.
2. Proven via 5 new tests: 2 acceptance (`is_null_filter_matches_only_documents_with_field_
   explicitly_null`, `is_not_null_filter_matches_only_documents_with_field_present_and_non_null`
   — both seeding a document missing the field entirely to prove the "field must be present"
   requirement), 2 agent-mode unit tests (clean rejection for each operator), and 1 security-
   compliance unit test confirming `filter_binds_field_to_uid` correctly treats neither operator
   as establishing an ownership binding — all passed on the first attempt.

**Full regression**: `cargo test -p embyr-server --no-fail-fast`. First attempt hit the
now-familiar cargo-sweep corruption pattern; a second attempt after confirming the sweep cleared
was clean apart from 2 known pre-existing Docker-contention flakes (`drl_b12_postgres_rate_limit`,
`security_rules_cel_parity_cp04` — `secrets_management` did not fail this run, consistent with its
own known intermittent nature).

**QUALITY_GATE**: 8 mutants, 7 caught, 1 legitimately unviable (`QueryFilter` has no `Default`
impl — same root cause as `firestore-or-filter-support`'s own established precedent), 0 missed.
Recovery from cargo-sweep corruption took unusually long this time (3 consecutive corrupted
warm-up attempts before a clean one) despite an otherwise healthy environment.

## Lessons Learned

1. **Reusing an already-proven pattern exactly, rather than inventing a new one, is the lazy
   AND correct choice when the shapes genuinely match.** `IS_NULL`/`IS_NOT_NULL` are structurally
   identical siblings to `IS_NAN`/`IS_NOT_NAN` in the same proto message — mirroring the existing
   4-call-site footprint exactly meant zero design risk and zero surprises during implementation.
2. **Confirming zero-touch-point predictions during DESIGN via direct grep, before writing any
   code, continues to pay off** (established during `firestore-or-filter-support`). Grepping
   every existing `IsNan`/`IsNotNan` reference up front correctly predicted the exact 4 call sites
   needing changes and confirmed `filter_binds_field_to_uid`/`collect_filter_fields`/`embyr-agent`'s
   own `proto_filter_to_domain` needed zero changes — verified with a new test, not just asserted.
3. **A deliberate design divergence from a sibling feature's own precedent is fine, as long as
   it's a conscious choice, not an inconsistency.** `IS_NOT_NAN`'s own existing SQL arm treats a
   missing field as passing "not nan" (permissive); `IS_NOT_NULL`'s new arm requires the field to
   be present (strict). Both match real Firestore's own independently-documented semantics for
   their respective operators — the difference is not a bug to reconcile, just two operators with
   different real-world rules.
4. **cargo-sweep corruption is now a confirmed, near-universal hazard across this entire
   session** — hitting essentially every feature's DELIVER and/or QUALITY_GATE stage at least
   once. This feature's own QUALITY_GATE recovery was unusually persistent: 3 consecutive
   corrupted warm-up-build attempts before a clean one, worse than any prior feature's 1-2-attempt
   pattern, despite disk space, Docker state, and system load all checking out healthy each time.
   Whether this reflects the hazard's frequency increasing over the session or simply an unlucky
   window for this one feature is not established — worth tracking if it recurs, not yet a
   pattern to draw a firm conclusion from.
5. **Applying a sibling feature's own invocation lessons proactively avoids repeating that
   specific detour, even when a DIFFERENT problem (environmental corruption) still requires
   separate recovery effort.** No stray positional filter token, `--timeout 240` from the very
   first attempt — both lessons from `firestore-or-filter-support`'s own QUALITY_GATE — meant this
   feature's mutation testing had a clean, correctly-scoped invocation from the start; the
   friction it did hit was entirely environmental (cargo-sweep), not a repeat of the prior
   feature's own invocation mistakes.

## Key Files

- `crates/embyr-core/src/domain/query.rs` — new `FilterOp::IsNull`/`IsNotNull` variants.
- `crates/embyr-core/src/access_control/mod.rs` — 1 new unit test confirming no compliance-check
  change needed.
- `crates/embyr-pg-storage/src/encoding/query.rs` — `append_field_filter`'s 2 new SQL predicates.
- `crates/embyr-server/src/grpc/handler.rs` — `translate_filter`'s 2 new `UnaryOp` arms.
- `crates/embyr-server/src/adapters/agent_backend.rs` — `domain_filter_op_to_agent`'s widened
  rejection arm.
- `tests/acceptance/us_04_query_collection.rs` — 2 new tests.
- `docs/feature/firestore-is-null-filter-support/feature-delta.md` — full DISCUSS/DESIGN
  narrative, including the blast-radius confirmation.
- `docs/feature/firestore-is-null-filter-support/deliver/mutation/mutation-report.md` — full
  account of the sweep-corruption recovery.

## Follow-Up Work

None specific to this feature. This closes the **last** unary-filter-shape gap: the proto's own
`UnaryFilter.Operator` enum has exactly 4 members, and all 4 (`IS_NAN`, `IS_NULL`, `IS_NOT_NAN`,
`IS_NOT_NULL`) are now supported. `backend_mode=agent` execution of any of the 4 remains
deferred (its own internal proto has no unary-filter equivalent at all) — unchanged from the
pre-existing `IS_NAN`/`IS_NOT_NAN` deferral, not a new gap.

Carried forward, unchanged, from `docs/product/known-gaps.md`: #5 (no TLS/mTLS on any listener),
#6 (no graceful shutdown), #7 (`secrets_management` Docker/LocalStack timing flakiness), #8 (CEL
"chaining" construct-detection gap).
