use base64::{engine::general_purpose::STANDARD, Engine};
use embyr_core::domain::{
    field_value::FieldValue,
    query::{
        validate_field_path, Aggregation, AggregationKind, FilterOp, FieldFilter, OrderBy, OrderDirection,
        QueryFilter,
    },
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
/// must NOT use it — see `push_value_equality`'s own doc). `Array`/`Map`
/// still hit this function's own `_ => panic!(...)` fallback — real
/// Firestore has no meaningful ordering for a composite type, so both are
/// rejected upstream in `handler.rs::translate_filter` before reaching here
/// (candidate id `firestore-range-operator-value-type-support`).
///
/// (finding #44, timestamp-cursor-pagination) `pub(crate)` — reused
/// verbatim by `embyr_pg_storage::backend_adapter`'s startAt/startAfter and
/// endAt/endBefore cursor-bound blocks. A cursor bound is a strict `<`/
/// `<=`/`>`/`>=` comparison, the exact same shape a range-operator filter
/// needs, so the SAME type-aware casting this function already established
/// for `Integer`/`String`/`Double`/`Boolean`/`Timestamp`/`Bytes`/`Reference`
/// closes the cursor-side gap too instead of re-deriving it.
pub(crate) fn push_scalar_comparison(
    qb: &mut QueryBuilder<Postgres>,
    field_path: &str,
    op: &str,
    value: &FieldValue,
) {
    // typed-order-by (P11): every arm is guarded by the stored type tag, so a
    // range filter / cursor only matches values of the bound value's own
    // Firestore type class (other types evaluate to NULL and are excluded)
    // and a mixed-type collection can no longer hit a cast error. Same type
    // classes and casts as `order_by_expr`, so both share one total order.
    let t = format!("fields->'{field_path}'->>'t'");
    let v = format!("fields->'{field_path}'->>'v'");
    match value {
        // Integer and Double are one numeric class, compared by exact numeric
        // value (G-001): the two-part key `(float8, residual)` of
        // `numeric_key_exprs`, never a lossy float8 alone.
        FieldValue::Integer(_) | FieldValue::Double(_) => {
            let (f, r) = numeric_sort_key(value).expect("numeric value has a sort key");
            let (primary, residual) = numeric_key_exprs(field_path);
            qb.push(format!("ROW({primary}, {residual}) {op} ROW("));
            qb.push_bind(f);
            qb.push(", ");
            qb.push_bind(r);
            qb.push("::numeric)");
        }
        // `COLLATE "C"`: Firestore compares strings by UTF-8 byte order,
        // which C collation gives on a UTF8 database (ORDER BY matches).
        FieldValue::String(s) => {
            qb.push(format!("CASE WHEN {t} = 'S' THEN {v} COLLATE \"C\" END {op} "));
            qb.push_bind(s.clone());
        }
        FieldValue::Boolean(b) => {
            qb.push(format!("CASE WHEN {t} = 'B' THEN ({v})::boolean END {op} "));
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
                "ROW(CASE WHEN {t} = 'TS' THEN (fields->'{field_path}'->'s')::bigint END, \
                 CASE WHEN {t} = 'TS' THEN (fields->'{field_path}'->'n')::int END) {op} ROW("
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
            qb.push(format!("CASE WHEN {t} = 'BY' THEN decode({v}, 'base64') END {op} decode("));
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
            qb.push(format!("CASE WHEN {t} = 'R' THEN {v} COLLATE \"C\" END {op} "));
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
    qb.push(format!("{} {op} ", equality_key_expr(field_path)));
    qb.push_bind(field_value_to_json(target));
    qb.push("::jsonb");
}

/// Key expression `push_value_equality` compares for an equality filter on
/// `field_path` (the whole stored `{"t":..,"v":..}` jsonb value). Shared with
/// the composite-index DDL so an index's equality columns are the exact
/// expression queries filter on.
pub fn equality_key_expr(field_path: &str) -> String {
    if let Err(e) = validate_field_path(field_path) {
        panic!("invalid field path reached SQL builder: {e}");
    }
    format!("fields->'{field_path}'")
}

/// Direction-less key expressions of the typed ORDER BY for `field_path`
/// (typed-order-by, P11): (1) type rank per Firestore's cross-type order —
/// null < boolean < NaN < numbers (integer + double together) < timestamp <
/// string < bytes < reference < array < map; (2) one typed value column per
/// type family, each `NULL` outside its own type so only the column of the
/// rank group's own type discriminates. `__name__` orders by document id
/// (byte order) — it is not a stored field.
///
/// The numbers (integer + double together) take TWO keys, see
/// `numeric_key_exprs` (exact int64 vs double ordering, G-001).
///
/// Single source of truth: `order_by_expr` appends the direction to each key
/// for queries, and the composite-index DDL wraps each key as an index
/// expression, so index and ORDER BY can never drift apart. Every function
/// and cast used is IMMUTABLE (jsonb operators, `float8`/`numeric`/`boolean`
/// text input, float8->bigint->numeric, jsonb->bigint/int, `decode`,
/// `COLLATE "C"`), as index expressions require. Changing any key here
/// changes the index expressions: indexes built on an older expression must
/// be dropped and re-created (docs/operations/runbook.md).
pub fn order_key_exprs(field_path: &str) -> Vec<String> {
    // field-path-defense-in-depth (finding #26): see `append_field_filter`'s
    // own identical guard comment for the full rationale.
    if let Err(e) = validate_field_path(field_path) {
        panic!("invalid field path reached SQL builder: {e}");
    }
    if field_path == "__name__" {
        // partition-query: the full document name, ordered by
        // `(collection_path, document_id)` bytewise. For a single collection
        // `collection_path` is constant, so this is plain document-id order;
        // for a collection group it makes the order total, which
        // `push_name_comparison` (cursors) relies on.
        return vec![
            "collection_path COLLATE \"C\"".to_string(),
            "document_id COLLATE \"C\"".to_string(),
        ];
    }
    let fp = field_path;
    let t = format!("fields->'{fp}'->>'t'");
    let v = format!("fields->'{fp}'->>'v'");
    let (num_primary, num_residual) = numeric_key_exprs(fp);
    vec![
        format!(
            "CASE {t} WHEN 'N' THEN 0 WHEN 'B' THEN 1 \
             WHEN 'D' THEN CASE WHEN {v} = 'NaN' THEN 2 ELSE 3 END WHEN 'I' THEN 3 \
             WHEN 'TS' THEN 4 WHEN 'S' THEN 5 WHEN 'BY' THEN 6 WHEN 'R' THEN 7 \
             WHEN 'A' THEN 8 WHEN 'M' THEN 9 END"
        ),
        format!("CASE WHEN {t} = 'B' THEN ({v})::boolean END"),
        num_primary,
        num_residual,
        format!("CASE WHEN {t} = 'TS' THEN (fields->'{fp}'->'s')::bigint END"),
        format!("CASE WHEN {t} = 'TS' THEN (fields->'{fp}'->'n')::int END"),
        format!("CASE WHEN {t} IN ('S','R') THEN {v} COLLATE \"C\" END"),
        format!("CASE WHEN {t} = 'BY' THEN decode({v}, 'base64') END"),
        format!("CASE WHEN {t} IN ('A','M') THEN fields->'{fp}'->'v' END"),
    ]
}

/// Two-part sort key of the numeric class (integer + double), as SQL
/// expressions `(primary, residual)`; both are NULL outside the class.
///
/// * `primary`: the value as `float8` (a double exactly; an integer rounded to
///   the nearest double, as `i64 as f64`). NaN is ranked below every number by
///   the type-rank key, `-0` and `0` are equal, +-Infinity are the extremes.
/// * `residual`: 0 for a double; for an integer `i - primary` (exact, `numeric`,
///   |residual| <= 1024). Integers that round to the same double above 2^53 are
///   thus ordered by their own int64 value, and sit on the correct side of that
///   double. `primary` >= 2^63 (i64::MAX rounds there) cannot be cast to
///   `bigint`, so 2^63 is substituted.
///
/// Lexicographic `(primary, residual)` order is the exact numeric order: rounding
/// to the nearest double is monotone and a double is its own rounding. All
/// functions are IMMUTABLE, so the keys are valid index expressions.
pub(crate) fn numeric_key_exprs(fp: &str) -> (String, String) {
    let t = format!("fields->'{fp}'->>'t'");
    let v = format!("fields->'{fp}'->>'v'");
    let f = format!("({v})::float8");
    (
        format!("CASE WHEN {t} IN ('I','D') THEN {f} END"),
        format!(
            "CASE WHEN {t} = 'I' THEN ({v})::numeric - \
             CASE WHEN {f} >= 9223372036854775808::float8 THEN 9223372036854775808::numeric \
             ELSE ({f})::bigint::numeric END \
             WHEN {t} = 'D' THEN 0::numeric END"
        ),
    )
}

/// Rust mirror of `numeric_key_exprs` for a bound value: `(primary, residual)`
/// of an integer or double, `None` for any other type.
pub fn numeric_sort_key(v: &FieldValue) -> Option<(f64, i64)> {
    match v {
        FieldValue::Integer(i) => {
            let f = *i as f64;
            Some((f, (i128::from(*i) - f as i128) as i64))
        }
        FieldValue::Double(d) => Some((*d, 0)),
        _ => None,
    }
}

/// The query's ORDER BY with Firestore's implicit `__name__` appended, in the
/// direction of the last explicit order (ascending when there is none) —
/// unless the last explicit order already is `__name__`. Makes every ordering
/// total, so limits, offsets and cursors page deterministically.
pub fn effective_order_by(order_by: &[OrderBy]) -> Vec<OrderBy> {
    let mut out = order_by.to_vec();
    if order_by.last().is_none_or(|o| o.field_path != "__name__") {
        let direction = order_by.last().map_or(OrderDirection::Ascending, |o| o.direction.clone());
        out.push(OrderBy { field_path: "__name__".to_string(), direction });
    }
    out
}

/// Full `ORDER BY` list (without the keyword) for a query: every clause of
/// `effective_order_by`.
pub fn order_by_clause(order_by: &[OrderBy]) -> String {
    effective_order_by(order_by).iter().map(order_by_expr).collect::<Vec<_>>().join(", ")
}

/// One cursor comparison `<key of field> op <value>`: `__name__` compares the
/// document name, every other field its typed value.
fn push_cursor_cmp(
    qb: &mut QueryBuilder<Postgres>,
    field_path: &str,
    op: &str,
    value: &FieldValue,
) -> Result<(), String> {
    if field_path == "__name__" {
        return match value {
            FieldValue::Reference(r) => push_name_comparison(qb, op, r),
            _ => None,
        }
        .map(|_| ())
        .ok_or_else(|| "a __name__ cursor value must be a document reference".to_string());
    }
    push_scalar_comparison(qb, field_path, op, value);
    Ok(())
}

/// Cursor bound over the effective order list `orders` (explicit orders plus
/// the implicit `__name__`, see `effective_order_by`): `values[k]` positions
/// `orders[k]`, and fewer values than orders is a prefix. Lexicographic
/// OR-of-AND expansion honouring each order's own direction.
///
/// `is_end`: `endAt`/`endBefore` (docs before the cursor) vs `startAt`/
/// `startAfter` (docs after it). `before`: the cursor's own flag. The bound is
/// inclusive for `startAt` and `endAt`; a prefix cursor `startAfter(v)` skips
/// every document whose leading values equal `v`.
pub fn push_cursor_predicate(
    qb: &mut QueryBuilder<Postgres>,
    orders: &[OrderBy],
    values: &[FieldValue],
    is_end: bool,
    before: bool,
) -> Result<(), String> {
    if values.len() > orders.len() {
        return Err(format!(
            "cursor has {} values but the query orders by only {} fields",
            values.len(),
            orders.len()
        ));
    }
    if values.is_empty() {
        qb.push("TRUE");
        return Ok(());
    }
    let inclusive = before != is_end;
    let strict = |dir: &OrderDirection| match (is_end, dir) {
        (false, OrderDirection::Ascending) | (true, OrderDirection::Descending) => ">",
        _ => "<",
    };
    let last = values.len() - 1;
    qb.push("(");
    for i in 0..values.len() {
        if i > 0 {
            qb.push(" OR ");
        }
        if values.len() > 1 {
            qb.push("(");
        }
        for k in 0..i {
            push_cursor_cmp(qb, &orders[k].field_path, "=", &values[k])?;
            qb.push(" AND ");
        }
        let op = match (i == last && inclusive, strict(&orders[i].direction)) {
            (true, ">") => ">=",
            (true, _) => "<=",
            (false, s) => s,
        };
        push_cursor_cmp(qb, &orders[i].field_path, op, &values[i])?;
        if values.len() > 1 {
            qb.push(")");
        }
    }
    qb.push(")");
    Ok(())
}

/// Split a Firestore reference value (`projects/p/databases/d/documents/c/id`)
/// into the stored `(collection_path, document_id)` pair; a name without the
/// `/databases/<db>/documents/` root is taken as already relative. `None` if
/// there is no collection part.
pub fn split_document_reference(reference: &str) -> Option<(&str, &str)> {
    let rel = reference
        .split_once("/databases/")
        .and_then(|(_, t)| t.split_once("/documents/"))
        .map_or(reference, |(_, p)| p);
    rel.rsplit_once('/').filter(|(c, id)| !c.is_empty() && !id.is_empty())
}

/// Cursor bound on `__name__`: compares `(collection_path, document_id)`
/// bytewise, the exact order `order_key_exprs("__name__")` sorts by.
/// `None` when `reference` is not a document name.
pub fn push_name_comparison(
    qb: &mut QueryBuilder<Postgres>,
    op: &str,
    reference: &str,
) -> Option<()> {
    let (collection_path, document_id) = split_document_reference(reference)?;
    qb.push(format!(
        "ROW(collection_path COLLATE \"C\", document_id COLLATE \"C\") {op} ROW("
    ));
    qb.push_bind(collection_path.to_string());
    qb.push(", ");
    qb.push_bind(document_id.to_string());
    qb.push(")");
    Some(())
}

/// Return a raw SQL ORDER BY expression for a single `OrderBy` clause: every
/// `order_key_exprs` key carrying the clause's direction.
///
/// This string is pushed as raw SQL (no bind parameter), since ORDER BY
/// expressions cannot be parameterised in Postgres. Same type classes/casts
/// as `push_scalar_comparison`, so ORDER BY, cursors and range filters share
/// one total order.
///
/// ponytail: array/map fall back to jsonb's own stable ordering, not
/// Firestore's element-wise rule.
pub fn order_by_expr(ob: &OrderBy) -> String {
    let dir = match ob.direction {
        OrderDirection::Ascending => "ASC",
        OrderDirection::Descending => "DESC",
    };
    order_key_exprs(&ob.field_path)
        .into_iter()
        .map(|k| format!("{k} {dir}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// WHERE predicate for an `OrderBy` clause: Firestore excludes documents that
/// lack the ordered field. `None` for `__name__` (always present).
pub fn order_by_presence_predicate(ob: &OrderBy) -> Option<String> {
    if let Err(e) = validate_field_path(&ob.field_path) {
        panic!("invalid field path reached SQL builder: {e}");
    }
    (ob.field_path != "__name__").then(|| format!("fields->'{}' IS NOT NULL", ob.field_path))
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

/// SELECT list computing every aggregation in ONE statement, one column per
/// entry (same order): `Count` -> `COUNT(*)` (capped via `LEAST` for
/// `up_to`), `Sum` -> `COALESCE(SUM(..),0)::float8`, `Avg` -> `AVG(..)`
/// (SQL NULL over no numeric inputs). Only I/D-tagged values are aggregated;
/// non-numeric values are ignored.
///
/// ponytail: `up_to` caps the result, not the scan; add a LIMIT sub-select
/// for the sole-COUNT case if scan cost matters.
///
/// Field paths are interpolated into SQL text, so (finding #26 guard, same
/// as `append_field_filter`) an unvalidated path panics here.
pub fn aggregation_select_list(aggregations: &[Aggregation]) -> String {
    let numeric = |fp: &str| {
        if let Err(e) = validate_field_path(fp) {
            panic!("invalid field path reached SQL builder: {e}");
        }
        format!(
            "CASE WHEN fields->'{fp}'->>'t' IN ('I','D') \
             THEN (fields->'{fp}'->>'v')::float8 ELSE NULL END"
        )
    };
    aggregations
        .iter()
        .map(|a| match &a.kind {
            AggregationKind::Count(None) => "COUNT(*)".to_string(),
            AggregationKind::Count(Some(n)) => format!("LEAST(COUNT(*), {n}::bigint)"),
            AggregationKind::Sum(fp) => format!("COALESCE(SUM({}), 0)::float8", numeric(fp)),
            AggregationKind::Avg(fp) => format!("AVG({})", numeric(fp)),
        })
        .collect::<Vec<_>>()
        .join(", ")
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
            // typed-order-by (P11): Integer now shares Double's float8 numeric
            // class (cross-type numeric comparison), so its cast is float8.
            (FilterOp::LessThan, "<", FieldValue::Integer(5), "::float8"),
            (FilterOp::LessThanOrEqual, "<=", FieldValue::Double(5.5), "::float8"),
            (FilterOp::GreaterThan, ">", FieldValue::Boolean(true), "::boolean"),
            (FilterOp::GreaterThanOrEqual, ">=", FieldValue::Integer(5), "::float8"),
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
        // typed-order-by (P11): each ROW element is now type-tag guarded.
        assert!(sql.contains("THEN (fields->'createdAt'->'s')::bigint END"), "got: {sql}");
        assert!(sql.contains("THEN (fields->'createdAt'->'n')::int END)"), "got: {sql}");
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
        assert!(sql.contains("THEN decode(fields->'payload'->>'v', 'base64') END"), "got: {sql}");
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
        // typed-order-by (P11): type-tag guarded, byte-order (C) collation.
        assert_eq!(
            sql.trim(),
            "CASE WHEN fields->'ownerRef'->>'t' = 'R' THEN fields->'ownerRef'->>'v' COLLATE \"C\" END >= $1"
        );
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

    fn ob(field: &str, direction: OrderDirection) -> OrderBy {
        OrderBy { field_path: field.to_string(), direction }
    }

    /// Regression guard: a legitimate field path still builds. Updated for
    /// typed-order-by (P11): the old exact string `fields->'f'->>'v' DESC`
    /// was the text-sort bug itself (5 sorted above 30), so the assertion now
    /// pins the new shape — type-rank key first, typed value keys after,
    /// EVERY key carrying the clause direction, and no raw text-`v` sort key.
    #[test]
    fn order_by_expr_still_works_for_a_legitimate_field_path() {
        let sql = order_by_expr(&ob("created_at", OrderDirection::Descending));
        assert!(sql.starts_with("CASE fields->'created_at'->>'t' WHEN 'N' THEN 0"), "got: {sql}");
        assert!(sql.contains("(fields->'created_at'->>'v')::float8"), "got: {sql}");
        assert_eq!(sql.matches(" DESC").count(), sql.split(", CASE").count(), "every key must be DESC: {sql}");
        assert!(!sql.contains("ASC"), "got: {sql}");
        assert!(!sql.starts_with("fields->"), "must not sort by raw text v: {sql}");
    }

    #[test]
    fn order_by_name_sorts_by_document_id_and_is_never_excluded() {
        let o = ob("__name__", OrderDirection::Ascending);
        assert_eq!(
            order_by_expr(&o),
            "collection_path COLLATE \"C\" ASC, document_id COLLATE \"C\" ASC"
        );
        assert_eq!(order_by_presence_predicate(&o), None);
    }

    #[test]
    fn split_document_reference_handles_full_and_relative_names() {
        for (r, want) in [
            ("projects/p/databases/(default)/documents/c/id", Some(("c", "id"))),
            ("projects/p/databases/(default)/documents/users/u1/posts/p9", Some(("users/u1/posts", "p9"))),
            // A project literally named like a path marker must not confuse the root strip.
            ("projects/documents/databases/(default)/documents/c/id", Some(("c", "id"))),
            ("projects/databases/databases/(default)/documents/c/id", Some(("c", "id"))),
            ("c/id", Some(("c", "id"))),
            ("id", None),
            ("projects/p/databases/(default)/documents/c/", None),
        ] {
            assert_eq!(split_document_reference(r), want, "{r}");
        }
    }

    #[test]
    fn order_by_excludes_documents_lacking_the_field() {
        let p = order_by_presence_predicate(&ob("score", OrderDirection::Ascending));
        assert_eq!(p.as_deref(), Some("fields->'score' IS NOT NULL"));
    }

    // Reference oracle: Firestore's cross-type order, written independently
    // of the SQL builder. Tags are the type-order positions.
    fn oracle_type_position(v: &FieldValue) -> u8 {
        match v {
            FieldValue::Null => 0,
            FieldValue::Boolean(_) => 1,
            FieldValue::Double(d) if d.is_nan() => 2,
            FieldValue::Integer(_) | FieldValue::Double(_) => 3,
            FieldValue::Timestamp(..) => 4,
            FieldValue::String(_) => 5,
            FieldValue::Bytes(_) => 6,
            FieldValue::Reference(_) => 7,
            FieldValue::Array(_) => 8,
            FieldValue::Map(_) => 9,
        }
    }

    /// Reads the rank `order_by_expr`'s SQL assigns to `v`'s stored type tag.
    fn sql_rank(sql: &str, v: &FieldValue) -> u8 {
        let digit_after = |pat: &str| -> u8 {
            let i = sql.find(pat).unwrap_or_else(|| panic!("`{pat}` missing in: {sql}"));
            sql[i + pat.len()..].chars().next().unwrap().to_digit(10).unwrap() as u8
        };
        let tag = field_value_to_json(v)["t"].as_str().unwrap().to_string();
        match v {
            FieldValue::Double(d) if d.is_nan() => digit_after("= 'NaN' THEN "),
            FieldValue::Double(_) => digit_after(" ELSE "),
            _ => digit_after(&format!("WHEN '{tag}' THEN ")),
        }
    }

    /// Property over all ordered pairs of a mixed-type sample: the SQL's rank
    /// key orders two values exactly as the Firestore oracle orders their
    /// types, and ranks tie exactly when the oracle puts them in one class
    /// (integer and double compare numerically, so they MUST tie on rank and
    /// be separated by the float8 value key). Value ordering inside a class
    /// executes in Postgres and is covered by the Docker acceptance test.
    #[test]
    fn sql_type_rank_agrees_with_firestore_cross_type_oracle() {
        use std::collections::BTreeMap;
        let sample = [
            FieldValue::Null,
            FieldValue::Boolean(false),
            FieldValue::Boolean(true),
            FieldValue::Double(f64::NAN),
            FieldValue::Integer(-1),
            FieldValue::Integer(30),
            FieldValue::Double(7.5),
            FieldValue::Timestamp(10, 0),
            FieldValue::String("a".into()),
            FieldValue::Bytes(vec![1]),
            FieldValue::Reference("projects/p/databases/d/documents/c/x".into()),
            FieldValue::Array(vec![FieldValue::Integer(1)]),
            FieldValue::Map(BTreeMap::new()),
        ];
        let sql = order_by_expr(&ob("f", OrderDirection::Ascending));
        for a in &sample {
            for b in &sample {
                assert_eq!(
                    sql_rank(&sql, a).cmp(&sql_rank(&sql, b)),
                    oracle_type_position(a).cmp(&oracle_type_position(b)),
                    "rank order of {a:?} vs {b:?} in: {sql}"
                );
            }
        }
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

    fn agg(alias: &str, kind: AggregationKind) -> Aggregation {
        Aggregation { alias: alias.to_string(), kind }
    }

    /// Several aggregations share one SELECT list: one column each, in
    /// request order; `up_to` caps only its own COUNT; SUM is COALESCEd to 0
    /// while AVG is not (NULL over an empty set); numeric ops only read I/D.
    #[test]
    fn aggregation_select_list_has_one_ordered_column_per_aggregation() {
        let sql = aggregation_select_list(&[
            agg("a", AggregationKind::Avg("score".into())),
            agg("b", AggregationKind::Count(None)),
            agg("c", AggregationKind::Count(Some(5))),
            agg("d", AggregationKind::Sum("score".into())),
        ]);
        let pos = |needle: &str| sql.find(needle).unwrap_or_else(|| panic!("{needle} missing in: {sql}"));
        let (avg, count, capped, sum) = (
            pos("AVG(CASE WHEN fields->'score'->>'t' IN ('I','D')"),
            pos(", COUNT(*), "),
            pos("LEAST(COUNT(*), 5::bigint)"),
            pos("COALESCE(SUM(CASE WHEN"),
        );
        assert!(avg < count && count < capped && capped < sum, "order wrong: {sql}");
        assert!(!sql[..count].contains("COALESCE"), "AVG must stay NULL over no inputs: {sql}");
    }

    #[test]
    #[should_panic(expected = "invalid field path")]
    fn aggregation_select_list_panics_on_unvalidated_malicious_field_path() {
        aggregation_select_list(&[agg("x", AggregationKind::Sum("x'); DROP TABLE documents; --".into()))]);
    }

    // ─── G-001: exact numeric order + implicit `__name__` tiebreak ─────────

    /// Exact comparison of two numbers (integer or double, never NaN),
    /// written independently of the key: integers are widened to i128, a
    /// double is split into its integer part and fraction.
    fn exact_cmp(a: &FieldValue, b: &FieldValue) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        // Value as (integer part, fraction sign) with infinities clamped far out.
        fn parts(v: &FieldValue) -> (i128, i8) {
            match v {
                FieldValue::Integer(i) => (i128::from(*i), 0),
                FieldValue::Double(d) if *d == f64::INFINITY => (i128::MAX, 0),
                FieldValue::Double(d) if *d == f64::NEG_INFINITY => (i128::MIN, 0),
                FieldValue::Double(d) => {
                    let fl = d.floor();
                    (fl as i128, if *d > fl { 1 } else { 0 })
                }
                _ => unreachable!(),
            }
        }
        let (ai, af) = parts(a);
        let (bi, bf) = parts(b);
        match ai.cmp(&bi) {
            Equal => af.cmp(&bf),
            o => o,
        }
    }

    fn key_cmp(a: &FieldValue, b: &FieldValue) -> std::cmp::Ordering {
        let (af, ar) = numeric_sort_key(a).unwrap();
        let (bf, br) = numeric_sort_key(b).unwrap();
        af.partial_cmp(&bf).unwrap().then(ar.cmp(&br))
    }

    /// The `(primary, residual)` key orders every integer/double pair exactly:
    /// around 2^53, i64::MAX against `i64::MAX as f64` (2^63), i64::MIN, mixed
    /// near-equal int/double, -0 == 0, +-Infinity.
    #[test]
    fn numeric_sort_key_orders_integers_and_doubles_exactly() {
        const P53: i64 = 1 << 53;
        let mut sample: Vec<FieldValue> = vec![
            FieldValue::Integer(i64::MIN),
            FieldValue::Integer(i64::MIN + 1),
            FieldValue::Integer(-P53 - 1),
            FieldValue::Integer(-P53),
            FieldValue::Integer(-1),
            FieldValue::Integer(0),
            FieldValue::Integer(1),
            FieldValue::Integer(P53 - 1),
            FieldValue::Integer(i64::MAX),
            FieldValue::Integer(i64::MAX - 1),
            FieldValue::Integer(i64::MAX - 511),
            FieldValue::Integer(i64::MAX - 512),
            FieldValue::Double(f64::NEG_INFINITY),
            FieldValue::Double(f64::INFINITY),
            FieldValue::Double(-0.0),
            FieldValue::Double(0.0),
            FieldValue::Double(0.5),
            FieldValue::Double(-0.5),
            FieldValue::Double(9_223_372_036_854_775_808.0), // 2^63 == i64::MAX as f64
            FieldValue::Double(-9_223_372_036_854_775_808.0), // -2^63 == i64::MIN
            FieldValue::Double(1.0e30),
        ];
        for k in -5..=5i64 {
            sample.push(FieldValue::Integer(P53 + k));
            sample.push(FieldValue::Integer(-P53 + k));
            sample.push(FieldValue::Double((P53 + k) as f64));
        }
        for a in &sample {
            for b in &sample {
                assert_eq!(key_cmp(a, b), exact_cmp(a, b), "{a:?} vs {b:?}");
            }
        }
        // The cases the old float8-only key got wrong.
        assert_eq!(key_cmp(&FieldValue::Integer(P53 + 1), &FieldValue::Integer(P53)), std::cmp::Ordering::Greater);
        assert_eq!(
            key_cmp(&FieldValue::Integer(i64::MAX), &FieldValue::Double(i64::MAX as f64)),
            std::cmp::Ordering::Less
        );
        assert_eq!(key_cmp(&FieldValue::Double(-0.0), &FieldValue::Double(0.0)), std::cmp::Ordering::Equal);
    }

    #[test]
    fn numeric_sort_key_is_none_outside_the_numeric_class() {
        assert_eq!(numeric_sort_key(&FieldValue::String("1".into())), None);
        assert_eq!(numeric_sort_key(&FieldValue::Integer(-3)), Some((-3.0, 0)));
        assert_eq!(numeric_sort_key(&FieldValue::Integer(i64::MAX)), Some((9_223_372_036_854_775_808.0, -1)));
    }

    #[test]
    fn order_key_exprs_has_two_numeric_keys_and_stays_cast_guarded() {
        let keys = order_key_exprs("n");
        assert_eq!(keys.len(), 9, "{keys:?}");
        let (primary, residual) = numeric_key_exprs("n");
        assert_eq!(keys[2], primary);
        assert_eq!(keys[3], residual);
        assert_eq!(primary, "CASE WHEN fields->'n'->>'t' IN ('I','D') THEN (fields->'n'->>'v')::float8 END");
        // Residual is exact `numeric` arithmetic; the 2^63 case is substituted
        // before the float8 -> bigint cast (which would overflow).
        assert!(residual.contains("(fields->'n'->>'v')::numeric -"), "{residual}");
        assert!(residual.contains(">= 9223372036854775808::float8 THEN 9223372036854775808::numeric"), "{residual}");
        assert!(residual.contains("::bigint::numeric"), "{residual}");
        assert!(residual.ends_with("WHEN fields->'n'->>'t' = 'D' THEN 0::numeric END"), "{residual}");
    }

    #[test]
    fn numeric_range_filter_compares_the_two_part_key_row_wise() {
        let sql = generated_sql(&filter("n", FilterOp::GreaterThan, FieldValue::Integer(5)));
        let (primary, residual) = numeric_key_exprs("n");
        assert_eq!(sql, format!("ROW({primary}, {residual}) > ROW($1, $2::numeric)"));
        // Same shape for a double bound (residual bound as 0).
        let sql = generated_sql(&filter("n", FilterOp::LessThanOrEqual, FieldValue::Double(1.5)));
        assert!(sql.starts_with(&format!("ROW({primary}, {residual}) <= ROW(")), "{sql}");
    }

    fn effective_names(order_by: &[OrderBy]) -> Vec<(String, OrderDirection)> {
        effective_order_by(order_by).into_iter().map(|o| (o.field_path, o.direction)).collect()
    }

    #[test]
    fn effective_order_by_appends_name_in_the_direction_of_the_last_order() {
        use OrderDirection::{Ascending as A, Descending as D};
        assert_eq!(effective_names(&[]), vec![("__name__".into(), A)]);
        assert_eq!(effective_names(&[ob("x", A)]), vec![("x".into(), A), ("__name__".into(), A)]);
        assert_eq!(effective_names(&[ob("x", D)]), vec![("x".into(), D), ("__name__".into(), D)]);
        // Only the LAST explicit order decides the implicit direction.
        assert_eq!(
            effective_names(&[ob("x", D), ob("y", A)]),
            vec![("x".into(), D), ("y".into(), A), ("__name__".into(), A)]
        );
        // An explicit trailing `__name__` is kept as is, never doubled.
        assert_eq!(effective_names(&[ob("x", A), ob("__name__", D)]), vec![("x".into(), A), ("__name__".into(), D)]);
    }

    #[test]
    fn order_by_clause_ends_with_the_name_tiebreak() {
        let sql = order_by_clause(&[ob("n", OrderDirection::Descending)]);
        assert!(
            sql.ends_with("collection_path COLLATE \"C\" DESC, document_id COLLATE \"C\" DESC"),
            "{sql}"
        );
        assert_eq!(
            order_by_clause(&[]),
            "collection_path COLLATE \"C\" ASC, document_id COLLATE \"C\" ASC"
        );
        assert_eq!(order_by_clause(&[ob("__name__", OrderDirection::Ascending)]).matches("document_id").count(), 1);
    }

    fn cursor_sql(orders: &[OrderBy], values: &[FieldValue], is_end: bool, before: bool) -> Result<String, String> {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("");
        push_cursor_predicate(&mut qb, &effective_order_by(orders), values, is_end, before)?;
        Ok(qb.sql().to_string())
    }


    /// SPEC.md §Cursors table, ASC and DESC, single value: the pre-G-001
    /// operator per cursor kind (the bound is one comparison on the field).
    #[test]
    fn single_value_cursor_uses_the_spec_operator_table() {
        use OrderDirection::{Ascending as A, Descending as D};
        // (is_end, before, direction, expected operator)
        for (is_end, before, dir, want) in [
            (false, true, A, ">="),  // startAt
            (false, false, A, ">"),  // startAfter
            (true, false, A, "<="),  // endAt
            (true, true, A, "<"),    // endBefore
            (false, true, D, "<="),
            (false, false, D, "<"),
            (true, false, D, ">="),
            (true, true, D, ">"),
        ] {
            let sql = cursor_sql(&[ob("s", dir.clone())], &[FieldValue::String("v".into())], is_end, before).unwrap();
            assert_eq!(
                sql,
                format!("(CASE WHEN fields->'s'->>'t' = 'S' THEN fields->'s'->>'v' COLLATE \"C\" END {want} $1)"),
                "is_end={is_end} before={before} dir={dir:?}"
            );
        }
    }

    /// A cursor ending with the document reference expands lexicographically
    /// over (field, __name__) with each order's own direction.
    #[test]
    fn cursor_with_name_expands_or_of_and_in_the_last_orders_direction() {
        let name = FieldValue::Reference("projects/p/databases/d/documents/c/id1".into());
        let vals = [FieldValue::String("v".into()), name];
        let key = "CASE WHEN fields->'s'->>'t' = 'S' THEN fields->'s'->>'v' COLLATE \"C\" END";
        let row = "ROW(collection_path COLLATE \"C\", document_id COLLATE \"C\")";
        // startAfter, DESC: field < v OR (field = v AND name < ref).
        let sql = cursor_sql(&[ob("s", OrderDirection::Descending)], &vals, false, false).unwrap();
        assert_eq!(sql, format!("(({key} < $1) OR ({key} = $2 AND {row} < ROW($3, $4)))"));
        // startAt, ASC: last comparison is inclusive.
        let sql = cursor_sql(&[ob("s", OrderDirection::Ascending)], &vals, false, true).unwrap();
        assert_eq!(sql, format!("(({key} > $1) OR ({key} = $2 AND {row} >= ROW($3, $4)))"));
        // endBefore, ASC: strictly before on both parts.
        let sql = cursor_sql(&[ob("s", OrderDirection::Ascending)], &vals, true, true).unwrap();
        assert_eq!(sql, format!("(({key} < $1) OR ({key} = $2 AND {row} < ROW($3, $4)))"));
    }

    #[test]
    fn cursor_without_name_is_a_prefix_and_never_touches_the_name_key() {
        let sql = cursor_sql(&[ob("s", OrderDirection::Ascending)], &[FieldValue::String("v".into())], false, false).unwrap();
        assert!(!sql.contains("document_id"), "{sql}");
    }

    #[test]
    fn cursor_on_name_order_alone_compares_the_document_name() {
        let r = FieldValue::Reference("projects/p/databases/d/documents/c/id1".into());
        let sql = cursor_sql(&[], &[r], false, true).unwrap();
        assert_eq!(sql, "(ROW(collection_path COLLATE \"C\", document_id COLLATE \"C\") >= ROW($1, $2))");
    }

    #[test]
    fn cursor_with_more_values_than_orders_or_a_non_reference_name_is_rejected() {
        let s = FieldValue::String("v".into());
        let e = cursor_sql(&[ob("s", OrderDirection::Ascending)], &[s.clone(), s.clone(), s.clone()], false, true).unwrap_err();
        assert!(e.contains("3 values") && e.contains("2 fields"), "{e}");
        let e = cursor_sql(&[], &[s], false, true).unwrap_err();
        assert!(e.contains("document reference"), "{e}");
    }
}
