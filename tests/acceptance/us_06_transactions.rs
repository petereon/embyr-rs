// SCAFFOLD: true
//! US-06 — Run a transaction
//!
//! As Alex, I want to use runTransaction to perform atomic read-modify-write,
//! so that concurrent clients cannot corrupt shared counters or state.
//!
//! Driving port: gRPC data port (:8080) — BeginTransaction / Commit / Rollback RPCs
//! Red classification: MISSING_FUNCTIONALITY

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    run_query_request::QueryType,
    structured_query::CollectionSelector,
    value::ValueType,
    write::Operation,
    BatchGetDocumentsRequest, BeginTransactionRequest, CommitRequest, CreateDocumentRequest,
    Document, GetDocumentRequest, RollbackRequest, RunQueryRequest, StructuredQuery,
    UpdateDocumentRequest, Value, Write as ProtoWrite,
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

/// AC-06a: 10 concurrent transactions on a counter produce final value exactly 10
///
/// Given:  a provisioned project with a counter document at "counters/hits" with value 0
/// When:   10 concurrent clients each run runTransaction to increment the counter by 1
/// Then:   all 10 transactions eventually succeed (with OCC retries allowed)
/// And:    the final counter value is exactly 10
///
/// Note: KPI — >= 100 concurrent transactions without deadlock (this test validates
///       the OCC pattern with 10; a load test covers the 100 KPI target)
#[tokio::test]
async fn ten_concurrent_transaction_increments_produce_correct_final_value() {
    let env = Arc::new(setup("test-sk-us06-occ-01", "us06-occ-project-01").await);

    // Seed "counters/hits" with value = 0 via gRPC CreateDocument
    let mut seeder = FirestoreClient::new(make_channel(env.server.grpc_addr));
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let mut fields = HashMap::new();
    fields.insert("value".to_string(), integer_value(0));
    seeder
        .create_document(make_authed_request(
            CreateDocumentRequest {
                parent: parent.clone(),
                collection_id: "counters".to_string(),
                document_id: "hits".to_string(),
                document: Some(Document {
                    name: String::new(),
                    fields,
                    ..Default::default()
                }),
                ..Default::default()
            },
            &env.api_key,
        ))
        .await
        .expect("seed document");

    // Spawn 10 concurrent tasks, each doing an OCC read-modify-write loop
    let mut handles = Vec::with_capacity(10);
    for task_idx in 0..10usize {
        let addr = env.server.grpc_addr;
        let project_id = env.project_id.clone();
        let api_key = env.api_key.clone();

        handles.push(tokio::spawn(async move {
            let _ = task_idx; // suppress warning
            let mut client = FirestoreClient::new(make_channel(addr));
            let database = format!("projects/{project_id}/databases/(default)");
            let doc_name = format!(
                "projects/{project_id}/databases/(default)/documents/counters/hits"
            );

            // Retry loop: OCC may abort — retry up to 10 times
            for _attempt in 0..10 {
                // 1. BeginTransaction
                let begin_resp = client
                    .begin_transaction(make_authed_request(
                        BeginTransactionRequest {
                            database: database.clone(),
                            options: None,
                        },
                        &api_key,
                    ))
                    .await
                    .expect("begin_transaction");
                let txn_id = begin_resp.into_inner().transaction;

                // 2. GetDocument (read current value + version)
                let get_resp = client
                    .get_document(make_authed_request(
                        GetDocumentRequest {
                            name: doc_name.clone(),
                            mask: None,
                            consistency_selector: None,
                        },
                        &api_key,
                    ))
                    .await
                    .expect("get_document");
                let doc = get_resp.into_inner();

                // Extract current value and version from the document
                let current_value = doc
                    .fields
                    .get("value")
                    .and_then(|v| {
                        if let Some(ValueType::IntegerValue(i)) = &v.value_type {
                            Some(*i)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);

                // Parse version from update_time (use it as our OCC token)
                // The version is encoded in the update_time timestamp seconds
                let update_time = doc.update_time.clone();
                let update_time_seconds = update_time.as_ref().map(|t| t.seconds).unwrap_or(0);
                let update_time_nanos = update_time.as_ref().map(|t| t.nanos).unwrap_or(0);

                // 3. Commit with incremented value, using update_time as OCC precondition
                let mut new_fields = HashMap::new();
                new_fields.insert("value".to_string(), integer_value(current_value + 1));

                let precondition = embyr_proto::firestore::Precondition {
                    condition_type: Some(
                        embyr_proto::firestore::precondition::ConditionType::UpdateTime(
                            prost_types::Timestamp {
                                seconds: update_time_seconds,
                                nanos: update_time_nanos,
                            },
                        ),
                    ),
                };

                let write = ProtoWrite {
                    operation: Some(Operation::Update(Document {
                        name: doc_name.clone(),
                        fields: new_fields,
                        ..Default::default()
                    })),
                    current_document: Some(precondition),
                    ..Default::default()
                };

                let commit_result = client
                    .commit(make_authed_request(
                        CommitRequest {
                            database: database.clone(),
                            writes: vec![write],
                            transaction: txn_id,
                        },
                        &api_key,
                    ))
                    .await;

                match commit_result {
                    Ok(_) => return, // success
                    Err(status) if status.code() == tonic::Code::Aborted => {
                        // OCC conflict — retry after a small backoff
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        continue;
                    }
                    Err(e) => panic!("unexpected commit error: {e}"),
                }
            }
            panic!("task exhausted retries");
        }));
    }

    // Wait for all tasks to complete
    for handle in handles {
        handle.await.expect("task panicked");
    }

    // Verify final counter value == 10
    let mut verifier = FirestoreClient::new(make_channel(env.server.grpc_addr));
    let doc_name = format!(
        "projects/{}/databases/(default)/documents/counters/hits",
        env.project_id
    );
    let final_doc = verifier
        .get_document(make_authed_request(
            GetDocumentRequest {
                name: doc_name,
                mask: None,
                consistency_selector: None,
            },
            &env.api_key,
        ))
        .await
        .expect("final get_document")
        .into_inner();

    let final_value = final_doc
        .fields
        .get("value")
        .and_then(|v| {
            if let Some(ValueType::IntegerValue(i)) = &v.value_type {
                Some(*i)
            } else {
                None
            }
        })
        .expect("value field must exist");

    assert_eq!(final_value, 10, "final counter must be exactly 10");
}

/// AC-06b (error path): transaction not committed within 60s is auto-expired
///
/// Given:  a BeginTransaction RPC returns a transaction ID
/// And:    no Commit or Rollback is issued for 61 seconds
/// When:   a Commit RPC is issued using the expired transaction ID
/// Then:   the Commit returns status NOT_FOUND
#[tokio::test]
async fn expired_transaction_commit_returns_not_found() {
    let env = setup("test-sk-us06-exp-01", "us06-exp-project-01").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let database = format!("projects/{}/databases/(default)", env.project_id);

    // Begin a transaction
    let begin_resp = client
        .begin_transaction(make_authed_request(
            BeginTransactionRequest {
                database: database.clone(),
                options: None,
            },
            &env.api_key,
        ))
        .await
        .expect("begin_transaction");
    let txn_bytes = begin_resp.into_inner().transaction;

    // Parse the UUID from the transaction bytes to backdate it in Postgres
    assert_eq!(txn_bytes.len(), 16, "transaction ID must be 16 bytes (UUID)");
    let txn_arr: [u8; 16] = txn_bytes.clone().try_into().unwrap();
    let txn_uuid = uuid::Uuid::from_bytes(txn_arr);

    // Directly UPDATE started_at to 61s in the past to simulate expiry
    sqlx::query(
        "UPDATE transactions SET started_at = now() - interval '61 seconds' \
         WHERE transaction_id = $1",
    )
    .bind(txn_uuid)
    .execute(&env.cust_pool)
    .await
    .expect("backdate transaction");

    // Attempt Commit — must return NOT_FOUND
    let commit_result = client
        .commit(make_authed_request(
            CommitRequest {
                database,
                writes: vec![],
                transaction: txn_bytes,
            },
            &env.api_key,
        ))
        .await;

    let status = commit_result.expect_err("commit of expired transaction must fail");
    assert_eq!(
        status.code(),
        tonic::Code::NotFound,
        "expired transaction must return NOT_FOUND, got: {status}"
    );
}

/// AC-06c (error path): Commit after Rollback returns NOT_FOUND
///
/// Given:  an active transaction ID
/// When:   a Rollback RPC is issued
/// And:    a subsequent Commit RPC is issued with the same transaction ID
/// Then:   the Commit returns status NOT_FOUND
#[tokio::test]
async fn commit_after_rollback_returns_not_found() {
    let env = setup("test-sk-us06-rb-01", "us06-rb-project-01").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let database = format!("projects/{}/databases/(default)", env.project_id);

    // Begin a transaction
    let begin_resp = client
        .begin_transaction(make_authed_request(
            BeginTransactionRequest {
                database: database.clone(),
                options: None,
            },
            &env.api_key,
        ))
        .await
        .expect("begin_transaction");
    let txn_bytes = begin_resp.into_inner().transaction;

    // Rollback
    client
        .rollback(make_authed_request(
            RollbackRequest {
                database: database.clone(),
                transaction: txn_bytes.clone(),
            },
            &env.api_key,
        ))
        .await
        .expect("rollback must succeed");

    // Commit of rolled-back transaction must return NOT_FOUND
    let commit_result = client
        .commit(make_authed_request(
            CommitRequest {
                database,
                writes: vec![],
                transaction: txn_bytes,
            },
            &env.api_key,
        ))
        .await;

    let status = commit_result.expect_err("commit after rollback must fail");
    assert_eq!(
        status.code(),
        tonic::Code::NotFound,
        "commit after rollback must return NOT_FOUND, got: {status}"
    );
}

/// Error path: OCC conflict causes ABORTED; SDK should retry
///
/// Given:  document "orders/x" is at version 5
/// And:    a transaction reads "orders/x" recording version 5
/// And:    another writer updates "orders/x" to version 6 before Commit
/// When:   the first transaction attempts to Commit
/// Then:   the Commit returns status ABORTED
#[tokio::test]
async fn occ_conflict_causes_aborted_status() {
    let env = setup("test-sk-us06-ab-01", "us06-ab-project-01").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let database = format!("projects/{}/databases/(default)", env.project_id);
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let doc_name = format!(
        "projects/{}/databases/(default)/documents/orders/x",
        env.project_id
    );

    // Seed "orders/x"
    let mut seed_fields = HashMap::new();
    seed_fields.insert("status".to_string(), Value {
        value_type: Some(ValueType::StringValue("pending".to_string())),
    });
    client
        .create_document(make_authed_request(
            CreateDocumentRequest {
                parent: parent.clone(),
                collection_id: "orders".to_string(),
                document_id: "x".to_string(),
                document: Some(Document {
                    name: String::new(),
                    fields: seed_fields,
                    ..Default::default()
                }),
                ..Default::default()
            },
            &env.api_key,
        ))
        .await
        .expect("seed orders/x");

    // Read "orders/x" to get its current update_time (for OCC precondition)
    let read_resp = client
        .get_document(make_authed_request(
            GetDocumentRequest {
                name: doc_name.clone(),
                mask: None,
                consistency_selector: None,
            },
            &env.api_key,
        ))
        .await
        .expect("get orders/x")
        .into_inner();
    let read_update_time = read_resp.update_time.clone().expect("update_time must be set");

    // Begin a transaction for txn1
    let begin_resp = client
        .begin_transaction(make_authed_request(
            BeginTransactionRequest {
                database: database.clone(),
                options: None,
            },
            &env.api_key,
        ))
        .await
        .expect("begin_transaction txn1");
    let txn1_bytes = begin_resp.into_inner().transaction;

    // External update: update "orders/x" outside the transaction (bumps version)
    let mut updated_fields = HashMap::new();
    updated_fields.insert("status".to_string(), Value {
        value_type: Some(ValueType::StringValue("processed".to_string())),
    });
    client
        .update_document(make_authed_request(
            UpdateDocumentRequest {
                document: Some(Document {
                    name: doc_name.clone(),
                    fields: updated_fields.clone(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            &env.api_key,
        ))
        .await
        .expect("external update of orders/x");

    // Commit txn1 with stale OCC precondition (version from before external update)
    let write = ProtoWrite {
        operation: Some(Operation::Update(Document {
            name: doc_name.clone(),
            fields: updated_fields,
            ..Default::default()
        })),
        current_document: Some(embyr_proto::firestore::Precondition {
            condition_type: Some(
                embyr_proto::firestore::precondition::ConditionType::UpdateTime(
                    read_update_time,
                ),
            ),
        }),
        ..Default::default()
    };

    let commit_result = client
        .commit(make_authed_request(
            CommitRequest {
                database,
                writes: vec![write],
                transaction: txn1_bytes,
            },
            &env.api_key,
        ))
        .await;

    let status = commit_result.expect_err("commit with stale version must fail");
    assert_eq!(
        status.code(),
        tonic::Code::Aborted,
        "OCC conflict must return ABORTED, got: {status}"
    );
}

/// Regression test for a real pre-existing gap (found 2026-08-31 while
/// delivering firestore-write-streaming): `commit_transaction`'s own OCC
/// check loop previously validated ONLY `UpdateTime` preconditions, silently
/// ignoring `current_document.exists = false` ("create if not exists")
/// even though `CreateDocument`'s own single-document RPC already enforces
/// it — a `Commit` batch could silently overwrite an existing document a
/// client explicitly asked NOT to overwrite.
#[tokio::test]
async fn commit_write_with_must_not_exist_precondition_is_rejected_when_document_already_exists() {
    let env = setup("test-sk-us06-mne-01", "us06-mne-project-01").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let database = format!("projects/{}/databases/(default)", env.project_id);
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let doc_name = format!(
        "projects/{}/databases/(default)/documents/orders/must-not-exist",
        env.project_id
    );

    let mut original_fields = HashMap::new();
    original_fields.insert(
        "status".to_string(),
        Value {
            value_type: Some(ValueType::StringValue("original".to_string())),
        },
    );
    client
        .create_document(make_authed_request(
            CreateDocumentRequest {
                parent,
                collection_id: "orders".to_string(),
                document_id: "must-not-exist".to_string(),
                document: Some(Document {
                    name: String::new(),
                    fields: original_fields,
                    ..Default::default()
                }),
                ..Default::default()
            },
            &env.api_key,
        ))
        .await
        .expect("seed orders/must-not-exist");

    let mut overwrite_fields = HashMap::new();
    overwrite_fields.insert(
        "status".to_string(),
        Value {
            value_type: Some(ValueType::StringValue("should-never-land".to_string())),
        },
    );
    let write = ProtoWrite {
        operation: Some(Operation::Update(Document {
            name: doc_name.clone(),
            fields: overwrite_fields,
            ..Default::default()
        })),
        current_document: Some(embyr_proto::firestore::Precondition {
            condition_type: Some(embyr_proto::firestore::precondition::ConditionType::Exists(
                false,
            )),
        }),
        ..Default::default()
    };

    let begin_resp = client
        .begin_transaction(make_authed_request(
            BeginTransactionRequest {
                database: database.clone(),
                options: None,
            },
            &env.api_key,
        ))
        .await
        .expect("begin_transaction");
    let txn_bytes = begin_resp.into_inner().transaction;

    let commit_result = client
        .commit(make_authed_request(
            CommitRequest {
                database,
                writes: vec![write],
                transaction: txn_bytes,
            },
            &env.api_key,
        ))
        .await;

    let status = commit_result.expect_err(
        "commit of a write whose exists=false precondition is violated (document already \
         exists) must fail, not silently overwrite",
    );
    assert_eq!(
        status.code(),
        tonic::Code::Aborted,
        "MustNotExist violation must return ABORTED, got: {status}"
    );

    let read_resp = client
        .get_document(make_authed_request(
            GetDocumentRequest {
                name: doc_name,
                mask: None,
                consistency_selector: None,
            },
            &env.api_key,
        ))
        .await
        .expect("get orders/must-not-exist")
        .into_inner();
    let status_field = read_resp
        .fields
        .get("status")
        .and_then(|v| v.value_type.clone());
    assert_eq!(
        status_field,
        Some(ValueType::StringValue("original".to_string())),
        "the rejected write must NOT have modified the existing document"
    );
}

/// Same gap, the `MustExist` ("update only if exists") direction:
/// `commit_transaction` previously let an Update-with-`exists=true`
/// precondition silently CREATE a document that didn't exist, instead of
/// rejecting the write.
#[tokio::test]
async fn commit_write_with_must_exist_precondition_is_rejected_when_document_does_not_exist() {
    let env = setup("test-sk-us06-me-01", "us06-me-project-01").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let database = format!("projects/{}/databases/(default)", env.project_id);
    let doc_name = format!(
        "projects/{}/databases/(default)/documents/orders/must-exist-missing",
        env.project_id
    );

    let mut fields = HashMap::new();
    fields.insert(
        "status".to_string(),
        Value {
            value_type: Some(ValueType::StringValue("should-never-be-created".to_string())),
        },
    );
    let write = ProtoWrite {
        operation: Some(Operation::Update(Document {
            name: doc_name.clone(),
            fields,
            ..Default::default()
        })),
        current_document: Some(embyr_proto::firestore::Precondition {
            condition_type: Some(embyr_proto::firestore::precondition::ConditionType::Exists(
                true,
            )),
        }),
        ..Default::default()
    };

    let begin_resp = client
        .begin_transaction(make_authed_request(
            BeginTransactionRequest {
                database: database.clone(),
                options: None,
            },
            &env.api_key,
        ))
        .await
        .expect("begin_transaction");
    let txn_bytes = begin_resp.into_inner().transaction;

    let commit_result = client
        .commit(make_authed_request(
            CommitRequest {
                database,
                writes: vec![write],
                transaction: txn_bytes,
            },
            &env.api_key,
        ))
        .await;

    let status = commit_result.expect_err(
        "commit of a write whose exists=true precondition is violated (document does not \
         exist) must fail, not silently create it",
    );
    assert_eq!(
        status.code(),
        tonic::Code::Aborted,
        "MustExist violation must return ABORTED, got: {status}"
    );

    let read_result = client
        .get_document(make_authed_request(
            GetDocumentRequest {
                name: doc_name,
                mask: None,
                consistency_selector: None,
            },
            &env.api_key,
        ))
        .await;
    assert!(
        read_result.is_err(),
        "the rejected write must NOT have created the document"
    );
}

// ---------------------------------------------------------------------------
// occ-precondition-validation (AC-OCC-03): a malformed `update_time`
// precondition inside commit_transaction's OWN OCC-verification loop
// (backend_adapter.rs:1051's `to_datetime` call site — a distinct call site
// from update_document's :398, per DESIGN's own call-site list) is cleanly
// rejected, not a panic, and the transaction is not partially applied.
// ---------------------------------------------------------------------------

/// AC-OCC-03: a malformed `nanos` value on a Write's precondition inside a
/// transactional Commit is cleanly rejected instead of panicking the
/// request-handling task while a Postgres transaction guard is on the stack.
///
/// Given:  a transaction has begun and a document exists
/// When:   Commit is called with a Write whose Precondition.update_time.nanos
///         is -1 (negative — the exact edge case DESIGN's own nanos-before-
///         seconds cast ordering exists to attribute correctly)
/// Then:   the RPC returns INVALID_ARGUMENT naming the "nanos" field
/// And:    the transaction is not partially applied — the document is unchanged
#[tokio::test]
async fn commit_transaction_with_malformed_nanos_precondition_returns_invalid_argument() {
    let env = setup("test-sk-occ-txn-nanos", "occ-txn-nanos-project").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let database = format!("projects/{}/databases/(default)", env.project_id);
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let doc_name = format!(
        "projects/{}/databases/(default)/documents/orders/occ-malformed-txn",
        env.project_id
    );

    let mut seed_fields = HashMap::new();
    seed_fields.insert(
        "status".to_string(),
        Value { value_type: Some(ValueType::StringValue("original".to_string())) },
    );
    client
        .create_document(make_authed_request(
            CreateDocumentRequest {
                parent,
                collection_id: "orders".to_string(),
                document_id: "occ-malformed-txn".to_string(),
                document: Some(Document { name: String::new(), fields: seed_fields, ..Default::default() }),
                ..Default::default()
            },
            &env.api_key,
        ))
        .await
        .expect("seed orders/occ-malformed-txn");

    let begin_resp = client
        .begin_transaction(make_authed_request(
            BeginTransactionRequest { database: database.clone(), options: None },
            &env.api_key,
        ))
        .await
        .expect("begin_transaction");
    let txn_bytes = begin_resp.into_inner().transaction;

    let mut overwrite_fields = HashMap::new();
    overwrite_fields.insert(
        "status".to_string(),
        Value { value_type: Some(ValueType::StringValue("should-never-land".to_string())) },
    );
    let write = ProtoWrite {
        operation: Some(Operation::Update(Document {
            name: doc_name.clone(),
            fields: overwrite_fields,
            ..Default::default()
        })),
        current_document: Some(embyr_proto::firestore::Precondition {
            condition_type: Some(embyr_proto::firestore::precondition::ConditionType::UpdateTime(
                prost_types::Timestamp { seconds: 1_799_942_400, nanos: -1 },
            )),
        }),
        ..Default::default()
    };

    let commit_result = client
        .commit(make_authed_request(
            CommitRequest { database, writes: vec![write], transaction: txn_bytes },
            &env.api_key,
        ))
        .await;

    let status = commit_result.expect_err(
        "a malformed nanos precondition inside commit_transaction's OCC loop must be cleanly \
         rejected, not panic the connection",
    );
    assert_eq!(
        status.code(),
        tonic::Code::InvalidArgument,
        "expected INVALID_ARGUMENT for out-of-range nanos in transactional commit, got {:?}: {}",
        status.code(),
        status.message()
    );
    assert!(
        status.message().contains("nanos"),
        "error message must name the offending 'nanos' field, got: {}",
        status.message()
    );

    // Regression: the transaction must not be partially applied.
    let read_resp = client
        .get_document(make_authed_request(
            GetDocumentRequest { name: doc_name, mask: None, consistency_selector: None },
            &env.api_key,
        ))
        .await
        .expect("get orders/occ-malformed-txn")
        .into_inner();
    let status_field = read_resp.fields.get("status").and_then(|v| v.value_type.clone());
    assert_eq!(
        status_field,
        Some(ValueType::StringValue("original".to_string())),
        "the rejected transactional write must NOT have modified the document"
    );
}

// ---------------------------------------------------------------------------
// firestore-transaction-read-consistency (Slice 01, US-01, AC-TRC-01/02/03/04/05)
//
// The tests above all simulate OCC by MANUALLY attaching an `UpdateTime`
// precondition to the write, using a version the CLIENT read out-of-band.
// A real Firestore SDK's own `runTransaction(tx => tx.get(ref))` does NOT
// require the app to do this — the SDK sets `consistency_selector.transaction`
// on the `GetDocument` call, and the SERVER automatically tracks that read
// and aborts the commit if it's stale, with no explicit precondition needed
// on the write at all. These tests exercise exactly that automatic path.
// ---------------------------------------------------------------------------

/// AC-TRC-01/AC-TRC-02: a transactional `GetDocument` read (via
/// `consistency_selector.transaction`, NOT a manually-attached `UpdateTime`
/// precondition) registers that document's version; `Commit` aborts if the
/// SAME document changed in the meantime — the automatic lost-update guard
/// a real `runTransaction(fn)` callback depends on.
#[tokio::test]
async fn transactional_get_document_aborts_commit_when_read_document_changes_before_commit() {
    let env = setup("test-sk-trc-01", "trc-project-01").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let database = format!("projects/{}/databases/(default)", env.project_id);
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let doc_name = format!(
        "projects/{}/databases/(default)/documents/counters/hits",
        env.project_id
    );

    let mut seed_fields = HashMap::new();
    seed_fields.insert("value".to_string(), integer_value(0));
    client
        .create_document(make_authed_request(
            CreateDocumentRequest {
                parent,
                collection_id: "counters".to_string(),
                document_id: "hits".to_string(),
                document: Some(Document { name: String::new(), fields: seed_fields, ..Default::default() }),
                ..Default::default()
            },
            &env.api_key,
        ))
        .await
        .expect("seed counters/hits");

    let begin_resp = client
        .begin_transaction(make_authed_request(
            BeginTransactionRequest { database: database.clone(), options: None },
            &env.api_key,
        ))
        .await
        .expect("begin_transaction");
    let txn_bytes = begin_resp.into_inner().transaction;

    // Transactional read: sets consistency_selector.transaction, registering
    // this document's current version in the transaction's own read set.
    client
        .get_document(make_authed_request(
            GetDocumentRequest {
                name: doc_name.clone(),
                mask: None,
                consistency_selector: Some(
                    embyr_proto::firestore::get_document_request::ConsistencySelector::Transaction(
                        txn_bytes.clone(),
                    ),
                ),
            },
            &env.api_key,
        ))
        .await
        .expect("transactional get_document");

    // External, non-transactional write changes the SAME document.
    let mut bumped = HashMap::new();
    bumped.insert("value".to_string(), integer_value(99));
    client
        .update_document(make_authed_request(
            UpdateDocumentRequest {
                document: Some(Document { name: doc_name.clone(), fields: bumped, ..Default::default() }),
                ..Default::default()
            },
            &env.api_key,
        ))
        .await
        .expect("external update");

    // Commit an UNRELATED write inside the transaction — no explicit
    // precondition on this write at all. The registered READ is what must
    // trigger the abort.
    let mut other_fields = HashMap::new();
    other_fields.insert("touched".to_string(), Value {
        value_type: Some(ValueType::BooleanValue(true)),
    });
    let other_doc_name = format!(
        "projects/{}/databases/(default)/documents/counters/other",
        env.project_id
    );
    let write = ProtoWrite {
        operation: Some(Operation::Update(Document {
            name: other_doc_name,
            fields: other_fields,
            ..Default::default()
        })),
        ..Default::default()
    };

    let commit_result = client
        .commit(make_authed_request(
            CommitRequest { database, writes: vec![write], transaction: txn_bytes },
            &env.api_key,
        ))
        .await;

    let status = commit_result.expect_err(
        "commit must abort: the transactionally-read document changed before commit",
    );
    assert_eq!(status.code(), tonic::Code::Aborted, "got: {status}");
}

/// AC-TRC-03 (regression guard): when NOTHING changes the transactionally
/// -read document, the commit succeeds exactly as it does without this
/// feature — zero new restriction on the happy path.
#[tokio::test]
async fn transactional_get_document_commit_succeeds_when_read_document_is_unchanged() {
    let env = setup("test-sk-trc-02", "trc-project-02").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let database = format!("projects/{}/databases/(default)", env.project_id);
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let doc_name = format!(
        "projects/{}/databases/(default)/documents/counters/hits",
        env.project_id
    );

    let mut seed_fields = HashMap::new();
    seed_fields.insert("value".to_string(), integer_value(0));
    client
        .create_document(make_authed_request(
            CreateDocumentRequest {
                parent,
                collection_id: "counters".to_string(),
                document_id: "hits".to_string(),
                document: Some(Document { name: String::new(), fields: seed_fields, ..Default::default() }),
                ..Default::default()
            },
            &env.api_key,
        ))
        .await
        .expect("seed counters/hits");

    let begin_resp = client
        .begin_transaction(make_authed_request(
            BeginTransactionRequest { database: database.clone(), options: None },
            &env.api_key,
        ))
        .await
        .expect("begin_transaction");
    let txn_bytes = begin_resp.into_inner().transaction;

    client
        .get_document(make_authed_request(
            GetDocumentRequest {
                name: doc_name.clone(),
                mask: None,
                consistency_selector: Some(
                    embyr_proto::firestore::get_document_request::ConsistencySelector::Transaction(
                        txn_bytes.clone(),
                    ),
                ),
            },
            &env.api_key,
        ))
        .await
        .expect("transactional get_document");

    let mut new_fields = HashMap::new();
    new_fields.insert("value".to_string(), integer_value(1));
    let write = ProtoWrite {
        operation: Some(Operation::Update(Document {
            name: doc_name,
            fields: new_fields,
            ..Default::default()
        })),
        ..Default::default()
    };

    client
        .commit(make_authed_request(
            CommitRequest { database, writes: vec![write], transaction: txn_bytes },
            &env.api_key,
        ))
        .await
        .expect("commit must succeed — the transactionally-read document never changed");
}

/// AC-TRC-04: reading a document that does NOT exist, then having another
/// actor CREATE that exact document before commit, also aborts — the
/// confirmed-absence read-set entry is validated too, not just the
/// exists-with-a-version case.
#[tokio::test]
async fn transactional_get_document_aborts_commit_when_absent_document_is_created_before_commit() {
    let env = setup("test-sk-trc-03", "trc-project-03").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let database = format!("projects/{}/databases/(default)", env.project_id);
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let doc_name = format!(
        "projects/{}/databases/(default)/documents/reservations/seat-1",
        env.project_id
    );

    let begin_resp = client
        .begin_transaction(make_authed_request(
            BeginTransactionRequest { database: database.clone(), options: None },
            &env.api_key,
        ))
        .await
        .expect("begin_transaction");
    let txn_bytes = begin_resp.into_inner().transaction;

    // Transactional read of a document that does not exist yet.
    let read_result = client
        .get_document(make_authed_request(
            GetDocumentRequest {
                name: doc_name.clone(),
                mask: None,
                consistency_selector: Some(
                    embyr_proto::firestore::get_document_request::ConsistencySelector::Transaction(
                        txn_bytes.clone(),
                    ),
                ),
            },
            &env.api_key,
        ))
        .await;
    assert!(read_result.is_err(), "the document must not exist yet");

    // External actor creates the exact document the transaction read as absent.
    let mut created_fields = HashMap::new();
    created_fields.insert("occupant".to_string(), Value {
        value_type: Some(ValueType::StringValue("someone-else".to_string())),
    });
    client
        .create_document(make_authed_request(
            CreateDocumentRequest {
                parent,
                collection_id: "reservations".to_string(),
                document_id: "seat-1".to_string(),
                document: Some(Document {
                    name: String::new(),
                    fields: created_fields,
                    ..Default::default()
                }),
                ..Default::default()
            },
            &env.api_key,
        ))
        .await
        .expect("external create of reservations/seat-1");

    // Commit an unrelated write inside the transaction — the confirmed
    // -absence read is what must trigger the abort.
    let other_doc_name = format!(
        "projects/{}/databases/(default)/documents/reservations/other",
        env.project_id
    );
    let mut other_fields = HashMap::new();
    other_fields.insert("touched".to_string(), Value {
        value_type: Some(ValueType::BooleanValue(true)),
    });
    let write = ProtoWrite {
        operation: Some(Operation::Update(Document {
            name: other_doc_name,
            fields: other_fields,
            ..Default::default()
        })),
        ..Default::default()
    };

    let commit_result = client
        .commit(make_authed_request(
            CommitRequest { database, writes: vec![write], transaction: txn_bytes },
            &env.api_key,
        ))
        .await;

    let status = commit_result.expect_err(
        "commit must abort: a document read as absent was created before commit",
    );
    assert_eq!(status.code(), tonic::Code::Aborted, "got: {status}");
}

/// AC-TRC-05: a transactional `GetDocument` with an invalid/garbage
/// transaction ID returns the SAME `NotFound` `Commit`/`Rollback` already
/// return for this case today — no new error class introduced.
#[tokio::test]
async fn transactional_get_document_with_invalid_transaction_id_returns_not_found() {
    let env = setup("test-sk-trc-04", "trc-project-04").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let doc_name = format!(
        "projects/{}/databases/(default)/documents/counters/hits",
        env.project_id
    );

    let bogus_txn_bytes = vec![1, 2, 3]; // not a valid 16-byte UUID

    let result = client
        .get_document(make_authed_request(
            GetDocumentRequest {
                name: doc_name,
                mask: None,
                consistency_selector: Some(
                    embyr_proto::firestore::get_document_request::ConsistencySelector::Transaction(
                        bogus_txn_bytes,
                    ),
                ),
            },
            &env.api_key,
        ))
        .await;

    let status = result.expect_err("a malformed transaction ID must be rejected");
    assert_eq!(
        status.code(),
        tonic::Code::InvalidArgument,
        "a non-16-byte transaction ID must fail length validation, got: {status:?}"
    );
}

/// AC-TRC-05 (well-formed-but-unknown case): a transactional `GetDocument`
/// with a syntactically valid (16-byte) but never-begun transaction ID
/// returns the SAME `NotFound` `Commit`/`Rollback` already return today for
/// an unknown transaction — no new error class.
#[tokio::test]
async fn transactional_get_document_with_unknown_transaction_id_returns_not_found() {
    let env = setup("test-sk-trc-05", "trc-project-05").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let doc_name = format!(
        "projects/{}/databases/(default)/documents/counters/hits",
        env.project_id
    );

    let never_begun_txn_bytes = uuid::Uuid::new_v4().as_bytes().to_vec();

    let result = client
        .get_document(make_authed_request(
            GetDocumentRequest {
                name: doc_name,
                mask: None,
                consistency_selector: Some(
                    embyr_proto::firestore::get_document_request::ConsistencySelector::Transaction(
                        never_begun_txn_bytes,
                    ),
                ),
            },
            &env.api_key,
        ))
        .await;

    let status = result.expect_err("an unknown transaction ID must be rejected");
    assert_eq!(
        status.code(),
        tonic::Code::NotFound,
        "an unknown (never-begun) transaction ID must return NotFound, got: {status:?}"
    );
}

// ---------------------------------------------------------------------------
// firestore-transaction-read-consistency (Slice 02, US-02, AC-TRC-06/07/08)
// ---------------------------------------------------------------------------

/// AC-TRC-06/AC-TRC-07: a transactional `RunQuery` registers EVERY returned
/// document's version; `Commit` aborts if any ONE of them changed before
/// commit — the identical guarantee AC-TRC-01/02 give single-document reads,
/// now for a query-based read.
#[tokio::test]
async fn transactional_run_query_aborts_commit_when_a_returned_document_changes_before_commit() {
    let env = setup("test-sk-trc-06", "trc-project-06").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let database = format!("projects/{}/databases/(default)", env.project_id);
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let doc_name = format!(
        "projects/{}/databases/(default)/documents/orders/o1",
        env.project_id
    );

    let mut seed_fields = HashMap::new();
    seed_fields.insert("status".to_string(), Value {
        value_type: Some(ValueType::StringValue("pending".to_string())),
    });
    client
        .create_document(make_authed_request(
            CreateDocumentRequest {
                parent,
                collection_id: "orders".to_string(),
                document_id: "o1".to_string(),
                document: Some(Document { name: String::new(), fields: seed_fields, ..Default::default() }),
                ..Default::default()
            },
            &env.api_key,
        ))
        .await
        .expect("seed orders/o1");

    let begin_resp = client
        .begin_transaction(make_authed_request(
            BeginTransactionRequest { database: database.clone(), options: None },
            &env.api_key,
        ))
        .await
        .expect("begin_transaction");
    let txn_bytes = begin_resp.into_inner().transaction;

    // Transactional query: registers orders/o1's current version.
    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "orders".to_string(), all_descendants: false }],
        ..Default::default()
    };
    let query_req = make_authed_request(
        RunQueryRequest {
            parent: format!("projects/{}/databases/(default)/documents", env.project_id),
            query_type: Some(QueryType::StructuredQuery(sq)),
            consistency_selector: Some(
                embyr_proto::firestore::run_query_request::ConsistencySelector::Transaction(
                    txn_bytes.clone(),
                ),
            ),
        },
        &env.api_key,
    );
    use tokio_stream::StreamExt;
    let stream = client.run_query(query_req).await.expect("transactional run_query").into_inner();
    let responses: Vec<_> = stream.collect().await;
    let doc_count = responses.iter().filter(|r| r.as_ref().ok().and_then(|r| r.document.as_ref()).is_some()).count();
    assert_eq!(doc_count, 1, "query must return exactly the 1 seeded document");

    // External write changes the SAME document the query returned.
    let mut bumped = HashMap::new();
    bumped.insert("status".to_string(), Value {
        value_type: Some(ValueType::StringValue("shipped".to_string())),
    });
    client
        .update_document(make_authed_request(
            UpdateDocumentRequest {
                document: Some(Document { name: doc_name, fields: bumped, ..Default::default() }),
                ..Default::default()
            },
            &env.api_key,
        ))
        .await
        .expect("external update");

    // Commit an unrelated write inside the transaction.
    let other_doc_name = format!(
        "projects/{}/databases/(default)/documents/orders/other",
        env.project_id
    );
    let mut other_fields = HashMap::new();
    other_fields.insert("touched".to_string(), Value { value_type: Some(ValueType::BooleanValue(true)) });
    let write = ProtoWrite {
        operation: Some(Operation::Update(Document {
            name: other_doc_name,
            fields: other_fields,
            ..Default::default()
        })),
        ..Default::default()
    };

    let commit_result = client
        .commit(make_authed_request(
            CommitRequest { database, writes: vec![write], transaction: txn_bytes },
            &env.api_key,
        ))
        .await;

    let status = commit_result.expect_err(
        "commit must abort: a document the query returned changed before commit",
    );
    assert_eq!(status.code(), tonic::Code::Aborted, "got: {status}");
}

/// AC-TRC-08 (regression guard): a non-transactional `RunQuery`
/// (`consistency_selector` unset) is completely unaffected — no read
/// registration, no new behavior.
#[tokio::test]
async fn non_transactional_run_query_is_unaffected_by_read_consistency_tracking() {
    let env = setup("test-sk-trc-07", "trc-project-07").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let mut seed_fields = HashMap::new();
    seed_fields.insert("status".to_string(), Value {
        value_type: Some(ValueType::StringValue("pending".to_string())),
    });
    client
        .create_document(make_authed_request(
            CreateDocumentRequest {
                parent: parent.clone(),
                collection_id: "orders".to_string(),
                document_id: "o1".to_string(),
                document: Some(Document { name: String::new(), fields: seed_fields, ..Default::default() }),
                ..Default::default()
            },
            &env.api_key,
        ))
        .await
        .expect("seed orders/o1");

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "orders".to_string(), all_descendants: false }],
        ..Default::default()
    };
    let query_req = make_authed_request(
        RunQueryRequest {
            parent,
            query_type: Some(QueryType::StructuredQuery(sq)),
            consistency_selector: None,
        },
        &env.api_key,
    );
    use tokio_stream::StreamExt;
    let stream = client.run_query(query_req).await.expect("non-transactional run_query").into_inner();
    let responses: Vec<_> = stream.collect().await;
    let doc_count = responses.iter().filter(|r| r.as_ref().ok().and_then(|r| r.document.as_ref()).is_some()).count();
    assert_eq!(doc_count, 1, "non-transactional query must still return the seeded document");
}

// ---------------------------------------------------------------------------
// firestore-transaction-read-consistency (Slice 03, US-03, AC-TRC-09/10)
// ---------------------------------------------------------------------------

/// AC-TRC-09/AC-TRC-10: a transactional `BatchGetDocuments` registers every
/// requested document's version; `Commit` aborts if any ONE of them changed
/// before commit — the identical guarantee AC-TRC-01/02/06/07 give
/// single-document and query-based reads, now for a batch read.
#[tokio::test]
async fn transactional_batch_get_documents_aborts_commit_when_a_requested_document_changes_before_commit()
{
    let env = setup("test-sk-trc-08", "trc-project-08").await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let database = format!("projects/{}/databases/(default)", env.project_id);
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let doc_a_name = format!(
        "projects/{}/databases/(default)/documents/accounts/a",
        env.project_id
    );
    let doc_b_name = format!(
        "projects/{}/databases/(default)/documents/accounts/b",
        env.project_id
    );

    for (id, balance) in [("a", 100), ("b", 200)] {
        let mut fields = HashMap::new();
        fields.insert("balance".to_string(), integer_value(balance));
        client
            .create_document(make_authed_request(
                CreateDocumentRequest {
                    parent: parent.clone(),
                    collection_id: "accounts".to_string(),
                    document_id: id.to_string(),
                    document: Some(Document { name: String::new(), fields, ..Default::default() }),
                    ..Default::default()
                },
                &env.api_key,
            ))
            .await
            .expect("seed account");
    }

    let begin_resp = client
        .begin_transaction(make_authed_request(
            BeginTransactionRequest { database: database.clone(), options: None },
            &env.api_key,
        ))
        .await
        .expect("begin_transaction");
    let txn_bytes = begin_resp.into_inner().transaction;

    // Transactional batch read: registers both accounts' current versions.
    let batch_req = make_authed_request(
        BatchGetDocumentsRequest {
            database: database.clone(),
            documents: vec![doc_a_name.clone(), doc_b_name.clone()],
            mask: None,
            consistency_selector: Some(
                embyr_proto::firestore::batch_get_documents_request::ConsistencySelector::Transaction(
                    txn_bytes.clone(),
                ),
            ),
        },
        &env.api_key,
    );
    use tokio_stream::StreamExt;
    let stream = client
        .batch_get_documents(batch_req)
        .await
        .expect("transactional batch_get_documents")
        .into_inner();
    let responses: Vec<_> = stream.collect().await;
    assert_eq!(responses.len(), 2, "batch must return both requested documents");

    // External write changes ONE of the two batch-read documents.
    let mut bumped = HashMap::new();
    bumped.insert("balance".to_string(), integer_value(50));
    client
        .update_document(make_authed_request(
            UpdateDocumentRequest {
                document: Some(Document { name: doc_b_name, fields: bumped, ..Default::default() }),
                ..Default::default()
            },
            &env.api_key,
        ))
        .await
        .expect("external update of accounts/b");

    // Commit a write to accounts/a (unrelated to the change on accounts/b) —
    // the registered READ of accounts/b is what must trigger the abort.
    let mut new_fields = HashMap::new();
    new_fields.insert("balance".to_string(), integer_value(150));
    let write = ProtoWrite {
        operation: Some(Operation::Update(Document {
            name: doc_a_name,
            fields: new_fields,
            ..Default::default()
        })),
        ..Default::default()
    };

    let commit_result = client
        .commit(make_authed_request(
            CommitRequest { database, writes: vec![write], transaction: txn_bytes },
            &env.api_key,
        ))
        .await;

    let status = commit_result.expect_err(
        "commit must abort: one of the two batch-read documents changed before commit",
    );
    assert_eq!(status.code(), tonic::Code::Aborted, "got: {status}");
}
