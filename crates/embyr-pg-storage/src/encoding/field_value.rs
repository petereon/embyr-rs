use base64::{engine::general_purpose::STANDARD, Engine};
use embyr_core::domain::field_value::FieldValue;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Serialize a `FieldValue` to its discriminated-union JSON representation.
///
/// Format: `{"t": "<discriminator>", ...}` — see DESIGN_CONTEXT for full schema.
pub fn field_value_to_json(fv: &FieldValue) -> Value {
    match fv {
        FieldValue::Null => json!({"t": "N"}),
        FieldValue::Boolean(b) => json!({"t": "B", "v": b}),
        FieldValue::Integer(i) => json!({"t": "I", "v": i}),
        FieldValue::Double(d) => {
            if d.is_nan() {
                json!({"t": "D", "v": "NaN"})
            } else if d.is_infinite() {
                json!({"t": "D", "v": if *d > 0.0 { "Inf" } else { "-Inf" }})
            } else {
                json!({"t": "D", "v": d})
            }
        }
        FieldValue::String(s) => json!({"t": "S", "v": s}),
        FieldValue::Bytes(b) => json!({"t": "BY", "v": STANDARD.encode(b)}),
        FieldValue::Reference(r) => json!({"t": "R", "v": r}),
        FieldValue::Timestamp(s, n) => json!({"t": "TS", "s": s, "n": n}),
        FieldValue::Array(arr) => {
            let vals: Vec<Value> = arr.iter().map(field_value_to_json).collect();
            json!({"t": "A", "v": vals})
        }
        FieldValue::Map(map) => {
            let obj: serde_json::Map<String, Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), field_value_to_json(v)))
                .collect();
            json!({"t": "M", "v": obj})
        }
    }
}

/// Deserialize a discriminated-union JSON value to a `FieldValue`.
///
/// Returns `None` if the value does not conform to the expected schema.
pub fn json_to_field_value(v: &Value) -> Option<FieldValue> {
    let t = v.get("t")?.as_str()?;
    match t {
        "N" => Some(FieldValue::Null),
        "B" => Some(FieldValue::Boolean(v.get("v")?.as_bool()?)),
        "I" => Some(FieldValue::Integer(v.get("v")?.as_i64()?)),
        "D" => {
            let val = v.get("v")?;
            if let Some(s) = val.as_str() {
                match s {
                    "NaN" => Some(FieldValue::Double(f64::NAN)),
                    "Inf" => Some(FieldValue::Double(f64::INFINITY)),
                    "-Inf" => Some(FieldValue::Double(f64::NEG_INFINITY)),
                    _ => None,
                }
            } else {
                Some(FieldValue::Double(val.as_f64()?))
            }
        }
        "S" => Some(FieldValue::String(v.get("v")?.as_str()?.to_string())),
        "R" => Some(FieldValue::Reference(v.get("v")?.as_str()?.to_string())),
        "BY" => {
            let b64 = v.get("v")?.as_str()?;
            let bytes = STANDARD.decode(b64).ok()?;
            Some(FieldValue::Bytes(bytes))
        }
        "TS" => {
            let s = v.get("s")?.as_i64()?;
            let n = v.get("n")?.as_i64()? as i32;
            Some(FieldValue::Timestamp(s, n))
        }
        "A" => {
            let arr = v.get("v")?.as_array()?;
            let vals: Option<Vec<_>> = arr.iter().map(json_to_field_value).collect();
            Some(FieldValue::Array(vals?))
        }
        "M" => {
            let obj = v.get("v")?.as_object()?;
            let mut map = BTreeMap::new();
            for (k, val) in obj {
                map.insert(k.clone(), json_to_field_value(val)?);
            }
            Some(FieldValue::Map(map))
        }
        _ => None,
    }
}

/// Serialize a document's field map to a JSON object of discriminated-union values.
pub fn fields_to_json(fields: &BTreeMap<String, FieldValue>) -> Value {
    let obj: serde_json::Map<String, Value> = fields
        .iter()
        .map(|(k, v)| (k.clone(), field_value_to_json(v)))
        .collect();
    Value::Object(obj)
}

/// Deserialize a JSON object of discriminated-union values to a field map.
///
/// Returns `None` if any value fails to deserialize.
pub fn json_to_fields(v: &Value) -> Option<BTreeMap<String, FieldValue>> {
    let obj = v.as_object()?;
    let mut map = BTreeMap::new();
    for (k, val) in obj {
        map.insert(k.clone(), json_to_field_value(val)?);
    }
    Some(map)
}
