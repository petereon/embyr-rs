# Mutation Report — field-path-defense-in-depth (finding #26)

Scope: `git diff a72dc45..e2ccafd -- crates/embyr-pg-storage/src/encoding/query.rs crates/embyr-pg-storage/src/backend_adapter.rs`

## Build scoping (verified)

```
cargo test -p embyr-pg-storage --lib -- --test-threads=1   # 35/35 pass, 0.03s test / ~70s cold compile
cargo mutants -p embyr-pg-storage --in-place --in-diff <diff> \
  -C --lib --timeout 120 \
  -- -- --test-threads=1
```
`--in-place` conflicts with `-j`/`--jobs` (rejected outright) — omitted, run defaults to serial.
Real per-mutant cost: **8 mutants tested in 16s** (baseline build/test both 0s, cached). timeout=120 had large margin; not needed.

## Result

```
Found 8 mutants to test
ok       Unmutated baseline in 0s build + 0s test
8 mutants tested in 16s: 6 caught, 2 unviable
```

0 missed, 0 timeout.

| # | Site | Mutant | Outcome |
|---|------|--------|---------|
| 1 | `run_query` (whole fn) | `-> Ok(vec![])` | caught |
| 2 | `run_query` (whole fn) | `-> Ok(vec![Default::default()])` | unviable (doesn't compile) |
| 3 | `run_aggregation_query` (whole fn) | `-> Ok(Default::default())` | unviable (doesn't compile) |
| 4 | `append_field_filter` (whole fn) | `-> ()` | caught |
| 5 | `order_by_expr` (whole fn) | `-> String::new()` | caught |
| 6 | `order_by_expr` (whole fn) | `-> "xyzzy".into()` | caught |
| 7 | `order_by_expr_bigint` (whole fn) | `-> String::new()` | caught |
| 8 | `order_by_expr_bigint` (whole fn) | `-> "xyzzy".into()` | caught |

## Classification note

All 8 generated mutants are `FnValue` (whole-function-body replacement) genre — cargo-mutants has no comparison/match-arm to mutate inside a bare `if let Err(e) = validate_field_path(...) { panic!(...) }` guard, so it can't produce a guard-only mutant for the 4 sites embedded inside larger trait methods (`run_query` x2, `run_aggregation_query` Sum/Avg). Those 4 sites' only available mutant is "replace the whole enclosing method" — a blunter instrument than the guard alone, but still caught (or unviable) every time. The other 3 sites (`append_field_filter`, `order_by_expr`, `order_by_expr_bigint`) are near-standalone functions where the whole-function mutant *is* effectively a guard-removal mutant, and those are caught cleanly by DELIVER's `should_panic` tests.

**No genuine gap.** 6/6 viable mutants caught, 2 unviable (compile failures, not a test-quality signal). No test changes made.
