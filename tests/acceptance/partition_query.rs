//! PartitionQuery — split points that partition a RunQuery result set.
//!
//! Driving port: gRPC data port (:8080) — PartitionQuery + RunQuery RPCs. The
//! returned `__name__` cursors, used as startAt/endBefore bounds of a
//! name-ordered RunQuery, must cut the collection into disjoint runs whose
//! concatenation is the whole ordered result.

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    run_query_request::QueryType, PartitionQueryRequest,
    partition_query_request::QueryType as PartitionQueryType,
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

fn parent(env: &TestEnv) -> String {
    format!("projects/{}/databases/(default)/documents", env.project_id)
}

fn query(collection: &str, start_at: Option<Cursor>, end_at: Option<Cursor>) -> StructuredQuery {
    StructuredQuery {
        from: vec![CollectionSelector { collection_id: collection.to_string(), all_descendants: false }],
        order_by: vec![Order {
            field: Some(FieldReference { field_path: "__name__".to_string() }),
            direction: Direction::Ascending as i32,
        }],
        start_at,
        end_at,
        ..Default::default()
    }
}

async fn partition(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    env: &TestEnv,
    collection: &str,
    partition_count: i64,
    page_size: i32,
    page_token: &str,
) -> Result<embyr_proto::firestore::PartitionQueryResponse, tonic::Status> {
    let req = authed(
        PartitionQueryRequest {
            parent: parent(env),
            query_type: Some(PartitionQueryType::StructuredQuery(query(collection, None, None))),
            partition_count,
            page_size,
            page_token: page_token.to_string(),
            ..Default::default()
        },
        &env.api_key,
    );
    client.partition_query(req).await.map(|r| r.into_inner())
}

/// Document ids from a name-ordered RunQuery bounded by `[start_at, end_at)`.
async fn ids_between(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    env: &TestEnv,
    collection: &str,
    start_at: Option<Cursor>,
    end_at: Option<Cursor>,
) -> Vec<String> {
    use tokio_stream::StreamExt;
    let req = authed(
        RunQueryRequest {
            parent: parent(env),
            query_type: Some(QueryType::StructuredQuery(query(collection, start_at, end_at))),
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

async fn client_and_items(env: &TestEnv, n: usize) -> FirestoreClient<tonic::transport::Channel> {
    let mut client = FirestoreClient::new(
        tonic::transport::Channel::from_shared(format!("http://{}", env.server.grpc_addr))
            .unwrap()
            .connect_lazy(),
    );
    for i in 0..n {
        seed(&mut client, env, "items", &format!("d{i:02}"), vec![("n", val(ValueType::IntegerValue(i as i64)))]).await;
    }
    client
}

/// Running the query between consecutive split cursors returns disjoint runs
/// that together are the whole name-ordered collection.
#[tokio::test]
async fn partition_cursors_cut_the_collection_into_disjoint_runs_covering_everything() {
    let env = setup("test-sk-partition-01", "partition-project-01").await;
    let mut client = client_and_items(&env, 10).await;

    let resp = partition(&mut client, &env, "items", 4, 0, "").await.unwrap();
    assert_eq!(resp.partitions.len(), 3, "partition_count 4 -> 3 split cursors");
    assert!(resp.next_page_token.is_empty());
    assert!(resp.partitions.iter().all(|c| c.before && c.values.len() == 1));

    let mut bounds: Vec<Option<Cursor>> = vec![None];
    bounds.extend(resp.partitions.iter().cloned().map(Some));
    bounds.push(None);

    let mut all = Vec::new();
    for w in bounds.windows(2) {
        let run = ids_between(&mut client, &env, "items", w[0].clone(), w[1].clone()).await;
        assert!(!run.is_empty(), "every partition has documents");
        all.extend(run);
    }
    let expected: Vec<String> = (0..10).map(|i| format!("d{i:02}")).collect();
    assert_eq!(all, expected);
}

/// `page_size` splits the cursor list across pages; the pages concatenate to
/// the unpaged answer.
#[tokio::test]
async fn partition_pages_concatenate_to_the_unpaged_cursor_list() {
    let env = setup("test-sk-partition-02", "partition-project-02").await;
    let mut client = client_and_items(&env, 10).await;

    let full = partition(&mut client, &env, "items", 4, 0, "").await.unwrap().partitions;

    let mut paged = Vec::new();
    let mut token = String::new();
    loop {
        let r = partition(&mut client, &env, "items", 4, 1, &token).await.unwrap();
        assert!(r.partitions.len() <= 1);
        paged.extend(r.partitions);
        if r.next_page_token.is_empty() {
            break;
        }
        token = r.next_page_token;
    }
    assert_eq!(paged, full);
}

/// partition_count 1 -> no cursors; < 1 and a negative page_size are rejected.
#[tokio::test]
async fn partition_count_edge_cases() {
    let env = setup("test-sk-partition-03", "partition-project-03").await;
    let mut client = client_and_items(&env, 3).await;

    let one = partition(&mut client, &env, "items", 1, 0, "").await.unwrap();
    assert!(one.partitions.is_empty() && one.next_page_token.is_empty());

    let zero = partition(&mut client, &env, "items", 0, 0, "").await.unwrap_err();
    assert_eq!(zero.code(), tonic::Code::InvalidArgument);

    let neg = partition(&mut client, &env, "items", 2, -1, "").await.unwrap_err();
    assert_eq!(neg.code(), tonic::Code::InvalidArgument);
}

/// A query shape PartitionQuery does not support (a limit) is rejected.
#[tokio::test]
async fn partition_query_rejects_a_limit() {
    let env = setup("test-sk-partition-04", "partition-project-04").await;
    let mut client = client_and_items(&env, 3).await;
    let mut sq = query("items", None, None);
    sq.limit = Some(2);
    let req = authed(
        PartitionQueryRequest {
            parent: parent(&env),
            query_type: Some(PartitionQueryType::StructuredQuery(sq)),
            partition_count: 2,
            ..Default::default()
        },
        &env.api_key,
    );
    let err = client.partition_query(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}
