//! Pure, IO-free DDL-string builders for real composite-index provisioning
//! (composite-index-real-creation, ADR-072 Decision A/D).
//!
//! Every `field_path` is validated via the EXISTING
//! `embyr_core::domain::query::validate_field_path` charset gate
//! (`^[a-zA-Z_][a-zA-Z0-9_.]*$`) before any DDL text is constructed
//! (AC-CXR-04) — the same reuse `encoding/query.rs`'s own WHERE/ORDER-BY
//! builders rely on, not a new, parallel validator.
//!
//! Expression shape (ADR-072 Decision D, updated for typed-order-by P11), for
//! `fields = [f_1, ..., f_n]`:
//! ```sql
//! CREATE INDEX CONCURRENTLY IF NOT EXISTS cix_<hex(id)>
//!   ON documents (
//!     project_id,
//!     collection_path,
//!     (<equality_key_expr(f_1)>), ..., (<equality_key_expr(f_{n-1})>),
//!     (<order_key_exprs(f_n)[0]>) {ASC|DESC}, ..., (<order_key_exprs(f_n)[8]>) {ASC|DESC}
//!   )
//!   WHERE NOT deleted;
//! ```
//! — leading literal `(project_id, collection_path)` matching `run_query`'s
//! own bound predicates; all-but-last fields as the equality filter's own key
//! expression; the last field as EVERY typed ORDER BY key with the direction.
//! Both key builders live in `embyr_pg_storage::encoding::query` and are the
//! SAME functions the query side calls, so the planner can match the index.
//! Indexes created before typed-order-by (old `->>'v'` text key) or before the
//! exact numeric two-part key (G-001; the numbers' single float8 key became a
//! `(float8, residual)` pair) no longer match ORDER BY and must be dropped and
//! re-created.

use uuid::Uuid;

use embyr_core::domain::query::validate_field_path;
use embyr_core::error::CoreError;
use embyr_pg_storage::encoding::query::{equality_key_expr, order_key_exprs};

use crate::admin::handlers::composite_indexes::{IndexFieldOrder, IndexFieldSpec};

/// Server-generated index name — never derived from user input (ADR-072
/// Decision A), so no identifier quoting is needed beyond the fixed `cix_`
/// prefix + hex UUID.
pub fn index_name_for(id: Uuid) -> String {
    format!("cix_{}", id.simple())
}

/// Validate every field's `field_path` via the existing charset gate.
/// AC-CXR-04: called BEFORE any DDL text is built, and also called by the
/// admin handler directly before the `composite_indexes` row is inserted.
pub fn validate_fields(fields: &[IndexFieldSpec]) -> Result<(), CoreError> {
    if fields.is_empty() {
        return Err(CoreError::InvalidArgument(
            "composite index requires at least one field".to_string(),
        ));
    }
    for f in fields {
        validate_field_path(&f.field)?;
    }
    Ok(())
}

/// Index columns for the trailing sort field: every ORDER BY key as a
/// parenthesised index expression carrying the field's direction.
fn sort_key_columns(f: &IndexFieldSpec) -> Vec<String> {
    let dir = match f.order {
        IndexFieldOrder::Asc => "ASC",
        IndexFieldOrder::Desc => "DESC",
    };
    order_key_exprs(&f.field).into_iter().map(|k| format!("({k}) {dir}")).collect()
}

/// Build the `CREATE INDEX CONCURRENTLY` DDL for `id`'s composite index
/// (ADR-072 Decision B/D). Re-validates every field path (defense in depth —
/// the handler already validates before INSERT).
pub fn build_create_index_sql(id: Uuid, fields: &[IndexFieldSpec]) -> Result<String, CoreError> {
    validate_fields(fields)?;

    let (last, rest) = fields.split_last().expect("validate_fields rejects empty");

    let mut columns: Vec<String> = vec!["project_id".to_string(), "collection_path".to_string()];
    columns.extend(rest.iter().map(|f| format!("({})", equality_key_expr(&f.field))));
    columns.extend(sort_key_columns(last));

    Ok(format!(
        "CREATE INDEX CONCURRENTLY IF NOT EXISTS {} ON documents ({}) WHERE NOT deleted",
        index_name_for(id),
        columns.join(", ")
    ))
}

/// Build the symmetric `DROP INDEX CONCURRENTLY IF EXISTS` DDL for `id`'s
/// composite index (ADR-072 Decision B, best-effort cleanup on failed build;
/// US-03 delete symmetry).
pub fn build_drop_index_sql(id: Uuid) -> String {
    format!("DROP INDEX CONCURRENTLY IF EXISTS {}", index_name_for(id))
}

// Test Budget: 3 behaviors (rejects a field path outside the safe charset
// before any DDL text exists / builds the exact locked expression shape for
// N equality fields + 1 trailing sort field / drop DDL targets the same
// deterministic name a matching create call would use) x 2 = 6 unit tests.
#[cfg(test)]
mod tests {
    use super::*;

    fn spec(field: &str, order: IndexFieldOrder) -> IndexFieldSpec {
        IndexFieldSpec { field: field.to_string(), order }
    }

    #[test]
    fn index_name_for_is_a_valid_postgres_identifier_derived_only_from_the_id() {
        let id = Uuid::parse_str("3fa85f64-5717-4562-b3fc-2c963f66afa6").unwrap();
        let name = index_name_for(id);
        assert_eq!(name, "cix_3fa85f6457174562b3fc2c963f66afa6");
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }

    #[test]
    fn build_create_index_sql_matches_the_locked_expression_shape_for_equality_plus_sort() {
        let id = Uuid::parse_str("3fa85f64-5717-4562-b3fc-2c963f66afa6").unwrap();
        let fields =
            vec![spec("category", IndexFieldOrder::Asc), spec("score", IndexFieldOrder::Desc)];

        let sql = build_create_index_sql(id, &fields).expect("valid fields must build");

        assert!(sql.starts_with(
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS cix_3fa85f6457174562b3fc2c963f66afa6 \
             ON documents (project_id, collection_path, (fields->'category'), \
             (CASE fields->'score'->>'t' WHEN 'N'"
        ));
        assert!(sql.ends_with(") DESC) WHERE NOT deleted"));
        // 9 typed ORDER BY keys, all descending; equality column carries no direction.
        assert_eq!(sql.matches(") DESC").count(), 9);
        assert!(!sql.contains("(fields->'category') ASC"));
    }

    // Drift guard: the index's trailing sort columns, stripped of their index
    // paren-wrapping, are byte-identical to the query-side ORDER BY, and the
    // equality column is the exact expression the equality filter compares.
    #[test]
    fn index_key_expressions_equal_the_query_side_expressions_for_every_field_and_direction() {
        use embyr_core::domain::field_value::FieldValue;
        use embyr_core::domain::query::{
            FieldFilter, FilterOp, OrderBy, OrderDirection, QueryFilter,
        };
        use embyr_pg_storage::encoding::query::{append_filter, order_by_expr};
        use sqlx::{Postgres, QueryBuilder};

        for field in ["score", "a.b", "_x1", "__name__"] {
            for (order, direction) in [
                (IndexFieldOrder::Asc, OrderDirection::Ascending),
                (IndexFieldOrder::Desc, OrderDirection::Descending),
            ] {
                let index_side = sort_key_columns(&spec(field, order))
                    .iter()
                    .map(|c| {
                        let (key, dir) = c.strip_prefix('(').unwrap().rsplit_once(") ").unwrap();
                        format!("{key} {dir}")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let query_side = order_by_expr(&OrderBy {
                    field_path: field.to_string(),
                    direction,
                });
                assert_eq!(index_side, query_side, "field={field}");
            }
        }

        let filter = QueryFilter::Field(FieldFilter {
            field_path: "category".to_string(),
            op: FilterOp::Equal,
            value: FieldValue::String("x".to_string()),
        });
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("");
        append_filter(&mut qb, &filter);
        let id = Uuid::nil();
        let sql = build_create_index_sql(
            id,
            &[spec("category", IndexFieldOrder::Asc), spec("score", IndexFieldOrder::Asc)],
        )
        .unwrap();
        let eq_col = sql.split(", ").nth(2).unwrap(); // "(fields->'category')"
        let eq_expr = eq_col.strip_prefix('(').unwrap().strip_suffix(')').unwrap();
        assert!(qb.sql().starts_with(&format!("{eq_expr} = ")));
    }

    #[test]
    fn build_create_index_sql_handles_a_single_field_with_no_leading_equality_columns() {
        let id = Uuid::parse_str("3fa85f64-5717-4562-b3fc-2c963f66afa6").unwrap();
        let fields = vec![spec("category", IndexFieldOrder::Asc)];

        let sql = build_create_index_sql(id, &fields).expect("valid single field must build");

        assert!(sql.starts_with(
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS cix_3fa85f6457174562b3fc2c963f66afa6 \
             ON documents (project_id, collection_path, (CASE fields->'category'->>'t' WHEN 'N'"
        ));
        assert!(sql.ends_with(") ASC) WHERE NOT deleted"));
        assert_eq!(sql.matches(") ASC").count(), 9);
    }

    #[test]
    fn build_create_index_sql_rejects_a_field_path_outside_the_safe_charset_before_building_any_sql()
    {
        let id = Uuid::parse_str("3fa85f64-5717-4562-b3fc-2c963f66afa6").unwrap();
        let fields = vec![spec("category\"); DROP TABLE documents; --", IndexFieldOrder::Asc)];

        let result = build_create_index_sql(id, &fields);

        assert!(matches!(result, Err(CoreError::InvalidArgument(_))));
    }

    #[test]
    fn build_create_index_sql_rejects_an_empty_field_list() {
        let id = Uuid::parse_str("3fa85f64-5717-4562-b3fc-2c963f66afa6").unwrap();
        assert!(matches!(build_create_index_sql(id, &[]), Err(CoreError::InvalidArgument(_))));
    }

    #[test]
    fn build_drop_index_sql_targets_the_same_deterministic_name_a_matching_create_call_would_use() {
        let id = Uuid::parse_str("3fa85f64-5717-4562-b3fc-2c963f66afa6").unwrap();
        let fields = vec![spec("category", IndexFieldOrder::Asc)];
        let create_sql = build_create_index_sql(id, &fields).unwrap();
        let drop_sql = build_drop_index_sql(id);

        assert_eq!(drop_sql, "DROP INDEX CONCURRENTLY IF EXISTS cix_3fa85f6457174562b3fc2c963f66afa6");
        assert!(create_sql.contains(&index_name_for(id)));
        assert!(drop_sql.contains(&index_name_for(id)));
    }
}
