//! P11 — RunQuery ORDER BY sorts by Firestore's typed order, not by text.
//!
//! Driving port: gRPC data port (:8080) — RunQuery RPC. Integers and doubles
//! compare numerically together (5 < 30, not "30" < "5"), mixed types order by
//! type first (null < bool < NaN < number < timestamp < string < bytes), and
//! documents lacking the ordered field are excluded. Cursors and limit must
//! agree with the same order.

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    run_query_request::QueryType,
    structured_query::{CollectionSelector, Direction, FieldReference, Order},
    value::ValueType,
    CreateDocumentRequest, Cursor, Document, RunQueryRequest, StructuredQuery, Value,
};
use embyr_server::{adapters::system_db::SystemDb, start_test_server};
use std::{collections::HashMap, sync::Arc};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

async fn start_postgres() -> (ContainerAsync<Postgres>, String) {
    let container = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("Failed to start Postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("Failed to get host port");
    (container, format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres"))
}

struct TestEnv {
    _sys_container: ContainerAsync<Postgres>,
    _cust_container: ContainerAsync<Postgres>,
    project_id: String,
    api_key: String,
    server: embyr_server::TestServer,
}

async fn setup(api_key: &str, project_id: &str) -> TestEnv {
    let (_sys_container, sys_url) = start_postgres().await;
    let (_cust_container, cust_url) = start_postgres().await;

    let system_db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    system_db.migrate().await.unwrap();

    let cust_pool = sqlx::PgPool::connect(&cust_url).await.unwrap();
    sqlx::migrate!("../../migrations/customer").run(&cust_pool).await.unwrap();

    let api_key_hash = argon2::hash_api_key(api_key.as_bytes()).unwrap();
    let pub_key = ecies::derive_public_key(api_key.as_bytes());
    let encrypted_dsn = ecies::encrypt(&pub_key, cust_url.as_bytes()).unwrap();

    let sys_pool = sqlx::PgPool::connect(&sys_url).await.unwrap();
    sqlx::query(
        "INSERT INTO projects \
         (id, status, backend_mode, api_key_hash_current, ecies_encrypted_dsn) \
         VALUES ($1, 'active', 'direct_pg', $2, $3)",
    )
    .bind(project_id)
    .bind(&api_key_hash)
    .bind(&encrypted_dsn)
    .execute(&sys_pool)
    .await
    .unwrap();

    let server = start_test_server(system_db).await;
    TestEnv {
        _sys_container,
        _cust_container,
        project_id: project_id.to_string(),
        api_key: api_key.to_string(),
        server,
    }
}

fn authed<T>(payload: T, api_key: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(payload);
    req.metadata_mut().insert("authorization", format!("bearer {api_key}").parse().unwrap());
    req
}

fn val(v: ValueType) -> Value {
    Value { value_type: Some(v) }
}

async fn seed(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    env: &TestEnv,
    collection: &str,
    doc_id: &str,
    fields: Vec<(&str, Value)>,
) {
    let req = authed(
        CreateDocumentRequest {
            parent: format!("projects/{}/databases/(default)/documents", env.project_id),
            collection_id: collection.to_string(),
            document_id: doc_id.to_string(),
            document: Some(Document {
                fields: fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect::<HashMap<_, _>>(),
                ..Default::default()
            }),
            ..Default::default()
        },
        &env.api_key,
    );
    client.create_document(req).await.expect("seed document should succeed");
}

/// RunQuery ordered by `score`; returns document ids in result order.
async fn ids_ordered_by_score(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    env: &TestEnv,
    collection: &str,
    direction: Direction,
    start_at: Option<Cursor>,
    limit: Option<i32>,
) -> Vec<String> {
    use tokio_stream::StreamExt;
    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: collection.to_string(), all_descendants: false }],
        order_by: vec![Order {
            field: Some(FieldReference { field_path: "score".to_string() }),
            direction: direction as i32,
        }],
        start_at,
        limit,
        ..Default::default()
    };
    let req = authed(
        RunQueryRequest {
            parent: format!("projects/{}/databases/(default)/documents", env.project_id),
            query_type: Some(QueryType::StructuredQuery(sq)),
            ..Default::default()
        },
        &env.api_key,
    );
    let stream = client.run_query(req).await.expect("RunQuery should succeed").into_inner();
    let responses: Vec<_> = stream.collect().await;
    responses
        .into_iter()
        .filter_map(|r| r.ok())
        .filter_map(|r| r.document)
        .map(|d| d.name.rsplit('/').next().unwrap().to_string())
        .collect()
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Integers 5, 30, 9, -1, 100 and double 7.5 order numerically (not
/// lexicographically); DESC reverses; limit and startAt cursors follow the
/// same order.
#[tokio::test]
async fn numeric_order_by_is_numeric_across_integers_and_doubles_with_limit_and_cursor() {
    let env = setup("test-sk-typed-order-01", "typed-order-project-01").await;
    let mut client = FirestoreClient::new(
        tonic::transport::Channel::from_shared(format!("http://{}", env.server.grpc_addr))
            .unwrap()
            .connect_lazy(),
    );
    for (id, v) in [
        ("i5", ValueType::IntegerValue(5)),
        ("i30", ValueType::IntegerValue(30)),
        ("i9", ValueType::IntegerValue(9)),
        ("im1", ValueType::IntegerValue(-1)),
        ("i100", ValueType::IntegerValue(100)),
        ("d7_5", ValueType::DoubleValue(7.5)),
    ] {
        seed(&mut client, &env, "nums", id, vec![("score", val(v))]).await;
    }

    let asc = ids_ordered_by_score(&mut client, &env, "nums", Direction::Ascending, None, None).await;
    assert_eq!(asc, strs(&["im1", "i5", "d7_5", "i9", "i30", "i100"]));

    let desc = ids_ordered_by_score(&mut client, &env, "nums", Direction::Descending, None, None).await;
    assert_eq!(desc, strs(&["i100", "i30", "i9", "d7_5", "i5", "im1"]));

    let limited = ids_ordered_by_score(&mut client, &env, "nums", Direction::Ascending, None, Some(3)).await;
    assert_eq!(limited, strs(&["im1", "i5", "d7_5"]));

    // startAt (inclusive) integer 9: skips the double 7.5 below it.
    let cursor = Cursor { values: vec![val(ValueType::IntegerValue(9))], before: true };
    let from_9 = ids_ordered_by_score(&mut client, &env, "nums", Direction::Ascending, Some(cursor), Some(2)).await;
    assert_eq!(from_9, strs(&["i9", "i30"]));

    // DESC startAt (inclusive) 30: the cursor walks downwards.
    let cursor = Cursor { values: vec![val(ValueType::IntegerValue(30))], before: true };
    let down = ids_ordered_by_score(&mut client, &env, "nums", Direction::Descending, Some(cursor), None).await;
    assert_eq!(down, strs(&["i30", "i9", "d7_5", "i5", "im1"]));
}

/// Mixed-type values order by Firestore's type order first
/// (null < bool < NaN < number < timestamp < string < bytes); a document
/// lacking the ordered field is excluded.
#[tokio::test]
async fn mixed_type_order_by_follows_firestore_type_order_and_excludes_missing_field() {
    let env = setup("test-sk-typed-order-02", "typed-order-project-02").await;
    let mut client = FirestoreClient::new(
        tonic::transport::Channel::from_shared(format!("http://{}", env.server.grpc_addr))
            .unwrap()
            .connect_lazy(),
    );
    for (id, v) in [
        ("s_abc", ValueType::StringValue("abc".to_string())),
        ("n_int", ValueType::IntegerValue(2)),
        ("b_true", ValueType::BooleanValue(true)),
        ("by", ValueType::BytesValue(vec![1u8].into())),
        ("ts", ValueType::TimestampValue(prost_types::Timestamp { seconds: 100, nanos: 0 })),
        ("n_dbl", ValueType::DoubleValue(1.5)),
        ("nan", ValueType::DoubleValue(f64::NAN)),
        ("null", ValueType::NullValue(0)),
        ("b_false", ValueType::BooleanValue(false)),
        ("s_ab", ValueType::StringValue("ab".to_string())),
    ] {
        seed(&mut client, &env, "mixed", id, vec![("score", val(v))]).await;
    }
    seed(&mut client, &env, "mixed", "no_score", vec![("other", val(ValueType::IntegerValue(1)))]).await;

    let asc = ids_ordered_by_score(&mut client, &env, "mixed", Direction::Ascending, None, None).await;
    assert_eq!(
        asc,
        strs(&["null", "b_false", "b_true", "nan", "n_dbl", "n_int", "ts", "s_ab", "s_abc", "by"])
    );

    let desc = ids_ordered_by_score(&mut client, &env, "mixed", Direction::Descending, None, None).await;
    let mut expected = strs(&["null", "b_false", "b_true", "nan", "n_dbl", "n_int", "ts", "s_ab", "s_abc", "by"]);
    expected.reverse();
    assert_eq!(desc, expected);
}

// ─── G-001: exact numeric order + implicit `__name__` tiebreak ────────────

use embyr_proto::firestore::structured_query::{
    field_filter::Operator as FieldOp, filter::FilterType, FieldFilter, Filter,
};

async fn client_for(env: &TestEnv) -> FirestoreClient<tonic::transport::Channel> {
    FirestoreClient::new(
        tonic::transport::Channel::from_shared(format!("http://{}", env.server.grpc_addr))
            .unwrap()
            .connect_lazy(),
    )
}

/// RunQuery with an arbitrary `StructuredQuery`; document ids in result order.
async fn run_ids(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    env: &TestEnv,
    sq: StructuredQuery,
) -> Vec<String> {
    use tokio_stream::StreamExt;
    let req = authed(
        RunQueryRequest {
            parent: format!("projects/{}/databases/(default)/documents", env.project_id),
            query_type: Some(QueryType::StructuredQuery(sq)),
            ..Default::default()
        },
        &env.api_key,
    );
    let stream = client.run_query(req).await.expect("RunQuery should succeed").into_inner();
    let responses: Vec<_> = stream.collect().await;
    responses
        .into_iter()
        .filter_map(|r| r.ok())
        .filter_map(|r| r.document)
        .map(|d| d.name.rsplit('/').next().unwrap().to_string())
        .collect()
}

fn order(field: &str, direction: Direction) -> Order {
    Order { field: Some(FieldReference { field_path: field.to_string() }), direction: direction as i32 }
}

fn from(collection: &str) -> Vec<CollectionSelector> {
    vec![CollectionSelector { collection_id: collection.to_string(), all_descendants: false }]
}

/// Integers around 2^53 and at i64::MAX sort by exact value, also against
/// doubles; equal values (integer 2^53 vs double 2^53, -0 vs 0) tie and fall
/// back to document-name order; infinities are the extremes.
#[tokio::test]
async fn integers_and_doubles_beyond_two_pow_53_sort_and_filter_exactly() {
    const P53: i64 = 1 << 53;
    let env = setup("test-sk-g001-num-01", "g001-num-project-01").await;
    let mut client = client_for(&env).await;
    for (id, v) in [
        ("i64max", ValueType::IntegerValue(i64::MAX)),
        ("i64max_1", ValueType::IntegerValue(i64::MAX - 1)),
        ("d2p63", ValueType::DoubleValue(9_223_372_036_854_775_808.0)), // i64::MAX as f64
        ("i53p3", ValueType::IntegerValue(P53 + 3)),
        ("d53p2", ValueType::DoubleValue((P53 + 2) as f64)),
        ("i53p1", ValueType::IntegerValue(P53 + 1)),
        ("i53", ValueType::IntegerValue(P53)),
        ("d53", ValueType::DoubleValue(P53 as f64)),
        ("im53m1", ValueType::IntegerValue(-P53 - 1)),
        ("i0", ValueType::IntegerValue(0)),
        ("d_zero", ValueType::DoubleValue(0.0)),
        ("d_negzero", ValueType::DoubleValue(-0.0)),
        ("d_neginf", ValueType::DoubleValue(f64::NEG_INFINITY)),
        ("d_inf", ValueType::DoubleValue(f64::INFINITY)),
        ("nan", ValueType::DoubleValue(f64::NAN)),
    ] {
        seed(&mut client, &env, "big", id, vec![("score", val(v))]).await;
    }

    let sq = StructuredQuery {
        from: from("big"),
        order_by: vec![order("score", Direction::Ascending)],
        ..Default::default()
    };
    assert_eq!(
        run_ids(&mut client, &env, sq).await,
        strs(&[
            "nan", "d_neginf", "im53m1", "d_negzero", "d_zero", "i0", "d53", "i53", "i53p1", "d53p2", "i53p3",
            "i64max_1", "i64max", "d2p63", "d_inf",
        ])
    );

    // DESC: exact reverse, equal values by name descending.
    let sq = StructuredQuery {
        from: from("big"),
        order_by: vec![order("score", Direction::Descending)],
        ..Default::default()
    };
    assert_eq!(
        run_ids(&mut client, &env, sq).await,
        strs(&[
            "d_inf", "d2p63", "i64max", "i64max_1", "i53p3", "d53p2", "i53p1", "i53", "d53", "i0", "d_zero",
            "d_negzero", "im53m1", "d_neginf", "nan",
        ])
    );

    // Range filters are exact too (NaN rows are left out of the assertions:
    // their range-filter behaviour is unchanged by this fix).
    let filtered = |v: ValueType, op: FieldOp| StructuredQuery {
        from: from("big"),
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(FieldReference { field_path: "score".to_string() }),
                op: op as i32,
                value: Some(val(v)),
            })),
        }),
        order_by: vec![order("score", Direction::Ascending)],
        ..Default::default()
    };
    // > integer 2^53 drops both 2^53 values but keeps 2^53 + 1 (float8 dropped it).
    let got: Vec<String> = run_ids(&mut client, &env, filtered(ValueType::IntegerValue(P53), FieldOp::GreaterThan))
        .await
        .into_iter()
        .filter(|id| id != "nan")
        .collect();
    assert_eq!(got, strs(&["i53p1", "d53p2", "i53p3", "i64max_1", "i64max", "d2p63", "d_inf"]));
    // < i64::MAX as a double: i64::MAX itself is below it, the double is not.
    let got = run_ids(
        &mut client,
        &env,
        filtered(ValueType::DoubleValue(9_223_372_036_854_775_808.0), FieldOp::LessThan),
    )
    .await;
    assert!(got.ends_with(&strs(&["i53p3", "i64max_1", "i64max"])), "{got:?}");
    assert!(!got.contains(&"d2p63".to_string()) && !got.contains(&"d_inf".to_string()), "{got:?}");
}

fn doc_ref(env: &TestEnv, collection: &str, id: &str) -> Value {
    val(ValueType::ReferenceValue(format!(
        "projects/{}/databases/(default)/documents/{collection}/{id}",
        env.project_id
    )))
}

fn score_of(id: &str) -> i64 {
    if id.starts_with("lo") {
        1
    } else if id.starts_with("mid") {
        2
    } else {
        3
    }
}

async fn seed_ties(client: &mut FirestoreClient<tonic::transport::Channel>, env: &TestEnv) {
    // Seeded out of name order on purpose; ties of 3 and 4 docs span page boundaries.
    for id in ["mid_d", "hi_b", "lo_b", "mid_a", "hi_a", "lo_a", "mid_c", "lo_c", "mid_b", "hi_c"] {
        seed(client, env, "ties", id, vec![("score", val(ValueType::IntegerValue(score_of(id))))]).await;
    }
}

/// Pages `ties` ordered by `score` (`page_size` rows per page) following a
/// `startAfter(score, last doc ref)` cursor; returns all ids in page order.
async fn page_through_ties(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    env: &TestEnv,
    direction: Direction,
    page_size: i32,
) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    loop {
        let start_at = seen.last().map(|id| Cursor {
            values: vec![val(ValueType::IntegerValue(score_of(id))), doc_ref(env, "ties", id)],
            before: false,
        });
        let sq = StructuredQuery {
            from: from("ties"),
            order_by: vec![order("score", direction)],
            start_at,
            limit: Some(page_size),
            ..Default::default()
        };
        let page = run_ids(client, env, sq).await;
        if page.is_empty() {
            return seen;
        }
        seen.extend(page);
        assert!(seen.len() <= 20, "paging does not terminate: {seen:?}");
    }
}

/// Documents tied on the ordered field come back in document-name order, and
/// a limit + `startAfter(score, ref)` walk visits every row exactly once.
#[tokio::test]
async fn tied_rows_page_deterministically_across_limit_boundaries_with_cursors() {
    let env = setup("test-sk-g001-tie-01", "g001-tie-project-01").await;
    let mut client = client_for(&env).await;
    seed_ties(&mut client, &env).await;
    let expected = strs(&["lo_a", "lo_b", "lo_c", "mid_a", "mid_b", "mid_c", "mid_d", "hi_a", "hi_b", "hi_c"]);

    let sq = StructuredQuery {
        from: from("ties"),
        order_by: vec![order("score", Direction::Ascending)],
        ..Default::default()
    };
    assert_eq!(run_ids(&mut client, &env, sq).await, expected);

    for page_size in [1, 2, 3, 4] {
        assert_eq!(
            page_through_ties(&mut client, &env, Direction::Ascending, page_size).await,
            expected,
            "page_size={page_size}"
        );
    }

    // A prefix cursor without a name keeps its meaning: startAfter(2) skips ALL score-2 rows.
    let sq = StructuredQuery {
        from: from("ties"),
        order_by: vec![order("score", Direction::Ascending)],
        start_at: Some(Cursor { values: vec![val(ValueType::IntegerValue(2))], before: false }),
        ..Default::default()
    };
    assert_eq!(run_ids(&mut client, &env, sq).await, strs(&["hi_a", "hi_b", "hi_c"]));
    // ... and startAt(2) begins at the first of them.
    let sq = StructuredQuery {
        from: from("ties"),
        order_by: vec![order("score", Direction::Ascending)],
        start_at: Some(Cursor { values: vec![val(ValueType::IntegerValue(2))], before: true }),
        limit: Some(2),
        ..Default::default()
    };
    assert_eq!(run_ids(&mut client, &env, sq).await, strs(&["mid_a", "mid_b"]));
    // endBefore(score, ref) cuts inside a run of ties.
    let sq = StructuredQuery {
        from: from("ties"),
        order_by: vec![order("score", Direction::Ascending)],
        end_at: Some(Cursor {
            values: vec![val(ValueType::IntegerValue(2)), doc_ref(&env, "ties", "mid_c")],
            before: true,
        }),
        ..Default::default()
    };
    assert_eq!(run_ids(&mut client, &env, sq).await, strs(&["lo_a", "lo_b", "lo_c", "mid_a", "mid_b"]));
}

/// The implicit name order follows the LAST explicit direction: descending
/// score gives descending names within a tie, and DESC paging with a name
/// cursor is gap- and duplicate-free; a query with no order is name-ordered.
#[tokio::test]
async fn implicit_name_order_follows_the_last_order_direction() {
    let env = setup("test-sk-g001-tie-02", "g001-tie-project-02").await;
    let mut client = client_for(&env).await;
    seed_ties(&mut client, &env).await;
    let expected = strs(&["hi_c", "hi_b", "hi_a", "mid_d", "mid_c", "mid_b", "mid_a", "lo_c", "lo_b", "lo_a"]);

    let sq = StructuredQuery {
        from: from("ties"),
        order_by: vec![order("score", Direction::Descending)],
        ..Default::default()
    };
    assert_eq!(run_ids(&mut client, &env, sq).await, expected);

    for page_size in [1, 3, 4] {
        assert_eq!(
            page_through_ties(&mut client, &env, Direction::Descending, page_size).await,
            expected,
            "page_size={page_size}"
        );
    }

    let sq = StructuredQuery { from: from("ties"), limit: Some(4), ..Default::default() };
    assert_eq!(run_ids(&mut client, &env, sq).await, strs(&["hi_a", "hi_b", "hi_c", "lo_a"]));

    // Explicit mixed directions: score DESC, then name ASC (last explicit order wins).
    let sq = StructuredQuery {
        from: from("ties"),
        order_by: vec![order("score", Direction::Descending), order("__name__", Direction::Ascending)],
        limit: Some(5),
        ..Default::default()
    };
    assert_eq!(run_ids(&mut client, &env, sq).await, strs(&["hi_a", "hi_b", "hi_c", "mid_a", "mid_b"]));
}
