// SCAFFOLD: true
//! US-04 — Query a collection
//!
//! As Alex, I want to use getDocs with where, orderBy, limit, and cursor operators,
//! so that I can retrieve filtered and sorted document sets.
//!
//! Driving port: gRPC data port (:8080) — RunQuery RPC
//! Red classification: MISSING_FUNCTIONALITY

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    run_query_request::QueryType,
    structured_query::{
        field_filter::Operator as FieldOp,
        unary_filter::Operator as UnaryOp,
        CollectionSelector, Direction, FieldFilter, FieldReference,
        Filter, Order, UnaryFilter,
        filter::FilterType,
    },
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
    sys_pool: sqlx::PgPool,
    cust_pool: sqlx::PgPool,
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
        sys_pool,
        cust_pool,
        project_id: project_id.to_string(),
        api_key: api_key.to_string(),
        server,
    }
}

fn make_channel(addr: std::net::SocketAddr) -> tonic::transport::Channel {
    tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect_lazy()
}

fn make_authed_request<T>(payload: T, api_key: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(payload);
    req.metadata_mut().insert(
        "authorization",
        format!("bearer {api_key}").parse().unwrap(),
    );
    req
}

fn integer_value(i: i64) -> Value {
    Value { value_type: Some(ValueType::IntegerValue(i)) }
}

fn string_value(s: &str) -> Value {
    Value { value_type: Some(ValueType::StringValue(s.to_string())) }
}

fn field_ref(path: &str) -> FieldReference {
    FieldReference { field_path: path.to_string() }
}

/// Seed a document into the customer DB via CreateDocument gRPC call.
async fn seed_document(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    collection: &str,
    doc_id: &str,
    fields: HashMap<String, Value>,
) {
    let parent = format!("projects/{project_id}/databases/(default)/documents");
    let req = make_authed_request(
        CreateDocumentRequest {
            parent,
            collection_id: collection.to_string(),
            document_id: doc_id.to_string(),
            document: Some(Document {
                name: String::new(),
                fields,
                ..Default::default()
            }),
            ..Default::default()
        },
        api_key,
    );
    client.create_document(req).await.expect("seed document should succeed");
}

/// Collect documents from a RunQuery stream, filtering out done=true messages.
async fn collect_query_docs(
    stream: tonic::codec::Streaming<embyr_proto::firestore::RunQueryResponse>,
) -> Vec<Document> {
    use tokio_stream::StreamExt;
    let responses: Vec<_> = stream.collect().await;
    responses
        .into_iter()
        .filter_map(|r| r.ok())
        .filter(|r| r.document.is_some())
        .map(|r| r.document.unwrap())
        .collect()
}

/// AC-04a: where filter returns only matching documents
///
/// Given:  a provisioned project with three users: ages 15, 20, 25
/// When:   a RunQuery RPC is called with filter age >= 18
/// Then:   the response contains exactly the users aged 20 and 25
/// And:    the user aged 15 is absent from the result
#[tokio::test]
async fn where_filter_returns_only_matching_documents() {
    let env = setup("test-sk-us04-where-01", "us04-where-project-01").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Seed 3 docs with ages 15, 20, 25
    for (doc_id, age) in [("user-15", 15i64), ("user-20", 20i64), ("user-25", 25i64)] {
        let mut fields = HashMap::new();
        fields.insert("age".to_string(), integer_value(age));
        seed_document(&mut client, &env.project_id, &env.api_key, "users", doc_id, fields).await;
    }

    // RunQuery with filter age >= 18
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let sq = StructuredQuery {
        from: vec![CollectionSelector {
            collection_id: "users".to_string(),
            all_descendants: false,
        }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("age")),
                op: FieldOp::GreaterThanOrEqual as i32,
                value: Some(integer_value(18)),
            })),
        }),
        ..Default::default()
    };
    let req = make_authed_request(
        RunQueryRequest {
            parent,
            query_type: Some(QueryType::StructuredQuery(sq)),
            ..Default::default()
        },
        &env.api_key,
    );

    let stream = client.run_query(req).await.expect("RunQuery should succeed").into_inner();
    let docs = collect_query_docs(stream).await;

    assert_eq!(docs.len(), 2, "expected exactly 2 documents (age 20 and 25), got {}", docs.len());

    // Verify both have age >= 18
    for doc in &docs {
        let age_val = doc.fields.get("age").expect("age field missing");
        match &age_val.value_type {
            Some(ValueType::IntegerValue(i)) => {
                assert!(*i >= 18, "expected age >= 18, got {i}");
                assert_ne!(*i, 15, "age=15 should be excluded");
            }
            other => panic!("expected IntegerValue for age, got {other:?}"),
        }
    }
}

/// AC-04b: orderBy returns documents in ascending field order
///
/// Given:  a provisioned project with documents having ages 25, 15, 20
/// When:   a RunQuery RPC with orderBy("age") ascending is executed
/// Then:   documents arrive in order age=15, age=20, age=25
#[tokio::test]
async fn order_by_returns_documents_in_ascending_order() {
    let env = setup("test-sk-us04-order-01", "us04-order-project-01").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Seed docs with ages in non-sorted order: 25, 15, 20
    for (doc_id, age) in [("doc-25", 25i64), ("doc-15", 15i64), ("doc-20", 20i64)] {
        let mut fields = HashMap::new();
        fields.insert("age".to_string(), integer_value(age));
        seed_document(&mut client, &env.project_id, &env.api_key, "scores", doc_id, fields).await;
    }

    // RunQuery with ORDER BY age ASC (numeric)
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let sq = StructuredQuery {
        from: vec![CollectionSelector {
            collection_id: "scores".to_string(),
            all_descendants: false,
        }],
        order_by: vec![Order {
            field: Some(field_ref("age")),
            direction: Direction::Ascending as i32,
        }],
        ..Default::default()
    };
    let req = make_authed_request(
        RunQueryRequest {
            parent,
            query_type: Some(QueryType::StructuredQuery(sq)),
            ..Default::default()
        },
        &env.api_key,
    );

    let stream = client.run_query(req).await.expect("RunQuery should succeed").into_inner();
    let docs = collect_query_docs(stream).await;

    assert_eq!(docs.len(), 3, "expected 3 documents, got {}", docs.len());

    let ages: Vec<i64> = docs
        .iter()
        .map(|d| {
            match &d.fields.get("age").expect("age field missing").value_type {
                Some(ValueType::IntegerValue(i)) => *i,
                other => panic!("expected IntegerValue for age, got {other:?}"),
            }
        })
        .collect();

    assert_eq!(ages, vec![15, 20, 25], "expected ascending order [15, 20, 25], got {ages:?}");
}

/// AC-04c: limit(N) returns at most N documents
///
/// Given:  a provisioned project with 10 documents in "items" collection
/// When:   a RunQuery RPC with limit=3 is executed
/// Then:   exactly 3 documents are returned
#[tokio::test]
async fn limit_returns_at_most_n_documents() {
    let env = setup("test-sk-us04-limit-01", "us04-limit-project-01").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Seed 10 documents
    for i in 0..10i64 {
        let mut fields = HashMap::new();
        fields.insert("idx".to_string(), integer_value(i));
        seed_document(
            &mut client,
            &env.project_id,
            &env.api_key,
            "items",
            &format!("item-{i:02}"),
            fields,
        )
        .await;
    }

    // RunQuery with limit=3
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let sq = StructuredQuery {
        from: vec![CollectionSelector {
            collection_id: "items".to_string(),
            all_descendants: false,
        }],
        limit: Some(3),
        ..Default::default()
    };
    let req = make_authed_request(
        RunQueryRequest {
            parent,
            query_type: Some(QueryType::StructuredQuery(sq)),
            ..Default::default()
        },
        &env.api_key,
    );

    let stream = client.run_query(req).await.expect("RunQuery should succeed").into_inner();
    let docs = collect_query_docs(stream).await;

    assert_eq!(docs.len(), 3, "expected exactly 3 documents, got {}", docs.len());
}

/// AC-04d: startAfter cursor skips the cursor document in paginated results
///
/// Given:  5 documents ordered by name: A, B, C, D, E
/// And:    document B is the cursor document
/// When:   a RunQuery RPC with startAfter(B) and limit=2 is executed
/// Then:   the response contains C and D (B excluded, A excluded)
#[tokio::test]
async fn start_after_cursor_skips_cursor_document() {
    let env = setup("test-sk-us04-cursor-01", "us04-cursor-project-01").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Seed 5 documents with name field A..E
    for name in ["A", "B", "C", "D", "E"] {
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), string_value(name));
        seed_document(
            &mut client,
            &env.project_id,
            &env.api_key,
            "letters",
            &format!("letter-{name}"),
            fields,
        )
        .await;
    }

    // RunQuery: orderBy name ASC, startAfter(B) exclusive (before=false), limit=2
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let sq = StructuredQuery {
        from: vec![CollectionSelector {
            collection_id: "letters".to_string(),
            all_descendants: false,
        }],
        order_by: vec![Order {
            field: Some(field_ref("name")),
            direction: Direction::Ascending as i32,
        }],
        start_at: Some(Cursor {
            values: vec![string_value("B")],
            before: false, // exclusive: skip B, start after B
        }),
        limit: Some(2),
        ..Default::default()
    };
    let req = make_authed_request(
        RunQueryRequest {
            parent,
            query_type: Some(QueryType::StructuredQuery(sq)),
            ..Default::default()
        },
        &env.api_key,
    );

    let stream = client.run_query(req).await.expect("RunQuery should succeed").into_inner();
    let docs = collect_query_docs(stream).await;

    assert_eq!(docs.len(), 2, "expected exactly 2 documents (C and D), got {}", docs.len());

    let names: Vec<String> = docs
        .iter()
        .map(|d| {
            match &d.fields.get("name").expect("name field missing").value_type {
                Some(ValueType::StringValue(s)) => s.clone(),
                other => panic!("expected StringValue for name, got {other:?}"),
            }
        })
        .collect();

    assert_eq!(names, vec!["C", "D"], "expected [C, D] after cursor B, got {names:?}");
}

/// AC-04e: IS_NAN filter behaves identically to where("score", "==", NaN)
///
/// Given:  documents with score values: 1.0, NaN, 2.0
/// When:   a RunQuery with IS_NAN filter on the score field is executed
/// Then:   only the document with NaN score is returned
#[tokio::test]
async fn is_nan_filter_returns_only_nan_documents() {
    let env = setup("test-sk-us04-nan-01", "us04-nan-project-01").await;

    // Seed docs directly via SQL — NaN cannot be represented in JSON, so we
    // insert the sentinel {"t":"D","v":"NaN"} directly for the NaN document.
    let fields_normal_1 = serde_json::json!({"score": {"t": "D", "v": 1.0}});
    let fields_nan = serde_json::json!({"score": {"t": "D", "v": "NaN"}});
    let fields_normal_2 = serde_json::json!({"score": {"t": "D", "v": 2.0}});

    let project_id = &env.project_id;
    sqlx::query(
        "INSERT INTO documents (project_id, collection_path, document_id, fields, version, create_time, update_time) \
         VALUES ($1, 'scores', 'score-1', $2::jsonb, 1, NOW(), NOW()), \
                ($1, 'scores', 'score-nan', $3::jsonb, 1, NOW(), NOW()), \
                ($1, 'scores', 'score-2', $4::jsonb, 1, NOW(), NOW())",
    )
    .bind(project_id.as_str())
    .bind(&fields_normal_1)
    .bind(&fields_nan)
    .bind(&fields_normal_2)
    .execute(&env.cust_pool)
    .await
    .expect("seed NaN documents");

    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let parent = format!("projects/{project_id}/databases/(default)/documents");
    let sq = StructuredQuery {
        from: vec![CollectionSelector {
            collection_id: "scores".to_string(),
            all_descendants: false,
        }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::UnaryFilter(UnaryFilter {
                op: UnaryOp::IsNan as i32,
                operand_type: Some(
                    embyr_proto::firestore::structured_query::unary_filter::OperandType::Field(
                        FieldReference { field_path: "score".to_string() },
                    ),
                ),
            })),
        }),
        ..Default::default()
    };
    let req = make_authed_request(
        RunQueryRequest {
            parent,
            query_type: Some(QueryType::StructuredQuery(sq)),
            ..Default::default()
        },
        &env.api_key,
    );

    let stream = client.run_query(req).await.expect("RunQuery should succeed").into_inner();
    let docs = collect_query_docs(stream).await;

    assert_eq!(docs.len(), 1, "expected exactly 1 NaN document, got {}", docs.len());
    let name = &docs[0].name;
    assert!(name.ends_with("score-nan"), "expected score-nan document, got {name}");
}

/// AC-04f (error path): query requiring composite index returns FAILED_PRECONDITION when no READY index
///
/// Given:  a collection with documents having fields "category" and "score"
/// And:    no composite index exists for (category ASC, score DESC)
/// When:   a RunQuery with where("category","==","A").orderBy("score","desc") is executed
/// Then:   the response returns status FAILED_PRECONDITION
#[tokio::test]
async fn query_without_ready_index_returns_failed_precondition() {
    let env = setup("test-sk-us04-idx-01", "us04-idx-project-01").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Seed a document with category and score fields
    let mut fields = HashMap::new();
    fields.insert("category".to_string(), string_value("A"));
    fields.insert("score".to_string(), integer_value(100));
    seed_document(&mut client, &env.project_id, &env.api_key, "products", "prod-1", fields).await;

    // Query: filter on "category" == "A", orderBy "score" DESC — requires composite index
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let sq = StructuredQuery {
        from: vec![CollectionSelector {
            collection_id: "products".to_string(),
            all_descendants: false,
        }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("category")),
                op: FieldOp::Equal as i32,
                value: Some(string_value("A")),
            })),
        }),
        order_by: vec![Order {
            field: Some(field_ref("score")),
            direction: Direction::Descending as i32,
        }],
        ..Default::default()
    };
    let req = make_authed_request(
        RunQueryRequest {
            parent,
            query_type: Some(QueryType::StructuredQuery(sq)),
            ..Default::default()
        },
        &env.api_key,
    );

    let result = client.run_query(req).await;
    match result {
        Err(status) => {
            assert_eq!(
                status.code(),
                tonic::Code::FailedPrecondition,
                "expected FAILED_PRECONDITION, got {:?}: {}",
                status.code(),
                status.message()
            );
        }
        Ok(_) => panic!("expected FAILED_PRECONDITION error but query succeeded"),
    }
}

/// AC-04g: same query succeeds after index is created and reaches READY status
///
/// Given:  a composite index for (category ASC, score DESC) has been created
/// And:    the index status is READY
/// When:   the same query from AC-04f is re-executed
/// Then:   the query returns results successfully
#[tokio::test]
async fn query_succeeds_after_index_reaches_ready_status() {
    let env = setup("test-sk-us04-idx-02", "us04-idx-project-02").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Seed a document with category and score fields
    let mut fields = HashMap::new();
    fields.insert("category".to_string(), string_value("B"));
    fields.insert("score".to_string(), integer_value(200));
    seed_document(&mut client, &env.project_id, &env.api_key, "products", "prod-2", fields).await;

    // Insert a READY composite index into the system DB for this project+collection
    sqlx::query(
        "INSERT INTO composite_indexes (project_id, collection_path, fields, status) \
         VALUES ($1, $2, $3::jsonb, 'ready')",
    )
    .bind(&env.project_id)
    .bind("products")
    .bind(serde_json::json!([{"field": "category", "order": "ASC"}, {"field": "score", "order": "DESC"}]))
    .execute(&env.sys_pool)
    .await
    .expect("insert ready composite index");

    // Same query as AC-04f — should now succeed with a READY index
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let sq = StructuredQuery {
        from: vec![CollectionSelector {
            collection_id: "products".to_string(),
            all_descendants: false,
        }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("category")),
                op: FieldOp::Equal as i32,
                value: Some(string_value("B")),
            })),
        }),
        order_by: vec![Order {
            field: Some(field_ref("score")),
            direction: Direction::Descending as i32,
        }],
        ..Default::default()
    };
    let req = make_authed_request(
        RunQueryRequest {
            parent,
            query_type: Some(QueryType::StructuredQuery(sq)),
            ..Default::default()
        },
        &env.api_key,
    );

    let stream = client.run_query(req).await.expect("query with READY index should succeed").into_inner();
    let docs = collect_query_docs(stream).await;
    assert!(!docs.is_empty(), "expected at least one document returned by indexed query");
}

/// AC-04h: collection group query returns documents from all sub-collections with matching name
///
/// Given:  projects/{p}/users/alice/events/e1 and projects/{p}/teams/beta/events/e2
/// When:   a collection group query for "events" is executed
/// Then:   both e1 and e2 appear in the results
#[tokio::test]
async fn collection_group_query_returns_documents_from_all_matching_subcollections() {
    let env = setup("test-sk-us04-cg-01", "us04-cg-project-01").await;

    let project_id = &env.project_id;

    // Seed two documents in nested sub-collections of "events" via direct SQL insert
    let fields_e1 = serde_json::json!({"kind": {"t": "S", "v": "click"}});
    let fields_e2 = serde_json::json!({"kind": {"t": "S", "v": "view"}});

    sqlx::query(
        "INSERT INTO documents \
         (project_id, collection_path, document_id, fields, version, create_time, update_time) \
         VALUES \
         ($1, 'users/alice/events', 'e1', $2::jsonb, 1, NOW(), NOW()), \
         ($1, 'teams/beta/events',  'e2', $3::jsonb, 1, NOW(), NOW())",
    )
    .bind(project_id.as_str())
    .bind(&fields_e1)
    .bind(&fields_e2)
    .execute(&env.cust_pool)
    .await
    .expect("seed collection group documents");

    // security-rules-collection-group-rules (ADR-032, Resolution 2): unlike
    // GetDocument/writes/non-group RunQuery, a collection-group query
    // (all_descendants=true) against a collection id with NO group rule is
    // now rejected outright by default — matching real Firestore's own
    // documented behavior (a regular per-path rule never governs a
    // collection-group query; an explicit collection-group rule is
    // required). This walking-skeleton scenario predates that model and
    // tests pure collection-group SQL-matching mechanics, not
    // authorization — so it now needs an explicit, permissive group rule to
    // opt in, exactly as a real Firestore developer would need to add
    // `match /{path=**}/events/{doc} { allow read; }` to keep this query
    // working after adopting collection-group rules.
    sqlx::query(
        "INSERT INTO group_access_rules (project_id, collection_id, condition_source) \
         VALUES ($1, 'events', 'true')",
    )
    .bind(project_id.as_str())
    .execute(&env.sys_pool)
    .await
    .expect("seed permissive group access rule for events");

    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Collection group query: all_descendants=true, collection_id="events"
    let parent = format!("projects/{project_id}/databases/(default)/documents");
    let sq = StructuredQuery {
        from: vec![CollectionSelector {
            collection_id: "events".to_string(),
            all_descendants: true,
        }],
        ..Default::default()
    };
    let req = make_authed_request(
        RunQueryRequest {
            parent,
            query_type: Some(QueryType::StructuredQuery(sq)),
            ..Default::default()
        },
        &env.api_key,
    );

    let stream = client.run_query(req).await.expect("collection group query should succeed").into_inner();
    let docs = collect_query_docs(stream).await;

    assert_eq!(docs.len(), 2, "expected both e1 and e2 documents, got {}", docs.len());

    let doc_ids: Vec<String> = docs.iter().map(|d| d.name.split('/').last().unwrap_or("").to_string()).collect();
    assert!(doc_ids.contains(&"e1".to_string()), "e1 missing from results: {doc_ids:?}");
    assert!(doc_ids.contains(&"e2".to_string()), "e2 missing from results: {doc_ids:?}");
}
