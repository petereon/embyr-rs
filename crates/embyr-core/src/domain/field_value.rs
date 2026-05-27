use std::collections::BTreeMap;

/// All 8 Firestore field value types (plus Array and Map composites).
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Double(f64),
    /// Wall-clock timestamp: (seconds since Unix epoch, subsecond nanos).
    Timestamp(i64, i32),
    String(String),
    Bytes(Vec<u8>),
    /// Firestore document reference path.
    Reference(String),
    Array(Vec<FieldValue>),
    Map(BTreeMap<String, FieldValue>),
}
