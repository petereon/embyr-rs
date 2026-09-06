# Evolution: firestore-transaction-read-consistency

**Date:** 2026-09-06
**Feature:** A transaction's `GetDocument`/`RunQuery`/`BatchGetDocuments` reads now participate in
the same optimistic-concurrency guarantee real Firestore gives them — a document read inside a
transaction that changes (or, for a confirmed-absent read, gets created) before that transaction
commits now aborts the commit (`ABORTED`), instead of silently allowing a lost-update race.
**Job:** JOB-01 (`sdk-compat`) — confirmed correct, not reassigned. See § Business Context.
**ADRs:** none (folded into feature-delta.md's own DESIGN section).

## This closes the highest-impact gap from this session's own production-readiness scan

A fresh code scan on 2026-09-06 (after this session's 5 prior arcs — admin UI/API,
production-hardening, security-rules CEL-parity, composite-index, and the query-filter
crash-elimination arc — were all already FINALIZED) ranked this gap #1 above every other finding,
including the just-closed crash-elimination arc: a panic is loud — the request fails visibly, a
correctly-written client sees an error and retries. An unenforced transactional read is **silent**
— both concurrent commits report success, and the data is simply wrong, discovered later if at
all. `docs/product/known-gaps.md` tracked this as row #1; this FINALIZE closes it.

## Business Context

SPEC.md already fully documented the wire contract this feature realizes (§Transaction,
§Read-Within-Transaction, §CommitTransaction: `GetDocumentForTransaction` records `(path, version)`
in the transaction's own read set; `CommitTransaction` re-validates every recorded read at commit,
aborting with `codes.Aborted` on any mismatch) — a documented-but-unbuilt gap, the same pattern as
several prior JOB-01 realizations this session (`aggregation-queries`,
`firestore-composite-indexes-admin-api`). Direct code inspection confirmed the `transactions` table
had no `reads` column at all, and `get_document`'s own `BackendAdapter` trait signature had no
`transaction_id` parameter whatsoever — the mechanism was entirely absent, not merely buggy. A real
`runTransaction(fn)` callback's own `tx.get(docRef)` call — the single most common transaction
pattern in real Firestore usage (read a counter, compute `+1`, write it back) — got zero protection:
two concurrent callers could both read the same stale value and both commits would succeed,
silently dropping an update. JOB-01/P1 Alex is the correct persona (confirmed by investigation, not
assumed): the failure mode corrupts Alex's OWN application data, not an operator-visible concern.

## Key Decisions

| Decision | Verdict |
|---|---|
| JOB-01/P1 Alex confirmed correct — Alex's own app data gets corrupted, not an operator-visible operational concern | feature-delta.md § Reading Confirmation |
| Mechanism: extend `get_document`'s trait signature to mirror `run_query`'s own already-existing `transaction_id: Option<&TransactionId>` parameter; add a `reads` JSONB column to `transactions`; add `verify_reads` as a direct sibling to the already-shipped `verify_versions` | feature-delta.md § Wave Decisions Summary |
| `backend_mode=agent`, `ReadTime`, and `BatchGetDocuments`' `new_transaction` auto-begin variant explicitly deferred | § Out of Scope |
| Read-set JSONB keys are bound parameters via `jsonb_build_object`, never string-interpolated | § System Constraints |

## Steps Completed

1. **Slice 01 (Walking Skeleton) — `GetDocument`**: `get_document`'s trait signature gained
   `transaction_id: Option<&TransactionId>`; a new migration
   (`migrations/customer/0005_transaction_reads.sql`) added a `reads JSONB` column to
   `transactions`; `record_read`/`verify_reads` (new functions in
   `crates/embyr-pg-storage/src/transactions/occ.rs`, siblings to the already-shipped
   `verify_versions`) register a read and re-validate it at commit; `handle_get_document` extracts
   `consistency_selector.transaction` and threads it through. Blast radius confirmed by direct
   grep BEFORE editing: 8 call sites in `handler.rs`, 4 in `embyr-agent/src/server.rs`, 1 impl each
   in `embyr-pg-storage`/`agent_backend.rs` — every site except the 3 read RPCs passes `None`.
   Proven end-to-end with a real two-actor lost-update race (`tests/acceptance/us_06_transactions.rs`).
2. **Slice 02 — `RunQuery`**: `run_query`'s own trait parameter (already `transaction_id: Option<&
   TransactionId>`, previously always ignored/hardcoded `None` at every call site) now registers
   every returned document via the identical mechanism Slice 01 built. `handle_run_query` extracts
   `consistency_selector.transaction`.
3. **Slice 03 — `BatchGetDocuments`**: `handle_batch_get_documents` extracts
   `consistency_selector.transaction` and threads it into its own per-document `get_document`
   calls — zero new pg-storage code, since `get_document` is already transaction-aware after Slice
   01 (mirrors `batch-get-documents`'s own established "N times, once per requested document,
   unchanged" precedent). Also switched that call site's error mapping from an ad-hoc
   `Status::internal`-only closure to the shared `core_error_to_status`, so a malformed/unknown
   transaction ID surfaces as `NotFound`/`InvalidArgument` instead of `internal` — every other
   error variant that call site could already produce still maps to `internal` via the same
   fallback arm.

**Full regression**: `cargo test -p embyr-server` (with `--no-fail-fast`, after discovering the
default fail-fast behavior silently never reached this test file at all — see § Lessons Learned).
2 pre-existing, unrelated issues found and bisection-confirmed NOT caused by this feature
(reproduced identically against the Slice-01-only committed state, before Slices 02/03 existed):
`secrets_management`'s `sm01`/`sm02` (spawns a real server + LocalStack container with only a 10s
exit-wait — pure Docker/AWS-SDK timing) and `security_rules_cel_parity_cp04` (a CEL rule-import
"chaining" construct isn't detected as offending — a pre-existing gap in rule-import validation,
unrelated to document reads/transactions). Both added to `docs/product/known-gaps.md` as separate,
low-priority tracked items.

**QUALITY_GATE**: 20 mutants (scoped down from 63 via `--exclude-re`, covering every line this
feature actually added or changed) — 16 caught, 3 unviable, 1 timeout, 0 missed. 100% effective
kill rate on every viable mutant, first pass.

## Lessons Learned

1. **Confirm blast radius by direct grep BEFORE changing a trait signature, not after.** Adding
   `transaction_id: Option<&TransactionId>` to `get_document` touched 8+4+2 call sites across 3
   crates. Grepping every call site first — and classifying each as "real transactional read"
   (gets `Some`) vs. "internal/access-rule/non-transactional read" (gets `None`) — turned a
   potentially error-prone refactor into a mechanical, low-risk one.
2. **Reuse an existing OCC mechanism's shape, don't generalize it.** `verify_reads` is a NEW
   function, a direct sibling to the already-shipped `verify_versions`, not a shared abstraction
   over both — they check semantically different things (write-attached explicit version vs.
   read-derived expected version-or-absence), and forcing one implementation would have obscured
   that distinction for a ~15-line function. Zero new `CoreError` variant was needed either way.
3. **A trailing sentinel message in a streaming RPC response inflates the naive response count.**
   `handle_run_query` always appends a `RunQueryResponse{continuation_selector: Done(true)}` after
   the document responses — a `responses.len() == 1` assertion for "1 seeded document" is off by
   one. Fixed by filtering for `.document.is_some()` before counting, matching this codebase's own
   established pattern elsewhere for consuming this stream.
4. **`cargo test` without `--no-fail-fast` stops at the first failing test BINARY, not just the
   first failing test.** The first full-regression pass reported a clean-looking picture, but had
   actually aborted alphabetically at the pre-existing `distributed_rate_limiting` flake — this
   feature's own new tests (`us_06_transactions`, alphabetically after `d`) never ran as part of
   that "full" regression at all, despite having been separately verified passing in isolation. A
   `--no-fail-fast` rerun was needed to get a true complete picture. That rerun itself hit a
   mid-run `cargo-sweep` shared-target-dir race cascading into 35 unrelated link-error failures —
   pure build-state noise from a periodic background process, not real regressions, but a reminder
   that a "full" regression claim needs `--no-fail-fast` (or per-target isolation) to actually be
   full.
5. **`git diff <commit>` without a second ref diffs against the WORKING TREE, not another commit**
   — it silently includes any uncommitted changes, even ones from entirely unrelated, pre-existing
   dirty files. Generating a QUALITY_GATE diff via `git diff 17fd491 -- <paths>` pulled in an
   unrelated pre-existing dirty `lib.rs`, inflating the mutant count with noise from code this
   feature never touched. Fixed by diffing commit-to-commit (`git diff 17fd491 6aefe85 -- <paths>`).
6. **cargo-mutants' `--test-workspace` flag only takes effect when `--in-diff` spans MULTIPLE
   packages** — a genuinely novel discovery this session, found only after `-p`, `--test-package`,
   and a `.cargo/mutants.toml` `test_package` config all silently failed to override the
   auto-detected per-mutant package for the `cargo test` invocation. See the mutation report for
   the full working command and the `--exclude-re`-based noise-reduction approach used instead of
   `--file` (which, combined with `--workspace`, re-triggers the same single-package-detection
   bug). Reusable for any future feature whose acceptance tests live in a different crate's
   `Cargo.toml` than most of the mutated source.

## Key Files

- `crates/embyr-core/src/storage/backend_adapter.rs` — `get_document`'s new `transaction_id`
  parameter.
- `crates/embyr-pg-storage/src/backend_adapter.rs` — `get_document`/`run_query` read-registration;
  `commit_transaction` extended to load and validate the `reads` column.
- `crates/embyr-pg-storage/src/transactions/occ.rs` — new `read_key`/`record_read`/`verify_reads`
  functions.
- `crates/embyr-server/src/adapters/agent_backend.rs` — `get_document`'s new parameter, ignored
  (agent-mode deferred, mirrors this adapter's own existing `run_query` pattern).
- `crates/embyr-agent/src/server.rs` — 4 internal `get_document` call sites updated to pass `None`.
- `crates/embyr-server/src/grpc/handler.rs` — `handle_get_document`/`handle_run_query`/
  `handle_batch_get_documents` all extract `consistency_selector.transaction`; 8 internal
  (non-transactional) call sites confirmed unaffected.
- `migrations/customer/0005_transaction_reads.sql` — new `reads JSONB` column.
- `tests/acceptance/us_06_transactions.rs` — 8 new real end-to-end tests (AC-TRC-01 through 10),
  alongside the file's own 6 pre-existing OCC/transaction tests (all still passing, zero
  regression).
- `docs/feature/firestore-transaction-read-consistency/feature-delta.md` — full DISCUSS/DESIGN
  narrative, 3 slice briefs.
- `docs/feature/firestore-transaction-read-consistency/deliver/mutation/mutation-report.md`.
- `docs/product/jobs.yaml`, JOB-01 — new NOTE appended (during DISCUSS).
- `docs/product/known-gaps.md` — row #1 closed; 2 new rows for the pre-existing, unrelated issues
  found during regression (`secrets_management` Docker/LocalStack timing; `security_rules_cel_
  parity_cp04` CEL chaining-detection gap).

## Follow-Up Work

- **`backend_mode=agent` transactional reads** — `AgentBackendAdapter`'s own internal wire protocol
  to the customer-VPC agent has no transaction-carrying field on `GetDocumentRequest` today;
  extending it is a separate, deferred candidate feature, mirroring the established
  `agent-mode-write-streaming`/`agent-mode-list-collection-ids`/`agent-mode-field-transforms`
  deferral pattern.
- **`ReadTime` consistency selector** (point-in-time reads at a past timestamp) — a distinct
  Firestore feature, unimplemented today, unaffected by this feature.
- **`BatchGetDocuments`' own `new_transaction` auto-begin variant** — a convenience that implicitly
  calls `BeginTransaction` as part of the same RPC; a smaller, separable enhancement, not built
  here.
- **`secrets_management` Docker/LocalStack timing flakiness** and **`security_rules_cel_parity_cp04`
  CEL "chaining" construct-detection gap** — both discovered as a byproduct of this feature's own
  regression testing, both confirmed pre-existing and unrelated (bisected against the Slice-01-only
  commit), both now tracked in `docs/product/known-gaps.md` for someone else to pick up.

Carried forward, unchanged, from `docs/product/known-gaps.md`: #2 (`endAt`/`endBefore` cursor
silently dropped), #3 (`Filter.or()` composite OR rejected), #4 (`IS_NULL`/`IS_NOT_NULL` unary
filter rejected), #5 (no TLS/mTLS on any listener), #6 (no graceful shutdown).
