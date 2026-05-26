use chrono::{DateTime, Utc};

use super::field_value::FieldValue;

#[derive(Debug, Clone, PartialEq, Eq)]
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
