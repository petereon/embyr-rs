//! Encoding: `FirestoreDocument` → agent proto `Document`.
//!
//! Translates domain types from `embyr_core` to `embyr_proto::agent` types at
//! the agent gRPC boundary. The storage crate must not import proto types, so
//! this translation lives here.

use embyr_core::domain::{document::FirestoreDocument, field_value::FieldValue};
use embyr_proto::agent::{value::ValueType, ArrayValue, Document, MapValue, Value};
use prost_types::Timestamp;

/// Convert a proto `Value` to a domain `FieldValue`.
pub fn proto_value_to_field_value(v: &Value) -> FieldValue {
    match &v.value_type {
        None => FieldValue::Null,
        Some(ValueType::NullValue(_)) => FieldValue::Null,
        Some(ValueType::BooleanValue(b)) => FieldValue::Boolean(*b),
        Some(ValueType::IntegerValue(i)) => FieldValue::Integer(*i),
        Some(ValueType::DoubleValue(d)) => FieldValue::Double(*d),
        Some(ValueType::StringValue(s)) => FieldValue::String(s.clone()),
        Some(ValueType::BytesValue(b)) => FieldValue::Bytes(b.clone()),
        Some(ValueType::ReferenceValue(r)) => FieldValue::Reference(r.clone()),
        Some(ValueType::TimestampValue(ts)) => FieldValue::Timestamp(ts.seconds, ts.nanos),
        Some(ValueType::ArrayValue(arr)) => {
            FieldValue::Array(arr.values.iter().map(proto_value_to_field_value).collect())
        }
        Some(ValueType::MapValue(map)) => FieldValue::Map(
            map.fields
                .iter()
                .map(|(k, v)| (k.clone(), proto_value_to_field_value(v)))
                .collect(),
        ),
    }
}

/// Convert a domain `FirestoreDocument` to the agent proto `Document`.
pub fn domain_doc_to_proto(doc: FirestoreDocument) -> Document {
    let name = format!(
        "projects/{}/databases/(default)/documents/{}/{}",
        doc.path.project_id.as_str(),
        doc.path.collection_path,
        doc.path.document_id,
    );
    Document {
        name,
        fields: doc
            .fields
            .iter()
            .map(|(k, v)| (k.clone(), field_value_to_proto(v)))
            .collect(),
        create_time: Some(Timestamp {
            seconds: doc.create_time.0,
            nanos: doc.create_time.1,
        }),
        update_time: Some(Timestamp {
            seconds: doc.update_time.0,
            nanos: doc.update_time.1,
        }),
        generation: doc.version,
    }
}

fn field_value_to_proto(fv: &FieldValue) -> Value {
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
            fields: map.iter().map(|(k, v)| (k.clone(), field_value_to_proto(v))).collect(),
        }),
    };
    Value { value_type: Some(vt) }
}
