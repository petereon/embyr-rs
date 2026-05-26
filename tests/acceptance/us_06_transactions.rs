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
    value::ValueType,
    write::Operation,
    BeginTransactionRequest, CommitRequest, CreateDocumentRequest, Document, GetDocumentRequest,
    RollbackRequest, UpdateDocumentRequest, Value, Write as ProtoWrite,
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
