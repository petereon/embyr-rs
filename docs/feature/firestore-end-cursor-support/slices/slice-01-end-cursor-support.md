# Slice 01: `endAt`/`endBefore` Cursor Support + `startAt`/`startAfter` DESC-Direction Fix (Walking Skeleton)

**Goal**: `RunQuery`'s `endAt`/`endBefore` cursors correctly bound results (previously silently
ignored); `startAt`/`startAfter` correctly flip their own operator for `DESC`-ordered queries
(a closely-coupled, pre-existing bug fixed as part of not duplicating it into the new code).

## IN scope
- New `cursor_operator(is_end, before, direction) -> &'static str` helper in
  `crates/embyr-pg-storage/src/backend_adapter.rs`, colocated with the cursor SQL-generation code.
- `start_at`'s own existing block: replace its hardcoded operator selection with a call to
  `cursor_operator`.
- New `end_at` block, structurally mirroring `start_at`'s own (`Integer`/`String`/`Double` value
  types, single-field `orderBy` only), using `cursor_operator` for its own operator.
- `handle_run_query`: parse `sq_proto.end_at` into a domain `Cursor`, mirroring `start_at`'s own
  existing translation exactly.

## OUT scope
- Extending cursor support to other `FieldValue` types (`Timestamp`/`Boolean`/etc.) — matches
  `start_at`'s own existing scope exactly.
- Multi-field cursors, page-token interaction — unaffected, unchanged.

## Learning hypothesis
**Disproves** (if it fails): `end_at` support is a trivial mirror of `start_at`'s own already-proven
mechanism, needing only a small shared operator helper. If `end_at` needs a fundamentally different
SQL shape than `start_at`'s own, that premise was wrong.
**Confirms** (if it succeeds): the operator-table fix (DESC direction) generalizes cleanly across
both cursor bounds via one small pure function, and this codebase's own established "mirror an
existing block" pattern (used repeatedly this session — range operators, malformed-filter-shape)
holds here too.

## Acceptance criteria
AC-EC-01 through AC-EC-05 (feature-delta.md § US-01).

## Dependencies
None — first slice.

## Effort estimate
≤0.5 day.

## Reference class
`crates/embyr-pg-storage/src/backend_adapter.rs`'s own existing `start_at` block (lines 598-628),
already shipped and proven.

## Production-data acceptance criterion
Real `RunQuery` calls against a real running server + real Postgres backend: seed multiple
documents, run queries with `endAt`/`endBefore` cursors on `ASC`-ordered fields and confirm the
correct subset is returned; run a `startAt` query on a `DESC`-ordered field and confirm the
DESC-flipped operator is applied. Not mocked, not synthetic-plumbing-only.
