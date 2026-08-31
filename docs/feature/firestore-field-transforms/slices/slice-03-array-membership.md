# Slice 03: Array Membership Updates Without Duplicates

**Feature**: firestore-field-transforms | **Story**: US-03 | **Release**: 3 | **Estimate**: 1 day

## Goal

Prove `appendMissingElements`/`removeAllFromArray` — merged into one slice because they share an identical read-filter-write shape, differing only in add-vs-remove-matching — actually reuse this codebase's already-derived `FieldValue::PartialEq` for structural-equality filtering, correctly avoiding duplicates and removing all matching occurrences.

## IN Scope

- 2 new `embyr-core::FieldTransform` variants: `AppendMissingElements`, `RemoveAllFromArray`, each carrying `Vec<FieldValue>`.
- PG apply reads the existing array field under the same `FOR UPDATE` lock (Slice 02's own read-under-lock shape, reused unchanged), filters via `FieldValue::PartialEq` (already derived, zero new equality logic — `crates/embyr-core/src/domain/field_value.rs`), re-encodes, writes back.
- `appendMissingElements`: creates the field as the incoming array if missing; skips values already present (structural equality); appends in order.
- `removeAllFromArray`: no-op (does not create the field) if missing; removes ALL matching occurrences, not just the first.
- Resolution of the `transform_results`-for-array-ops question (Escalation 2, feature-delta.md) — implement per DESIGN's own resolution against the clarified `docs/SPEC.md` table, not a silent guess.

## OUT Scope

- `setToServerValue`/numeric transforms (Slices 01/02).
- Dotted/nested `field_path` targets.
- `backend_mode=agent`.

## Learning Hypothesis

**Disproves if it fails**: `FieldValue`'s already-derived `PartialEq` cannot be reused directly for array-membership filtering without new equality logic (e.g. `Timestamp`/`Reference`/`Map`-valued array elements compare incorrectly), or the read-filter-write shape cannot correctly avoid duplicate entries on `appendMissingElements` or correctly remove ALL occurrences on `removeAllFromArray`.

**Confirms if it succeeds**: this feature's third and final transform group works correctly, completing all 6 real transform kinds using entirely already-proven or already-derived primitives (row locking from Slice 02, structural equality already on `FieldValue`) — zero genuinely new mechanism introduced by this slice.

## Acceptance Criteria

- [ ] AC-03-01: `arrayUnion` adds elements not already present (structural equality via `FieldValue::PartialEq`).
- [ ] AC-03-02: `arrayUnion` is idempotent — re-adding an already-present element produces no duplicate.
- [ ] AC-03-03: `arrayRemove` removes ALL matching occurrences of a given element, not just the first.
- [ ] AC-03-04: `arrayUnion` against a missing field creates it as the incoming array.
- [ ] AC-03-05: `arrayRemove` against a missing field is a no-op and does NOT create the field.

## Production-Data Taste Test

Real Postgres, a real `trip_entries.sharedWithUserIds` array field on Maria Santos's `kilimanjaro-trek` document containing existing user IDs (`["u-diego"]`), an `arrayUnion` call re-adding `"u-diego"` (already present) plus `"u-priya"` (genuinely new) in the same transform, and a separate `arrayRemove` call against a field with a pre-existing duplicate — asserting no duplicate entry results and removal matches all occurrences. No mocked adapter.

## Dependencies

- Slice 01's own plumbing and Slice 02's own read-under-lock shape (both must ship first).
- `FieldValue::PartialEq` (shipped) — hard dependency, reused unchanged.
- Escalation 2 resolution (transform_results shape for array ops) — needed before this slice's own DELIVER, not before DESIGN starts.

## Reference Class

Slice 02's own read-compute-write-under-lock shape, applied here as read-filter-write instead of read-arithmetic-write. `FieldValue`'s derived `PartialEq` (`crates/embyr-core/src/domain/field_value.rs`) as the direct reuse target for structural equality.

## Pre-Slice SPIKE

Not needed — the read-under-lock shape is proven by Slice 02, and structural equality is already fully available via `PartialEq`. The one open question (Escalation 2) is a documentation/decision gap, not a technical unknown requiring research.
