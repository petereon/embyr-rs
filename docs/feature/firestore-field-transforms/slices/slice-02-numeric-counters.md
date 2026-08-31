# Slice 02: Numeric Counters Compute Atomically and Type-Correctly

**Feature**: firestore-field-transforms | **Story**: US-02 | **Release**: 2 | **Estimate**: 1.5 days

## Goal

Prove `increment`/`maximum`/`minimum` — merged into one slice because they share an identical read-compute-write-under-lock shape, differing only in the arithmetic/comparison operator — actually compute atomically (no lost updates under concurrency) and preserve integer/double typing per `docs/SPEC.md`'s own promotion rule.

## IN Scope

- 3 new `embyr-core::FieldTransform` variants: `Increment`, `Maximum`, `Minimum`, each carrying a `FieldValue` operand.
- PG apply loop extends the already-proven `FOR UPDATE` row-locking idiom (`verify_versions`'s own pattern) to read the `fields` JSONB column (not just `update_time`), decode the target field, compute the new value in Rust (type-preserving match arms on `FieldValue::Integer(i64)`/`Double(f64)`), re-encode, `UPDATE` — all inside `commit_transaction`'s existing single `pg_txn`.
- Missing-field handling: `increment` treats missing as 0/0.0 per SPEC.md; `maximum`/`minimum` set the field directly to the given value (proposed SPEC.md addition, not yet locally documented — confirm with DESIGN).
- `InvalidArgument` rejection when the existing field value is non-numeric.
- Integer-overflow handling on `increment` (Escalation 1 — recommend `checked_add` → `InvalidArgument`, pending DESIGN confirmation).

## OUT Scope

- `setToServerValue`/array transforms (Slices 01/03).
- Dotted/nested `field_path` targets.
- `backend_mode=agent`.

## Learning Hypothesis

**Disproves if it fails**: extending the already-proven `FOR UPDATE` row-locking idiom to a read-compute-write shape (instead of Slice 01's read-nothing-just-write shape, or the existing OCC loop's read-compare-abort shape) cannot preserve integer/double typing correctly, or cannot guarantee atomicity — two concurrent `increment` calls against the same field lose an update.

**Confirms if it succeeds**: the row-locking idiom generalizes cleanly to a compute-and-persist shape, the highest real-world-impact transform group (counters, inventory) works correctly under real concurrency, and Slice 03 can reuse the identical read-under-lock shape.

## Acceptance Criteria

- [ ] AC-02-01: `increment` against an existing numeric field preserves integer/double type per SPEC.md's promotion rule.
- [ ] AC-02-02: Two concurrent `increment` calls against the same field both land — no lost update.
- [ ] AC-02-03: `increment` against a missing field treats it as 0 (integer delta) or 0.0 (double delta).
- [ ] AC-02-04: `maximum`/`minimum` correctly compare against the existing value and set the field to whichever is greater/lesser, type-preserved.
- [ ] AC-02-05: `increment`/`maximum`/`minimum` against a non-numeric existing value is rejected with `InvalidArgument`, document unmodified.

## Production-Data Taste Test

Real Postgres, a real `trip_entries.viewCount` field on Maria Santos's `kilimanjaro-trek` document starting at a real integer value, two concurrent `Commit` calls each incrementing by 1 — asserting the final persisted value reflects both increments (no lost update) and remains an integer type — plus a real `maximum`/`minimum` case against a real pre-existing numeric field (`promoBoostCount`). No mocked adapter, no synthetic single-caller shortcut.

## Dependencies

- Slice 01's own domain-model/wire-shape/response-wiring plumbing (must ship first).
- `verify_versions`'s own `FOR UPDATE` idiom (shipped) — extended, not replaced.
- Escalation 1 (integer overflow) and the `maximum`/`minimum` missing-field SPEC.md addition — both need DESIGN resolution before/during this slice's own DELIVER.

## Reference Class

`verify_versions` (`crates/embyr-pg-storage/src/transactions/occ.rs`) — the direct precedent for `SELECT ... FOR UPDATE` inside `commit_transaction`'s own single `pg_txn`, here extended from a read-compare-abort shape to a read-compute-write shape.

## Pre-Slice SPIKE

Not needed for the row-locking extension itself (direct generalization of an already-proven idiom). Escalation 1 (integer overflow) should be resolved as a DESIGN decision, not a SPIKE — no local or high-confidence external evidence exists to research toward; a documented default (`checked_add` → `InvalidArgument`) is the pragmatic path.
