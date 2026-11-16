//! US-04b — Queries are scoped to the request's parent path
//!
//! As Alex, I want a query on `users/u1/posts` to return u1's posts only, so
//! that subcollections with the same name under different parents stay
//! isolated — for RunQuery, RunAggregationQuery and Listen alike.
//!
//! Driving port: gRPC data port (:8080) — RunQuery / RunAggregationQuery /
//! Listen RPCs. Helpers mirror `us_04_query_collection.rs` /
//! `us_05_listen_realtime.rs`.
//!
//! bypass: example-based, not property-based — every case needs two Postgres
//! testcontainers; the pure path-building rule is unit-tested in
//! `grpc::handler::collection_path_under_parent_tests`.

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    listen_request, listen_response,
    run_aggregation_query_request::QueryType as AggregationQueryType,
    run_query_request::QueryType,
    structured_aggregation_query::{
        aggregation::{Count, Operator as AggregationOperator},
        Aggregation, QueryType as StructuredAggQueryType,
    },
    structured_query::CollectionSelector,
    target::{self, query_target},
    target_change::TargetChangeType,
    value::ValueType,
    CreateDocumentRequest, Document, ListenRequest, RunAggregationQueryRequest, RunQueryRequest,
    StructuredAggregationQuery, StructuredQuery, Target,
};
use embyr_server::{adapters::system_db::SystemDb, start_test_server, TestServer};
use std::{collections::HashMap, sync::Arc, time::Duration};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};
use tokio_stream::StreamExt;

type Client = FirestoreClient<tonic::transport::Channel>;

async fn start_postgres() -> (ContainerAsync<Postgres>, String) {
    let container = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("Failed to start Postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("Failed to get host port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    (container, url)
}

struct TestEnv {
    _sys_container: ContainerAsync<Postgres>,
    _cust_container: ContainerAsync<Postgres>,
    project_id: String,
    api_key: String,
    server: TestServer,
}

impl TestEnv {
    /// `""` -> database root; `"users/u1"` -> that document.
    fn parent(&self, document_path: &str) -> String {
        let root = format!("projects/{}/databases/(default)/documents", self.project_id);
        if document_path.is_empty() {
            root
        } else {
            format!("{root}/{document_path}")
        }
    }

    fn authed<T>(&self, payload: T) -> tonic::Request<T> {
        let mut req = tonic::Request::new(payload);
        req.metadata_mut().insert(
            "authorization",
            format!("bearer {}", self.api_key).parse().unwrap(),
        );
        req
    }

    fn client(&self) -> Client {
        FirestoreClient::new(
            tonic::transport::Channel::from_shared(format!("http://{}", self.server.grpc_addr))
                .unwrap()
                .connect_lazy(),
        )
    }
}

async fn setup(api_key: &str, project_id: &str) -> TestEnv {
    let (_sys_container, sys_url) = start_postgres().await;
    let (_cust_container, cust_url) = start_postgres().await;

    let system_db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    system_db.migrate().await.unwrap();

    let cust_pool = sqlx::PgPool::connect(&cust_url).await.unwrap();
    sqlx::migrate!("../../migrations/customer")
        .run(&cust_pool)
        .await
        .unwrap();

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

/// Create `{parent_document}/{collection}/{doc_id}` via CreateDocument.
async fn seed(env: &TestEnv, client: &mut Client, parent_document: &str, collection: &str, doc_id: &str) {
    let req = env.authed(CreateDocumentRequest {
        parent: env.parent(parent_document),
        collection_id: collection.to_string(),
        document_id: doc_id.to_string(),
        document: Some(Document { name: String::new(), fields: HashMap::new(), ..Default::default() }),
        ..Default::default()
    });
    client.create_document(req).await.expect("seed document should succeed");
}

/// Two parents with a same-named subcollection, plus a same-named top-level
/// collection: `users/u1/posts/{a1,a2}`, `users/u2/posts/{b1}`, `posts/{top1}`.
async fn seed_posts_under_two_parents(env: &TestEnv, client: &mut Client) {
    seed(env, client, "users/u1", "posts", "a1").await;
    seed(env, client, "users/u1", "posts", "a2").await;
    seed(env, client, "users/u2", "posts", "b1").await;
    seed(env, client, "", "posts", "top1").await;
}

fn posts_query() -> StructuredQuery {
    StructuredQuery {
        from: vec![CollectionSelector { collection_id: "posts".to_string(), all_descendants: false }],
        ..Default::default()
    }
}

/// Sorted document paths (relative to `/documents/`) a RunQuery returns.
async fn query_posts(env: &TestEnv, client: &mut Client, parent_document: &str) -> Vec<String> {
    let req = env.authed(RunQueryRequest {
        parent: env.parent(parent_document),
        query_type: Some(QueryType::StructuredQuery(posts_query())),
        ..Default::default()
    });
    let stream = client.run_query(req).await.expect("RunQuery should succeed").into_inner();
    let responses: Vec<_> = stream.collect().await;
    let mut paths: Vec<String> = responses
        .into_iter()
        .map(|r| r.expect("RunQuery stream item should be Ok"))
        .filter_map(|r| r.document)
        .map(|d| d.name.split("/documents/").nth(1).unwrap().to_string())
        .collect();
    paths.sort();
    paths
}

/// Given:  `posts` subcollections under users u1 and u2, and a top-level `posts`
/// When:   RunQuery targets `posts` with parent `users/u1` (then `users/u2`)
/// Then:   only that parent's own posts are returned
#[tokio::test]
async fn run_query_on_subcollection_returns_only_that_parents_documents() {
    let env = setup("test-sk-us04b-subcoll-01", "us04b-subcoll-project-01").await;
    let mut client = env.client();
    seed_posts_under_two_parents(&env, &mut client).await;

    assert_eq!(
        query_posts(&env, &mut client, "users/u1").await,
        vec!["users/u1/posts/a1", "users/u1/posts/a2"]
    );
    assert_eq!(query_posts(&env, &mut client, "users/u2").await, vec!["users/u2/posts/b1"]);
}

/// Given:  the same data
/// When:   RunQuery targets `posts` with the database-root parent
/// Then:   only the top-level `posts` documents are returned (unchanged behaviour)
#[tokio::test]
async fn run_query_on_top_level_collection_excludes_subcollection_documents() {
    let env = setup("test-sk-us04b-subcoll-02", "us04b-subcoll-project-02").await;
    let mut client = env.client();
    seed_posts_under_two_parents(&env, &mut client).await;

    assert_eq!(query_posts(&env, &mut client, "").await, vec!["posts/top1"]);
}

/// Given:  the same data
/// When:   a COUNT aggregation targets `posts` with parent `users/u1`
/// Then:   the count is 2 — u1's posts only
#[tokio::test]
async fn count_aggregation_on_subcollection_counts_only_that_parents_documents() {
    let env = setup("test-sk-us04b-subcoll-03", "us04b-subcoll-project-03").await;
    let mut client = env.client();
    seed_posts_under_two_parents(&env, &mut client).await;

    let saq = StructuredAggregationQuery {
        query_type: Some(StructuredAggQueryType::StructuredQuery(posts_query())),
        aggregations: vec![Aggregation {
            operator: Some(AggregationOperator::Count(Count { up_to: None })),
            alias: String::new(),
        }],
    };
    let req = env.authed(RunAggregationQueryRequest {
        parent: env.parent("users/u1"),
        query_type: Some(AggregationQueryType::StructuredAggregationQuery(saq)),
        ..Default::default()
    });
    let mut stream = client
        .run_aggregation_query(req)
        .await
        .expect("RunAggregationQuery should succeed")
        .into_inner();
    let result = stream
        .next()
        .await
        .expect("expected one RunAggregationQueryResponse")
        .expect("aggregation stream item should be Ok")
        .result
        .expect("response must carry an AggregationResult");

    let count = result.aggregate_fields.get("field_1").and_then(|v| v.value_type.clone());
    assert_eq!(count, Some(ValueType::IntegerValue(2)));
}

/// Given:  a Listen stream on `posts` with parent `users/u1`
/// When:   a post is written under `users/u2`, then one under `users/u1`
/// Then:   the initial snapshot holds u1's posts only, and the first live
///         change delivered is u1's write — u2's write never reaches the listener
#[tokio::test]
async fn listen_on_subcollection_receives_only_that_parents_writes() {
    let env = setup("test-sk-us04b-subcoll-04", "us04b-subcoll-project-04").await;
    let mut listener = env.client();
    let mut writer = env.client();
    seed_posts_under_two_parents(&env, &mut writer).await;

    let add_target = ListenRequest {
        database: format!("projects/{}/databases/(default)", env.project_id),
        target_change: Some(listen_request::TargetChange::AddTarget(Target {
            target_id: 1,
            target_type: Some(target::TargetType::Query(target::QueryTarget {
                parent: env.parent("users/u1"),
                query_type: Some(query_target::QueryType::StructuredQuery(posts_query())),
            })),
            ..Default::default()
        })),
        ..Default::default()
    };
    let mut stream = listener
        .listen(env.authed(tokio_stream::once(add_target)))
        .await
        .expect("listen should succeed")
        .into_inner();

    // Initial snapshot: every DocumentChange up to CURRENT.
    let mut snapshot = Vec::new();
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("timed out waiting for CURRENT")
            .expect("stream ended before CURRENT")
            .expect("stream error before CURRENT");
        match msg.response_type {
            Some(listen_response::ResponseType::DocumentChange(dc)) => {
                snapshot.push(dc.document.expect("change carries a document").name);
            }
            Some(listen_response::ResponseType::TargetChange(tc))
                if tc.target_change_type == TargetChangeType::Current as i32 =>
            {
                break;
            }
            _ => {}
        }
    }
    snapshot.sort();
    assert_eq!(
        snapshot,
        vec![
            format!("{}/posts/a1", env.parent("users/u1")),
            format!("{}/posts/a2", env.parent("users/u1")),
        ]
    );

    // u2's write goes first: if it leaked, it would be the first change seen.
    seed(&env, &mut writer, "users/u2", "posts", "b-live").await;
    seed(&env, &mut writer, "users/u1", "posts", "a-live").await;

    let first_change = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let msg = stream.next().await?.ok()?;
            if let Some(listen_response::ResponseType::DocumentChange(dc)) = msg.response_type {
                return dc.document.map(|d| d.name);
            }
        }
    })
    .await
    .expect("expected a DocumentChange within 5 seconds of the write");

    assert_eq!(first_change, Some(format!("{}/posts/a-live", env.parent("users/u1"))));
}
