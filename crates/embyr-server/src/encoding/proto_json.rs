//! proto3-JSON <-> prost conversions for the Firestore messages the web SDK
//! sends/receives over BrowserChannel (rest/browser_channel.rs).
//!
//! Hand-written subset (no pbjson dependency): `ListenRequest`, `WriteRequest`
//! in; `ListenResponse`, `WriteResponse`, error status out. Field names are
//! the proto3-JSON camelCase ones; int64 is a decimal string; bytes are
//! standard base64; Timestamps are RFC 3339 strings; enums are value names.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chrono::{DateTime, SecondsFormat};
use embyr_proto::firestore::{
    document_transform::{field_transform::TransformType, FieldTransform, ServerValue},
    listen_request, listen_response,
    precondition::ConditionType,
    structured_query::{
        composite_filter, field_filter, filter::FilterType, unary_filter, CollectionSelector,
        CompositeFilter, Direction, FieldFilter, FieldReference, Filter, Order, Projection,
        UnaryFilter,
    },
    target::{query_target, DocumentsTarget, QueryTarget, ResumeType, TargetType},
    value::ValueType,
    write::Operation,
    ArrayValue, Cursor, Document, DocumentMask, LatLng, ListenRequest, ListenResponse, MapValue,
    Precondition, StructuredQuery, Target, Value, Write, WriteRequest, WriteResponse,
};
use prost_types::Timestamp;
use serde_json::{json, Map, Value as J};
use std::collections::HashMap;

type R<T> = Result<T, String>;

// ── JSON accessors ──────────────────────────────────────────────────────────

fn obj(v: &J) -> R<&Map<String, J>> {
    v.as_object().ok_or_else(|| format!("expected object, got {v}"))
}

fn text(v: &J) -> R<String> {
    v.as_str().map(str::to_owned).ok_or_else(|| format!("expected string, got {v}"))
}

fn opt_text(m: &Map<String, J>, k: &str) -> R<String> {
    m.get(k).map(text).transpose().map(Option::unwrap_or_default)
}

/// int32/int64: JSON number or decimal string.
fn int(v: &J) -> R<i64> {
    match v {
        J::Number(n) => n.as_i64().ok_or_else(|| format!("expected integer, got {v}")),
        J::String(s) => s.parse().map_err(|_| format!("expected integer, got {v}")),
        _ => Err(format!("expected integer, got {v}")),
    }
}

fn int32(v: &J) -> R<i32> {
    i32::try_from(int(v)?).map_err(|_| format!("int32 out of range: {v}"))
}

fn list<'a>(m: &'a Map<String, J>, k: &str) -> R<&'a [J]> {
    match m.get(k) {
        None | Some(J::Null) => Ok(&[]),
        Some(J::Array(a)) => Ok(a),
        Some(v) => Err(format!("{k}: expected array, got {v}")),
    }
}

fn bytes(m: &Map<String, J>, k: &str) -> R<Vec<u8>> {
    match m.get(k) {
        None => Ok(vec![]),
        Some(v) => B64.decode(text(v)?).map_err(|e| format!("{k}: bad base64: {e}")),
    }
}

fn timestamp(v: &J) -> R<Timestamp> {
    let dt = DateTime::parse_from_rfc3339(&text(v)?).map_err(|e| format!("bad timestamp: {e}"))?;
    Ok(Timestamp { seconds: dt.timestamp(), nanos: dt.timestamp_subsec_nanos() as i32 })
}

fn timestamp_json(t: &Timestamp) -> J {
    let s = DateTime::from_timestamp(t.seconds, t.nanos.max(0) as u32)
        .map(|d| d.to_rfc3339_opts(SecondsFormat::AutoSi, true))
        .unwrap_or_default();
    J::String(s)
}

fn enum_of<T>(m: &Map<String, J>, k: &str, parse: fn(&str) -> Option<T>) -> R<T> {
    let s = text(m.get(k).ok_or_else(|| format!("missing {k}"))?)?;
    parse(&s).ok_or_else(|| format!("{k}: unknown enum value {s}"))
}

fn field_ref(v: &J) -> R<FieldReference> {
    Ok(FieldReference { field_path: opt_text(obj(v)?, "fieldPath")? })
}

// ── Value ───────────────────────────────────────────────────────────────────

fn fields_from(v: Option<&J>) -> R<HashMap<String, Value>> {
    match v {
        None | Some(J::Null) => Ok(HashMap::new()),
        Some(v) => obj(v)?.iter().map(|(k, v)| Ok((k.clone(), value_from(v)?))).collect(),
    }
}

fn values_from(m: &Map<String, J>) -> R<Vec<Value>> {
    list(m, "values")?.iter().map(value_from).collect()
}

pub fn value_from(v: &J) -> R<Value> {
    let m = obj(v)?;
    let (k, x) = m.iter().next().ok_or("empty Value")?;
    let vt = match k.as_str() {
        "nullValue" => ValueType::NullValue(0),
        "booleanValue" => ValueType::BooleanValue(x.as_bool().ok_or("booleanValue: not a bool")?),
        "integerValue" => ValueType::IntegerValue(int(x)?),
        "doubleValue" => ValueType::DoubleValue(match x {
            J::Number(n) => n.as_f64().ok_or("doubleValue: bad number")?,
            J::String(s) => match s.as_str() {
                "NaN" => f64::NAN,
                "Infinity" => f64::INFINITY,
                "-Infinity" => f64::NEG_INFINITY,
                _ => return Err(format!("doubleValue: bad string {s}")),
            },
            _ => return Err("doubleValue: not a number".into()),
        }),
        "timestampValue" => ValueType::TimestampValue(timestamp(x)?),
        "stringValue" => ValueType::StringValue(text(x)?),
        "bytesValue" => ValueType::BytesValue(B64.decode(text(x)?).map_err(|e| e.to_string())?),
        "referenceValue" => ValueType::ReferenceValue(text(x)?),
        "geoPointValue" => {
            let g = obj(x)?;
            let f = |k: &str| g.get(k).and_then(J::as_f64).unwrap_or(0.0);
            ValueType::GeoPointValue(LatLng { latitude: f("latitude"), longitude: f("longitude") })
        }
        "arrayValue" => ValueType::ArrayValue(ArrayValue { values: values_from(obj(x)?)? }),
        "mapValue" => ValueType::MapValue(MapValue { fields: fields_from(obj(x)?.get("fields"))? }),
        other => return Err(format!("unknown Value type {other}")),
    };
    Ok(Value { value_type: Some(vt) })
}

fn fields_json(f: &HashMap<String, Value>) -> J {
    J::Object(f.iter().map(|(k, v)| (k.clone(), value_json(v))).collect())
}

pub fn value_json(v: &Value) -> J {
    match v.value_type.as_ref() {
        None | Some(ValueType::NullValue(_)) => json!({"nullValue": null}),
        Some(ValueType::BooleanValue(b)) => json!({"booleanValue": b}),
        Some(ValueType::IntegerValue(i)) => json!({"integerValue": i.to_string()}),
        Some(ValueType::DoubleValue(d)) => match serde_json::Number::from_f64(*d) {
            Some(n) => json!({"doubleValue": n}),
            None if d.is_nan() => json!({"doubleValue": "NaN"}),
            None if *d > 0.0 => json!({"doubleValue": "Infinity"}),
            None => json!({"doubleValue": "-Infinity"}),
        },
        Some(ValueType::TimestampValue(t)) => json!({"timestampValue": timestamp_json(t)}),
        Some(ValueType::StringValue(s)) => json!({"stringValue": s}),
        Some(ValueType::BytesValue(b)) => json!({"bytesValue": B64.encode(b)}),
        Some(ValueType::ReferenceValue(r)) => json!({"referenceValue": r}),
        Some(ValueType::GeoPointValue(g)) => {
            json!({"geoPointValue": {"latitude": g.latitude, "longitude": g.longitude}})
        }
        Some(ValueType::ArrayValue(a)) => {
            json!({"arrayValue": {"values": a.values.iter().map(value_json).collect::<Vec<_>>()}})
        }
        Some(ValueType::MapValue(m)) => json!({"mapValue": {"fields": fields_json(&m.fields)}}),
    }
}

fn document_json(d: &Document) -> J {
    let mut o = json!({"name": d.name, "fields": fields_json(&d.fields)});
    if let Some(t) = &d.create_time {
        o["createTime"] = timestamp_json(t);
    }
    if let Some(t) = &d.update_time {
        o["updateTime"] = timestamp_json(t);
    }
    o
}

// ── Query / Target ──────────────────────────────────────────────────────────

fn filter_from(v: &J) -> R<Filter> {
    let m = obj(v)?;
    let ft = if let Some(c) = m.get("compositeFilter") {
        let c = obj(c)?;
        FilterType::CompositeFilter(CompositeFilter {
            op: enum_of(c, "op", composite_filter::Operator::from_str_name)? as i32,
            filters: list(c, "filters")?.iter().map(filter_from).collect::<R<_>>()?,
        })
    } else if let Some(f) = m.get("fieldFilter") {
        let f = obj(f)?;
        FilterType::FieldFilter(FieldFilter {
            field: Some(field_ref(f.get("field").ok_or("fieldFilter: missing field")?)?),
            op: enum_of(f, "op", field_filter::Operator::from_str_name)? as i32,
            value: Some(value_from(f.get("value").ok_or("fieldFilter: missing value")?)?),
        })
    } else if let Some(u) = m.get("unaryFilter") {
        let u = obj(u)?;
        FilterType::UnaryFilter(UnaryFilter {
            op: enum_of(u, "op", unary_filter::Operator::from_str_name)? as i32,
            operand_type: Some(unary_filter::OperandType::Field(field_ref(
                u.get("field").ok_or("unaryFilter: missing field")?,
            )?)),
        })
    } else {
        return Err("unsupported filter".into());
    };
    Ok(Filter { filter_type: Some(ft) })
}

fn cursor_from(v: &J) -> R<Cursor> {
    let m = obj(v)?;
    Ok(Cursor { values: values_from(m)?, before: m.get("before").and_then(J::as_bool).unwrap_or(false) })
}

fn query_from(v: &J) -> R<StructuredQuery> {
    let m = obj(v)?;
    let from = list(m, "from")?
        .iter()
        .map(|c| {
            let c = obj(c)?;
            Ok(CollectionSelector {
                collection_id: opt_text(c, "collectionId")?,
                all_descendants: c.get("allDescendants").and_then(J::as_bool).unwrap_or(false),
            })
        })
        .collect::<R<_>>()?;
    let order_by = list(m, "orderBy")?
        .iter()
        .map(|o| {
            let o = obj(o)?;
            Ok(Order {
                field: Some(field_ref(o.get("field").ok_or("orderBy: missing field")?)?),
                direction: match o.get("direction") {
                    None => Direction::Ascending as i32,
                    Some(_) => enum_of(o, "direction", Direction::from_str_name)? as i32,
                },
            })
        })
        .collect::<R<_>>()?;
    let select = match m.get("select") {
        None => None,
        Some(s) => Some(Projection {
            fields: list(obj(s)?, "fields")?.iter().map(field_ref).collect::<R<_>>()?,
        }),
    };
    Ok(StructuredQuery {
        select,
        from,
        r#where: m.get("where").map(filter_from).transpose()?,
        order_by,
        start_at: m.get("startAt").map(cursor_from).transpose()?,
        end_at: m.get("endAt").map(cursor_from).transpose()?,
        offset: m.get("offset").map(int32).transpose()?.unwrap_or(0),
        limit: m.get("limit").map(int32).transpose()?,
    })
}

fn target_from(v: &J) -> R<Target> {
    let m = obj(v)?;
    let target_type = if let Some(q) = m.get("query") {
        let q = obj(q)?;
        Some(TargetType::Query(QueryTarget {
            parent: opt_text(q, "parent")?,
            query_type: Some(query_target::QueryType::StructuredQuery(query_from(
                q.get("structuredQuery").ok_or("query target: missing structuredQuery")?,
            )?)),
        }))
    } else if let Some(d) = m.get("documents") {
        let docs = list(obj(d)?, "documents")?.iter().map(text).collect::<R<_>>()?;
        Some(TargetType::Documents(DocumentsTarget { documents: docs }))
    } else {
        return Err("target needs query or documents".into());
    };
    let resume_type = if m.contains_key("resumeToken") {
        Some(ResumeType::ResumeToken(bytes(m, "resumeToken")?))
    } else {
        m.get("readTime").map(timestamp).transpose()?.map(ResumeType::ReadTime)
    };
    Ok(Target {
        target_id: m.get("targetId").map(int32).transpose()?.unwrap_or(0),
        once: m.get("once").and_then(J::as_bool).unwrap_or(false),
        target_type,
        resume_type,
    })
}

pub fn listen_request_from_json(v: &J) -> R<ListenRequest> {
    let m = obj(v)?;
    let target_change = if let Some(t) = m.get("addTarget") {
        Some(listen_request::TargetChange::AddTarget(target_from(t)?))
    } else {
        m.get("removeTarget").map(int32).transpose()?.map(listen_request::TargetChange::RemoveTarget)
    };
    Ok(ListenRequest {
        database: opt_text(m, "database")?,
        labels: m
            .get("labels")
            .map(|l| obj(l)?.iter().map(|(k, v)| Ok((k.clone(), text(v)?))).collect::<R<_>>())
            .transpose()?
            .unwrap_or_default(),
        target_change,
    })
}

// ── Write ───────────────────────────────────────────────────────────────────

fn field_transform_from(v: &J) -> R<FieldTransform> {
    let m = obj(v)?;
    let arr = |k: &str| -> R<ArrayValue> { Ok(ArrayValue { values: values_from(obj(&m[k])?)? }) };
    let tt = if m.contains_key("setToServerValue") {
        match enum_of(m, "setToServerValue", ServerValue::from_str_name)? {
            ServerValue::RequestTime => TransformType::SetToServerValue(ServerValue::RequestTime as i32),
            ServerValue::Unspecified => return Err("setToServerValue unspecified".into()),
        }
    } else if let Some(x) = m.get("increment") {
        TransformType::Increment(value_from(x)?)
    } else if let Some(x) = m.get("maximum") {
        TransformType::Maximum(value_from(x)?)
    } else if let Some(x) = m.get("minimum") {
        TransformType::Minimum(value_from(x)?)
    } else if m.contains_key("appendMissingElements") {
        TransformType::AppendMissingElements(arr("appendMissingElements")?)
    } else if m.contains_key("removeAllFromArray") {
        TransformType::RemoveAllFromArray(arr("removeAllFromArray")?)
    } else {
        return Err("unsupported field transform".into());
    };
    Ok(FieldTransform { field_path: opt_text(m, "fieldPath")?, transform_type: Some(tt) })
}

fn write_from(v: &J) -> R<Write> {
    let m = obj(v)?;
    let operation = if let Some(u) = m.get("update") {
        let u = obj(u)?;
        Some(Operation::Update(Document {
            name: opt_text(u, "name")?,
            fields: fields_from(u.get("fields"))?,
            ..Default::default()
        }))
    } else if let Some(d) = m.get("delete") {
        Some(Operation::Delete(text(d)?))
    } else {
        return Err("write needs update or delete".into());
    };
    let current_document = match m.get("currentDocument") {
        None => None,
        Some(c) => {
            let c = obj(c)?;
            let ct = if let Some(e) = c.get("exists") {
                ConditionType::Exists(e.as_bool().ok_or("currentDocument.exists: not a bool")?)
            } else if let Some(t) = c.get("updateTime") {
                ConditionType::UpdateTime(timestamp(t)?)
            } else {
                return Err("empty currentDocument".into());
            };
            Some(Precondition { condition_type: Some(ct) })
        }
    };
    Ok(Write {
        update_mask: match m.get("updateMask") {
            None => None,
            Some(u) => Some(DocumentMask {
                field_paths: list(obj(u)?, "fieldPaths")?.iter().map(text).collect::<R<_>>()?,
            }),
        },
        update_transforms: list(m, "updateTransforms")?
            .iter()
            .map(field_transform_from)
            .collect::<R<_>>()?,
        current_document,
        operation,
    })
}

pub fn write_request_from_json(v: &J) -> R<WriteRequest> {
    let m = obj(v)?;
    Ok(WriteRequest {
        database: opt_text(m, "database")?,
        stream_id: opt_text(m, "streamId")?,
        writes: list(m, "writes")?.iter().map(write_from).collect::<R<_>>()?,
        stream_token: bytes(m, "streamToken")?,
    })
}

// ── Responses out ───────────────────────────────────────────────────────────

fn ids(v: &[i32]) -> J {
    json!(v)
}

pub fn listen_response_json(r: &ListenResponse) -> J {
    match r.response_type.as_ref() {
        Some(listen_response::ResponseType::TargetChange(t)) => {
            let ty = embyr_proto::firestore::target_change::TargetChangeType::try_from(t.target_change_type)
                .map(|e| e.as_str_name())
                .unwrap_or("NO_CHANGE");
            let mut o = json!({"targetChangeType": ty, "targetIds": ids(&t.target_ids)});
            if !t.resume_token.is_empty() {
                o["resumeToken"] = J::String(B64.encode(&t.resume_token));
            }
            if let Some(c) = &t.cause {
                o["cause"] = json!({"code": c.code, "message": c.message});
            }
            if let Some(rt) = &t.read_time {
                o["readTime"] = timestamp_json(rt);
            }
            json!({"targetChange": o})
        }
        Some(listen_response::ResponseType::DocumentChange(c)) => json!({"documentChange": {
            "document": c.document.as_ref().map(document_json),
            "targetIds": ids(&c.target_ids),
            "removedTargetIds": ids(&c.removed_target_ids),
        }}),
        Some(listen_response::ResponseType::DocumentDelete(d)) => json!({"documentDelete": {
            "document": d.document,
            "removedTargetIds": ids(&d.removed_target_ids),
            "readTime": d.read_time.as_ref().map(timestamp_json),
        }}),
        Some(listen_response::ResponseType::DocumentRemove(d)) => json!({"documentRemove": {
            "document": d.document,
            "removedTargetIds": ids(&d.removed_target_ids),
            "readTime": d.read_time.as_ref().map(timestamp_json),
        }}),
        Some(listen_response::ResponseType::Filter(f)) => {
            json!({"filter": {"targetId": f.target_id, "count": f.count}})
        }
        None => json!({}),
    }
}

pub fn write_response_json(r: &WriteResponse) -> J {
    json!({
        "streamId": r.stream_id,
        "streamToken": B64.encode(&r.stream_token),
        "writeResults": r.write_results.iter().map(|w| json!({
            "updateTime": w.update_time.as_ref().map(timestamp_json),
            "transformResults": w.transform_results.iter().map(value_json).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "commitTime": r.commit_time.as_ref().map(timestamp_json),
    })
}

/// The shape the web SDK's WebChannel transport reads as a stream error:
/// `{"error": {"code", "message", "status"}}` (webchannel_connection.ts).
pub fn status_error_json(s: &tonic::Status) -> J {
    // tonic's Debug name ("PermissionDenied") -> canonical "PERMISSION_DENIED".
    let mut name = String::new();
    for (i, c) in format!("{:?}", s.code()).chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            name.push('_');
        }
        name.push(c.to_ascii_uppercase());
    }
    json!({"error": {"code": s.code() as i32, "message": s.message(), "status": name}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_round_trips_every_type() {
        let j = json!({"mapValue": {"fields": {
            "s": {"stringValue": "x"}, "i": {"integerValue": "-7"}, "d": {"doubleValue": 1.5},
            "n": {"nullValue": null}, "b": {"booleanValue": true},
            "t": {"timestampValue": "2024-01-02T03:04:05.123Z"},
            "y": {"bytesValue": "AQID"}, "r": {"referenceValue": "projects/p/databases/(default)/documents/a/b"},
            "a": {"arrayValue": {"values": [{"integerValue": "1"}]}},
            "nan": {"doubleValue": "NaN"},
        }}});
        assert_eq!(value_json(&value_from(&j).unwrap()), j);
    }

    #[test]
    fn listen_request_add_target_query() {
        let j = json!({"database": "projects/p/databases/(default)", "addTarget": {
            "query": {"parent": "projects/p/databases/(default)/documents", "structuredQuery": {
                "from": [{"collectionId": "c"}],
                "where": {"fieldFilter": {"field": {"fieldPath": "a"}, "op": "EQUAL", "value": {"integerValue": "1"}}},
                "orderBy": [{"field": {"fieldPath": "__name__"}, "direction": "ASCENDING"}],
                "limit": 5}},
            "targetId": 2, "resumeToken": "AQID"}});
        let r = listen_request_from_json(&j).unwrap();
        let Some(listen_request::TargetChange::AddTarget(t)) = r.target_change else { panic!() };
        assert_eq!(t.target_id, 2);
        assert_eq!(t.resume_type, Some(ResumeType::ResumeToken(vec![1, 2, 3])));
        let Some(TargetType::Query(q)) = t.target_type else { panic!() };
        let Some(query_target::QueryType::StructuredQuery(sq)) = q.query_type else { panic!() };
        assert_eq!(sq.limit, Some(5));
        assert_eq!(sq.from[0].collection_id, "c");
        assert!(matches!(
            sq.r#where.unwrap().filter_type,
            Some(FilterType::FieldFilter(ref f)) if f.op == field_filter::Operator::Equal as i32
        ));
    }

    #[test]
    fn listen_request_remove_target() {
        let r = listen_request_from_json(&json!({"database": "d", "removeTarget": 4})).unwrap();
        assert_eq!(r.target_change, Some(listen_request::TargetChange::RemoveTarget(4)));
    }

    #[test]
    fn write_request_update_with_mask_precondition_transform() {
        let j = json!({"streamToken": "AQID", "writes": [{
            "update": {"name": "projects/p/databases/(default)/documents/c/d",
                       "fields": {"a": {"stringValue": "x"}}},
            "updateMask": {"fieldPaths": ["a"]},
            "currentDocument": {"exists": false},
            "updateTransforms": [{"fieldPath": "t", "setToServerValue": "REQUEST_TIME"},
                                 {"fieldPath": "n", "increment": {"integerValue": "1"}}]}]});
        let r = write_request_from_json(&j).unwrap();
        assert_eq!(r.stream_token, vec![1, 2, 3]);
        let w = &r.writes[0];
        assert_eq!(w.update_mask.as_ref().unwrap().field_paths, vec!["a"]);
        assert_eq!(
            w.current_document.as_ref().unwrap().condition_type,
            Some(ConditionType::Exists(false))
        );
        assert_eq!(w.update_transforms.len(), 2);
        assert!(matches!(w.operation, Some(Operation::Update(_))));
    }

    #[test]
    fn write_request_delete_and_handshake() {
        let r = write_request_from_json(&json!({"database": "d"})).unwrap();
        assert!(r.writes.is_empty() && r.stream_id.is_empty() && r.stream_token.is_empty());
        let r = write_request_from_json(&json!({"writes": [{"delete": "x"}]})).unwrap();
        assert!(matches!(r.writes[0].operation, Some(Operation::Delete(ref d)) if d == "x"));
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert!(listen_request_from_json(&json!({"addTarget": {}})).is_err());
        assert!(write_request_from_json(&json!({"writes": [{}]})).is_err());
        assert!(value_from(&json!({"wat": 1})).is_err());
    }

    #[test]
    fn listen_response_target_change_and_document_change() {
        let r = ListenResponse {
            response_type: Some(listen_response::ResponseType::TargetChange(
                embyr_proto::firestore::TargetChange {
                    target_change_type: 3,
                    target_ids: vec![2],
                    resume_token: vec![1, 2, 3],
                    ..Default::default()
                },
            )),
        };
        assert_eq!(
            listen_response_json(&r),
            json!({"targetChange": {"targetChangeType": "CURRENT", "targetIds": [2], "resumeToken": "AQID"}})
        );
        let r = ListenResponse {
            response_type: Some(listen_response::ResponseType::DocumentChange(
                embyr_proto::firestore::DocumentChange {
                    document: Some(Document {
                        name: "n".into(),
                        update_time: Some(Timestamp { seconds: 0, nanos: 0 }),
                        ..Default::default()
                    }),
                    target_ids: vec![2],
                    ..Default::default()
                },
            )),
        };
        let j = listen_response_json(&r);
        assert_eq!(j["documentChange"]["document"]["updateTime"], "1970-01-01T00:00:00Z");
        assert_eq!(j["documentChange"]["targetIds"], json!([2]));
    }

    #[test]
    fn error_status_names() {
        let j = status_error_json(&tonic::Status::permission_denied("no"));
        assert_eq!(j["error"]["status"], "PERMISSION_DENIED");
        assert_eq!(j["error"]["code"], 7);
        assert_eq!(status_error_json(&tonic::Status::unauthenticated("x"))["error"]["status"], "UNAUTHENTICATED");
    }
}
