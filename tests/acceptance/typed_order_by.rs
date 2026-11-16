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
