# Known Gaps — Firestore Parity & Production Readiness

Tracked from a fresh code scan on 2026-09-06 (post the 5-arc parity work this session:
admin UI/API, production-hardening, security-rules CEL-parity, composite-index,
query-filter crash-elimination — all FINALIZED, not re-listed here). Status column updated
as each gap is picked up.

| # | Gap | Location | Real-client reachable? | Severity | Status |
|---|---|---|---|---|---|
| 1 | Transactions ignore reads — no snapshot isolation, no OCC protection for read-then-write | `crates/embyr-server/src/grpc/handler.rs` (GetDocument/RunQuery/BatchGetDocuments ignore `consistency_selector.transaction`); `crates/embyr-pg-storage/.../backend_adapter.rs:916` (`commit_transaction` only checks preconditions on writes) | Yes — routine `runTransaction(fn)` usage | Blocks production (silent lost-update race) | IN PROGRESS |
| 2 | `endAt`/`endBefore` cursor silently dropped — hardcoded `None`, never parsed | `crates/embyr-server/src/grpc/handler.rs:3048`; `crates/embyr-pg-storage` has zero `end_at` handling | Yes — routine backward pagination/windowing | Blocks production (silent wrong results, no error) | Not started |
| 3 | `Filter.or()` composite OR rejected | `crates/embyr-server/src/grpc/handler.rs:4000` | Yes — real GA Firestore feature | Degrades feature (clean `INVALID_ARGUMENT`, not a crash/wrong-data) | Not started |
| 4 | `IS_NULL`/`IS_NOT_NULL` unary filter rejected | `crates/embyr-server/src/grpc/handler.rs:4015-4019` | Likely — some SDKs lower `where(f,'==',null)` to this shape; unconfirmed, needs live-SDK verification | Degrades feature | Not started |
| 5 | No TLS/mTLS on any of the 3 listeners (:8080 gRPC, :8081 REST/gRPC-Web, :9090 Admin) | grep across `embyr-server/src`: zero `rustls`/`TlsAcceptor` hits | Depends on deploy topology (fine if a TLS-terminating LB sits in front) | Blocks production unless mitigated at the LB | Not started |
| 6 | No graceful shutdown — drops in-flight connections on deploy/restart | grep: zero `ctrl_c`/shutdown-signal hits in `embyr-server` | N/A — operational | Degrades feature (not data-corrupting) | Not started |

## Notes

- Checked and clean, not tracked here: no `todo!()`/`unimplemented!()` anywhere in the
  workspace; CEL functions, list-size limits (30/10), two-simultaneous-`IN` all confirmed
  fine, matching prior memory.
- `SmtpEmailSender::new_scaffold()`'s panic is dead code (never called from any request
  path, `Noop` is the wired default) — cosmetic only, not tracked here.
- #1 and #2 are the two gaps that matter most for "can this ship as a Firestore-compatible
  service": both let a well-behaved client silently get wrong data, a worse failure class
  than any of the panics closed by this session's 5 arcs.
