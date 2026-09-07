# Mutation Testing Report — firestore-or-filter-support

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-07
**Scope**: `crates/embyr-core/src/access_control/mod.rs` (`filter_binds_field_to_uid`'s new
`CompositeOr` arm — **security-critical**), `crates/embyr-pg-storage/src/encoding/query.rs`
(`append_filter`'s OR-join/parenthesization), `crates/embyr-server/src/grpc/handler.rs`
(`translate_filter`'s new `Or` arm, `collect_filter_fields`'s `CompositeOr` reuse), and
`crates/embyr-server/src/adapters/agent_backend.rs` (`domain_filter_to_agent_filter`'s clean
rejection). A multi-package `--in-diff` (`git diff 97504b2 fa766fa -- crates/embyr-core
crates/embyr-pg-storage crates/embyr-server`) — 12 mutants total.

`cargo mutants --workspace --in-place --timeout 240 --in-diff <diff> --test-workspace true --
--test us_04_query_collection --test security_rules_query_path_or_composed_query_compliance --lib`

(`--workspace --test-workspace true` required because the diff spans `embyr-core`,
`embyr-pg-storage`, and `embyr-server` — the same reason established during
`firestore-transaction-read-consistency`'s own QUALITY_GATE.)

## Invocation tuning — three false starts before a valid run

This QUALITY_GATE took three corrective iterations to reach a trustworthy invocation, each
worth recording:

1. **Stray positional filter swallowed unrelated targets' coverage.** The first attempt combined
   `--lib agent_backend::tests --test us_04_query_collection --test
   security_rules_query_path_or_composed_query_compliance` in one `--` arg list. `cargo test`
   treats a bare trailing token as its own GLOBAL positional test-name filter, applied across
   *every* selected target — not per-target. `agent_backend::tests` matched nothing in the two
   `--test` binaries, silently zeroing their coverage to "0 tests... N filtered out" while only
   the `--lib` target got real signal. Caught by reading the per-mutant log
   (`mutants.out/log/...access_control...mod.rs_line_2152...log`) and noticing the acceptance
   binaries ran 0 tests instead of their real counts (15 and 7). Fix: drop the bare filter token;
   let `--lib` and both `--test` targets run their full suites unfiltered.
2. **Timeout too short for Postgres-backed baseline.** The corrected invocation used
   `--timeout 90`, but `us_04_query_collection` alone takes ~120s under testcontainers Postgres —
   the *unmutated baseline* itself timed out (`*** result: Timeout`), aborting the run before any
   mutant was tested. Fix: `--timeout 240`.
3. **`--test-workspace true` forces a full-workspace rebuild per mutant.** With a diff spanning
   `embyr-core` (a foundational dependency of nearly every test binary in the workspace),
   `--test-workspace true` triggers a `cargo test --no-run --workspace` build phase per mutant —
   not just a rebuild of the 3 selected targets. The first mutant alone took ~1.5 hours of wall
   clock before producing a result; the full 12-mutant run took ~2 hours. This is inherent to
   `--test-workspace`'s correctness guarantee (a mutant in `embyr-core` can only be caught by
   integration tests that live in a different package), not a misconfiguration — but it means
   the per-mutant cost estimate from prior features (30-90 min total) badly undersold this
   feature's actual cost given its foundational-crate touch point.

## Environmental corruption — the recurring cargo-sweep race, again

The full 2-hour run reported **12 mutants tested: 7 caught, 0 missed, 5 unviable** — all 5
"unviable" results were in `crates/embyr-server/src/grpc/handler.rs` (`collect_filter_fields`
×3, `translate_filter` ×2). Their logs showed the SAME `extern location ... does not exist`
signature against completely unrelated crates (`proc_macro2`, `futures_task`, `rand`,
`icu_normalizer`, `aws_lc_rs`) seen repeatedly this session during DELIVER's own regression
verification — the periodic `cargo-sweep-shared-target.sh` background process racing a live
build, not a real compile failure of the mutated code. Confirmed via `pgrep -fl "cargo-sweep
sweep"` that the sweep was active during this window.

**Recovery**: since `handler.rs` and all 3 test targets live in the single `embyr-server`
package, the 5 affected mutants did not need `--workspace`/`--test-workspace` at all — only the
`embyr-core`/`embyr-pg-storage` mutants genuinely required cross-crate integration coverage.
After confirming the sweep had cleared and doing a `cargo build -p embyr-server --tests`
warm-up, a targeted single-package re-run (`cargo mutants -p embyr-server --in-place --timeout
240 --in-diff <handler.rs-only diff> -- --test us_04_query_collection --test
security_rules_query_path_or_composed_query_compliance --lib`) finished in ~11 minutes instead
of hours, and gave real (uncorrupted) results for all 5: **2 caught, 3 unviable**.

The 3 remaining "unviable" results were checked individually and are genuine, not corruption —
each fails with `error[E0277]: the trait bound ... Default is not satisfied` for `FilterOp` or
`QueryFilter` (`crates/embyr-core/src/domain/query.rs:89` and `:112` — neither type derives
`Default`). `cargo mutants`'s own `Default::default()` substitution mutant is structurally
impossible to compile for these types; this is a normal, expected `unviable` classification, not
a coverage gap.

## Final result — 9 caught, 3 unviable, 0 missed

| Mutant | Location | Result |
|---|---|---|
| `filter_binds_field_to_uid -> true` | `access_control/mod.rs:2152` | **CAUGHT** (security-critical) |
| `filter_binds_field_to_uid -> false` | `access_control/mod.rs:2152` | **CAUGHT** (security-critical) |
| `append_filter -> ()` | `encoding/query.rs:18` | **CAUGHT** |
| `> == in append_filter` | `encoding/query.rs:31` | **CAUGHT** |
| `> < in append_filter` | `encoding/query.rs:31` | **CAUGHT** |
| `> >= in append_filter` | `encoding/query.rs:31` | **CAUGHT** |
| `domain_filter_to_agent_filter -> Ok(Default::default())` | `agent_backend.rs:326` | **CAUGHT** |
| `collect_filter_fields -> vec![]` | `handler.rs:641` | **CAUGHT** |
| `collect_filter_fields -> vec![("", Default::default())]` | `handler.rs:641` | unviable (no `Default` for `FilterOp`) |
| `collect_filter_fields -> vec![("xyzzy", Default::default())]` | `handler.rs:641` | unviable (no `Default` for `FilterOp`) |
| `translate_filter -> None` | `handler.rs:3991` | **CAUGHT** |
| `translate_filter -> Some(Ok(Default::default()))` | `handler.rs:3991` | unviable (no `Default` for `QueryFilter`) |

**Security-critical portion**: both `filter_binds_field_to_uid` mutants (forcing the new
`CompositeOr` arm to always return `true` or always return `false`) were caught on the first,
uncorrupted pass. Forcing `true` would wrongly admit
`an_or_query_where_only_one_branch_proves_ownership_is_rejected` (AC-OR-04), which asserts
rejection; forcing `false` would wrongly reject
`an_or_query_where_every_branch_proves_ownership_is_admitted` (AC-OR-05), which asserts
admission. This is direct computational confirmation that the `.all()` fix (versus the
`.any()` that would have reopened the ADR-031 access-control bypass) is actually exercised by
the test suite, not merely reasoned about.

## Verdict

**PASS** — every viable mutant caught, 0 missed, no fixes required. All 12 mutants resolved
across two runs (one full-workspace, one single-package retry after a confirmed environmental
corruption), with no test-strengthening needed at any point.
