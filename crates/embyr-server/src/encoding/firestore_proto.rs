use std::collections::{BTreeMap, HashMap};

use embyr_core::domain::{document::FirestoreDocument, field_value::FieldValue};
use embyr_proto::firestore::{value::ValueType, ArrayValue, Document, MapValue, Value};
use prost_types::Timestamp;

/// Convert a `FieldValue` to its protobuf `Value` representation.
pub fn field_value_to_proto(fv: &FieldValue) -> Value {
    let vt = match fv {
        FieldValue::Null => ValueType::NullValue(0),
        FieldValue::Boolean(b) => ValueType::BooleanValue(*b),
        FieldValue::Integer(i) => ValueType::IntegerValue(*i),
        FieldValue::Double(d) => ValueType::DoubleValue(*d),
        FieldValue::String(s) => ValueType::StringValue(s.clone()),
        FieldValue::Bytes(b) => ValueType::BytesValue(b.clone()),
        FieldValue::Reference(r) => ValueType::ReferenceValue(r.clone()),
        FieldValue::Timestamp(s, n) => {
            ValueType::TimestampValue(Timestamp { seconds: *s, nanos: *n })
        }
        FieldValue::Array(arr) => ValueType::ArrayValue(ArrayValue {
            values: arr.iter().map(field_value_to_proto).collect(),
        }),
        FieldValue::Map(map) => ValueType::MapValue(MapValue {
            fields: map
                .iter()
                .map(|(k, v)| (k.clone(), field_value_to_proto(v)))
                .collect(),
        }),
    };
    Value { value_type: Some(vt) }
}

/// Convert a document's field map to a proto `HashMap<String, Value>`.
pub fn fields_to_proto(
    fields: &std::collections::BTreeMap<String, FieldValue>,
) -> HashMap<String, Value> {
    fields
        .iter()
        .map(|(k, v)| (k.clone(), field_value_to_proto(v)))
        .collect()
}

/// Convert a proto `Value` to a domain `FieldValue`.
///
/// Returns `None` if the value type is unrecognised or structurally invalid.
pub fn proto_value_to_field_value(v: &Value) -> Option<FieldValue> {
    match v.value_type.as_ref()? {
        ValueType::NullValue(_) => Some(FieldValue::Null),
        ValueType::BooleanValue(b) => Some(FieldValue::Boolean(*b)),
        ValueType::IntegerValue(i) => Some(FieldValue::Integer(*i)),
        ValueType::DoubleValue(d) => Some(FieldValue::Double(*d)),
        ValueType::StringValue(s) => Some(FieldValue::String(s.clone())),
        ValueType::BytesValue(b) => Some(FieldValue::Bytes(b.to_vec())),
        ValueType::ReferenceValue(r) => Some(FieldValue::Reference(r.clone())),
        ValueType::TimestampValue(ts) => Some(FieldValue::Timestamp(ts.seconds, ts.nanos)),
        ValueType::ArrayValue(arr) => {
            let vals: Option<Vec<_>> =
                arr.values.iter().map(proto_value_to_field_value).collect();
            Some(FieldValue::Array(vals?))
        }
        ValueType::MapValue(mv) => {
            let mut map = BTreeMap::new();
            for (k, v) in &mv.fields {
                map.insert(k.clone(), proto_value_to_field_value(v)?);
            }
            Some(FieldValue::Map(map))
        }
        ValueType::GeoPointValue(_) => None,
    }
}

/// Convert a proto field map (`HashMap<String, Value>`) to domain field map.
///
/// Returns `None` if any value fails conversion.
pub fn proto_fields_to_domain(
    fields: &HashMap<String, Value>,
) -> Option<BTreeMap<String, FieldValue>> {
    fields
        .iter()
        .map(|(k, v)| proto_value_to_field_value(v).map(|fv| (k.clone(), fv)))
        .collect()
}

/// Convert a `FirestoreDocument` to its proto `Document` representation.
pub fn document_to_proto(doc: FirestoreDocument) -> Document {
    let name = format!(
        "projects/{}/databases/(default)/documents/{}/{}",
        doc.path.project_id.as_str(),
        doc.path.collection_path,
        doc.path.document_id,
    );
    Document {
        name,
        fields: fields_to_proto(&doc.fields),
        create_time: Some(Timestamp {
            seconds: doc.create_time.0,
            nanos: doc.create_time.1,
        }),
        update_time: Some(Timestamp {
            seconds: doc.update_time.0,
            nanos: doc.update_time.1,
        }),
    }
}
