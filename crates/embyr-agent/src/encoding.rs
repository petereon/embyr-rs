//! Encoding: `FirestoreDocument` → agent proto `Document`.
//!
//! Translates domain types from `embyr_core` to `embyr_proto::agent` types at
//! the agent gRPC boundary. The storage crate must not import proto types, so
//! this translation lives here.

use embyr_core::domain::query::{
    validate_field_path, AggregateValue, Aggregation, AggregationKind, MAX_AGGREGATIONS,
};
use embyr_core::domain::{document::FirestoreDocument, field_value::FieldValue};
use embyr_core::error::CoreError;
use embyr_proto::agent::{
    aggregation_op, aggregation_result, value::ValueType, AggregationField,
    AggregationOp, AggregationResult, ArrayValue, Document, MapValue, Value,
};
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

/// Translate the request's `AggregationOp`s to domain `Aggregation`s.
///
/// Aliases are positional (`"0"`, `"1"`, ...): the agent answers by position
/// and `embyr-server` owns the client-visible aliases. Field paths are
/// validated here because the SQL builder interpolates them (it panics on an
/// invalid one); `up_to` 0 means "no cap".
pub fn proto_aggregations_to_domain(
    ops: &[AggregationOp],
) -> Result<Vec<Aggregation>, CoreError> {
    if ops.len() > MAX_AGGREGATIONS {
        return Err(CoreError::InvalidArgument(format!(
            "at most {MAX_AGGREGATIONS} aggregations are allowed, got {}",
            ops.len()
        )));
    }
    let field = |f: &AggregationField| -> Result<String, CoreError> {
        validate_field_path(&f.field_path)?;
        Ok(f.field_path.clone())
    };
    ops.iter()
        .enumerate()
        .map(|(i, op)| {
            let kind = match &op.op {
                Some(aggregation_op::Op::Count(c)) if c.up_to < 0 => {
                    return Err(CoreError::InvalidArgument("count up_to must not be negative".into()))
                }
                Some(aggregation_op::Op::Count(c)) => {
                    AggregationKind::Count((c.up_to > 0).then_some(c.up_to))
                }
                Some(aggregation_op::Op::Sum(f)) => AggregationKind::Sum(field(f)?),
                Some(aggregation_op::Op::Avg(f)) => AggregationKind::Avg(field(f)?),
                None => return Err(CoreError::InvalidArgument("aggregation operator is required".into())),
            };
            Ok(Aggregation { alias: i.to_string(), kind })
        })
        .collect()
}

/// Domain `AggregateValue` -> wire `AggregationResult` (AVG over an empty set
/// is the one result with no `value` member set).
pub fn aggregate_value_to_proto(v: AggregateValue) -> AggregationResult {
    AggregationResult {
        value: match v {
            AggregateValue::Count(n) => Some(aggregation_result::Value::Count(n)),
            AggregateValue::Sum(d) => Some(aggregation_result::Value::Sum(d)),
            AggregateValue::Avg(d) => d.map(aggregation_result::Value::Avg),
        },
    }
}

#[cfg(test)]
mod aggregation_encoding_tests {
    use super::*;
    use embyr_proto::agent::AggregationCount;

    fn count(up_to: i64) -> AggregationOp {
        AggregationOp { op: Some(aggregation_op::Op::Count(AggregationCount { up_to })) }
    }
    fn sum(f: &str) -> AggregationOp {
        AggregationOp { op: Some(aggregation_op::Op::Sum(AggregationField { field_path: f.into() })) }
    }
    fn avg(f: &str) -> AggregationOp {
        AggregationOp { op: Some(aggregation_op::Op::Avg(AggregationField { field_path: f.into() })) }
    }

    #[test]
    fn ops_translate_in_order_with_zero_up_to_meaning_uncapped() {
        let got = proto_aggregations_to_domain(&[count(0), count(7), sum("a.b"), avg("c")]).unwrap();
        let kinds: Vec<_> = got.into_iter().map(|a| a.kind).collect();
        assert_eq!(
            kinds,
            [
                AggregationKind::Count(None),
                AggregationKind::Count(Some(7)),
                AggregationKind::Sum("a.b".into()),
                AggregationKind::Avg("c".into()),
            ]
        );
    }

    #[test]
    fn invalid_ops_are_rejected_before_reaching_the_sql_builder() {
        let too_many: Vec<_> = (0..=MAX_AGGREGATIONS).map(|_| count(0)).collect();
        for bad in [
            vec![sum("x'); DROP TABLE documents; --")],
            vec![avg("")],
            vec![count(-1)],
            vec![AggregationOp { op: None }],
            too_many,
        ] {
            assert!(matches!(proto_aggregations_to_domain(&bad), Err(CoreError::InvalidArgument(_))));
        }
    }

    #[test]
    fn values_map_to_results_with_empty_avg_as_unset() {
        use aggregation_result::Value;
        let got: Vec<_> = [
            AggregateValue::Count(3),
            AggregateValue::Sum(2.5),
            AggregateValue::Avg(Some(1.5)),
            AggregateValue::Avg(None),
        ]
        .into_iter()
        .map(|v| aggregate_value_to_proto(v).value)
        .collect();
        assert_eq!(got, [Some(Value::Count(3)), Some(Value::Sum(2.5)), Some(Value::Avg(1.5)), None]);
    }
}
