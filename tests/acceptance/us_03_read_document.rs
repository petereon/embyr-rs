// SCAFFOLD: true
//! US-03 — Read a document
//!
//! As Alex, I want to call getDoc and receive the current document,
//! so that I can verify round-trip correctness and build read-heavy features.
//!
//! Driving port: gRPC data port (:8080) — GetDocument RPC
//! Red classification: MISSING_FUNCTIONALITY

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    value::ValueType,
    CreateDocumentRequest, Document, GetDocumentRequest, Value,
};
use embyr_server::{adapters::system_db::SystemDb, start_test_server};
use prost_types::Timestamp;
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

fn string_value(s: &str) -> Value {
    Value { value_type: Some(ValueType::StringValue(s.to_string())) }
}

fn integer_value(i: i64) -> Value {
    Value { value_type: Some(ValueType::IntegerValue(i)) }
}

/// AC-03a: getDoc on an existing document returns exists=true with correct fields
///
/// Given:  a provisioned project with a document at path "users/alice"
///         containing fields: name="Alice", age=30
/// When:   a GetDocument RPC is called for "users/alice"
/// Then:   the response document exists=true
/// And:    the fields match name="Alice" and age=30 exactly
#[tokio::test]
async fn get_existing_document_returns_correct_fields() {
    let env = setup("test-sk-us03-read-01", "us03-read-project-01").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Seed a document via SQL INSERT directly into the customer DB
    let fields_json = serde_json::json!({
        "name": {"t": "S", "v": "Alice"},
        "age": {"t": "I", "v": 30}
    });
    // We need the cust pool — rebuild from env. Since we stored sys_pool but not cust_pool,
    // use CreateDocument via gRPC instead (simpler, matches design intent).
    let parent = format!(
        "projects/{}/databases/(default)/documents",
        env.project_id
    );
    let mut fields = HashMap::new();
    fields.insert("name".to_string(), string_value("Alice"));
    fields.insert("age".to_string(), integer_value(30));

    let create_req = make_authed_request(
        CreateDocumentRequest {
            parent,
            collection_id: "users".to_string(),
            document_id: "alice".to_string(),
            document: Some(Document {
                name: String::new(),
                fields,
                ..Default::default()
            }),
            ..Default::default()
        },
        &env.api_key,
    );
    client
        .create_document(create_req)
        .await
        .expect("CreateDocument should succeed");

    // Now read it back
    let doc_name = format!(
        "projects/{}/databases/(default)/documents/users/alice",
        env.project_id
    );
    let get_req = make_authed_request(
        GetDocumentRequest { name: doc_name.clone(), ..Default::default() },
        &env.api_key,
    );
    let response = client
        .get_document(get_req)
        .await
        .expect("GetDocument should succeed");
    let doc = response.into_inner();

    assert!(
        doc.name.ends_with("users/alice"),
        "unexpected document name: {}",
        doc.name
    );

    let name_field = doc.fields.get("name").expect("name field missing");
    match &name_field.value_type {
        Some(ValueType::StringValue(s)) => assert_eq!(s, "Alice"),
        other => panic!("expected StringValue(\"Alice\"), got {other:?}"),
    }

    let age_field = doc.fields.get("age").expect("age field missing");
    match &age_field.value_type {
        Some(ValueType::IntegerValue(i)) => assert_eq!(*i, 30),
        other => panic!("expected IntegerValue(30), got {other:?}"),
    }

    let _ = fields_json; // suppress unused warning
}

/// AC-03b (error path): getDoc on a non-existent path returns NOT_FOUND
///
/// Given:  a provisioned project with no document at "users/ghost"
/// When:   a GetDocument RPC is called for "users/ghost"
/// Then:   the call returns status NOT_FOUND (correct Firestore behavior for missing documents)
#[tokio::test]
async fn get_nonexistent_document_returns_missing_not_error() {
    let env = setup("test-sk-us03-read-02", "us03-read-project-02").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let doc_name = format!(
        "projects/{}/databases/(default)/documents/users/ghost",
        env.project_id
    );
    let get_req = make_authed_request(
        GetDocumentRequest { name: doc_name, ..Default::default() },
        &env.api_key,
    );

    let result = client.get_document(get_req).await;
    assert!(result.is_err(), "expected error for non-existent document");
    let status = result.unwrap_err();
    assert_eq!(
        status.code(),
        tonic::Code::NotFound,
        "expected NOT_FOUND, got {:?}: {}",
        status.code(),
        status.message()
    );

    let _ = &env.sys_pool; // suppress unused warning
}

/// AC-03c: all Firestore field types round-trip correctly
///
/// Given:  a document written with fields of all supported types:
///         string, integer, float, boolean, timestamp, array, map, null, bytes, reference
/// When:   the document is read back via GetDocument
/// Then:   each field value matches the original exactly including type encoding
#[tokio::test]
async fn all_field_types_round_trip_correctly() {
    let env = setup("test-sk-us03-read-03", "us03-read-project-03").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Build a field map with all 10 FieldValue types
    let mut fields: HashMap<String, Value> = HashMap::new();

    // Null
    fields.insert(
        "null_field".to_string(),
        Value { value_type: Some(ValueType::NullValue(0)) },
    );
    // Boolean
    fields.insert(
        "bool_field".to_string(),
        Value { value_type: Some(ValueType::BooleanValue(true)) },
    );
    // Integer (i64::MAX)
    fields.insert(
        "int_field".to_string(),
        Value { value_type: Some(ValueType::IntegerValue(i64::MAX)) },
    );
    // Double
    fields.insert(
        "double_field".to_string(),
        Value { value_type: Some(ValueType::DoubleValue(3.14)) },
    );
    // String
    fields.insert(
        "str_field".to_string(),
        Value { value_type: Some(ValueType::StringValue("hello".to_string())) },
    );
    // Bytes
    fields.insert(
        "bytes_field".to_string(),
        Value { value_type: Some(ValueType::BytesValue(vec![0x01, 0x02, 0xfe, 0xff])) },
    );
    // Reference
    fields.insert(
        "ref_field".to_string(),
        Value {
            value_type: Some(ValueType::ReferenceValue(
                "projects/other/databases/(default)/documents/col/doc".to_string(),
            )),
        },
    );
    // Timestamp
    fields.insert(
        "ts_field".to_string(),
        Value {
            value_type: Some(ValueType::TimestampValue(Timestamp {
                seconds: 1_700_000_000,
                nanos: 123_456_789,
            })),
        },
    );
    // Array containing IntegerValue(1) and StringValue("x")
    fields.insert(
        "arr_field".to_string(),
        Value {
            value_type: Some(ValueType::ArrayValue(
                embyr_proto::firestore::ArrayValue {
                    values: vec![
                        Value { value_type: Some(ValueType::IntegerValue(1)) },
                        Value { value_type: Some(ValueType::StringValue("x".to_string())) },
                    ],
                },
            )),
        },
    );
    // Map containing nested_str: StringValue("nested")
    {
        let mut map_fields = HashMap::new();
        map_fields.insert(
            "nested_str".to_string(),
            Value { value_type: Some(ValueType::StringValue("nested".to_string())) },
        );
        fields.insert(
            "map_field".to_string(),
            Value {
                value_type: Some(ValueType::MapValue(embyr_proto::firestore::MapValue {
                    fields: map_fields,
                })),
            },
        );
    }

    let parent = format!(
        "projects/{}/databases/(default)/documents",
        env.project_id
    );
    let create_req = make_authed_request(
        CreateDocumentRequest {
            parent,
            collection_id: "test_col".to_string(),
            document_id: "doc1".to_string(),
            document: Some(Document {
                name: String::new(),
                fields,
                ..Default::default()
            }),
            ..Default::default()
        },
        &env.api_key,
    );
    client
        .create_document(create_req)
        .await
        .expect("CreateDocument with all field types should succeed");

    // Read it back
    let doc_name = format!(
        "projects/{}/databases/(default)/documents/test_col/doc1",
        env.project_id
    );
    let get_req = make_authed_request(
        GetDocumentRequest { name: doc_name, ..Default::default() },
        &env.api_key,
    );
    let response = client
        .get_document(get_req)
        .await
        .expect("GetDocument should succeed");
    let doc = response.into_inner();

    // Assert each field type round-trips correctly

    // Null
    let null_val = doc.fields.get("null_field").expect("null_field missing");
    match &null_val.value_type {
        Some(ValueType::NullValue(_)) => {}
        other => panic!("null_field: expected NullValue, got {other:?}"),
    }

    // Boolean
    let bool_val = doc.fields.get("bool_field").expect("bool_field missing");
    match &bool_val.value_type {
        Some(ValueType::BooleanValue(b)) => assert!(*b, "bool_field must be true"),
        other => panic!("bool_field: expected BooleanValue(true), got {other:?}"),
    }

    // Integer
    let int_val = doc.fields.get("int_field").expect("int_field missing");
    match &int_val.value_type {
        Some(ValueType::IntegerValue(i)) => {
            assert_eq!(*i, i64::MAX, "int_field must be i64::MAX")
        }
        other => panic!("int_field: expected IntegerValue(i64::MAX), got {other:?}"),
    }

    // Double — allow small floating-point tolerance
    let double_val = doc.fields.get("double_field").expect("double_field missing");
    match &double_val.value_type {
        Some(ValueType::DoubleValue(d)) => {
            let diff = (*d - 3.14_f64).abs();
            assert!(diff < 1e-9_f64, "double_field: expected ≈3.14, got {d}");
        }
        other => panic!("double_field: expected DoubleValue(3.14), got {other:?}"),
    }

    // String
    let str_val = doc.fields.get("str_field").expect("str_field missing");
    match &str_val.value_type {
        Some(ValueType::StringValue(s)) => assert_eq!(s, "hello"),
        other => panic!("str_field: expected StringValue(\"hello\"), got {other:?}"),
    }

    // Bytes
    let bytes_val = doc.fields.get("bytes_field").expect("bytes_field missing");
    match &bytes_val.value_type {
        Some(ValueType::BytesValue(b)) => {
            assert_eq!(b.as_slice(), &[0x01u8, 0x02, 0xfe, 0xff])
        }
        other => panic!("bytes_field: expected BytesValue([0x01,0x02,0xfe,0xff]), got {other:?}"),
    }

    // Reference
    let ref_val = doc.fields.get("ref_field").expect("ref_field missing");
    match &ref_val.value_type {
        Some(ValueType::ReferenceValue(r)) => assert_eq!(
            r,
            "projects/other/databases/(default)/documents/col/doc"
        ),
        other => panic!("ref_field: expected ReferenceValue, got {other:?}"),
    }

    // Timestamp
    let ts_val = doc.fields.get("ts_field").expect("ts_field missing");
    match &ts_val.value_type {
        Some(ValueType::TimestampValue(ts)) => {
            assert_eq!(ts.seconds, 1_700_000_000, "ts_field.seconds mismatch");
            assert_eq!(ts.nanos, 123_456_789, "ts_field.nanos mismatch");
        }
        other => panic!("ts_field: expected TimestampValue, got {other:?}"),
    }

    // Array: [IntegerValue(1), StringValue("x")]
    let arr_val = doc.fields.get("arr_field").expect("arr_field missing");
    match &arr_val.value_type {
        Some(ValueType::ArrayValue(arr)) => {
            assert_eq!(arr.values.len(), 2, "arr_field must have 2 elements");
            match &arr.values[0].value_type {
                Some(ValueType::IntegerValue(i)) => assert_eq!(*i, 1),
                other => panic!("arr_field[0]: expected IntegerValue(1), got {other:?}"),
            }
            match &arr.values[1].value_type {
                Some(ValueType::StringValue(s)) => assert_eq!(s, "x"),
                other => panic!("arr_field[1]: expected StringValue(\"x\"), got {other:?}"),
            }
        }
        other => panic!("arr_field: expected ArrayValue, got {other:?}"),
    }

    // Map: {nested_str: StringValue("nested")}
    let map_val = doc.fields.get("map_field").expect("map_field missing");
    match &map_val.value_type {
        Some(ValueType::MapValue(mv)) => {
            let nested = mv.fields.get("nested_str").expect("nested_str missing in map_field");
            match &nested.value_type {
                Some(ValueType::StringValue(s)) => assert_eq!(s, "nested"),
                other => panic!("map_field.nested_str: expected StringValue(\"nested\"), got {other:?}"),
            }
        }
        other => panic!("map_field: expected MapValue, got {other:?}"),
    }

    let _ = &env.sys_pool; // suppress unused warning
}

/// Error path: getDoc with wrong API key returns unauthenticated
///
/// Given:  a provisioned project
/// When:   a GetDocument RPC uses the wrong API key
/// Then:   the call returns status UNAUTHENTICATED
#[tokio::test]
async fn get_document_with_wrong_key_returns_unauthenticated() {
    let env = setup("test-sk-us03-read-04", "us03-read-project-04").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let doc_name = format!(
        "projects/{}/databases/(default)/documents/users/any",
        env.project_id
    );
    let get_req = make_authed_request(
        GetDocumentRequest { name: doc_name, ..Default::default() },
        "wrong-api-key-totally-invalid",
    );

    let result = client.get_document(get_req).await;
    assert!(result.is_err(), "expected error for wrong API key");
    let status = result.unwrap_err();
    assert_eq!(
        status.code(),
        tonic::Code::Unauthenticated,
        "expected UNAUTHENTICATED, got {:?}: {}",
        status.code(),
        status.message()
    );

    let _ = &env.sys_pool; // suppress unused warning
}
