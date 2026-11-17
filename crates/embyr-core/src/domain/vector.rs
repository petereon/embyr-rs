//! Vector search (`findNearest`): exact brute-force k-nearest-neighbours over
//! documents already fetched by the normal (gated) query path. Pure, no IO.
//!
//! A Firestore vector value is a map `{"__type__": "__vector__", "value":
//! [f64...]}`; it is stored and returned as an ordinary `FieldValue::Map`, so
//! it round-trips byte-for-byte without any special storage support.
//!
//! ponytail: O(n * d) scan, ceiling `MAX_VECTOR_SCAN_DOCS` candidates per
//! query. Upgrade path: an optional pgvector index for customer databases that
//! have the extension (see docs/feature/vector-search/feature-delta.md).

use std::collections::BTreeMap;

use crate::domain::{document::FirestoreDocument, field_value::FieldValue, query::validate_field_path};
use crate::error::CoreError;

pub const VECTOR_TYPE_KEY: &str = "__type__";
pub const VECTOR_TYPE_TAG: &str = "__vector__";
pub const VECTOR_VALUE_KEY: &str = "value";
/// Firestore's maximum embedding dimension (queries and stored vectors).
pub const MAX_VECTOR_DIMENSION: usize = 2048;
/// Firestore's maximum `limit` of a `findNearest` query.
pub const MAX_NEAREST_LIMIT: usize = 1000;
/// Most candidate documents a vector query will scan; more is rejected
/// (`FailedPrecondition`) rather than answered from a truncated scan.
pub const MAX_VECTOR_SCAN_DOCS: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistanceMeasure {
    Euclidean,
    /// Cosine DISTANCE, `1 - cosine similarity`, in `[0, 2]`.
    Cosine,
    /// The dot product itself; larger is nearer.
    DotProduct,
}

/// A validated `findNearest` request.
#[derive(Debug, Clone)]
pub struct NearestQuery {
    vector_field: String,
    query_vector: Vec<f64>,
    measure: DistanceMeasure,
    limit: usize,
    distance_result_field: Option<String>,
    distance_threshold: Option<f64>,
}

impl NearestQuery {
    pub fn new(
        vector_field: &str,
        query_vector: Vec<f64>,
        measure: DistanceMeasure,
        limit: i64,
        distance_result_field: Option<&str>,
        distance_threshold: Option<f64>,
    ) -> Result<Self, CoreError> {
        let bad = |m: String| Err(CoreError::InvalidArgument(m));
        validate_field_path(vector_field)?;
        if let Some(f) = distance_result_field {
            validate_field_path(f)?;
        }
        if !(1..=MAX_NEAREST_LIMIT as i64).contains(&limit) {
            return bad(format!("find_nearest limit must be between 1 and {MAX_NEAREST_LIMIT}"));
        }
        if query_vector.is_empty() || query_vector.len() > MAX_VECTOR_DIMENSION {
            return bad(format!(
                "find_nearest query_vector dimension must be between 1 and {MAX_VECTOR_DIMENSION}"
            ));
        }
        if query_vector.iter().any(|x| !x.is_finite()) {
            return bad("find_nearest query_vector must contain only finite numbers".into());
        }
        if measure == DistanceMeasure::Cosine && norm(&query_vector) == 0.0 {
            return bad("COSINE distance is undefined for a zero query_vector".into());
        }
        if distance_threshold.is_some_and(f64::is_nan) {
            return bad("find_nearest distance_threshold must not be NaN".into());
        }
        Ok(Self {
            vector_field: vector_field.to_string(),
            query_vector,
            measure,
            limit: limit as usize,
            distance_result_field: distance_result_field.map(str::to_string),
            distance_threshold,
        })
    }
}

/// The components of a vector value, or `None` if `value` is not one (wrong
/// shape, non-numeric or non-finite component, empty or over-long).
pub fn vector_components(value: &FieldValue) -> Option<Vec<f64>> {
    let FieldValue::Map(m) = value else { return None };
    match m.get(VECTOR_TYPE_KEY) {
        Some(FieldValue::String(t)) if t == VECTOR_TYPE_TAG => {}
        _ => return None,
    }
    let FieldValue::Array(items) = m.get(VECTOR_VALUE_KEY)? else { return None };
    if items.is_empty() || items.len() > MAX_VECTOR_DIMENSION {
        return None;
    }
    items
        .iter()
        .map(|v| match v {
            FieldValue::Double(d) if d.is_finite() => Some(*d),
            FieldValue::Integer(i) => Some(*i as f64),
            _ => None,
        })
        .collect()
}

fn norm(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Distance (or dot product) between equal-length vectors; `None` if it is
/// undefined (length mismatch, zero-norm cosine operand, non-finite result).
pub fn distance(measure: DistanceMeasure, a: &[f64], b: &[f64]) -> Option<f64> {
    if a.len() != b.len() {
        return None;
    }
    let d = match measure {
        DistanceMeasure::Euclidean => {
            a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f64>().sqrt()
        }
        DistanceMeasure::DotProduct => dot(a, b),
        DistanceMeasure::Cosine => {
            let (na, nb) = (norm(a), norm(b));
            if na == 0.0 || nb == 0.0 {
                return None;
            }
            (1.0 - dot(a, b) / (na * nb)).clamp(0.0, 2.0)
        }
    };
    // `+ 0.0` turns -0.0 into 0.0 so ties compare equal under `total_cmp`.
    d.is_finite().then_some(d + 0.0)
}

/// The `limit` documents nearest to the query vector, nearest first (ties by
/// document name). Documents whose vector field is missing, not a vector, of a
/// different dimension, or with an undefined distance are excluded. When a
/// distance result field was requested it is written into each returned
/// document.
pub fn nearest(q: &NearestQuery, docs: Vec<FirestoreDocument>) -> Vec<FirestoreDocument> {
    let mut scored: Vec<(f64, FirestoreDocument)> = docs
        .into_iter()
        .filter_map(|doc| {
            let v = vector_components(get_path(&doc.fields, &q.vector_field)?)?;
            let d = distance(q.measure, &q.query_vector, &v)?;
            let keep = q.distance_threshold.is_none_or(|t| match q.measure {
                DistanceMeasure::DotProduct => d >= t,
                _ => d <= t,
            });
            keep.then_some((d, doc))
        })
        .collect();
    scored.sort_by(|(da, a), (db, b)| {
        let by_distance = match q.measure {
            DistanceMeasure::DotProduct => db.total_cmp(da),
            _ => da.total_cmp(db),
        };
        by_distance.then_with(|| name(a).cmp(&name(b)))
    });
    scored.truncate(q.limit);
    scored
        .into_iter()
        .map(|(d, mut doc)| {
            if let Some(f) = &q.distance_result_field {
                set_path(&mut doc.fields, f, FieldValue::Double(d));
            }
            doc
        })
        .collect()
}

fn name(d: &FirestoreDocument) -> (&str, &str) {
    (&d.path.collection_path, &d.path.document_id)
}

fn get_path<'a>(fields: &'a BTreeMap<String, FieldValue>, path: &str) -> Option<&'a FieldValue> {
    let mut parts = path.split('.');
    let mut cur = fields.get(parts.next()?)?;
    for p in parts {
        match cur {
            FieldValue::Map(m) => cur = m.get(p)?,
            _ => return None,
        }
    }
    Some(cur)
}

fn set_path(fields: &mut BTreeMap<String, FieldValue>, path: &str, v: FieldValue) {
    match path.split_once('.') {
        None => {
            fields.insert(path.to_string(), v);
        }
        Some((head, rest)) => {
            let e = fields.entry(head.to_string()).or_insert_with(|| FieldValue::Map(BTreeMap::new()));
            if !matches!(e, FieldValue::Map(_)) {
                *e = FieldValue::Map(BTreeMap::new());
            }
            if let FieldValue::Map(m) = e {
                set_path(m, rest, v);
            }
        }
    }
}

/// `Err` if `value` is a vector-tagged map whose `value` array exceeds
/// Firestore's 2048-dimension limit. Used by the document write limits.
pub fn check_vector_dimension(value: &FieldValue) -> Result<(), CoreError> {
    if let FieldValue::Map(m) = value {
        if matches!(m.get(VECTOR_TYPE_KEY), Some(FieldValue::String(t)) if t == VECTOR_TYPE_TAG) {
            if let Some(FieldValue::Array(items)) = m.get(VECTOR_VALUE_KEY) {
                if items.len() > MAX_VECTOR_DIMENSION {
                    return Err(CoreError::InvalidArgument(format!(
                        "vector dimension {} exceeds the maximum of {MAX_VECTOR_DIMENSION}",
                        items.len()
                    )));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{document::DocumentPath, project::ProjectId};
    use proptest::prelude::*;

    fn vec_value(xs: &[f64]) -> FieldValue {
        FieldValue::Map(BTreeMap::from([
            (VECTOR_TYPE_KEY.to_string(), FieldValue::String(VECTOR_TYPE_TAG.to_string())),
            (
                VECTOR_VALUE_KEY.to_string(),
                FieldValue::Array(xs.iter().map(|x| FieldValue::Double(*x)).collect()),
            ),
        ]))
    }

    fn doc(id: &str, fields: Vec<(&str, FieldValue)>) -> FirestoreDocument {
        FirestoreDocument {
            path: DocumentPath {
                project_id: ProjectId::new("p").unwrap(),
                collection_path: "c".into(),
                document_id: id.into(),
            },
            fields: fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            create_time: (0, 0),
            update_time: (0, 0),
            version: 1,
        }
    }

    fn vdoc(id: &str, xs: &[f64]) -> FirestoreDocument {
        doc(id, vec![("emb", vec_value(xs))])
    }

    fn q(measure: DistanceMeasure, limit: i64, qv: &[f64]) -> NearestQuery {
        NearestQuery::new("emb", qv.to_vec(), measure, limit, None, None).unwrap()
    }

    fn ids(docs: &[FirestoreDocument]) -> Vec<&str> {
        docs.iter().map(|d| d.path.document_id.as_str()).collect()
    }

    #[test]
    fn known_distances() {
        use DistanceMeasure::*;
        assert_eq!(distance(Euclidean, &[0.0, 0.0], &[3.0, 4.0]), Some(5.0));
        assert_eq!(distance(DotProduct, &[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]), Some(32.0));
        assert_eq!(distance(Cosine, &[1.0, 0.0], &[2.0, 0.0]), Some(0.0)); // same direction
        assert_eq!(distance(Cosine, &[1.0, 0.0], &[0.0, 5.0]), Some(1.0)); // orthogonal
        assert_eq!(distance(Cosine, &[1.0, 0.0], &[-3.0, 0.0]), Some(2.0)); // opposite
    }

    #[test]
    fn undefined_distances_are_none() {
        use DistanceMeasure::*;
        assert_eq!(distance(Euclidean, &[1.0], &[1.0, 2.0]), None);
        assert_eq!(distance(Cosine, &[0.0, 0.0], &[1.0, 0.0]), None);
        assert_eq!(distance(DotProduct, &[f64::MAX], &[f64::MAX]), None); // overflow
    }

    #[test]
    fn euclidean_orders_nearest_first_and_truncates() {
        let docs = vec![vdoc("far", &[9.0, 9.0]), vdoc("near", &[1.0, 0.0]), vdoc("mid", &[3.0, 3.0])];
        let out = nearest(&q(DistanceMeasure::Euclidean, 2, &[0.0, 0.0]), docs);
        assert_eq!(ids(&out), ["near", "mid"]);
    }

    #[test]
    fn cosine_ignores_magnitude() {
        let docs = vec![vdoc("big_off", &[100.0, 50.0]), vdoc("tiny_on", &[0.001, 0.0])];
        let out = nearest(&q(DistanceMeasure::Cosine, 2, &[1.0, 0.0]), docs);
        assert_eq!(ids(&out), ["tiny_on", "big_off"]);
    }

    #[test]
    fn dot_product_larger_is_nearer() {
        let docs = vec![vdoc("a", &[1.0, 1.0]), vdoc("b", &[5.0, 5.0]), vdoc("c", &[-1.0, -1.0])];
        let out = nearest(&q(DistanceMeasure::DotProduct, 3, &[1.0, 1.0]), docs);
        assert_eq!(ids(&out), ["b", "a", "c"]);
    }

    #[test]
    fn missing_wrong_type_wrong_dimension_and_bad_components_are_excluded() {
        let not_vector_map = FieldValue::Map(BTreeMap::from([(
            VECTOR_VALUE_KEY.to_string(),
            FieldValue::Array(vec![FieldValue::Double(0.0), FieldValue::Double(0.0)]),
        )]));
        let docs = vec![
            vdoc("ok", &[1.0, 1.0]),
            doc("missing", vec![("other", FieldValue::Integer(1))]),
            doc("string", vec![("emb", FieldValue::String("x".into()))]),
            doc("plain_array", vec![("emb", FieldValue::Array(vec![FieldValue::Double(0.0); 2]))]),
            doc("untagged", vec![("emb", not_vector_map)]),
            vdoc("dim3", &[1.0, 1.0, 1.0]),
            vdoc("dim1", &[1.0]),
            doc(
                "nan",
                vec![(
                    "emb",
                    FieldValue::Map(BTreeMap::from([
                        (VECTOR_TYPE_KEY.to_string(), FieldValue::String(VECTOR_TYPE_TAG.into())),
                        (
                            VECTOR_VALUE_KEY.to_string(),
                            FieldValue::Array(vec![FieldValue::Double(f64::NAN), FieldValue::Double(0.0)]),
                        ),
                    ])),
                )],
            ),
        ];
        let out = nearest(&q(DistanceMeasure::Euclidean, 10, &[0.0, 0.0]), docs);
        assert_eq!(ids(&out), ["ok"]);
    }

    #[test]
    fn zero_norm_document_vector_excluded_for_cosine_only() {
        let docs = vec![vdoc("zero", &[0.0, 0.0]), vdoc("one", &[1.0, 0.0])];
        assert_eq!(ids(&nearest(&q(DistanceMeasure::Cosine, 5, &[1.0, 0.0]), docs.clone())), ["one"]);
        assert_eq!(ids(&nearest(&q(DistanceMeasure::Euclidean, 5, &[1.0, 0.0]), docs)), ["one", "zero"]);
    }

    #[test]
    fn threshold_semantics_per_measure() {
        let docs = vec![vdoc("a", &[1.0, 0.0]), vdoc("b", &[2.0, 0.0]), vdoc("c", &[3.0, 0.0])];
        let mk = |m, t| NearestQuery::new("emb", vec![0.0, 0.0], m, 10, None, Some(t)).unwrap();
        // euclid distances 1,2,3: keep <= 2 (boundary inclusive)
        assert_eq!(ids(&nearest(&mk(DistanceMeasure::Euclidean, 2.0), docs.clone())), ["a", "b"]);
        // dot with [1,0]: 1,2,3: keep >= 2 (boundary inclusive), largest first
        let dq = NearestQuery::new("emb", vec![1.0, 0.0], DistanceMeasure::DotProduct, 10, None, Some(2.0)).unwrap();
        assert_eq!(ids(&nearest(&dq, docs.clone())), ["c", "b"]);
        // cosine distance 0 for all (same direction as [1,0]); threshold below 0 keeps none
        let cq = NearestQuery::new("emb", vec![1.0, 0.0], DistanceMeasure::Cosine, 10, None, Some(-0.5)).unwrap();
        assert!(nearest(&cq, docs).is_empty());
    }

    #[test]
    fn ties_break_by_document_name() {
        let docs = vec![vdoc("b", &[1.0, 0.0]), vdoc("a", &[0.0, 1.0]), vdoc("c", &[-1.0, 0.0])];
        let out = nearest(&q(DistanceMeasure::Euclidean, 2, &[0.0, 0.0]), docs);
        assert_eq!(ids(&out), ["a", "b"]);
    }

    #[test]
    fn distance_result_field_is_written_top_level_and_nested() {
        let docs = vec![vdoc("a", &[3.0, 4.0])];
        let top = NearestQuery::new("emb", vec![0.0, 0.0], DistanceMeasure::Euclidean, 1, Some("dist"), None).unwrap();
        let out = nearest(&top, docs.clone());
        assert!(matches!(out[0].fields.get("dist"), Some(FieldValue::Double(d)) if *d == 5.0));
        assert!(out[0].fields.contains_key("emb")); // the vector itself is untouched
        let nested = NearestQuery::new("emb", vec![0.0, 0.0], DistanceMeasure::Euclidean, 1, Some("m.d"), None).unwrap();
        let out = nearest(&nested, docs);
        let Some(FieldValue::Map(m)) = out[0].fields.get("m") else { panic!("m missing") };
        assert!(matches!(m.get("d"), Some(FieldValue::Double(d)) if *d == 5.0));
    }

    #[test]
    fn vector_field_can_be_nested() {
        let nested = FieldValue::Map(BTreeMap::from([("v".to_string(), vec_value(&[1.0, 0.0]))]));
        let d = doc("a", vec![("meta", nested)]);
        let nq = NearestQuery::new("meta.v", vec![1.0, 0.0], DistanceMeasure::Euclidean, 1, None, None).unwrap();
        assert_eq!(ids(&nearest(&nq, vec![d])), ["a"]);
    }

    #[test]
    fn integer_components_are_accepted() {
        let v = FieldValue::Map(BTreeMap::from([
            (VECTOR_TYPE_KEY.to_string(), FieldValue::String(VECTOR_TYPE_TAG.into())),
            (VECTOR_VALUE_KEY.to_string(), FieldValue::Array(vec![FieldValue::Integer(3), FieldValue::Double(4.0)])),
        ]));
        assert_eq!(vector_components(&v), Some(vec![3.0, 4.0]));
    }

    #[test]
    fn request_validation() {
        let ok = |lim, qv: Vec<f64>, m| NearestQuery::new("emb", qv, m, lim, None, None);
        use DistanceMeasure::*;
        assert!(ok(1, vec![1.0], Euclidean).is_ok());
        assert!(ok(1000, vec![1.0; 2048], Euclidean).is_ok());
        assert!(ok(0, vec![1.0], Euclidean).is_err());
        assert!(ok(-1, vec![1.0], Euclidean).is_err());
        assert!(ok(1001, vec![1.0], Euclidean).is_err());
        assert!(ok(1, vec![], Euclidean).is_err());
        assert!(ok(1, vec![1.0; 2049], Euclidean).is_err());
        assert!(ok(1, vec![f64::NAN], Euclidean).is_err());
        assert!(ok(1, vec![f64::INFINITY], DotProduct).is_err());
        assert!(ok(1, vec![0.0, 0.0], Cosine).is_err());
        assert!(ok(1, vec![0.0, 0.0], Euclidean).is_ok());
        assert!(NearestQuery::new("bad path!", vec![1.0], Euclidean, 1, None, None).is_err());
        assert!(NearestQuery::new("emb", vec![1.0], Euclidean, 1, Some("a b"), None).is_err());
        assert!(NearestQuery::new("emb", vec![1.0], Euclidean, 1, None, Some(f64::NAN)).is_err());
    }

    #[test]
    fn write_dimension_limit() {
        assert!(check_vector_dimension(&vec_value(&vec![0.0; 2048])).is_ok());
        assert!(check_vector_dimension(&vec_value(&vec![0.0; 2049])).is_err());
        // an untagged map or plain array is never a vector
        assert!(check_vector_dimension(&FieldValue::Array(vec![FieldValue::Null; 5000])).is_ok());
    }

    fn measure() -> impl Strategy<Value = DistanceMeasure> {
        prop_oneof![
            Just(DistanceMeasure::Euclidean),
            Just(DistanceMeasure::Cosine),
            Just(DistanceMeasure::DotProduct)
        ]
    }

    proptest! {
        /// Result is at most `limit`, every document has the query dimension,
        /// and distances come back sorted nearest first.
        #[test]
        fn top_k_is_bounded_and_sorted(
            m in measure(),
            limit in 1i64..8,
            qv in proptest::collection::vec(-10.0f64..10.0, 3),
            vecs in proptest::collection::vec(proptest::collection::vec(-10.0f64..10.0, 1..5), 0..20),
        ) {
            prop_assume!(m != DistanceMeasure::Cosine || norm(&qv) > 0.0);
            let docs: Vec<_> = vecs.iter().enumerate().map(|(i, v)| vdoc(&format!("d{i:02}"), v)).collect();
            let nq = NearestQuery::new("emb", qv.clone(), m, limit, Some("dist"), None).unwrap();
            let out = nearest(&nq, docs);
            prop_assert!(out.len() <= limit as usize);
            let ds: Vec<f64> = out.iter().map(|d| match d.fields.get("dist") {
                Some(FieldValue::Double(x)) => *x,
                _ => panic!("dist missing"),
            }).collect();
            for d in &out {
                prop_assert_eq!(vector_components(d.fields.get("emb").unwrap()).unwrap().len(), qv.len());
            }
            for w in ds.windows(2) {
                match m {
                    DistanceMeasure::DotProduct => prop_assert!(w[0] >= w[1]),
                    _ => prop_assert!(w[0] <= w[1]),
                }
            }
        }

        /// No excluded document beats the worst returned one (exactness), and
        /// a threshold never lets a violating document through.
        #[test]
        fn exact_and_threshold_respected(
            m in measure(),
            qv in proptest::collection::vec(-10.0f64..10.0, 2),
            vecs in proptest::collection::vec(proptest::collection::vec(-10.0f64..10.0, 2), 1..20),
            thr in -20.0f64..20.0,
        ) {
            prop_assume!(m != DistanceMeasure::Cosine || norm(&qv) > 0.0);
            let docs: Vec<_> = vecs.iter().enumerate().map(|(i, v)| vdoc(&format!("d{i:02}"), v)).collect();
            let nq = NearestQuery::new("emb", qv.clone(), m, 3, None, Some(thr)).unwrap();
            let out = nearest(&nq, docs.clone());
            let passes = |d: f64| match m { DistanceMeasure::DotProduct => d >= thr, _ => d <= thr };
            let all: Vec<f64> = vecs.iter().filter_map(|v| distance(m, &qv, v)).filter(|d| passes(*d)).collect();
            prop_assert_eq!(out.len(), all.len().min(3));
            for d in &out {
                let v = vector_components(d.fields.get("emb").unwrap()).unwrap();
                prop_assert!(passes(distance(m, &qv, &v).unwrap()));
            }
        }
    }
}
