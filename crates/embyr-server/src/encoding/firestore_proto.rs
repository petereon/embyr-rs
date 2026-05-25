use std::collections::HashMap;

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
