//! Query-target request parsing helpers for Listen. The stream itself is
//! driven by `listen_session`.

/// Extract `collection_id` from a `QueryTarget` by reading `from[0]` in the
/// `StructuredQuery`.
pub(crate) fn collection_id_from_query_target(qt: &embyr_proto::firestore::target::QueryTarget) -> Result<String, String> {
    use embyr_proto::firestore::target::query_target::QueryType;
    match &qt.query_type {
        Some(QueryType::StructuredQuery(sq)) => {
            sq.from
                .first()
                .map(|cs| cs.collection_id.clone())
                .ok_or_else(|| "structured query has no collection selector".into())
        }
        None => Err("QueryTarget has no query_type".into()),
    }
}

/// Extract and translate the caller's own `where_` filter from a
/// `QueryTarget`'s `StructuredQuery`, reusing `translate_filter()` — the
/// SAME function `handle_run_query` already uses — unchanged. security-
/// rules-realtime (ADR-033 § Decision — Subscribe-Time Composition, US-02).
pub(crate) fn filter_from_query_target(
    qt: &embyr_proto::firestore::target::QueryTarget,
) -> Result<Option<embyr_core::domain::query::QueryFilter>, String> {
    use embyr_proto::firestore::target::query_target::QueryType;
    match &qt.query_type {
        Some(QueryType::StructuredQuery(sq)) => sq
            .r#where
            .as_ref()
            .and_then(crate::grpc::handler::translate_filter)
            .transpose(),
        None => Err("QueryTarget has no query_type".into()),
    }
}

/// Extract `project_id` from a Firestore parent path.
///
/// Format: `projects/{pid}/databases/(default)/documents`
pub(crate) fn project_id_from_parent(parent: &str) -> Result<String, String> {
    let mut parts = parent.splitn(5, '/');
    match (parts.next(), parts.next()) {
        (Some("projects"), Some(pid)) if !pid.is_empty() => Ok(pid.to_string()),
        _ => Err(format!("invalid parent path: {parent}")),
    }
}
