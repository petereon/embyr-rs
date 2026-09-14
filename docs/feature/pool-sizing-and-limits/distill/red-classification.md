# RED Classification — pool-sizing-and-limits

Pre-DELIVER fail-for-the-right-reason gate (`nw-distill` skill). Run against
the CURRENT, unfixed `crates/embyr-server/src/config.rs`,
`crates/embyr-server/src/adapters/system_db.rs`,
`crates/embyr-pg-storage/src/backend_adapter.rs`, `crates/embyr-server/src/
grpc/handler.rs`, `crates/embyr-server/src/adapters/project_auth.rs`
(ADR-079's fix not yet applied — that is DELIVER's job).

Command:
```
cargo test -p embyr-server --test production_readiness -- \
  --test-threads=1 --include-ignored pool_sizing
```
→ `3 passed; 8 failed; 0 ignored; 59 filtered out; finished in 74.64s`

Zero stray Docker containers left behind (testcontainers auto-cleanup on
`Drop`, confirmed via `docker ps -a` post-run).

## RED-required (8) — the feature this DISTILL pass exists to drive

| Test | File | Classification | Empirically observed pre-fix behavior |
|---|---|---|---|
| `tenant_pool_override_honored_and_saturated_request_fails_fast` (walking skeleton) | `pr11_pool_sizing_tenant_pool.rs` | MISSING_FUNCTIONALITY (correct RED) | `EMBYR_TENANT_DB_MAX_CONNECTIONS=2`/`EMBYR_TENANT_DB_ACQUIRE_TIMEOUT_SECS=1` are silently ignored today. With 2 of the 5 hardcoded connections held busy by real, externally-row-locked writes (empirically confirmed still-pending via `task.is_finished()==false` at the assertion point — the saturation mechanism itself is proven engaged, not a vacuous setup), a 3rd, unlocked request trivially gets one of the 3 remaining hardcoded connections and succeeds in ~94ms. `result.is_err()` fails: `Ok(CommitResponse{...})` instead of a pool-timeout error. |
| `non_numeric_system_max_connections_fails_startup` | `pr12_pool_sizing_invalid_config.rs` | MISSING_FUNCTIONALITY (correct RED) | `EMBYR_SYSTEM_DB_MAX_CONNECTIONS=not_a_number` — server never reads this var, starts normally, does not exit within 3s (`got None`, expected `Some(1)`). |
| `non_numeric_tenant_max_connections_fails_startup` | same | MISSING_FUNCTIONALITY (correct RED) | Same shape for `EMBYR_TENANT_DB_MAX_CONNECTIONS=not_a_number` — `got None`. |
| `non_numeric_tenant_acquire_timeout_fails_startup` | same | MISSING_FUNCTIONALITY (correct RED) | Same shape for `EMBYR_TENANT_DB_ACQUIRE_TIMEOUT_SECS=not_a_number` — `got None`. |
| `non_numeric_listener_max_connections_fails_startup` | same | MISSING_FUNCTIONALITY (correct RED) | Same shape for `EMBYR_LISTENER_DB_MAX_CONNECTIONS=not_a_number` — `got None`. |
| `non_numeric_listener_acquire_timeout_fails_startup` | same | MISSING_FUNCTIONALITY (correct RED) | Same shape for `EMBYR_LISTENER_DB_ACQUIRE_TIMEOUT_SECS=not_a_number` — `got None`. |
| `zero_tenant_max_connections_fails_startup` | same | MISSING_FUNCTIONALITY (correct RED) | `EMBYR_TENANT_DB_MAX_CONNECTIONS=0` — not read, no rejection — `got None`. Confirms `0` needs the SAME `parse_positive_u32` non-positive check as non-numeric, not merely a `u32::parse` success path. |
| `zero_listener_acquire_timeout_fails_startup` | same | MISSING_FUNCTIONALITY (correct RED) | `EMBYR_LISTENER_DB_ACQUIRE_TIMEOUT_SECS=0` — same shape — `got None`. |

All 8 failures were verified to fail at the assertion (never an `ImportError`/
`FIXTURE_BROKEN`/panic-before-assertion) — every subprocess started cleanly,
every `stderr` capture succeeded, every gRPC call completed. No test in this
set failed due to a Docker/testcontainers setup problem.

## Not RED (confirmed correct today AND must stay correct after the fix — regression guards)

| Test | Reason |
|---|---|
| `tenant_pool_default_max_connections_preserved_when_unset` (AC-PSL-01) | Today's hardcoded `PostgresBackendAdapter::new()` default (`max_connections(5)`) already coincides with the new documented default — kept as an explicit, falsifiable scenario (holds 2 of 5 connections busy via the same row-lock mechanism, requires the remaining 3 concurrent unlocked requests to all succeed) so a future accidental change to the default constant would legitimately red it, mirroring `pr10_healthz_dependency_checks.rs`'s own precedent for AC-HDC-04/09. |
| `one_tenants_saturated_pool_does_not_affect_another_tenants_pool` (AC-PSL-06) | Per-tenant pool isolation already exists structurally today (`CredentialCache` keys one dedicated `PostgresBackendAdapter`/pool per project) — DISCUSS's own System Constraints section states this explicitly ("isolation already exists by construction"). This feature must not WEAKEN it; the test proves the invariant holds both before and after the fix. |
| `listen_still_delivers_document_changes_with_listener_pool_fields_threaded_through` | No pool-sizing env vars are set in this scenario (defaults only) — `Listen` already works today with the hardcoded `max_connections(2)`, no `acquire_timeout`. Proves the mechanical field-threading DESIGN describes (`handle_listen`'s 2 literals becoming `self.listener_db_*` field reads) does not regress the RPC once DELIVER lands it; not new functionality today. |

## Saturation mechanism (why it is deterministic, not a query-speed race)

`backend_adapter.rs`'s `commit_transaction` takes a real `SELECT
EXISTS(... FOR UPDATE)` / `SELECT ... FOR UPDATE` lock on the target document
row as part of its OCC precondition check for `MustExist`/`MustNotExist`/
`UpdateTime` preconditions (confirmed by direct code read, `backend_adapter.rs:1061-1183`).
A real, uncommitted `SELECT ... FOR UPDATE` opened directly against the
tenant's Postgres from the TEST (bypassing the SUT) therefore genuinely
blocks a concurrent SUT `Commit` RPC carrying a `current_document.exists =
true` precondition against the SAME row — the blocked RPC holds one of the
SUT's own pool connections for as long as the external lock is held. This
was empirically verified during DISTILL (not merely asserted): a
`task.is_finished()` check 400ms after issuing the two locked writes
confirmed both were still genuinely pending, ruling out a race-condition
false pass.

Two dead ends ruled out before landing on this mechanism (documented so
DELIVER does not rediscover them):
1. **`transaction: vec![]` as an "implicit" `Commit`** — rejected by the
   server as `InvalidArgument: invalid transaction ID length`. `Commit`
   always requires a real transaction id; `commit_update_requiring_exists`
   (`tests/production_readiness/common/mod.rs`) now calls `BeginTransaction`
   first, exactly mirroring `security_rules_write_path/common/mod.rs`'s own
   two-call shape.
2. **Table-level `LOCK TABLE ... ACCESS EXCLUSIVE`** — considered and
   rejected as the saturation mechanism: it blocks at the Postgres
   query-execution level (after a connection is already checked out), not
   at the sqlx pool-acquire level, so it cannot distinguish "pool exhausted,
   fails fast on `acquire_timeout`" from "query blocked, waits on the
   external lock indefinitely" — the wrong thing to prove for AC-PSL-04.
   Row-level `FOR UPDATE` on the SAME document row the SUT's own write path
   already locks is what makes a connection genuinely unavailable to the
   POOL, not merely slow to execute.

## Verdict

8 of 8 RED-required tests fail for the **right reason**: the assertion
fires because the new env-var-driven pool sizing/timeout/validation
genuinely does not exist yet — never because of an import error, fixture
bug, or setup failure. 3 regression-guard tests pass today and are expected
to keep passing after DELIVER's fix. Handoff to DELIVER is unblocked.
