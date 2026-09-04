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
/// ADR-038). `Sum`/`Avg` carry the field path to aggregate over; only `Count`
/// is functional in Slice 01 — the Postgres adapter's `Sum`/`Avg` branches
/// return `CoreError::FailedPrecondition` until Slices 03/04.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggregationKind {
    Count,
    Sum(String),
    Avg(String),
}

/// A `StructuredQuery` paired with a single aggregation to compute over its
/// result set, plus the caller-supplied (or server-synthesized) alias for the
/// result. Mirrors real Firestore's `StructuredAggregationQuery` (ADR-038).
#[derive(Debug, Clone)]
pub struct AggregationQuery {
    pub query: StructuredQuery,
    pub aggregation: AggregationKind,
    pub alias: String,
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
