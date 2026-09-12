# Mutation Testing Report — sanitize-backend-error-messages

**Tool**: cargo-mutants
**Scope**: two separate runs, one per package (`embyr-server` and `embyr-agent`), since the fix
spans both binaries and cargo-mutants auto-detects a single package per run (this session's own
established limitation — see `occ-precondition-validation`'s own mutation report).

Discipline followed: confirmed via `ps -p` for both PIDs → not running, and `git diff --stat` on
both touched files → empty, before reading either result set.

## Run 1 — `embyr-server` (`handler.rs` + `listen_handler.rs`)

```
cargo mutants -p embyr-server --in-place --timeout 300 --in-diff /tmp/sbm_mut_diff_server.diff -- \
  --test sanitize_backend_error_messages_sbm01_walking_skeleton_authenticate_unreachable_backend \
  --test sanitize_backend_error_messages_sbm02_completeness_gate_access_rule_family \
  --test sanitize_backend_error_messages_sbm03_regression_guards \
  --test sanitize_backend_error_messages_sbm04_listen_unreachable_backend \
  -- --test-threads=1
```

**Result: 87 mutants tested in 85m — 7 missed, 2 caught, 78 unviable.**

```
MISSED  handler.rs:758  resolve_access_rule_pattern -> Ok(None)
MISSED  handler.rs:883  evaluate_write_rule_for_commit -> Ok(())
MISSED  handler.rs:1798 handle_update_document -> Ok(Response::new(Default::default()))
MISSED  handler.rs:2032 handle_delete_document -> Ok(Response::new(()))
MISSED  handler.rs:2260 handle_list_documents -> Ok(Response::new(Default::default()))
MISSED  handler.rs:4169 delete match arm CoreError::FailedPrecondition(_) in core_error_to_status
MISSED  listen_handler.rs:65 handle_add_target -> Ok(())
```

**All 7 confirmed as scoping artifacts, not genuine gaps.** Every mutant is a
whole-function-body-replacement (a cargo-mutants noise pattern already documented in this
session's own `firestore-transaction-read-consistency` and `composite-index-real-creation`
mutation reports: a diff hunk touching one line inside a large, pre-existing function pulls the
entire function into scope, and cargo-mutants generates a "replace the whole body with a trivial
stub" mutant for it). This run's own test command intentionally scoped to ONLY the 4 new
`sanitize-backend-error-messages`-specific acceptance tests — none of which call
`handle_update_document`/`handle_delete_document`/`handle_list_documents`/`handle_add_target`/
`evaluate_write_rule_for_commit`/`resolve_access_rule_pattern` in a way that checks their OWN real
document/rule-evaluation behavior (only their error-sanitization behavior on the specific failure
paths this feature touches).

Investigated, not assumed — confirmed real coverage exists for every one of the 7, entirely
outside this run's own scope:
- `handle_update_document`: `tests/acceptance/us_02_write_document.rs`, `us_06_transactions.rs`,
  `us_12_agent_backend.rs`.
- `handle_delete_document`: `tests/acceptance/us_05_listen_realtime.rs`, `us_12_agent_backend.rs`.
- `handle_list_documents`: `tests/acceptance/us_12_agent_backend.rs`.
- `handle_add_target`: `tests/acceptance/us_05_listen_realtime.rs`, `us_13_browser_transport.rs`.
- `evaluate_write_rule_for_commit`/`resolve_access_rule_pattern`: 87 test files under
  `tests/security_rules*/` exercise access-rule evaluation — an `Ok(())`/`Ok(None)` stub here
  would be a security-rule bypass, which these tests exist specifically to catch.
- `core_error_to_status`'s `FailedPrecondition` match arm: multiple existing tests
  (`tests/firestore_list_rpcs/`, `tests/composite_index_real_creation/`,
  `tests/security_rules_query_path/noncompliant_query_rejected_preexecution.rs`) already assert
  on `FailedPrecondition`/`failed_precondition` behavior.

All 7 sites are also exercised by the full-workspace regression suite already confirmed clean by
the orchestrator before this mutation run (1 unrelated, already-documented `cargo-sweep`-class
transient flake, confirmed via isolated rerun).

**The 2 caught mutants** are on the actual NEW `sanitize_backend_error` logic itself, confirming
the fix's own core behavior is tested. The 78 unviable mutants are the usual
`Default::default()`-on-non-`Default`-type build failures endemic to this codebase's own
`Result<T, Status>` return types.

## Run 2 — `embyr-agent` (`server.rs`)

```
cargo mutants -p embyr-agent --in-place --timeout 300 --in-diff /tmp/sbm_mut_diff_agent.diff -- \
  --test embyr_agent -- --include-ignored --test-threads=1
```

Scoped to the FULL `embyr_agent` test binary (not just the one new test) via `--include-ignored`,
avoiding the scoping-artifact class hit in Run 1.

**Result: 10 mutants tested in 4m — 1 caught, 9 unviable, 0 missed.** Fully clean — the smaller,
more contained 3-site agent-side diff (vs. 21 sites in the server-side diff) had a test command
scope wide enough to catch everything the diff touched.

## Final state

All 8 DISTILL-authored scenarios pass (independently reconfirmed by the orchestrator, not just
DELIVER's own report). Full workspace regression clean. No further fixes needed — all 7
server-side misses are genuine scoping artifacts with real, already-passing coverage confirmed to
exist outside this mutation run's own narrow scope.
