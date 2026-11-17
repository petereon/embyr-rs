//! Vector search (`findNearest`) via RunQuery.
//!
//! Driving port: gRPC data port (:8080) — RunQuery with
//! `StructuredQuery.find_nearest`, GetDocument for the round-trip check.
//! Exact brute-force kNN over the filter's matches; see
//! docs/feature/vector-search/feature-delta.md.

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    run_query_request::QueryType,
    structured_query::{
        field_filter::Operator as FieldOp, filter::FilterType, find_nearest::DistanceMeasure,
        CollectionSelector, FieldFilter, FieldReference, Filter, FindNearest,
    },
    value::ValueType,
    ArrayValue, CreateDocumentRequest, Document, GetDocumentRequest, MapValue, RunQueryRequest,
    StructuredQuery, Value,
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
    sys_url: String,
    cust_url: String,
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
        sys_url,
        cust_url,
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

fn parent(env: &TestEnv) -> String {
    format!("projects/{}/databases/(default)/documents", env.project_id)
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
            parent: parent(env),
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

/// A Firestore vector value as an SDK puts it on the wire.
fn vector(xs: &[f64]) -> Value {
    val(ValueType::MapValue(MapValue {
        fields: HashMap::from([
            ("__type__".to_string(), val(ValueType::StringValue("__vector__".into()))),
            (
                "value".to_string(),
                val(ValueType::ArrayValue(ArrayValue {
                    values: xs.iter().map(|x| val(ValueType::DoubleValue(*x))).collect(),
                })),
            ),
        ]),
    }))
}

fn nearest_query(
    collection: &str,
    qv: &[f64],
    measure: DistanceMeasure,
    limit: i32,
    result_field: &str,
    threshold: Option<f64>,
) -> StructuredQuery {
    StructuredQuery {
        from: vec![CollectionSelector { collection_id: collection.to_string(), all_descendants: false }],
        find_nearest: Some(FindNearest {
            vector_field: Some(FieldReference { field_path: "emb".to_string() }),
            query_vector: Some(vector(qv)),
            distance_measure: measure as i32,
            limit: Some(limit),
            distance_result_field: result_field.to_string(),
            distance_threshold: threshold,
        }),
        ..Default::default()
    }
}

async fn run(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    env: &TestEnv,
    sq: StructuredQuery,
) -> Result<Vec<Document>, tonic::Status> {
    use tokio_stream::StreamExt;
    let req = authed(
        RunQueryRequest {
            parent: parent(env),
            query_type: Some(QueryType::StructuredQuery(sq)),
            ..Default::default()
        },
        &env.api_key,
    );
    let mut stream = client.run_query(req).await?.into_inner();
    let mut docs = Vec::new();
    while let Some(item) = stream.next().await {
        if let Some(d) = item?.document {
            docs.push(d);
        }
    }
    Ok(docs)
}

fn ids(docs: &[Document]) -> Vec<String> {
    docs.iter().map(|d| d.name.rsplit('/').next().unwrap().to_string()).collect()
}

fn dist(d: &Document, field: &str) -> f64 {
    match d.fields[field].value_type {
        Some(ValueType::DoubleValue(x)) => x,
        ref other => panic!("{field} is not a double: {other:?}"),
    }
}

async fn client_with_points(env: &TestEnv) -> FirestoreClient<tonic::transport::Channel> {
    let mut client = FirestoreClient::new(
        tonic::transport::Channel::from_shared(format!("http://{}", env.server.grpc_addr))
            .unwrap()
            .connect_lazy(),
    );
    // Euclidean distances from the origin: a=1, b=2, c=5, d=10.
    for (id, v, cat) in [
        ("a", [1.0, 0.0], "y"),
        ("b", [0.0, 2.0], "x"),
        ("c", [3.0, 4.0], "x"),
        ("d", [10.0, 0.0], "y"),
    ] {
        seed(
            &mut client,
            env,
            "pts",
            id,
            vec![("emb", vector(&v)), ("cat", val(ValueType::StringValue(cat.into())))],
        )
        .await;
    }
    // Must never appear: no vector, not a vector, wrong dimension.
    seed(&mut client, env, "pts", "no_vec", vec![("cat", val(ValueType::StringValue("x".into())))]).await;
    seed(&mut client, env, "pts", "str_vec", vec![("emb", val(ValueType::StringValue("nope".into())))]).await;
    seed(&mut client, env, "pts", "dim3", vec![("emb", vector(&[0.0, 0.0, 0.1]))]).await;
    client
}

#[tokio::test]
async fn euclidean_returns_the_k_nearest_in_order() {
    let env = setup("test-sk-vector-01", "vector-project-01").await;
    let mut client = client_with_points(&env).await;
    let q = nearest_query("pts", &[0.0, 0.0], DistanceMeasure::Euclidean, 3, "", None);
    let docs = run(&mut client, &env, q).await.unwrap();
    assert_eq!(ids(&docs), ["a", "b", "c"]);
    assert!(!docs[0].fields.contains_key("dist"), "no result field unless requested");
}

#[tokio::test]
async fn cosine_orders_by_angle_not_magnitude() {
    let env = setup("test-sk-vector-02", "vector-project-02").await;
    let mut client = client_with_points(&env).await;
    // Query along +x: a and d point exactly along +x (distance 0, ties by
    // name), c is about 53 degrees off, b is orthogonal (distance 1).
    let q = nearest_query("pts", &[1.0, 0.0], DistanceMeasure::Cosine, 4, "cd", None);
    let docs = run(&mut client, &env, q).await.unwrap();
    assert_eq!(ids(&docs), ["a", "d", "c", "b"]);
    assert!(dist(&docs[0], "cd").abs() < 1e-12);
    assert!((dist(&docs[3], "cd") - 1.0).abs() < 1e-12);
}

#[tokio::test]
async fn dot_product_returns_largest_first_and_the_dot_product_itself() {
    let env = setup("test-sk-vector-03", "vector-project-03").await;
    let mut client = client_with_points(&env).await;
    let q = nearest_query("pts", &[1.0, 0.0], DistanceMeasure::DotProduct, 4, "dp", None);
    let docs = run(&mut client, &env, q).await.unwrap();
    assert_eq!(ids(&docs), ["d", "c", "a", "b"]);
    assert_eq!(dist(&docs[0], "dp"), 10.0);
    assert_eq!(dist(&docs[3], "dp"), 0.0);
}

#[tokio::test]
async fn distance_result_field_and_threshold() {
    let env = setup("test-sk-vector-04", "vector-project-04").await;
    let mut client = client_with_points(&env).await;
    let q = nearest_query("pts", &[0.0, 0.0], DistanceMeasure::Euclidean, 10, "dist", Some(5.0));
    let docs = run(&mut client, &env, q).await.unwrap();
    assert_eq!(ids(&docs), ["a", "b", "c"], "distance <= 5, boundary inclusive");
    let got: Vec<f64> = docs.iter().map(|d| dist(d, "dist")).collect();
    assert_eq!(got, [1.0, 2.0, 5.0]);

    // DOT_PRODUCT keeps results >= threshold.
    let q = nearest_query("pts", &[1.0, 0.0], DistanceMeasure::DotProduct, 10, "", Some(3.0));
    let docs = run(&mut client, &env, q).await.unwrap();
    assert_eq!(ids(&docs), ["d", "c"]);
}

#[tokio::test]
async fn a_filter_narrows_the_candidates_before_ranking() {
    let env = setup("test-sk-vector-05", "vector-project-05").await;
    let mut client = client_with_points(&env).await;
    let mut sq = nearest_query("pts", &[0.0, 0.0], DistanceMeasure::Euclidean, 2, "", None);
    sq.r#where = Some(Filter {
        filter_type: Some(FilterType::FieldFilter(FieldFilter {
            field: Some(FieldReference { field_path: "cat".into() }),
            op: FieldOp::Equal as i32,
            value: Some(val(ValueType::StringValue("x".into()))),
        })),
    });
    let docs = run(&mut client, &env, sq).await.unwrap();
    assert_eq!(ids(&docs), ["b", "c"], "a is nearest overall but cat=y");
}

#[tokio::test]
async fn a_vector_written_by_an_sdk_reads_back_identical() {
    let env = setup("test-sk-vector-06", "vector-project-06").await;
    let mut client = client_with_points(&env).await;
    let v = [0.1, -2.5, 1e-9, 123456.789, f64::MIN_POSITIVE, 0.3 + 0.6];
    seed(&mut client, &env, "rt", "v", vec![("emb", vector(&v))]).await;
    let got = client
        .get_document(authed(
            GetDocumentRequest { name: format!("{}/rt/v", parent(&env)), ..Default::default() },
            &env.api_key,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got.fields["emb"], vector(&v));
}

#[tokio::test]
async fn invalid_requests_are_invalid_argument() {
    let env = setup("test-sk-vector-07", "vector-project-07").await;
    let mut client = client_with_points(&env).await;
    let ia = tonic::Code::InvalidArgument;
    let e = DistanceMeasure::Euclidean;

    let q = nearest_query("pts", &[0.0, 0.0], e, 0, "", None);
    assert_eq!(run(&mut client, &env, q).await.unwrap_err().code(), ia);
    let q = nearest_query("pts", &[0.0, 0.0], e, 1001, "", None);
    assert_eq!(run(&mut client, &env, q).await.unwrap_err().code(), ia);

    // A base-query limit cannot be combined with find_nearest.
    let mut q = nearest_query("pts", &[0.0, 0.0], e, 2, "", None);
    q.limit = Some(5);
    assert_eq!(run(&mut client, &env, q).await.unwrap_err().code(), ia);

    let q = nearest_query("pts", &[0.0, 0.0], DistanceMeasure::Cosine, 2, "", None);
    assert_eq!(run(&mut client, &env, q).await.unwrap_err().code(), ia);

    // Writing a vector over 2048 dimensions is rejected.
    let req = authed(
        CreateDocumentRequest {
            parent: parent(&env),
            collection_id: "pts".into(),
            document_id: "huge".into(),
            document: Some(Document {
                fields: HashMap::from([("emb".to_string(), vector(&vec![0.0; 2049]))]),
                ..Default::default()
            }),
            ..Default::default()
        },
        &env.api_key,
    );
    assert_eq!(client.create_document(req).await.unwrap_err().code(), ia);
}

#[tokio::test]
async fn a_rule_that_denies_listing_denies_the_vector_query() {
    let env = setup("test-sk-vector-08", "vector-project-08").await;
    let mut client = client_with_points(&env).await;
    let sys_pool = sqlx::PgPool::connect(&env.sys_url).await.unwrap();
    sqlx::query(
        "INSERT INTO access_rules (project_id, collection_path, condition_source) \
         VALUES ($1, 'pts', 'false')",
    )
    .bind(&env.project_id)
    .execute(&sys_pool)
    .await
    .unwrap();
    let q = nearest_query("pts", &[0.0, 0.0], DistanceMeasure::Euclidean, 3, "", None);
    assert_eq!(run(&mut client, &env, q).await.unwrap_err().code(), tonic::Code::PermissionDenied);
}

#[tokio::test]
async fn a_scan_over_the_cap_is_failed_precondition() {
    let env = setup("test-sk-vector-09", "vector-project-09").await;
    let mut client = client_with_points(&env).await;
    let cust = sqlx::PgPool::connect(&env.cust_url).await.unwrap();
    sqlx::query(
        "INSERT INTO documents (project_id, collection_path, document_id) \
         SELECT $1, 'big', 'd' || g FROM generate_series(1, 20001) g",
    )
    .bind(&env.project_id)
    .execute(&cust)
    .await
    .unwrap();
    let q = nearest_query("big", &[0.0, 0.0], DistanceMeasure::Euclidean, 3, "", None);
    assert_eq!(run(&mut client, &env, q).await.unwrap_err().code(), tonic::Code::FailedPrecondition);
    // A small collection is unaffected.
    let q = nearest_query("pts", &[0.0, 0.0], DistanceMeasure::Euclidean, 1, "", None);
    assert_eq!(ids(&run(&mut client, &env, q).await.unwrap()), ["a"]);
}
