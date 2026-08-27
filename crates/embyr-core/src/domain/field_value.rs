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

impl FieldValue {
    /// Translate a `serde_json::Value` into its closest `FieldValue`
    /// structural analog, preserving exact JSON type (no coercion — a JSON
    /// string never becomes `Boolean`, ADR-034 § Decision — Shared JSON
    /// Translation). Single source of truth: `embyr-server`'s admin
    /// simulation handlers and `embyr-core::client_identity`'s custom-claims
    /// decode (ADR-034, US-01) both delegate here rather than each
    /// maintaining their own copy of these match arms.
    pub fn from_json_value(value: &serde_json::Value) -> FieldValue {
        match value {
            serde_json::Value::Null => FieldValue::Null,
            serde_json::Value::Bool(b) => FieldValue::Boolean(*b),
            serde_json::Value::String(s) => FieldValue::String(s.clone()),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    FieldValue::Integer(i)
                } else {
                    FieldValue::Double(n.as_f64().unwrap_or(0.0))
                }
            }
            serde_json::Value::Array(items) => {
                FieldValue::Array(items.iter().map(FieldValue::from_json_value).collect())
            }
            serde_json::Value::Object(map) => FieldValue::Map(
                map.iter()
                    .map(|(k, v)| (k.clone(), FieldValue::from_json_value(v)))
                    .collect(),
            ),
        }
    }
}
