//! Pure rules for Firestore-style TTL deletion (go-live finding P5).
//!
//! A TTL policy is `(project, collection group, timestamp field)`. A document
//! is eligible once that field holds a `Timestamp` strictly before "now";
//! a missing, null or non-timestamp field is never eligible. No IO here: the
//! server's `ttl_sweeper` drives these rules against a backend adapter.

use std::collections::BTreeMap;

use super::field_value::FieldValue;
use super::query::{validate_field_path, FieldFilter, FilterOp, QueryFilter, StructuredQuery};
use crate::error::CoreError;

/// Documents fetched (and deleted in one commit) per sweep step.
pub const BATCH_SIZE: i32 = 500;

const MAX_ID_BYTES: usize = 1500;

/// Validate a policy's `collection_group` (a collection id, not a path) and
/// `field_path`.
///
/// `field_path` goes through the same validator that guards SQL
/// interpolation in `embyr-pg-storage` (audit #11/#26) and is further limited
/// to a TOP-LEVEL field: the query machinery resolves a dotted path as one
/// literal key, so a nested path would silently never match.
pub fn validate_ttl_policy(collection_group: &str, field_path: &str) -> Result<(), CoreError> {
    let bad = |m: &str| Err(CoreError::InvalidArgument(m.to_string()));
    if collection_group.is_empty()
        || collection_group.len() > MAX_ID_BYTES
        || collection_group.contains('/')
        || collection_group.chars().any(char::is_control)
        || collection_group == "."
        || collection_group == ".."
        || (collection_group.len() > 4
            && collection_group.starts_with("__")
            && collection_group.ends_with("__"))
    {
        return bad("collection_group must be a single collection id (no '/', not '.', '..' or '__x__')");
    }
    validate_field_path(field_path)?;
    if field_path.contains('.') {
        return bad("field_path must be a top-level field (nested paths are not supported)");
    }
    Ok(())
}

/// True iff `fields[field_path]` is a `Timestamp` strictly before `now`
/// (`(seconds, nanos)` UTC).
pub fn is_expired(fields: &BTreeMap<String, FieldValue>, field_path: &str, now: (i64, i32)) -> bool {
    matches!(fields.get(field_path), Some(FieldValue::Timestamp(s, n)) if (*s, *n) < now)
}

/// True iff the document's collection path ends in the collection id `group`.
/// Defence in depth over the adapter's `LIKE '%/<id>'` collection-group
/// predicate, where `_`/`%` in an id act as wildcards: never delete outside
/// the configured group.
pub fn in_collection_group(collection_path: &str, group: &str) -> bool {
    collection_path.rsplit('/').next() == Some(group)
}

/// Limit for the next step: `BATCH_SIZE` clamped to what is left of the
/// per-run cap; `None` once the cap is spent.
pub fn next_batch_limit(cap: u64, deleted: u64) -> Option<i32> {
    let left = cap.checked_sub(deleted).filter(|l| *l > 0)?;
    Some(left.min(BATCH_SIZE as u64) as i32)
}

/// The collection-group query selecting candidate documents: `field < now`
/// (typed timestamp comparison — other types never match).
pub fn expired_documents_query(
    collection_group: &str,
    field_path: &str,
    now: (i64, i32),
    limit: i32,
) -> StructuredQuery {
    StructuredQuery {
        collection_id: collection_group.to_string(),
        all_descendants: true,
        filter: Some(QueryFilter::Field(FieldFilter {
            field_path: field_path.to_string(),
            op: FilterOp::LessThan,
            value: FieldValue::Timestamp(now.0, now.1),
        })),
        order_by: vec![],
        limit: Some(limit),
        offset: None,
        start_at: None,
        end_at: None,
        since_update_time: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(v: FieldValue) -> BTreeMap<String, FieldValue> {
        BTreeMap::from([("exp".to_string(), v)])
    }

    #[test]
    fn past_timestamp_is_expired_future_and_equal_are_not() {
        let now = (1_000, 500);
        assert!(is_expired(&fields(FieldValue::Timestamp(999, 999)), "exp", now));
        assert!(is_expired(&fields(FieldValue::Timestamp(1_000, 499)), "exp", now));
        assert!(!is_expired(&fields(FieldValue::Timestamp(1_000, 500)), "exp", now));
        assert!(!is_expired(&fields(FieldValue::Timestamp(1_001, 0)), "exp", now));
    }

    #[test]
    fn missing_null_and_non_timestamp_fields_are_never_expired() {
        let now = (1_000, 0);
        assert!(!is_expired(&BTreeMap::new(), "exp", now));
        for v in [
            FieldValue::Null,
            FieldValue::Integer(1),
            FieldValue::Double(1.0),
            FieldValue::String("2001-01-01T00:00:00Z".into()),
            FieldValue::Boolean(true),
            FieldValue::Array(vec![FieldValue::Timestamp(1, 0)]),
            FieldValue::Map(BTreeMap::from([("s".to_string(), FieldValue::Integer(1))])),
        ] {
            assert!(!is_expired(&fields(v.clone()), "exp", now), "{v:?}");
        }
        // other field holds the timestamp, configured one is absent
        assert!(!is_expired(&fields(FieldValue::Timestamp(1, 0)), "other", now));
    }

    #[test]
    fn group_match_is_exact_on_the_last_path_segment() {
        assert!(in_collection_group("sessions", "sessions"));
        assert!(in_collection_group("users/u1/sessions", "sessions"));
        assert!(!in_collection_group("sessions/s1/events", "sessions"));
        assert!(!in_collection_group("users/u1/xsessions", "sessions"));
        // LIKE-wildcard lookalike: `_` must not match another character
        assert!(!in_collection_group("a/userXsessions", "user_sessions"));
        assert!(in_collection_group("a/user_sessions", "user_sessions"));
    }

    #[test]
    fn batch_limit_is_clamped_by_the_cap_and_ends_at_zero() {
        assert_eq!(next_batch_limit(10_000, 0), Some(500));
        assert_eq!(next_batch_limit(10_000, 9_800), Some(200));
        assert_eq!(next_batch_limit(10_000, 10_000), None);
        assert_eq!(next_batch_limit(10_000, 12_000), None);
        assert_eq!(next_batch_limit(0, 0), None);
        assert_eq!(next_batch_limit(100, 0), Some(100));
    }

    #[test]
    fn query_is_a_limited_collection_group_less_than_timestamp() {
        let q = expired_documents_query("sessions", "exp", (7, 8), 42);
        assert!(q.all_descendants);
        assert_eq!(q.collection_id, "sessions");
        assert_eq!(q.limit, Some(42));
        match q.filter {
            Some(QueryFilter::Field(f)) => {
                assert_eq!(f.field_path, "exp");
                assert_eq!(f.op, FilterOp::LessThan);
                assert!(matches!(f.value, FieldValue::Timestamp(7, 8)));
            }
            other => panic!("unexpected filter {other:?}"),
        }
    }

    #[test]
    fn valid_policies_pass_and_unsafe_ones_are_rejected() {
        assert!(validate_ttl_policy("sessions", "expireAt").is_ok());
        assert!(validate_ttl_policy("user-sessions_1", "_exp").is_ok());
        for (g, f) in [
            ("", "exp"),
            ("a/b", "exp"),
            (".", "exp"),
            ("..", "exp"),
            ("__name__", "exp"),
            ("bad\nid", "exp"),
            ("sessions", ""),
            ("sessions", "x'); DROP TABLE documents; --"),
            ("sessions", "a b"),
            ("sessions", "a.b"),
            ("sessions", "1abc"),
        ] {
            assert!(
                matches!(validate_ttl_policy(g, f), Err(CoreError::InvalidArgument(_))),
                "({g:?}, {f:?}) must be rejected"
            );
        }
        assert!(validate_ttl_policy(&"x".repeat(1501), "exp").is_err());
    }
}
