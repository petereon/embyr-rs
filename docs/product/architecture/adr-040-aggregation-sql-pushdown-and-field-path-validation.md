# ADR-040: Aggregation SQL Push-Down and Field-Path Validation

## Status

Accepted

## Context

Slices 01/03/04 require a real `SELECT COUNT(*)`/`SUM(...)`/`AVG(...)` push-down for `PostgresBackendAdapter` (Postgres-family `backend_mode`s), reusing `append_filter` and the collection-group WHERE-clause branch from `run_query` unchanged (Slice 01's own Learning Hypothesis).

During DESIGN's own reading of `crates/embyr-pg-storage/src/encoding/query.rs` (required reading item 7), a second finding, related to but distinct from ADR-039's: **field paths are raw-string-interpolated into SQL fragments throughout the existing query-building code** —

```rust
qb.push(format!("fields->'{}'->>'v' = 'NaN'", f.field_path));               // append_field_filter, IS_NAN
qb.push(format!("(fields->'{}'->>'v')::bigint {} ", f.field_path, op));     // append_field_filter, comparisons
qb.push(format!("fields->'{}'->>'v' {}", ob.field_path, dir));              // order_by_expr
```

— with **zero field-path validation anywhere in `embyr-server`/`embyr-pg-storage`**, despite `docs/SPEC.md` §"Invariants" (Invariant 6) documenting `^[a-zA-Z_][a-zA-Z0-9_.]*$` as a requirement, and despite this feature's own DISCUSS slices (Slice 01/03 Technical Notes) explicitly assuming this validation "reuses the existing... invariant." No such enforcement exists to reuse. The only implementation resembling it anywhere in the codebase is `embyr-agent`'s own private `validate_field_path` (`crates/embyr-agent/src/server.rs:114`), which is weaker than SPEC.md's own documented invariant (it only rejects consecutive dots, not the full character-class regex), and lives in a separately-deployed binary this feature does not touch.

This is a genuine, previously-undocumented, latent SQL-injection-shaped gap in the already-shipped `RunQuery` (Postgres-family) path — not exploited to date only because no caller has yet supplied a malformed field path, not because anything structurally prevents it. This feature's own SUM/AVG aggregations introduce the FIRST place a caller-supplied field path is interpolated into an **aggregate** SQL expression (`SUM(...)`, `AVG(...)`) rather than a WHERE comparison — the same risk class, requiring a fresh, deliberate decision now, not an inherited assumption.

A related, positive finding: `crates/embyr-pg-storage/src/encoding/field_value.rs` confirms documents are stored with a **type-tagged JSON encoding** — `{"t": "I"|"D"|"S"|"B"|..., "v": <value>}` — making numeric-type detection for SUM/AVG a safe tag comparison, not a value-parsing gamble.

## Decision Drivers

1. **Never introduce a new, deliberately-designed SQL-interpolation site without validating its only variable component (the field path).** This is non-negotiable regardless of whether the pre-existing sites are fixed.
2. **Simplicity/laziness first** (standing session practice) — no new crate dependency for a statically-known character-class check.
3. **AC-01-12/AC-01-18 exclusion semantics** must be exact: "missing field" and "non-numeric value" are both silently excluded, never erroring, and the exclusion must be TYPE-aware (a string `"42"` is not numeric, even though it looks numeric), matching Firestore's own type system.
4. **AC-01-13 vs. AC-01-19**: SUM over zero/all-excluded rows returns `0`; AVG over zero/all-excluded rows returns null/absent — two DIFFERENT defaults for the same "no data" condition, both correctness-critical (AC-01-19 names the null-vs-zero distinction as this feature's single highest-consequence design risk).

## Decision

### 1. Field-path validation — new, first real enforcement of SPEC.md Invariant 6 outside `embyr-agent`

Add a pure, IO-free `validate_field_path(path: &str) -> Result<(), CoreError>` to `embyr-core` (`crates/embyr-core/src/domain/query.rs`), hand-implementing `^[a-zA-Z_][a-zA-Z0-9_.]*$` via a character scan — zero new crate dependency (`regex` is not added; the check is a handful of `char::is_ascii_alphanumeric`/first-char comparisons). `handle_run_aggregation_query` calls this on the SUM/AVG field selector before calling any adapter; a failure returns `Status::invalid_argument`, satisfying AC-01-15, before any SQL is built.

### 2. SQL push-down (`PostgresBackendAdapter::run_aggregation_query`, `crates/embyr-pg-storage/src/backend_adapter.rs`)

WHERE-clause construction (project scoping, collection/collection-group branch, `append_filter`, `NOT deleted`) is copied byte-for-byte from `run_query`; only the `SELECT` clause differs, and `ORDER BY`/`LIMIT`/`OFFSET`/cursor logic is omitted entirely (not meaningful for aggregation, per DISCUSS's own System Constraints).

- **COUNT**: `SELECT COUNT(*) FROM documents WHERE <shared WHERE-clause>`.
- **SUM**: `SELECT COALESCE(SUM(CASE WHEN fields->'{field}'->>'t' IN ('I','D') THEN (fields->'{field}'->>'v')::float8 ELSE NULL END), 0) FROM documents WHERE <shared WHERE-clause>`. The type-tag check makes this cast crash-free by construction (only `I`/`D`-tagged values ever reach the `::float8` cast); Postgres's own `float8` input parser already safely accepts this encoding's own `"NaN"`/`"Inf"`/`"-Inf"` string sentinels for non-finite doubles, so no extra casing is required for them. `COALESCE(..., 0)` supplies AC-01-13's required `0` default (Postgres's native `SUM()` returns SQL `NULL` over zero/all-`NULL` input otherwise).
- **AVG**: identical `CASE` expression, wrapped in bare `AVG(...)` — **no** `COALESCE`. Postgres's native `AVG()` aggregate already excludes `NULL` inputs from both numerator and denominator by definition — exactly AC-01-18's requirement, delivered by the SQL primitive itself rather than hand-written counting logic. A zero/all-excluded result set yields SQL `NULL` natively, mapped via `sqlx`'s `Option<f64>` row-decoding to `AggregateValue::Avg(None)` — AC-01-19's null-vs-zero distinction is structural (the primitive's own defined behavior), not a convention this code must remember to uphold.

### 3. Response value mapping

`AggregateValue::Count(i64)` → proto `Value::IntegerValue`. `AggregateValue::Sum(f64)` → `Value::DoubleValue` (always double, even for whole-number sums like `341200`; real Firestore preserves integer-typed sums as `integerValue` when every summed input is an integer — this is a **documented simplification**, not a functional gap: Firestore SDKs surface aggregate results as a plain numeric type in the client language regardless of the wire int/double distinction, making this very unlikely to be client-observable). `AggregateValue::Avg(Some(f64))` → `Value::DoubleValue`. `AggregateValue::Avg(None)` → `Value::NullValue`, present under its alias key in the `aggregate_fields` map (not an absent key — matches real Firestore's own documented behavior and SPEC.md's "null/absent" phrasing under the "present-but-null" reading).

## Alternatives Considered

1. **`regex` crate for field-path validation** — Rejected. Adds a new workspace dependency for a statically-known, trivial character-class check; a hand-written scan is fewer lines, zero new dependency, and easier to audit for exactly this security-critical purpose (simplicity-first, per standing session practice).
2. **Value-pattern-based numeric detection** (`fields->'{field}'->>'v' ~ '^-?[0-9...'`, ignoring the type tag) — Rejected once the type-tagged encoding was discovered: strictly worse — slower, AND would misclassify a numeric-looking STRING value (e.g., `amount_cents: "42"`, stored as `{"t":"S","v":"42"}`) as numeric, violating AC-01-12's "non-numeric value... silently excluded" requirement. Firestore's own type system, and this codebase's own storage encoding, both faithfully distinguish the string `"42"` from the number `42`; the type-tag check preserves that distinction, the regex approach would not.
3. **Retroactively adding `validate_field_path` to `RunQuery`'s own existing filter/order-by/cursor field paths** — Rejected as OUT OF SCOPE for this feature's own slices (mirrors Resolution 3's own precedent of naming, not silently fixing, pre-existing gaps outside the feature's walking skeleton) — but named explicitly, at HIGH priority given its SQL-interpolation nature, as a recommended near-term follow-up in this DESIGN's Handoff.
4. **A PL/pgSQL helper function (`safe_numeric(jsonb)`) instead of an inline `CASE`** — Rejected. Adds a schema migration and a server-side function to maintain for a three-line inline expression; no quality-attribute justification for the added operational surface.

## Consequences

**Positive**: SUM/AVG are correct and crash-free against real, messy data (mixed types, non-finite doubles, missing fields) by construction, not by convention. This is the FIRST real enforcement of SPEC.md's own Invariant 6 in the Postgres-family path, closing a portion of a latent security gap as a byproduct of doing this feature correctly. Zero new dependencies.

**Negative**: the field-path-validation gap for `RunQuery`'s own existing filter/order-by/cursor paths remains open (named, not fixed) — a second, HIGH-priority flagged item for the orchestrator, alongside ADR-039's `RunQuery`/agent-mode filter-forwarding finding. SUM's `DoubleValue`-always simplification is a minor, documented Firestore wire-fidelity gap (Decision 3), not a functional one.
