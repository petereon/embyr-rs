# Mutation Testing Report — firestore-is-null-filter-support

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-08
**Scope**: `crates/embyr-pg-storage/src/encoding/query.rs` (`append_field_filter`'s new
`IsNull`/`IsNotNull` SQL predicates), `crates/embyr-server/src/grpc/handler.rs`
(`translate_filter`'s new `UnaryOp::IsNull`/`IsNotNull` arms), and
`crates/embyr-server/src/adapters/agent_backend.rs` (`domain_filter_op_to_agent`'s widened
rejection). A multi-package `--in-diff` (`git diff f9e1632 f5a5a1c -- crates/embyr-core
crates/embyr-pg-storage crates/embyr-server`) — 8 mutants total. None in `crates/embyr-core`
itself: the diff there is a new test only, confirming DESIGN's own zero-touch-point prediction
for `filter_binds_field_to_uid`.

`cargo mutants --workspace --in-place --timeout 240 --in-diff <diff> --test-workspace true --
--test us_04_query_collection --lib`

No invocation-tuning detour was needed this time — the stray-positional-filter bug and
too-short-timeout lessons from `firestore-or-filter-support`'s own QUALITY_GATE were applied
proactively from the start (no trailing filter token, `--timeout 240` from the first attempt).

## Recurring cargo-sweep corruption — hit unusually hard this time

The full 8-mutant run completed cleanly for 5 of 8: `append_field_filter -> ()`, both `IsNull`/
`IsNotNull` match-arm deletions in `append_field_filter`, `domain_filter_op_to_agent ->
Ok(Default::default())`, and `translate_filter -> None` — all **caught**. The remaining 3 (all in
`handler.rs`: `translate_filter -> Some(Ok(Default::default()))`, and the `UnaryOp::IsNull`/
`IsNotNull` match-arm deletions at lines 4102/4103) came back "unviable" with the same
`extern location ... does not exist` / `can't find crate for hyper` cargo-sweep signature seen
repeatedly this session.

Recovery took **3 consecutive corrupted warm-up-build attempts** before a clean one succeeded —
notably worse than any prior feature's own 1-2-attempt recovery pattern, despite the environment
otherwise checking healthy each time (129GiB free disk, 0 stray Docker containers, load average
~3.0). Whether this reflects the sweep hazard escalating in frequency or simply an unlucky
window for this particular feature is not established either way — flagged honestly rather than
generalized from one data point.

Since `handler.rs` and its own tests (`us_04_query_collection`, `agent_backend::tests`) all live
in the single `embyr-server` package, the 3 affected mutants were retested with a fast,
single-package invocation (`cargo mutants -p embyr-server --in-place --timeout 240 --in-diff
<handler.rs-only diff> -- --test us_04_query_collection --lib`, no `--workspace`/
`--test-workspace` needed) once the sweep genuinely cleared — the same recovery pattern
established during `firestore-or-filter-support`'s own QUALITY_GATE. This retest completed in
~11 minutes with a clean, uncorrupted result for all 4 mutants in scope (the 2 previously-
corrupted ones plus a clean re-confirmation of `translate_filter`'s own 2 mutants).

## Final result — 7 caught, 1 unviable, 0 missed

| Mutant | Location | Result |
|---|---|---|
| `append_field_filter -> ()` | `encoding/query.rs:47` | **CAUGHT** |
| delete match arm `IsNull` | `encoding/query.rs:63` | **CAUGHT** |
| delete match arm `IsNotNull` | `encoding/query.rs:67` | **CAUGHT** |
| `domain_filter_op_to_agent -> Ok(Default::default())` | `agent_backend.rs:299` | **CAUGHT** |
| `translate_filter -> None` | `handler.rs:3991` | **CAUGHT** |
| `translate_filter -> Some(Ok(Default::default()))` | `handler.rs:3991` | unviable (no `Default` for `QueryFilter`) |
| delete match arm `UnaryOp::IsNull` | `handler.rs:4102` | **CAUGHT** (single-package retest, after sweep-corruption recovery) |
| delete match arm `UnaryOp::IsNotNull` | `handler.rs:4103` | **CAUGHT** (single-package retest, after sweep-corruption recovery) |

The single "unviable" result was verified by reading its actual compile error directly —
`error[E0277]: the trait bound embyr_core::domain::query::QueryFilter: Default is not satisfied`
— the exact same root cause as `firestore-or-filter-support`'s own established precedent
(`QueryFilter` derives only `Debug, Clone`, no `Default`). Not corruption; a genuine, expected
compile-time impossibility for this substitution mutant.

## Verdict

**PASS** — every viable mutant caught, 0 missed, no fixes required. All 8 mutants resolved
across two runs (one full-workspace, one single-package retry after confirmed, unusually
persistent environmental corruption during recovery), with no test-strengthening needed.
