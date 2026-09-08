# Known Gaps — Firestore Parity & Production Readiness

Tracked from a fresh code scan on 2026-09-06 (post the 5-arc parity work this session:
admin UI/API, production-hardening, security-rules CEL-parity, composite-index,
query-filter crash-elimination — all FINALIZED, not re-listed here). Status column updated
as each gap is picked up.

| # | Gap | Location | Real-client reachable? | Severity | Status |
|---|---|---|---|---|---|
| 1 | Transactions ignore reads — no snapshot isolation, no OCC protection for read-then-write | `crates/embyr-server/src/grpc/handler.rs` (GetDocument/RunQuery/BatchGetDocuments ignore `consistency_selector.transaction`); `crates/embyr-pg-storage/.../backend_adapter.rs:916` (`commit_transaction` only checks preconditions on writes) | Yes — routine `runTransaction(fn)` usage | Blocks production (silent lost-update race) | **CLOSED** 2026-09-06 — see `docs/evolution/2026-09-06-firestore-transaction-read-consistency.md` |
| 2 | `endAt`/`endBefore` cursor silently dropped — hardcoded `None`, never parsed | `crates/embyr-server/src/grpc/handler.rs:3048`; `crates/embyr-pg-storage` has zero `end_at` handling | Yes — routine backward pagination/windowing | Blocks production (silent wrong results, no error) | **CLOSED** 2026-09-07 — see `docs/evolution/2026-09-07-firestore-end-cursor-support.md` (also fixed a closely-coupled pre-existing `startAt`/`startAfter` DESC-direction bug found along the way) |
| 3 | `Filter.or()` composite OR rejected | `crates/embyr-server/src/grpc/handler.rs:4000` | Yes — real GA Firestore feature | Degrades feature (clean `INVALID_ARGUMENT`, not a crash/wrong-data) | **CLOSED** 2026-09-07 — see `docs/evolution/2026-09-07-firestore-or-filter-support.md` (DISCUSS found and avoided a worse risk than the gap itself: the naive fix would have reopened an access-control bypass in `filter_binds_field_to_uid`) |
| 4 | `IS_NULL`/`IS_NOT_NULL` unary filter rejected | `crates/embyr-server/src/grpc/handler.rs:4015-4019` | Confirmed reachable — every official SDK lowers `where(f,'==',null)`/`where(f,'!=',null)` to this proto shape (same reason `IS_NAN` exists as a unary op) | Degrades feature | **CLOSED** 2026-09-08 — see `docs/evolution/2026-09-07-firestore-is-null-filter-support.md` (closes the last unary-filter-shape gap; all 4 `UnaryFilter.Operator` members now supported) |
| 5 | No TLS/mTLS on any of the 3 listeners (:8080 gRPC, :8081 REST/gRPC-Web, :9090 Admin) | grep across `embyr-server/src`: zero `rustls`/`TlsAcceptor` hits | Depends on deploy topology (fine if a TLS-terminating LB sits in front) | Blocks production unless mitigated at the LB | **IN PROGRESS — `firestore-tls-support`** (DISCUSS complete 2026-09-08; plain server-side TLS via 2 opt-in env vars, mTLS explicitly deferred — see `docs/feature/firestore-tls-support/feature-delta.md`) |
| 6 | No graceful shutdown — drops in-flight connections on deploy/restart | grep: zero `ctrl_c`/shutdown-signal hits in `embyr-server` | N/A — operational | Degrades feature (not data-corrupting) | **CLOSED (stale finding)** 2026-09-08 — direct inspection during `firestore-tls-support` DISCUSS found this was already fully implemented in `crates/embyr-server/src/main.rs` (SIGTERM+Ctrl-C handling, in-flight-request draining) as part of the `production-readiness` feature (commit 6f58e89, 2026-08-09) — predates this scan. Proven by `tests/production_readiness/acceptance/pr04_graceful_shutdown.rs` (AC-PR-04-01/02/03). The original scan's own grep was apparently run against a stale checkout or missed this file; no new work needed |
| 7 | `secrets_management` sm01/sm02 tests fail consistently — server+LocalStack container doesn't exit within the test's own 10s wait | `tests/secrets_management/acceptance/sm01_admin_key_secrets_manager.rs:207`/`sm02_encryption_key_secrets_manager.rs` | Test-only — Docker/AWS-SDK timing, not a runtime code path a real client hits | Test flakiness/reliability, not a production data/crash risk | Not started — found as a byproduct of firestore-transaction-read-consistency's own regression testing, bisection-confirmed pre-existing (reproduces without that feature's diff) |
| 8 | CEL rule-import "chaining" construct (a `get()` whose path is built from another `get()`'s own result) isn't detected as an offending import — only 1 of 3 expected offending blocks named | `tests/security_rules_cel_parity/acceptance/cp04_reject_out_of_scope_imports.rs:264`; underlying detection logic in `crates/embyr-server`'s rule-import validation | Yes — a real rule author could write a chaining `get()` and not be warned it's unsupported | Degrades feature (a real product gap in rule-import validation, not data-corrupting) | Not started — found as a byproduct of firestore-transaction-read-consistency's own regression testing, bisection-confirmed pre-existing |

## Notes

- Checked and clean, not tracked here: no `todo!()`/`unimplemented!()` anywhere in the
  workspace; CEL functions, list-size limits (30/10), two-simultaneous-`IN` all confirmed
  fine, matching prior memory.
- `SmtpEmailSender::new_scaffold()`'s panic is dead code (never called from any request
  path, `Noop` is the wired default) — cosmetic only, not tracked here.
- #1 and #2 were the two gaps that mattered most for "can this ship as a Firestore-compatible
  service": both let a well-behaved client silently get wrong data, a worse failure class
  than any of the panics closed by this session's 5 arcs. Both are now closed
  (`firestore-transaction-read-consistency`, 2026-09-06; `firestore-end-cursor-support`,
  2026-09-07).
- #7 and #8 are pre-existing, unrelated issues discovered as a byproduct of
  `firestore-transaction-read-consistency`'s own regression testing — both bisection-confirmed to
  reproduce without that feature's diff, neither touches transactional reads.
