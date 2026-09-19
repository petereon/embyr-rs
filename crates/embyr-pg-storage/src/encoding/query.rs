use base64::{engine::general_purpose::STANDARD, Engine};
use embyr_core::domain::{
    field_value::FieldValue,
    query::{validate_field_path, FilterOp, FieldFilter, OrderBy, OrderDirection, QueryFilter},
};
use sqlx::{Postgres, QueryBuilder};

use crate::encoding::field_value::field_value_to_json;

/// Append a `QueryFilter` to the builder as a SQL predicate.
///
/// Composite (AND) filters are expanded recursively, unparenthesized (always
/// correct — AND-of-AND is associative). `CompositeOr` (firestore-or-filter
/// -support) filters are OR-joined and the WHOLE expression is parenthesized
/// — required for correct precedence now that OR can appear nested inside
/// AND context (or vice versa).
pub fn append_filter(qb: &mut QueryBuilder<Postgres>, filter: &QueryFilter) {
    match filter {
        QueryFilter::Field(f) => append_field_filter(qb, f),
        QueryFilter::Composite(filters) => {
            for (i, f) in filters.iter().enumerate() {
                if i > 0 {
                    qb.push(" AND ");
                }
                append_filter(qb, f);
            }
        }
        QueryFilter::CompositeOr(filters) => {
            qb.push("(");
            for (i, f) in filters.iter().enumerate() {
                if i > 0 {
                    qb.push(" OR ");
                }
                append_filter(qb, f);
            }
            qb.push(")");
        }
    }
}

/// Append a single field comparison predicate.
///
/// Values are bound via `push_bind` — never interpolated — to prevent SQL injection.
/// IS_NAN uses string sentinel equality: `fields->'f'->>'v' = 'NaN'`.
pub fn append_field_filter(qb: &mut QueryBuilder<Postgres>, f: &FieldFilter) {
    // field-path-defense-in-depth (finding #26): every current caller
    // already validates upstream before reaching here — this guard is
    // purely additive hardening for a future caller that doesn't. Panics
    // (not `Result`) to match this file's own existing convention for the
    // identical class of "assumed validated upstream, can only fire on a
    // caller bug" defect (see the `_ => panic!(...)` arms below).
    if let Err(e) = validate_field_path(&f.field_path) {
        panic!("invalid field path reached SQL builder: {e}");
    }

    // Handle IS_NAN and IS_NOT_NAN as special cases (no value binding required).
    match f.op {
        FilterOp::IsNan => {
            qb.push(format!("fields->'{}'->>'v' = 'NaN'", f.field_path));
            return;
        }
        FilterOp::IsNotNan => {
            qb.push(format!(
                "(fields->'{fp}' IS NULL OR fields->'{fp}'->>'v' != 'NaN')",
                fp = f.field_path
            ));
            return;
        }
        // firestore-is-null-filter-support: FieldValue::Null encodes as
        // `{"t": "N"}` (no `v` key). Unlike IS_NOT_NAN above, a MISSING
        // field matches neither IS_NULL nor IS_NOT_NULL — real Firestore's
        // own documented semantics require the field to be present.
        FilterOp::IsNull => {
            qb.push(format!("fields->'{}'->>'t' = 'N'", f.field_path));
            return;
        }
        FilterOp::IsNotNull => {
            qb.push(format!(
                "(fields->'{fp}' IS NOT NULL AND fields->'{fp}'->>'t' != 'N')",
                fp = f.field_path
            ));
            return;
        }
        _ => {}
    }

    match f.op {
        FilterOp::LessThan => push_scalar_comparison(qb, &f.field_path, "<", &f.value),
        FilterOp::LessThanOrEqual => push_scalar_comparison(qb, &f.field_path, "<=", &f.value),
        FilterOp::GreaterThan => push_scalar_comparison(qb, &f.field_path, ">", &f.value),
        FilterOp::GreaterThanOrEqual => push_scalar_comparison(qb, &f.field_path, ">=", &f.value),
        // firestore-equal-notequal-value-type-support (Slice 01, US-01,
        // AC-ENV-01 through AC-ENV-07): reuses `push_value_equality`
        // (built for `In`/`NotIn`) instead of `push_scalar_comparison` —
        // equality/inequality never needed type-specific casting, only a
        // match against the value AS STORED, which whole-object JSON
        // comparison already provides uniformly for every `FieldValue`
        // variant (including `Timestamp`/`Bytes`/`Reference`/`Array`/`Map`,
        // which `push_scalar_comparison`'s own narrower match panics on).
        FilterOp::Equal => push_value_equality(qb, &f.field_path, &f.value, false),
        FilterOp::NotEqual => push_value_equality(qb, &f.field_path, &f.value, true),
        // firestore-query-filter-operator-support (Slice 01, US-01,
        // AC-QFO-01/02/03): `f.value` is the single scalar being searched
        // for inside the array field — JSONB `@>` containment against a
        // one-element array checks "does the array field contain this
        // element", reusing `field_value_to_json` unchanged.
        FilterOp::ArrayContains => push_array_contains(qb, &f.field_path, &f.value),
        // firestore-query-filter-operator-support (Slice 02, US-02,
        // AC-QFO-04/05): `f.value` is a `FieldValue::Array` of targets —
        // OR of `push_array_contains` checks, one per target ("contains AT
        // LEAST ONE"). Resolution 3: an empty target list matches nothing.
        FilterOp::ArrayContainsAny => {
            let FieldValue::Array(targets) = &f.value else {
                panic!("unsupported filter value type for ArrayContainsAny: {:?}", f.value)
            };
            if targets.is_empty() {
                qb.push("FALSE");
            } else {
                qb.push("(");
                for (i, target) in targets.iter().enumerate() {
                    if i > 0 {
                        qb.push(" OR ");
                    }
                    push_array_contains(qb, &f.field_path, target);
                }
                qb.push(")");
            }
        }
        // firestore-query-filter-operator-support (Slice 03, US-03,
        // AC-QFO-06/07): `f.value` is a `FieldValue::Array` of targets —
        // OR of whole-value equality checks, one per target ("equals
        // ANY"). Resolution 3: an empty target list matches nothing.
        FilterOp::In => {
            let FieldValue::Array(targets) = &f.value else {
                panic!("unsupported filter value type for In: {:?}", f.value)
            };
            if targets.is_empty() {
                qb.push("FALSE");
            } else {
                qb.push("(");
                for (i, target) in targets.iter().enumerate() {
                    if i > 0 {
                        qb.push(" OR ");
                    }
                    push_value_equality(qb, &f.field_path, target, false);
                }
                qb.push(")");
            }
        }
        // firestore-query-filter-operator-support (Slice 04, US-04,
        // AC-QFO-08/09/10): `f.value` is a `FieldValue::Array` of excluded
        // targets — AND of whole-value inequality checks, one per target
        // ("not equal to ANY"). Real Firestore's own live-verified
        // field-must-exist rule (a document missing the field entirely is
        // excluded from `not-in` results) falls out of Postgres's own NULL
        // three-valued-logic propagation for free here: if the field is
        // absent, `fields->'{f}'` (the WHOLE discriminated-union value,
        // not just its unwrapped "v") is SQL NULL, so EVERY per-target
        // `!=` comparison is NULL, so the AND-chain is NULL, so the row is
        // excluded from WHERE. Comparing the WHOLE `{"t":...,"v":...}`
        // object (via `push_value_equality`), rather than extracting `->>
        // 'v'` the way `push_scalar_comparison` does for the pre-existing
        // operators, is what correctly distinguishes a field explicitly
        // set to `Null` (`{"t":"N"}`, a real JSONB value, not equal to any
        // non-null target) from a field that is entirely ABSENT (`fields
        // ->'{f}'` itself is SQL NULL) — extracting `->>'v'` first would
        // conflate the two (a `Null`-valued field has no "v" key either,
        // so `->>'v'` is NULL in BOTH cases), discovered via a real,
        // failing acceptance test during this slice's own DELIVER
        // (AC-QFO-10). Resolution 3: an empty target list means "not equal
        // to any of zero excluded values", vacuously true for any EXISTING
        // field — the one case needing an explicit fallback, since there's
        // no per-target clause left to derive the field-must-exist
        // property from.
        FilterOp::NotIn => {
            let FieldValue::Array(targets) = &f.value else {
                panic!("unsupported filter value type for NotIn: {:?}", f.value)
            };
            if targets.is_empty() {
                qb.push(format!("fields->'{}' IS NOT NULL", f.field_path));
            } else {
                qb.push("(");
                for (i, target) in targets.iter().enumerate() {
                    if i > 0 {
                        qb.push(" AND ");
                    }
                    push_value_equality(qb, &f.field_path, target, true);
                }
                qb.push(")");
            }
        }
        _ => panic!("unsupported filter op: {:?}", f.op),
    }
}

/// firestore-query-filter-operator-support (Slice 03, US-03, ADR — Design):
/// extracted from `append_field_filter`'s own original inline match — a
/// pure refactor, zero behavior change for the 4 range operators
/// (`LessThan` through `GreaterThanOrEqual`), which it continues to serve
/// exclusively — `In`/`NotIn`/`Equal`/`NotEqual` all use `push_value_
/// equality` instead (below, widened by firestore-equal-notequal-value
/// -type-support to cover `Equal`/`NotEqual` too), NOT this function, since
/// range comparisons need the unwrapped, type-cast `->>'v'` extraction this
/// function provides, while equality/inequality against a whole stored
/// value does not (and, as discovered via AC-QFO-10's own failing test,
/// must NOT use it — see `push_value_equality`'s own doc). This function's
/// own `_ => panic!(...)` fallback still fires for `Timestamp`/`Bytes`/
/// `Reference`/`Array`/`Map`-valued RANGE-comparison targets — a real,
/// separately-evidenced, deferred gap (candidate id `firestore-range
/// -operator-value-type-support`), since range comparisons need ordering,
/// not equality, and whole-object JSON comparison has no meaningful
/// "less than" for a composite type.
fn push_scalar_comparison(qb: &mut QueryBuilder<Postgres>, field_path: &str, op: &str, value: &FieldValue) {
    match value {
        FieldValue::Integer(v) => {
            qb.push(format!("(fields->'{field_path}'->>'v')::bigint {op} "));
            qb.push_bind(*v);
        }
        FieldValue::String(s) => {
            qb.push(format!("fields->'{field_path}'->>'v' {op} "));
            qb.push_bind(s.clone());
        }
        FieldValue::Double(d) => {
            qb.push(format!("(fields->'{field_path}'->>'v')::float8 {op} "));
            qb.push_bind(*d);
        }
        FieldValue::Boolean(b) => {
            qb.push(format!("(fields->'{field_path}'->>'v')::boolean {op} "));
            qb.push_bind(*b);
        }
        // firestore-range-operator-value-type-support (Slice 01, US-01,
        // AC-RNG-01): `Timestamp` is stored as `{"t":"TS","s":..,"n":..}`
        // — no single `"v"` key, so the `->>'v'`-extraction pattern above
        // doesn't apply. Postgres `ROW(...)` comparison compares its own
        // elements lexicographically (seconds first, nanos as tiebreaker),
        // directly matching chronological ordering with zero arithmetic
        // -overflow risk (vs. combining into one `seconds*1e9+nanos` value).
        FieldValue::Timestamp(s, n) => {
            qb.push(format!(
                "ROW((fields->'{field_path}'->'s')::bigint, (fields->'{field_path}'->'n')::int) {op} ROW("
            ));
            qb.push_bind(*s);
            qb.push(", ");
            qb.push_bind(*n);
            qb.push(")");
        }
        // firestore-range-operator-value-type-support (Slice 01, US-01,
        // AC-RNG-02): live-verified real Firestore orders `Bytes` by raw
        // byte value — standard base64's own alphabet (`A-Za-z0-9+/`) does
        // NOT preserve byte-value ordering when compared as TEXT (e.g. `+`
        // sorts after the entire alphanumeric range in base64's own
        // alphabet, but its raw byte value 0x2B sorts BEFORE digits and
        // letters). Decoding both sides to raw `bytea` in SQL and letting
        // Postgres's own byte-wise `bytea` comparison operator do the work
        // is what correctly matches real Firestore's own semantic — reuses
        // `STANDARD` (the SAME base64 engine `field_value_to_json` already
        // encodes with), never a second, divergent encoding.
        FieldValue::Bytes(b) => {
            qb.push(format!("decode(fields->'{field_path}'->>'v', 'base64') {op} decode("));
            qb.push_bind(STANDARD.encode(b));
            qb.push(", 'base64')");
        }
        // firestore-range-operator-value-type-support (Slice 01, US-01,
        // AC-RNG-03): live-verified real Firestore orders `Reference`
        // structurally (path-segment-by-segment), not as a flat string —
        // but for SAME-DEPTH references (the evidenced, common case: a
        // `Reference`-valued field virtually always points into one fixed
        // target collection), flat lexicographic string comparison and
        // true segment-wise comparison produce IDENTICAL results (a shared
        // prefix compares character-for-character equal up to the first
        // differing segment, where both mechanisms then compare that
        // segment's own text identically). Cross-depth reference
        // comparison is a named, zero-evidence, deferred divergence
        // (feature-delta.md § Out of Scope) — not silently claimed correct
        // in every case.
        FieldValue::Reference(r) => {
            qb.push(format!("fields->'{field_path}'->>'v' {op} "));
            qb.push_bind(r.clone());
        }
        // firestore-range-operator-value-type-support (Slice 02, US-02):
        // `Array`/`Map` should NEVER actually reach this function — a
        // range-operator filter against either is rejected upstream, at
        // proto-translation time, in `embyr-server`'s own
        // `translate_filter` (reusing its own existing `Result<_, String>`
        // -> `Status::invalid_argument` mechanism, matching real
        // Firestore's own confirmed rejection of `Array` range queries,
        // and a conservative default for `Map`'s own unconfirmed support).
        // This remains a defensive panic for genuinely-unreachable
        // post-validation code, not a live code path.
        _ => panic!("unsupported filter value type: {:?}", value),
    }
}

/// firestore-query-filter-operator-support (Slice 01, US-01, ADR —
/// Design): does the array field at `field_path` contain `target` as one
/// of its own elements — JSONB `@>` containment against a one-element
/// array, reusing `field_value_to_json` (the SAME discriminated-union
/// encoding every document field is already stored with) so the bound
/// JSON structurally matches an existing array element exactly.
fn push_array_contains(qb: &mut QueryBuilder<Postgres>, field_path: &str, target: &FieldValue) {
    qb.push(format!("fields->'{field_path}'->'v' @> "));
    qb.push_bind(serde_json::json!([field_value_to_json(target)]));
    qb.push("::jsonb");
}

/// firestore-query-filter-operator-support (Slice 03/04, US-03/US-04),
/// widened by firestore-equal-notequal-value-type-support (Slice 01,
/// US-01) to cover `Equal`/`NotEqual` too: used by `In`/`NotIn`/`Equal`/
/// `NotEqual` — compares the field's own FULL discriminated-union JSON
/// value (`{"t": ..., "v": ...}`, via `field_value_to_json`) against
/// `target`'s own identical encoding, rather than extracting and casting
/// the unwrapped `"v"` the way `push_scalar_comparison` does. This is what
/// makes it work UNIFORMLY for every `FieldValue` variant with zero
/// per-type dispatch — `field_value_to_json` already covers all 10,
/// including `Timestamp`/`Bytes`/`Reference`/`Array`/`Map` (the 5 types
/// `push_scalar_comparison`'s own narrower match panics on).
///
/// This distinction is load-bearing, not stylistic: a field explicitly
/// stored as `FieldValue::Null` encodes to `{"t":"N"}` — a real JSONB
/// value with no `"v"` key at all. Extracting `->>'v'` first (as `push_
/// scalar_comparison` does) would yield SQL `NULL` for a `Null`-valued
/// field, INDISTINGUISHABLE from a field that is entirely ABSENT (where
/// `fields->'{field_path}'` itself is `NULL`) — silently breaking
/// `NotIn`'s own live-verified field-must-exist rule (a present-but-null
/// field would be wrongly excluded, identically to an absent one).
/// Comparing the WHOLE object instead means only genuine absence produces
/// SQL `NULL`; a `Null`-valued field compares as a real, distinct JSONB
/// value, correctly satisfying `!=` against any non-null target. Found via
/// a real, failing acceptance test (AC-QFO-10) during this feature's own
/// DELIVER, not by inspection alone. A useful side effect: this comparison
/// supports every `FieldValue` variant (no per-type dispatch, no
/// "unsupported filter value type" panic) since `field_value_to_json`
/// already covers all of them.
fn push_value_equality(qb: &mut QueryBuilder<Postgres>, field_path: &str, target: &FieldValue, negate: bool) {
    let op = if negate { "!=" } else { "=" };
    qb.push(format!("fields->'{field_path}' {op} "));
    qb.push_bind(field_value_to_json(target));
    qb.push("::jsonb");
}

/// Return a raw SQL ORDER BY expression for a single `OrderBy` clause.
///
/// This string is pushed as raw SQL (no bind parameter), since ORDER BY
/// expressions cannot be parameterised in Postgres.
pub fn order_by_expr(ob: &OrderBy) -> String {
    // field-path-defense-in-depth (finding #26): see `append_field_filter`'s
    // own identical guard comment for the full rationale.
    if let Err(e) = validate_field_path(&ob.field_path) {
        panic!("invalid field path reached SQL builder: {e}");
    }
    let dir = match ob.direction {
        OrderDirection::Ascending => "ASC",
        OrderDirection::Descending => "DESC",
    };
    format!("fields->'{}'->>'v' {}", ob.field_path, dir)
}

/// Return a raw SQL ORDER BY expression with explicit bigint cast (for integer fields).
pub fn order_by_expr_bigint(field_path: &str, dir: &str) -> String {
    // field-path-defense-in-depth (finding #26): see `append_field_filter`'s
    // own identical guard comment for the full rationale. Dead code today
    // (zero callers workspace-wide) but still `pub fn` and part of the
    // exported SQL-building surface.
    if let Err(e) = validate_field_path(field_path) {
        panic!("invalid field path reached SQL builder: {e}");
    }
    format!("(fields->'{}'->>'v')::bigint {}", field_path, dir)
}

// ─── collection-group-query-index (ADR-080 Decision D) ────────────────────
//
// Single-source-of-truth builder for the `all_descendants=true` predicate,
// extracted as a public, pure function so it can be shared (mirroring
// `append_filter`/`order_by_expr`'s own existing "one function, four call
// sites" convention, ADR-040 §2) by all 4 mirrored call sites in
// `backend_adapter.rs` (`run_query` + `run_aggregation_query`'s `Count`/
// `Sum`/`Avg` arms) AND by this feature's own acceptance tests (so an
// `EXPLAIN` in a test reflects the exact SQL production code executes, with
// zero drift risk — the same reasoning `composite_index_real_creation`'s own
// `explain_category_equal_electronics_order_by_score_desc` test helper
// already established for a different predicate).
//
// `schema_available == true`: push ADR-080's hybrid shape —
//   collection_id = $N OR (collection_id IS NULL AND (collection_path = $N
//   OR collection_path LIKE '%/' || $N))
// `schema_available == false`: push today's byte-for-byte-unchanged LIKE-only
//   shape (never references `collection_id`).
pub fn push_all_descendants_predicate(
    qb: &mut QueryBuilder<Postgres>,
    collection_id: &str,
    schema_available: bool,
) {
    if schema_available {
        qb.push("(collection_id = ");
        qb.push_bind(collection_id.to_string());
        qb.push(" OR (collection_id IS NULL AND (collection_path = ");
        qb.push_bind(collection_id.to_string());
        qb.push(" OR collection_path LIKE ");
        qb.push_bind(format!("%/{collection_id}"));
        qb.push(")))");
    } else {
        qb.push("(collection_path = ");
        qb.push_bind(collection_id.to_string());
        qb.push(" OR collection_path LIKE ");
        qb.push_bind(format!("%/{collection_id}"));
        qb.push(")");
    }
}

/// ADR-080 Decision C — pure TTL-boundary decision for the schema-capability
/// cache's `Unavailable` branch. `Available` is cached permanently (never
/// calls this); `Unavailable` re-checks once `checked_at.elapsed() >= ttl`.
/// Extracted as a standalone pure function (not inlined in
/// `PostgresBackendAdapter::schema_capability`) specifically so this
/// feature's one new piece of non-trivial decision logic is directly
/// unit-testable without a database (DISTILL's own "Not scaffolded" note —
/// this is DELIVER's own inner-loop extraction).
pub fn is_probe_stale(checked_at: std::time::Instant, ttl: std::time::Duration) -> bool {
    checked_at.elapsed() >= ttl
}

#[cfg(test)]
mod tests {
    //! firestore-query-filter-operator-support (Slices 01-04) — pure, IO
    //! -free unit coverage for `append_field_filter`'s own SQL generation.
    //! `QueryBuilder::sql()` exposes the generated SQL text without needing
    //! a live DB connection — used here to verify shape (never a panic,
    //! correct composition/parenthesization) directly; real EXECUTION
    //! correctness is proven separately by this feature's own Docker
    //! -backed acceptance tests.
    use super::*;

    fn filter(field: &str, op: FilterOp, value: FieldValue) -> FieldFilter {
        FieldFilter { field_path: field.to_string(), op, value }
    }

    fn generated_sql(f: &FieldFilter) -> String {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("");
        append_field_filter(&mut qb, f);
        qb.sql().to_string()
    }

    #[test]
    fn array_contains_generates_a_single_jsonb_containment_check() {
        let f = filter("tags", FilterOp::ArrayContains, FieldValue::String("urgent".to_string()));
        let sql = generated_sql(&f);
        assert!(sql.contains("fields->'tags'->'v' @>"), "got: {sql}");
        assert!(sql.contains("::jsonb"), "got: {sql}");
    }

    /// Uses 3 targets (not 2) so the join-boundary logic (`if i > 0`) is
    /// exercised across BOTH boundaries, not just one — catches a mutant
    /// that flips `>` to `==`/`>=` and shifts WHERE the separator lands
    /// (e.g. before the first item instead of between items) without
    /// changing the separator's own overall PRESENCE or total occurrence
    /// count, which a looser `contains(" OR ")`-only assertion would miss.
    #[test]
    fn array_contains_any_ors_exactly_n_minus_one_separators_between_n_targets() {
        let f = filter(
            "tags",
            FilterOp::ArrayContainsAny,
            FieldValue::Array(vec![
                FieldValue::String("a".to_string()),
                FieldValue::String("b".to_string()),
                FieldValue::String("c".to_string()),
            ]),
        );
        let sql = generated_sql(&f);
        assert_eq!(sql.matches("@>").count(), 3, "expected 3 containment checks, got: {sql}");
        assert_eq!(sql.matches(" OR ").count(), 2, "expected exactly 2 separators for 3 targets, got: {sql}");
        assert!(!sql.trim_start().starts_with("( OR"), "must not lead with a separator, got: {sql}");
    }

    #[test]
    fn array_contains_any_with_an_empty_target_list_generates_false() {
        let f = filter("tags", FilterOp::ArrayContainsAny, FieldValue::Array(vec![]));
        assert_eq!(generated_sql(&f).trim(), "FALSE");
    }

    /// 3 targets, exact separator count — same join-boundary rationale as
    /// `array_contains_any_ors_exactly_n_minus_one_separators_between_n_targets`.
    #[test]
    fn in_ors_exactly_n_minus_one_separators_between_n_targets() {
        let f = filter(
            "status",
            FilterOp::In,
            FieldValue::Array(vec![
                FieldValue::String("a".to_string()),
                FieldValue::String("b".to_string()),
                FieldValue::String("c".to_string()),
            ]),
        );
        let sql = generated_sql(&f);
        assert_eq!(sql.matches(" = ").count(), 3, "expected 3 equality checks, got: {sql}");
        assert_eq!(sql.matches(" OR ").count(), 2, "expected exactly 2 separators for 3 targets, got: {sql}");
    }

    #[test]
    fn in_with_an_empty_target_list_generates_false() {
        let f = filter("status", FilterOp::In, FieldValue::Array(vec![]));
        assert_eq!(generated_sql(&f).trim(), "FALSE");
    }

    /// `In` compares the WHOLE discriminated-union JSON value (not the
    /// unwrapped `->>'v'`), so a heterogeneous target list needs no
    /// per-type dispatch at all — every target, regardless of its own
    /// `FieldValue` variant, compares uniformly via `field_value_to_json`.
    #[test]
    fn in_with_a_heterogeneous_target_list_compares_whole_values_uniformly() {
        let f = filter(
            "value",
            FilterOp::In,
            FieldValue::Array(vec![FieldValue::Integer(1), FieldValue::String("two".to_string())]),
        );
        let sql = generated_sql(&f);
        assert_eq!(sql.matches("fields->'value' = ").count(), 2, "got: {sql}");
        assert_eq!(sql.matches("::jsonb").count(), 2, "got: {sql}");
    }

    /// A target list containing `FieldValue::Null` — an exotic type that
    /// would panic under the OLD `push_scalar_comparison`-based design
    /// (`_ => panic!("unsupported filter value type")`) — now works, since
    /// `push_value_equality` supports every `FieldValue` variant uniformly.
    #[test]
    fn in_with_a_null_target_does_not_panic() {
        let f = filter("status", FilterOp::In, FieldValue::Array(vec![FieldValue::Null]));
        let sql = generated_sql(&f);
        assert!(sql.contains("fields->'status' = "), "got: {sql}");
    }

    /// 3 targets, exact separator count — same join-boundary rationale as
    /// `array_contains_any_ors_exactly_n_minus_one_separators_between_n_targets`.
    #[test]
    fn not_in_ands_exactly_n_minus_one_separators_between_n_targets() {
        let f = filter(
            "status",
            FilterOp::NotIn,
            FieldValue::Array(vec![
                FieldValue::String("a".to_string()),
                FieldValue::String("b".to_string()),
                FieldValue::String("c".to_string()),
            ]),
        );
        let sql = generated_sql(&f);
        assert_eq!(sql.matches(" != ").count(), 3, "expected 3 inequality checks, got: {sql}");
        assert_eq!(sql.matches(" AND ").count(), 2, "expected exactly 2 separators for 3 targets, got: {sql}");
    }

    #[test]
    fn not_in_with_an_empty_target_list_generates_a_field_exists_check() {
        let f = filter("status", FilterOp::NotIn, FieldValue::Array(vec![]));
        assert_eq!(generated_sql(&f).trim(), "fields->'status' IS NOT NULL");
    }

    /// AC-QFO-09/AC-QFO-10 (SQL-shape half): `NotIn` compares the WHOLE
    /// `fields->'{field}'` value (never `->>'v'`), which is exactly the
    /// property that lets a `Null`-valued field (a real JSONB value,
    /// `{"t":"N"}`) be distinguished from an absent one (`fields->'{field}'`
    /// itself SQL `NULL`) — see `push_value_equality`'s own doc comment for
    /// the full reasoning. The real end-to-end proof (both cases actually
    /// behaving differently against a live Postgres) is this feature's own
    /// `qfo04_not_in.rs` acceptance test.
    #[test]
    fn not_in_compares_the_whole_field_value_never_the_unwrapped_v() {
        let f = filter(
            "status",
            FilterOp::NotIn,
            FieldValue::Array(vec![FieldValue::String("closed".to_string())]),
        );
        let sql = generated_sql(&f);
        assert!(sql.contains("fields->'status' != "), "got: {sql}");
        assert!(!sql.contains("->>'v'"), "must not extract the unwrapped value, got: {sql}");
    }

    /// Regression guard (AC-QFO-07): `Equal`'s own generated SQL is
    /// unchanged after the `push_scalar_comparison` extraction.
    #[test]
    fn equal_compares_the_whole_field_value_after_firestore_equal_notequal_value_type_support() {
        // Superseded scenario (firestore-equal-notequal-value-type-support,
        // Slice 01): before this feature, `Equal` used `push_scalar_
        // comparison`'s own `->>'v'`-extraction shape
        // (`fields->'category'->>'v' = $1`); it now uses `push_value_
        // equality`'s own whole-object shape, matching `In`/`NotIn`'s own
        // pattern (and, unlike the old shape, working uniformly for every
        // `FieldValue` type, not just the 4 scalar ones).
        let f = filter("category", FilterOp::Equal, FieldValue::String("B".to_string()));
        let sql = generated_sql(&f);
        assert_eq!(sql.trim(), "fields->'category' = $1::jsonb");
    }

    /// AC-ENV-06 (regression guard, type-matched case): a type-matched
    /// `NotEqual` still correctly excludes the matching value.
    #[test]
    fn not_equal_compares_the_whole_field_value() {
        let f = filter("category", FilterOp::NotEqual, FieldValue::String("B".to_string()));
        let sql = generated_sql(&f);
        assert_eq!(sql.trim(), "fields->'category' != $1::jsonb");
    }

    /// AC-ENV-01/02/03/04: `Equal` no longer panics on `Timestamp`/`Bytes`/
    /// `Reference`/`Array`/`Map` — the exact 5 types that crashed before
    /// this feature.
    #[test]
    fn equal_does_not_panic_on_previously_crashing_value_types() {
        let cases = vec![
            FieldValue::Timestamp(1_700_000_000, 0),
            FieldValue::Bytes(vec![1, 2, 3]),
            FieldValue::Reference("projects/p/databases/(default)/documents/c/d".to_string()),
            FieldValue::Array(vec![FieldValue::String("a".to_string())]),
            FieldValue::Map(std::collections::BTreeMap::from([(
                "k".to_string(),
                FieldValue::String("v".to_string()),
            )])),
        ];
        for value in cases {
            let f = filter("field", FilterOp::Equal, value.clone());
            let sql = generated_sql(&f);
            assert_eq!(sql.trim(), "fields->'field' = $1::jsonb", "failed for {value:?}");
        }
    }

    /// AC-ENV-07 (documented behavior tightening, Resolution 2): a
    /// cross-numeric-type target no longer relies on the old permissive
    /// `->>'v'`-text-cast coercion — the generated SQL now compares whole
    /// JSON values, so an `Integer(5)` target and a `Double(5.0)` target
    /// produce DIFFERENT bound JSON (proving they'd no longer accidentally
    /// match a cross-typed stored value the way the old shape could).
    #[test]
    fn equal_on_cross_numeric_types_binds_distinct_json_values() {
        let int_target = field_value_to_json(&FieldValue::Integer(5));
        let double_target = field_value_to_json(&FieldValue::Double(5.0));
        assert_ne!(
            int_target, double_target,
            "an Integer(5) and a Double(5.0) target must bind as distinct JSON values"
        );
    }

    #[test]
    #[should_panic(expected = "unsupported filter value type for In")]
    fn in_with_a_non_array_value_panics_with_a_named_message() {
        let f = filter("status", FilterOp::In, FieldValue::String("not-an-array".to_string()));
        generated_sql(&f);
    }

    /// ADR-080 Decision C — `is_probe_stale` TTL boundary: not-yet-elapsed
    /// is fresh (not stale), already-elapsed is stale. Two behaviors
    /// (fresh, stale), parametrized per this feature's own test-budget
    /// discipline rather than two separate near-duplicate test functions.
    #[test]
    fn is_probe_stale_reflects_whether_the_ttl_has_elapsed() {
        use std::thread::sleep;
        use std::time::Duration;

        let checked_at = std::time::Instant::now();
        assert!(
            !is_probe_stale(checked_at, Duration::from_secs(30)),
            "a freshly-checked timestamp under a long TTL must not be stale"
        );

        sleep(Duration::from_millis(20));
        assert!(
            is_probe_stale(checked_at, Duration::from_millis(10)),
            "an elapsed-past-TTL timestamp must be stale"
        );
    }

    /// Coverage for `push_scalar_comparison`'s own 4 typed-cast match arms
    /// (`Integer`/`String`/`Double`/`Boolean`) and all 6 pre-existing
    /// comparison operators it serves — this file's own PRE-EXISTING
    /// behavior (unchanged by this feature's own refactor, AC-QFO-07's own
    /// regression-guard scope), but previously untested by ANY unit test in
    /// this module (only `Equal`+`String` had direct coverage before this
    /// feature's own QUALITY_GATE) — a gap this feature's own diff exposed
    /// (the refactor moved these lines into scope for `--in-diff`) rather
    /// than one this feature introduced.
    #[test]
    fn push_scalar_comparison_covers_every_operator_and_value_type() {
        // firestore-equal-notequal-value-type-support (Slice 01): `NotEqual`
        // no longer routes through `push_scalar_comparison` (it uses
        // `push_value_equality` now, covered separately by `not_equal_
        // compares_the_whole_field_value` above) — this test now covers
        // only the 4 range operators `push_scalar_comparison` still serves.
        let cases: Vec<(FilterOp, &str, FieldValue, &str)> = vec![
            (FilterOp::LessThan, "<", FieldValue::Integer(5), "::bigint"),
            (FilterOp::LessThanOrEqual, "<=", FieldValue::Double(5.5), "::float8"),
            (FilterOp::GreaterThan, ">", FieldValue::Boolean(true), "::boolean"),
            (FilterOp::GreaterThanOrEqual, ">=", FieldValue::Integer(5), "::bigint"),
        ];
        for (op, op_str, value, cast) in cases {
            let f = filter("n", op, value);
            let sql = generated_sql(&f);
            assert!(
                sql.contains(&format!(" {op_str} ")),
                "expected operator '{op_str}' in generated SQL, got: {sql}"
            );
            assert!(sql.contains(cast), "expected cast '{cast}' in generated SQL, got: {sql}");
        }
    }

    /// AC-RNG-01: `Timestamp` uses a `ROW(...)` comparison over its own
    /// `(seconds, nanos)` fields, never `->>'v'` extraction (Timestamp has
    /// no `"v"` key).
    #[test]
    fn greater_than_on_timestamp_uses_row_comparison() {
        let f = filter("createdAt", FilterOp::GreaterThan, FieldValue::Timestamp(1_700_000_000, 500));
        let sql = generated_sql(&f);
        assert!(sql.contains("ROW((fields->'createdAt'->'s')::bigint"), "got: {sql}");
        assert!(sql.contains("(fields->'createdAt'->'n')::int)"), "got: {sql}");
        assert!(sql.contains(" > ROW("), "got: {sql}");
    }

    /// AC-RNG-02: `Bytes` decodes to raw `bytea` for comparison, never
    /// compares the base64 TEXT directly — proven with a byte pair whose
    /// base64 text ordering DISAGREES with true byte-value ordering.
    /// `\xFF` (255) has a HIGHER byte value than `\x00` (0), but base64
    /// -encodes to `"/w=="` vs `"AA=="` — `'/'` (0x2F) sorts BEFORE `'A'`
    /// (0x41) as TEXT, which would be backwards if compared as base64
    /// strings directly.
    #[test]
    fn less_than_on_bytes_decodes_to_bytea_not_base64_text() {
        let f = filter("payload", FilterOp::LessThan, FieldValue::Bytes(vec![0xFF]));
        let sql = generated_sql(&f);
        assert!(sql.contains("decode(fields->'payload'->>'v', 'base64')"), "got: {sql}");
        assert!(sql.contains(" < decode("), "got: {sql}");
        assert!(sql.contains(", 'base64')"), "got: {sql}");
        assert!(!sql.contains("->>'v' <"), "must not compare the base64 TEXT directly, got: {sql}");
    }

    /// AC-RNG-03: `Reference` uses flat string comparison (same shape as
    /// `String`), matching real Firestore's own segment-wise ordering for
    /// the evidenced same-depth case.
    #[test]
    fn greater_than_or_equal_on_reference_uses_string_comparison() {
        let f = filter(
            "ownerRef",
            FilterOp::GreaterThanOrEqual,
            FieldValue::Reference("projects/p/databases/(default)/documents/users/alice".to_string()),
        );
        let sql = generated_sql(&f);
        assert_eq!(sql.trim(), "fields->'ownerRef'->>'v' >= $1");
    }

    /// Regression guard: `Array`/`Map` still panic if they somehow reach
    /// `push_scalar_comparison` directly (the real fix — rejecting them
    /// upstream in `translate_filter` — is proven separately in
    /// `embyr-server`'s own acceptance tests; this proves the defensive
    /// fallback itself still exists and is exercised).
    #[test]
    #[should_panic(expected = "unsupported filter value type")]
    fn array_still_panics_if_it_somehow_reaches_push_scalar_comparison_directly() {
        let f = filter("tags", FilterOp::GreaterThan, FieldValue::Array(vec![]));
        generated_sql(&f);
    }

    // field-path-defense-in-depth (finding #26): `append_field_filter`,
    // `order_by_expr`, `order_by_expr_bigint` are 3 of the 7 SQL-building
    // call sites that must now independently reject an unvalidated field
    // path, rather than trusting every upstream caller validated first.
    // Calling these functions DIRECTLY with a malicious path — bypassing
    // the real upstream gate entirely — is the actual defense being tested.

    /// Defense-in-depth: a caller that skips upstream `validate_field_path`
    /// and calls `append_field_filter` directly with an injection-shaped
    /// field path must panic, never silently build interpolable SQL text.
    #[test]
    #[should_panic(expected = "invalid field path")]
    fn append_field_filter_panics_on_unvalidated_malicious_field_path() {
        let f = filter(
            "x'); DROP TABLE documents; --",
            FilterOp::Equal,
            FieldValue::String("v".to_string()),
        );
        generated_sql(&f);
    }

    /// Regression guard: a legitimate field path is unaffected by the new
    /// guard — `order_by_expr` still produces the same ORDER BY fragment.
    #[test]
    fn order_by_expr_still_works_for_a_legitimate_field_path() {
        let ob = OrderBy { field_path: "created_at".to_string(), direction: OrderDirection::Descending };
        assert_eq!(order_by_expr(&ob), "fields->'created_at'->>'v' DESC");
    }

    /// Defense-in-depth: `order_by_expr` called directly with a malicious
    /// field path (bypassing upstream validation) must panic, not build SQL.
    #[test]
    #[should_panic(expected = "invalid field path")]
    fn order_by_expr_panics_on_unvalidated_malicious_field_path() {
        let ob = OrderBy {
            field_path: "x'); DROP TABLE documents; --".to_string(),
            direction: OrderDirection::Ascending,
        };
        order_by_expr(&ob);
    }

    /// Regression guard: a legitimate field path is unaffected by the new
    /// guard — `order_by_expr_bigint` still produces the same fragment.
    #[test]
    fn order_by_expr_bigint_still_works_for_a_legitimate_field_path() {
        assert_eq!(order_by_expr_bigint("score", "ASC"), "(fields->'score'->>'v')::bigint ASC");
    }

    /// Defense-in-depth: `order_by_expr_bigint` (dead code today, but still
    /// `pub fn` and part of the exported SQL-building surface) called
    /// directly with a malicious field path must panic, not build SQL.
    #[test]
    #[should_panic(expected = "invalid field path")]
    fn order_by_expr_bigint_panics_on_unvalidated_malicious_field_path() {
        order_by_expr_bigint("x'); DROP TABLE documents; --", "ASC");
    }
}
