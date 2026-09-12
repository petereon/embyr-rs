# Mutation Testing Report — agent-field-path-validation

**Tool**: cargo-mutants
**Scope**: `crates/embyr-agent/src/server.rs`, `--in-diff` scoped to the DELIVER commit
(`git diff c6103fb 7f832b4 -- crates/embyr-agent/src/server.rs`).

## Command

```
cargo mutants -p embyr-agent --in-place --timeout 300 --in-diff /tmp/afp_mut_diff.diff -- \
  --test embyr_agent -- --include-ignored --test-threads=1
```

## Result: 1 candidate mutant — unviable, 0 viable mutants

```
Found 1 mutant to test
1 mutant tested in 2m: 1 unviable
```

## Interpretation

**This is the expected, correct result for this diff's own shape — confirmed, not assumed.** The
fix is a pure deletion (removing `embyr-agent`'s own weak, 12-line `validate_field_path` function)
plus a single call-site substitution (`.map_err(core_error_to_status)`, reusing an
already-existing function). The diff introduces:
- No new conditionals, literals, comparisons, or arithmetic for cargo-mutants to mutate.
- No new logic at all — the REAL validation logic this feature now delegates to
  (`embyr_core::domain::query::validate_field_path`) is pre-existing code, untouched by this
  diff, and already carries its own mutation coverage from when it was originally built (used by
  `embyr-server` and, more recently, `composite-index-real-creation`'s own DDL builder).

The single candidate mutant found was unviable (a build failure in cargo-mutants' own scratch
build), leaving zero mutants to classify as caught or missed. This is not a gap — there is
genuinely no new mutable logic in this diff to test via mutation. The feature's own correctness is
instead proven by the 11 real, empirically-RED-verified acceptance scenarios (SQL-injection probes
rejected on both `RunQuery` and `RunAggregationQuery`, legitimate field paths still work, the
consecutive-dot behavioral delta is explicit and intentional, cross-binary message parity holds),
all independently reconfirmed passing by the orchestrator (63/63 across both test targets).

## Final state

63/63 tests pass (58 in the full `embyr_agent` suite, 5 in the cross-binary parity target).
Full workspace regression: clean (1 unrelated, already-documented `drl_b12_postgres_rate_limit`
transient flake, confirmed via isolated rerun).
