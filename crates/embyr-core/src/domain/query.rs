use chrono::{DateTime, Utc};

use super::field_value::FieldValue;
use crate::error::CoreError;

/// Validate a Firestore field path against the SPEC.md-mandated character
/// class `^[a-zA-Z_][a-zA-Z0-9_.]*$` (SPEC.md Invariant 6, "Limits and
/// Constraints" table).
///
/// This is the single root-cause guard: every client-supplied field path
/// (query filters, order-by clauses) must pass through here before it is
/// ever interpolated into raw SQL text by `embyr-pg-storage`'s query
/// builder. Values are always bound via `push_bind` and are never at risk;
/// the field path itself is raw-string-interpolated, so this is the only
/// thing standing between a crafted field path and SQL injection.
pub fn validate_field_path(path: &str) -> Result<(), CoreError> {
    if is_valid_field_path(path) {
        Ok(())
    } else {
        Err(CoreError::InvalidArgument(format!(
            "field path must match ^[a-zA-Z_][a-zA-Z0-9_.]*$, got: {path}"
        )))
    }
}

fn is_valid_field_path(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

// Test Budget: 2 behaviors (accepts spec-compliant paths / rejects
// non-compliant paths incl. SQL metacharacters) x 2 = 4 tests.
#[cfg(test)]
mod field_path_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Behavior 1: any path built only from the spec's allowed character
        /// class, starting with a letter or underscore, is accepted —
        /// zero behavior change for legitimate field paths (e.g. "user_id",
        /// "address.city").
        #[test]
        fn spec_compliant_paths_always_accepted(
            first in "[a-zA-Z_]",
            rest in "[a-zA-Z0-9_.]{0,20}",
        ) {
            let path = format!("{first}{rest}");
            prop_assert!(validate_field_path(&path).is_ok());
        }

        /// Behavior 2: any spec-compliant path with ONE disallowed
        /// character appended (quote, semicolon, space, dash, parens) is
        /// rejected with InvalidArgument, not silently accepted or panicking.
        #[test]
        fn path_with_any_disallowed_char_is_rejected(
            prefix in "[a-zA-Z_][a-zA-Z0-9_.]{0,10}",
            bad in prop::sample::select(vec!['\'', ';', ' ', '-', '(', ')', '=']),
        ) {
            let path = format!("{prefix}{bad}");
            prop_assert!(matches!(
                validate_field_path(&path),
                Err(CoreError::InvalidArgument(_))
            ));
        }
    }

    /// Documents the exact exploit-shaped payloads from the security brief.
    #[test]
    fn known_sql_injection_payloads_rejected() {
        for payload in ["x'); DROP TABLE documents; --", "x' OR '1'='1"] {
            assert!(
                matches!(validate_field_path(payload), Err(CoreError::InvalidArgument(_))),
                "expected InvalidArgument rejection for: {payload}"
            );
        }
    }

    #[test]
    fn empty_string_rejected() {
        assert!(matches!(validate_field_path(""), Err(CoreError::InvalidArgument(_))));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    Equal,
    NotEqual,
    ArrayContains,
    In,
    NotIn,
    ArrayContainsAny,
    IsNan,
    IsNotNan,
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone)]
pub struct FieldFilter {
    pub field_path: String,
    pub op: FilterOp,
    pub value: FieldValue,
}

#[derive(Debug, Clone)]
pub enum QueryFilter {
    Field(FieldFilter),
    /// Composite AND filter.
    Composite(Vec<QueryFilter>),
    /// Composite OR filter (firestore-or-filter-support). A document
    /// matches iff it matches AT LEAST ONE child filter — the union, not
    /// the intersection `Composite` represents. Strictly additive: every
    /// existing `Composite` call site is unaffected by this variant.
    CompositeOr(Vec<QueryFilter>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone)]
pub struct OrderBy {
    pub field_path: String,
    pub direction: OrderDirection,
}

#[derive(Debug, Clone)]
pub struct Cursor {
    pub values: Vec<FieldValue>,
    pub before: bool,
}

#[derive(Debug, Clone)]
pub struct StructuredQuery {
    pub collection_id: String,
    pub all_descendants: bool,
    pub filter: Option<QueryFilter>,
    pub order_by: Vec<OrderBy>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
    pub start_at: Option<Cursor>,
    pub end_at: Option<Cursor>,
    /// When set, only return documents with update_time > since_update_time.
    /// Used for resume-token delta delivery in Listen streams (step 05-03).
    pub since_update_time: Option<DateTime<Utc>>,
}

/// Opaque byte token for resuming a listen stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeToken(pub Vec<u8>);

/// The aggregation operator requested by an `AggregationQuery` (aggregation-queries,
/// ADR-038). `Sum`/`Avg` carry the field path to aggregate over; `Count`
/// carries an optional `up_to` cap (result = `min(count, up_to)`, always > 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggregationKind {
    Count(Option<i64>),
    Sum(String),
    Avg(String),
}

/// One aggregation of an `AggregationQuery` with its resolved (non-empty,
/// unique) result alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aggregation {
    pub alias: String,
    pub kind: AggregationKind,
}

/// A `StructuredQuery` paired with the aggregations to compute over its
/// result set in ONE pass. Mirrors real Firestore's
/// `StructuredAggregationQuery` (ADR-038). Adapters return one
/// `AggregateValue` per entry of `aggregations`, in the same order.
#[derive(Debug, Clone)]
pub struct AggregationQuery {
    pub query: StructuredQuery,
    pub aggregations: Vec<Aggregation>,
}

/// Firestore allows at most 5 aggregations per request.
pub const MAX_AGGREGATIONS: usize = 5;

/// Resolve the result aliases of a request's aggregations (client aliases in
/// request order; `None`/empty = unset). Firestore semantics: empty aliases
/// get `field_<n>` with `n` counting only the generated ones, from 1
/// (skipping any name an explicit alias already took); duplicate explicit
/// aliases, zero aggregations, or more than `MAX_AGGREGATIONS` are invalid.
pub fn resolve_aggregation_aliases(requested: &[&str]) -> Result<Vec<String>, CoreError> {
    if requested.is_empty() || requested.len() > MAX_AGGREGATIONS {
        return Err(CoreError::InvalidArgument(format!(
            "between 1 and {MAX_AGGREGATIONS} aggregations are required, got {}",
            requested.len()
        )));
    }
    let mut taken = std::collections::HashSet::new();
    for alias in requested.iter().filter(|a| !a.is_empty()) {
        if !taken.insert(*alias) {
            return Err(CoreError::InvalidArgument(format!("duplicate aggregation alias: {alias}")));
        }
    }
    let mut next = 1;
    Ok(requested
        .iter()
        .map(|alias| {
            if !alias.is_empty() {
                return alias.to_string();
            }
            loop {
                let generated = format!("field_{next}");
                next += 1;
                if !taken.contains(generated.as_str()) {
                    return generated;
                }
            }
        })
        .collect())
}

/// The computed result of an `AggregationQuery` (ADR-040 § Response value
/// mapping). `Avg(None)` distinguishes "zero matching documents" from a real
/// average of zero — never conflated with `Avg(Some(0.0))`.
#[derive(Debug, Clone, PartialEq)]
pub enum AggregateValue {
    Count(i64),
    Sum(f64),
    Avg(Option<f64>),
}

#[cfg(test)]
mod aggregation_alias_tests {
    use super::*;

    #[test]
    fn empty_aliases_get_field_n_counting_only_generated_ones_from_one() {
        let got = resolve_aggregation_aliases(&["total", "", "n", ""]).unwrap();
        assert_eq!(got, ["total", "field_1", "n", "field_2"]);
    }

    #[test]
    fn generated_alias_skips_a_name_an_explicit_alias_already_took() {
        let got = resolve_aggregation_aliases(&["", "field_1"]).unwrap();
        assert_eq!(got, ["field_2", "field_1"]);
    }

    #[test]
    fn duplicate_explicit_alias_zero_or_too_many_aggregations_are_invalid() {
        for bad in [vec![], vec!["a", "a"], vec!["a", "b", "c", "d", "e", "f"]] {
            assert!(
                matches!(resolve_aggregation_aliases(&bad), Err(CoreError::InvalidArgument(_))),
                "expected InvalidArgument for {bad:?}"
            );
        }
    }

    #[test]
    fn several_empty_aliases_are_not_duplicates() {
        assert_eq!(resolve_aggregation_aliases(&["", ""]).unwrap(), ["field_1", "field_2"]);
    }
}
