# Evolution: firestore-end-cursor-support

**Date:** 2026-09-07
**Feature:** `RunQuery`'s `endAt`/`endBefore` cursors now correctly bound results (previously
silently ignored); `startAt`/`startAfter` now correctly flip their own comparison operator for
`DESC`-ordered queries (a closely-coupled, pre-existing bug fixed as a byproduct of not shipping a
second copy of it in new code).
**Job:** JOB-01 (`sdk-compat`) — ordinary SDK cursor methods, no reassignment needed.
**ADRs:** none (folded into feature-delta.md's own DESIGN section).

## This closes gap #2 from the 2026-09-06 production-readiness scan

Same severity class as gap #1 (`firestore-transaction-read-consistency`, closed the day before): a
well-behaved client silently gets WRONG results, not an error. A client paginating backward
(`endBefore` on the last-seen document) or windowing a range (`endAt` to bound a report) got a
query that returned more rows than it should, with no signal anything was wrong.

## Business Context

`sq_proto.end_at` was parsed nowhere in `handle_run_query` — hardcoded `None` — and
`embyr-pg-storage`'s own `run_query` had zero `end_at` SQL-generation logic at all. While reading
`start_at`'s own already-shipped mechanism to mirror it (per DESIGN's own instruction), direct
comparison against SPEC.md's documented cursor operator table surfaced a second, genuinely
pre-existing bug: `start_at`/`startAfter`'s own operator selection ignored the query's sort
direction entirely, always applying the `ASC` operator even when the query was `DESC`-ordered.
Shipping `end_at` by literally copying `start_at`'s own (subtly wrong) logic would have shipped
the same bug twice; instead, one shared, direction-aware `cursor_operator` helper fixes both.

## Key Decisions

| Decision | Verdict |
|---|---|
| JOB-01/P1 Alex confirmed correct — ordinary SDK cursor methods | feature-delta.md § Orchestrator Decisions |
| Mechanism: one shared `cursor_operator(is_end, before, direction)` helper used by both `start_at` and `end_at`, rather than duplicating (and doubly shipping the DESC bug in) 2 separate blocks | § Architecture Design, D1 |
| Scope matches `start_at`'s own existing limits exactly: `Integer`/`String`/`Double` value types only, single-field `orderBy` only | § Out of Scope |

## Steps Completed

1. **Slice 01 (Walking Skeleton, entire feature)**: `cursor_operator` helper added to
   `crates/embyr-pg-storage/src/backend_adapter.rs`, implementing SPEC.md's own documented 4-cursor
   × 2-direction operator table exactly. `start_at`'s existing block now calls it instead of a
   hardcoded ASC-only operator; a new, structurally-mirrored `end_at` block does the same.
   `handle_run_query` gains one new field-population line for `end_at`, mirroring `start_at`'s own
   translation exactly. Proven end-to-end: `endAt` inclusive bound, `endBefore` exclusive bound, and
   `startAt` on a `DESC`-ordered field correctly flipping its own operator — all in
   `tests/acceptance/us_04_query_collection.rs`.

**Full regression**: `cargo test -p embyr-server --no-fail-fast` (the `--no-fail-fast` lesson from
the prior feature applied from the start this time). Clean apart from 3 already-documented
pre-existing, unrelated issues (`distributed_rate_limiting`'s Postgres-contention flake,
`secrets_management`'s Docker/LocalStack timing, `security_rules_cel_parity_cp04`'s CEL
chaining-detection gap) plus one confirmed-transient Docker port-mapping flake
(`firestore_composite_indexes_admin_api_cix01`, passed clean on isolated rerun).

**QUALITY_GATE**: 21 mutants, clean multi-package diff (no `--exclude-re` needed this time). First
pass found 2 real gaps — `start_at`'s own `Integer`/`Double` cursor value types had zero test
coverage of any kind despite `start_at` itself already having shipped. Closed with a new test; see
§ Lessons Learned for the correction needed along the way, and the mutation report for the full
account of a second computational rerun invalidated by environment corruption, verified instead by
direct inspection.

## Lessons Learned

1. **Never blindly mirror existing code — read it critically enough to catch its own bugs.**
   `start_at`'s own operator selection (`let op = if cursor.before { ">=" } else { ">" };`) looked
   like a reasonable thing to copy for `end_at`. Direct comparison against SPEC.md's own documented
   cursor operator table caught that it was wrong for `DESC`-ordered queries — a bug that had
   shipped, unnoticed, before this feature. Mirroring an existing block is the right lazy default,
   but "mirror" should mean "reuse the proven SHAPE," not "assume the CONTENTS are correct without
   checking against the spec."
2. **QUALITY_GATE on new code can surface coverage gaps in already-shipped code it merely
   extends.** `start_at`'s own `Integer`/`Double` arms were untested from the day they shipped —
   every cursor test in this codebase happened to use `String` values. This feature's own mutation
   testing found it only because the SAME 3-arm match shape was duplicated into new (`end_at`)
   code, making cargo-mutants generate an equivalent mutant for the pre-existing block too.
3. **When 2 structurally-identical blocks exist in the same function, verify the EXACT reported
   line number before writing a fix — don't assume which occurrence a mutant refers to.** The
   first fix attempt added `Integer`/`Double` coverage to `end_at` when the actual gap was in
   `start_at` — both blocks generate an identically-worded mutant description ("delete match arm
   `FieldValue::Integer(v)` in `run_query`"), differing only by line number. Reading the mutant
   report's own line number against the CURRENT file state (line numbers shift after every edit)
   is the only reliable way to know which block actually needs the new test.
4. **`git diff <commit>` (a single ref) diffs against the WORKING TREE, not another commit** — hit
   again this feature (2nd occurrence this session, after `firestore-transaction-read-consistency`)
   when generating the QUALITY_GATE diff pulled in an unrelated, pre-existing dirty file. Diff
   commit-to-commit (`git diff <a> <b> -- <paths>`) every time, no exceptions.
5. **A background maintenance process can invalidate an entire mutation-testing pass, and that's
   worth recognizing rather than endlessly retrying.** A periodic `cargo-sweep-shared-target.sh`
   process (confirmed independently racing builds multiple times this session, including one
   unusually long 30+ minute sweep) corrupted a full computational rerun mid-flight — every
   "unviable" result traced to the SAME `No such file or directory` error against unrelated crates,
   not a real compile failure of the mutated code. Recognizing environment noise for what it is,
   discarding the invalid run, and verifying the fix by direct inspection against a genuinely valid
   earlier pass was the right call rather than a 3rd multi-hour rerun against a system this
   demonstrably contended.

## Key Files

- `crates/embyr-pg-storage/src/backend_adapter.rs` — new `cursor_operator` helper; `start_at`'s
  existing block updated to use it; new `end_at` block mirroring `start_at`'s own shape.
- `crates/embyr-server/src/grpc/handler.rs` — `handle_run_query`'s new `end_at` translation.
- `tests/acceptance/us_04_query_collection.rs` — 4 new tests: `endAt` inclusive, `endBefore`
  exclusive, `startAt` on `DESC` order, and the combined Integer/Double coverage test (covering both
  `start_at` and `end_at`).
- `docs/feature/firestore-end-cursor-support/feature-delta.md` — full DISCUSS/DESIGN narrative.
- `docs/feature/firestore-end-cursor-support/deliver/mutation/mutation-report.md` — full account of
  both fix attempts and the environment-corruption-invalidated second pass.
- `docs/product/jobs.yaml`, JOB-01 — new NOTE appended (during DISCUSS).

## Follow-Up Work

- **Extending cursor support (either bound) to more `FieldValue` types** — `start_at`/`end_at` both
  match `Integer`/`String`/`Double` only, silently no-op for `Timestamp`/`Boolean`/`Bytes`/
  `Reference`. Matches this feature's own explicitly-locked scope (mirrors `start_at`'s existing
  limits exactly); widening both is a separate, later follow-up, not started here.
- **Multi-field cursors** — explicitly out of scope, unchanged from `start_at`'s own existing
  single-field-orderBy-only limitation.

Carried forward, unchanged, from `docs/product/known-gaps.md`: #3 (`Filter.or()` composite OR
rejected), #4 (`IS_NULL`/`IS_NOT_NULL` unary filter rejected), #5 (no TLS/mTLS on any listener), #6
(no graceful shutdown), #7 (`secrets_management` Docker/LocalStack timing flakiness), #8 (CEL
"chaining" construct-detection gap).
