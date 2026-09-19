# field-path-defense-in-depth (finding #26)

FINALIZED 2026-09-19

## Business Context

Server-side field-path SQL interpolation was injection-safe only because every entry point happened to funnel through one central `validate_field_path` gate — zero defense-in-depth if a future caller skipped it. Finding #11 (the `embyr-agent`'s own divergent, weaker field-path validator, closed 2026-09-12) proved this isn't hypothetical: a second real call path into the same raw-interpolating SQL builder already existed and was under-guarded. This feature adds a redundant guard directly at each SQL-building call site so a future caller bug can't reach raw interpolation even if it bypasses upstream validation.

## Key Decisions

| Decision | Reasoning |
|---|---|
| `panic!`, not `Result<_, E>` or `debug_assert!` | Matches this file's existing convention for "assumed validated upstream, can only fire on a caller bug" defects (see pre-existing `_ => panic!(...)` arms in the same file). A `Result` would force every caller — all of which already validate — to handle an error that should be unreachable. `debug_assert!` would compile out in release builds, defeating defense-in-depth. |
| 7 call sites guarded | `append_field_filter`, `order_by_expr`, `order_by_expr_bigint` (all in `encoding/query.rs`); `run_query`'s startAt/startAfter and endAt/endBefore cursor blocks, `run_aggregation_query`'s Sum and Avg arms (all in `backend_adapter.rs`). Every place a field path reaches a `format!`-interpolated SQL fragment. |
| `push_all_descendants_predicate` out of scope | Interpolates a fixed collection-path structure, not a caller-supplied field path — not part of the vulnerable class this finding addresses. |
| No ADR | Small, local hardening pattern (repeat a guard at each call site) — doesn't rise to an architectural decision. |

## Lessons

- **RED-verification masking**: when adding the same guard to several call sites inside one large function, a `should_panic` test that exercises the function can pass even if only *one* of several guards in that function actually fires — a different code path may already panic first for an unrelated reason, masking whether the specific guard under test was needed at all. DELIVER caught and resolved this by checking each guard's RED state in isolation (temporarily remove just that guard, confirm the specific test goes green→fails) rather than trusting the aggregate "tests pass" signal.
- **Whole-function-stub mutation noise, again**: cargo-mutants has no comparison/match-arm to mutate inside a bare `if let Err(e) = f(x) { panic!() }` — for guards embedded in larger functions, the only mutant it can generate is "replace the whole enclosing method," a much blunter instrument than the guard alone. Still useful (all 6 viable mutants caught), but the classification has to say so explicitly rather than implying the guard itself was surgically mutated.

## Key Files

- `crates/embyr-pg-storage/src/encoding/query.rs` — `append_field_filter`, `order_by_expr`, `order_by_expr_bigint`
- `crates/embyr-pg-storage/src/backend_adapter.rs` — `run_query` (2 cursor guards), `run_aggregation_query` (Sum/Avg guards)
- `docs/feature/field-path-defense-in-depth/deliver/mutation/mutation-report.md` — mutation testing detail

## Follow-Up

- Finding #27 (Dependencies — duplicate dependency versions) — unrelated, still open.
- No new gaps opened by this feature.
