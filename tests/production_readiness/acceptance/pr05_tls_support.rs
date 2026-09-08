// @real-io @US-01
//! firestore-tls-support (US-01) — in-process TLS termination on all 3
//! listeners, opt-in via `EMBYR_TLS_CERT_PATH`/`EMBYR_TLS_KEY_PATH`.
//!
//! Acceptance criteria verified here:
//!   AC-TLS-02: a real TLS handshake succeeds against the gRPC listener
//!              (:8080) when both vars are correctly configured, and a
//!              Firestore RPC completes over that connection.
//!   AC-TLS-03: a real TLS handshake succeeds against the Admin listener
//!              (:9090) when both vars are correctly configured, and
//!              `GET /healthz` returns 200 over that connection.
//!   AC-TLS-04: a real TLS handshake succeeds against the REST/gRPC-Web
//!              listener (:8081) when both vars are correctly configured,
//!              and a request completes over that connection.
//!   AC-TLS-05: exactly one of the two vars set causes startup to exit
//!              non-zero before any port binds, naming the specific
//!              missing variable.
//!   AC-TLS-06: both vars set but pointing at a missing file causes
//!              startup to exit non-zero before any port binds, naming
//!              the specific missing file.
//!   AC-TLS-07: both vars set but pointing at unparseable PEM content
//!              causes startup to exit non-zero before any port binds,
//!              naming the parse failure.
//!
//! AC-TLS-01 (regression guard) is deliberately NOT re-tested here: it is
//! already proven by every existing acceptance test that connects via a
//! plain (`http://`) channel/client while `spawn_all_servers`'s 2 new
//! trailing parameters are `None, None` at every existing test-server
//! constructor call site (zero behavior change). The representative
//! regression-guard files QUALITY_GATE/DELIVER must keep green,
//! unmodified, are:
//!   - `tests/acceptance/us_04_query_collection.rs` (plaintext gRPC via
//!     `make_channel`/`http://`)
//!   - `tests/production_readiness/acceptance/pr01_config_from_env.rs`
//!     (plaintext Admin `GET /healthz`)
//!   - `tests/acceptance/us_13_browser_transport.rs` (plaintext
//!     REST/gRPC-Web)
//!
//! Driving ports: gRPC `:8080` (tonic), Admin `:9090` (axum), REST/
//!   gRPC-Web `:8081` (axum + tonic-web) — all 3 already-existing
//!   listeners, TLS-wrapped per DESIGN's `TlsMaterial`/`accept_maybe_tls`.
//!
//! AC-TLS-02/03/04 use `embyr_server::start_test_server_with_tls` (an
//! in-process `TestServer`, real Postgres via testcontainers) — NOT
//! `#[ignore]`, matching `us_04_query_collection.rs`'s own convention for
//! in-process acceptance tests.
//! AC-TLS-05/06/07 spawn the real `embyr-server` binary as a subprocess —
//! `#[ignore]`, matching `pr01_config_from_env.rs`'s own convention for
//! subprocess-based startup-error tests.
//!
//! Scaffold state: NONE created by this DISTILL pass. `embyr_server::
//!   config::TlsMaterial`, `embyr_server::start_test_server_with_tls`, the
//!   2 new `ConfigError` variants, and the `EMBYR_TLS_CERT_PATH`/
//!   `EMBYR_TLS_KEY_PATH` env vars do not exist yet — these tests are
//!   expected to FAIL TO COMPILE until DELIVER implements DESIGN's own
//!   fully-specified Handoff Package. This is deliberate: DESIGN already
//!   pins every signature this file depends on, so an intermediate
//!   RED-scaffold stub would be replaced wholesale by DELIVER's real
//!   implementation rather than incrementally filled in.

use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient, value::ValueType, CreateDocumentRequest, Document, Value,
};
use embyr_server::{adapters::system_db::SystemDb, config::TlsMaterial, start_test_server_with_tls};
use prost::Message;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

use crate::common::{ServerProcess, TEST_ENCRYPTION_KEY};

// ─── Test-only TLS material (self-signed, server-only — no mTLS) ────────────

/// Generate a self-signed test certificate (SANs: `localhost`, `127.0.0.1`)
/// and its private key, PEM-encoded. Mirrors the CA-generation half of
/// `tests/acceptance/embyr_agent/mod.rs::test_tls_config()`'s own
/// `rcgen` pattern, applied directly to a leaf cert since this feature's
/// scope (D4) excludes mTLS — no separate client cert is needed.
fn generate_self_signed_test_cert() -> (Vec<u8>, Vec<u8>) {
    let params =
        rcgen::CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .expect("valid test cert params");
    let key_pair = rcgen::KeyPair::generate().expect("generate test keypair");
    let cert = params.self_signed(&key_pair).expect("self-sign test cert");
    (
        cert.pem().into_bytes(),
        key_pair.serialize_pem().into_bytes(),
    )
}

/// Build a `TlsMaterial` from raw PEM bytes the same way DESIGN's own
/// `load_tls_material` does — test-only duplication of the parse step
/// (acceptable: this is fixture setup, not the code under test).
fn build_test_tls_material(cert_pem: Vec<u8>, key_pem: Vec<u8>) -> TlsMaterial {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls_pemfile::certs(&mut cert_pem.as_slice())
            .collect::<Result<_, _>>()
            .expect("parse test server cert PEM");
    let key = rustls_pemfile::private_key(&mut key_pem.as_slice())
        .expect("parse test server key PEM")
        .expect("test key PEM must contain exactly one private key");

    let rustls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("build rustls::ServerConfig from self-signed test cert/key");

    TlsMaterial {
        cert_pem,
        key_pem,
        rustls_config: Arc::new(rustls_config),
    }
}

/// Encode a protobuf message as a gRPC-Web frame (mirrors
/// `tests/acceptance/us_13_browser_transport.rs::grpc_web_encode`).
fn grpc_web_encode(proto_bytes: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(5 + proto_bytes.len());
    frame.push(0x00u8);
    frame.extend_from_slice(&(proto_bytes.len() as u32).to_be_bytes());
    frame.extend_from_slice(proto_bytes);
    frame
}

fn string_value(s: &str) -> Value {
    Value {
        value_type: Some(ValueType::StringValue(s.to_string())),
    }
}

async fn start_postgres() -> (ContainerAsync<Postgres>, String) {
    let container = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get postgres host port");
    (
        container,
        format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres"),
    )
}

// ─── AC-TLS-02/03/04 shared setup — one TLS-configured TestServer ───────────

struct TlsTestEnv {
    _sys_container: ContainerAsync<Postgres>,
    _cust_container: ContainerAsync<Postgres>,
    project_id: String,
    api_key: String,
    server_cert_pem: Vec<u8>,
    server: embyr_server::TestServer,
}

/// Provision a real project (system + customer Postgres, both via
/// testcontainers) and start a `TestServer` whose 3 listeners are all
/// wrapped in TLS using one freshly-generated self-signed cert/key pair.
async fn setup_tls_server() -> TlsTestEnv {
    let (_sys_container, sys_url) = start_postgres().await;
    let (_cust_container, cust_url) = start_postgres().await;

    let system_db = Arc::new(SystemDb::new(&sys_url).await.expect("SystemDb::new"));
    system_db.migrate().await.expect("migrate system db");

    let cust_pool = sqlx::PgPool::connect(&cust_url)
        .await
        .expect("connect customer db");
    sqlx::migrate!("../../migrations/customer")
        .run(&cust_pool)
        .await
        .expect("run customer migrations");

    let project_id = "tls-support-project";
    let api_key = "test-sk-tls-support-01";
    let api_key_hash = argon2::hash_api_key(api_key.as_bytes()).expect("hash api key");
    let pub_key = ecies::derive_public_key(api_key.as_bytes());
    let encrypted_dsn = ecies::encrypt(&pub_key, cust_url.as_bytes()).expect("encrypt dsn");

    let sys_pool = sqlx::PgPool::connect(&sys_url)
        .await
        .expect("connect system db");
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
    .expect("seed project");

    let (cert_pem, key_pem) = generate_self_signed_test_cert();
    let tls = build_test_tls_material(cert_pem.clone(), key_pem);
    let server = start_test_server_with_tls(system_db, tls).await;

    TlsTestEnv {
        _sys_container,
        _cust_container,
        project_id: project_id.to_string(),
        api_key: api_key.to_string(),
        server_cert_pem: cert_pem,
        server,
    }
}

// ─── AC-TLS-02: gRPC listener terminates TLS ────────────────────────────────

/// A real TLS handshake succeeds against the gRPC listener when configured,
/// and a Firestore RPC completes over the encrypted connection.
///
/// Journey:
///   Given: EMBYR_TLS_CERT_PATH/EMBYR_TLS_KEY_PATH point to a valid PEM pair
///   When:  a gRPC client connects to :8080 using TLS credentials
///   Then:  the TLS handshake succeeds
///   And:   the client's Firestore RPC completes normally over the
///          encrypted connection
///
/// @real-io @US-01 @AC-TLS-02
#[tokio::test]
async fn grpc_listener_terminates_tls_when_configured() {
    let env = setup_tls_server().await;

    let ca_cert = tonic::transport::Certificate::from_pem(&env.server_cert_pem);
    let client_tls = tonic::transport::ClientTlsConfig::new()
        .domain_name("localhost")
        .ca_certificate(ca_cert);
    let channel = tonic::transport::Channel::from_shared(format!(
        "https://{}",
        env.server.grpc_addr
    ))
    .expect("valid gRPC TLS URI")
    .tls_config(client_tls)
    .expect("apply client TLS config")
    .connect()
    .await
    .expect("TLS handshake with the gRPC listener must succeed");

    let mut client = FirestoreClient::new(channel);
    let parent = format!(
        "projects/{}/databases/(default)/documents",
        env.project_id
    );
    let mut fields = HashMap::new();
    fields.insert("greeting".to_string(), string_value("hello-over-tls"));

    let mut req = tonic::Request::new(CreateDocumentRequest {
        parent,
        collection_id: "tls_probe".to_string(),
        document_id: "doc-1".to_string(),
        document: Some(Document {
            name: String::new(),
            fields,
            ..Default::default()
        }),
        ..Default::default()
    });
    req.metadata_mut().insert(
        "authorization",
        format!("bearer {}", env.api_key).parse().unwrap(),
    );

    let doc = client
        .create_document(req)
        .await
        .expect("Firestore RPC must complete over the encrypted TLS connection")
        .into_inner();

    assert!(
        doc.name.ends_with("tls_probe/doc-1"),
        "expected the created document name to end with tls_probe/doc-1, got {}",
        doc.name
    );
}

// ─── AC-TLS-03: Admin listener terminates TLS ───────────────────────────────

/// A real TLS handshake succeeds against the Admin listener when
/// configured, and `GET /healthz` returns 200 over that connection.
///
/// Journey (chained from AC-TLS-02's own TLS-configured server):
///   Given: EMBYR_TLS_CERT_PATH/EMBYR_TLS_KEY_PATH point to a valid PEM pair
///   When:  an HTTPS client connects to :9090 using TLS
///   Then:  the TLS handshake succeeds
///   And:   GET /healthz over that TLS connection returns HTTP 200
///
/// @real-io @US-01 @AC-TLS-03
#[tokio::test]
async fn admin_listener_terminates_tls_when_configured() {
    let env = setup_tls_server().await;

    let root_cert = reqwest::Certificate::from_pem(&env.server_cert_pem)
        .expect("parse self-signed test cert as trusted root");
    let client = reqwest::Client::builder()
        .add_root_certificate(root_cert)
        .build()
        .expect("build TLS-trusting reqwest client");

    let url = format!("https://{}/healthz", env.server.admin_addr);
    let resp = client
        .get(&url)
        .send()
        .await
        .expect("TLS handshake with the Admin listener must succeed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "GET /healthz over TLS must return 200"
    );
}

// ─── AC-TLS-04: REST/gRPC-Web listener terminates TLS ───────────────────────

/// A real TLS handshake succeeds against the REST/gRPC-Web listener when
/// configured, and a gRPC-Web request completes over that connection.
///
/// Journey (chained from AC-TLS-03's own TLS-configured server):
///   Given: EMBYR_TLS_CERT_PATH/EMBYR_TLS_KEY_PATH point to a valid PEM pair
///   When:  a browser-style HTTPS client connects to :8081 using TLS
///   Then:  the TLS handshake succeeds
///   And:   a REST/gRPC-Web request completes normally over the encrypted
///          connection
///
/// @real-io @US-01 @AC-TLS-04
#[tokio::test]
async fn rest_grpc_web_listener_terminates_tls_when_configured() {
    let env = setup_tls_server().await;

    let root_cert = reqwest::Certificate::from_pem(&env.server_cert_pem)
        .expect("parse self-signed test cert as trusted root");
    let client = reqwest::Client::builder()
        .add_root_certificate(root_cert)
        .http1_only()
        .build()
        .expect("build TLS-trusting reqwest client");

    let mut fields = HashMap::new();
    fields.insert("title".to_string(), string_value("grpc-web-over-tls"));
    let parent = format!(
        "projects/{}/databases/(default)/documents",
        env.project_id
    );
    let create_req = CreateDocumentRequest {
        parent,
        collection_id: "tls_probe_web".to_string(),
        document_id: "doc-1".to_string(),
        document: Some(Document {
            name: String::new(),
            fields,
            ..Default::default()
        }),
        ..Default::default()
    };
    let frame = grpc_web_encode(&create_req.encode_to_vec());

    let url = format!(
        "https://{}/google.firestore.v1.Firestore/CreateDocument",
        env.server.rest_addr
    );
    let resp = client
        .post(&url)
        .header("content-type", "application/grpc-web+proto")
        .header("authorization", format!("bearer {}", env.api_key))
        .header("x-grpc-web", "1")
        .body(frame)
        .send()
        .await
        .expect("TLS handshake + gRPC-Web request over the REST listener must succeed");

    assert_eq!(
        resp.status(),
        200,
        "expected HTTP 200 from gRPC-Web CreateDocument over TLS"
    );
}

// ─── AC-TLS-05: partial TLS config → exit non-zero, name missing var ────────

/// Startup fails fast when only `EMBYR_TLS_CERT_PATH` is set (its pair
/// partner `EMBYR_TLS_KEY_PATH` is missing) — a config mistake, never a
/// degraded-but-running state.
///
/// Journey (error path):
///   Given: DATABASE_URL/EMBYR_ADMIN_KEY/EMBYR_ENCRYPTION_KEY are all set
///   And:   EMBYR_TLS_CERT_PATH is set, EMBYR_TLS_KEY_PATH is unset
///   When:  Sam starts embyr-server
///   Then:  the process exits with a non-zero code before any port is bound
///   And:   stderr names EMBYR_TLS_KEY_PATH specifically as the missing
///          required variable
///
/// @error @US-01 @AC-TLS-05
#[tokio::test]
#[ignore]
async fn exits_nonzero_when_only_cert_path_is_set() {
    let mut server = ServerProcess::start_env_only(&[
        (
            "DATABASE_URL",
            "postgres://postgres:postgres@127.0.0.1:65535/embyr",
        ),
        ("EMBYR_ADMIN_KEY", "testkey"),
        ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ("EMBYR_TLS_CERT_PATH", "/tmp/embyr-tls-support-test-cert.pem"),
        // EMBYR_TLS_KEY_PATH intentionally absent
    ]);

    let exit_code = server.wait_for_exit(Duration::from_secs(3)).await;
    let stderr = server.drain_stderr();

    assert_ne!(
        exit_code,
        Some(0),
        "server must not exit 0 with a partial TLS config; got {exit_code:?}"
    );
    assert!(
        stderr.contains("EMBYR_TLS_KEY_PATH"),
        "stderr must name EMBYR_TLS_KEY_PATH as the missing half of the pair; got: {stderr}"
    );
    assert!(
        !ServerProcess::port_is_bound(server.grpc_port),
        "no port may bind when TLS config is invalid"
    );
}

// ─── AC-TLS-06: missing file → exit non-zero, name the path ─────────────────

/// Startup fails fast when the configured key file does not exist on disk —
/// never a panic, never a silent plaintext fallback.
///
/// Journey (error path, chained from AC-TLS-05):
///   Given: EMBYR_TLS_CERT_PATH and EMBYR_TLS_KEY_PATH are both set
///   And:   the file at EMBYR_TLS_KEY_PATH does not exist on disk
///   When:  Sam starts embyr-server
///   Then:  the process exits with a non-zero code before any port is bound
///   And:   stderr names the specific missing file path
///   And:   no listener falls back to plaintext
///
/// @error @US-01 @AC-TLS-06
#[tokio::test]
#[ignore]
async fn exits_nonzero_when_key_file_does_not_exist() {
    let (cert_pem, _key_pem) = generate_self_signed_test_cert();
    let mut cert_file = tempfile::NamedTempFile::new().expect("create temp cert file");
    cert_file
        .write_all(&cert_pem)
        .expect("write temp cert file");

    let missing_key_path = std::env::temp_dir().join("embyr-tls-support-missing-key.pem");
    let _ = std::fs::remove_file(&missing_key_path);

    let mut server = ServerProcess::start_env_only(&[
        (
            "DATABASE_URL",
            "postgres://postgres:postgres@127.0.0.1:65535/embyr",
        ),
        ("EMBYR_ADMIN_KEY", "testkey"),
        ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ("EMBYR_TLS_CERT_PATH", cert_file.path().to_str().unwrap()),
        (
            "EMBYR_TLS_KEY_PATH",
            missing_key_path.to_str().unwrap(),
        ),
    ]);

    let exit_code = server.wait_for_exit(Duration::from_secs(3)).await;
    let stderr = server.drain_stderr();

    assert_ne!(
        exit_code,
        Some(0),
        "server must not exit 0 when the key file is missing; got {exit_code:?}"
    );
    assert!(
        stderr.contains(missing_key_path.to_str().unwrap())
            || stderr.contains("EMBYR_TLS_KEY_PATH"),
        "stderr must name the missing key file path; got: {stderr}"
    );
    assert!(
        !ServerProcess::port_is_bound(server.grpc_port),
        "no listener may fall back to plaintext when TLS config is invalid"
    );
}

// ─── AC-TLS-07: unparseable PEM → exit non-zero, name the parse failure ─────

/// Startup fails fast when the configured cert file's content is not valid
/// PEM — never a panic, never a silent plaintext fallback.
///
/// Journey (error path, chained from AC-TLS-06):
///   Given: EMBYR_TLS_CERT_PATH and EMBYR_TLS_KEY_PATH both point to
///          existing, readable files
///   And:   the file at EMBYR_TLS_CERT_PATH does not contain valid PEM
///          certificate data
///   When:  Sam starts embyr-server
///   Then:  the process exits with a non-zero code before any port is bound
///   And:   stderr names the PEM parse failure
///   And:   no listener falls back to plaintext
///
/// @error @US-01 @AC-TLS-07
#[tokio::test]
#[ignore]
async fn exits_nonzero_when_cert_content_is_not_valid_pem() {
    let (_cert_pem, key_pem) = generate_self_signed_test_cert();

    let mut bad_cert_file = tempfile::NamedTempFile::new().expect("create temp cert file");
    bad_cert_file
        .write_all(b"not a pem file")
        .expect("write invalid cert content");
    let mut key_file = tempfile::NamedTempFile::new().expect("create temp key file");
    key_file.write_all(&key_pem).expect("write temp key file");

    let mut server = ServerProcess::start_env_only(&[
        (
            "DATABASE_URL",
            "postgres://postgres:postgres@127.0.0.1:65535/embyr",
        ),
        ("EMBYR_ADMIN_KEY", "testkey"),
        ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        (
            "EMBYR_TLS_CERT_PATH",
            bad_cert_file.path().to_str().unwrap(),
        ),
        ("EMBYR_TLS_KEY_PATH", key_file.path().to_str().unwrap()),
    ]);

    let exit_code = server.wait_for_exit(Duration::from_secs(3)).await;
    let stderr = server.drain_stderr();

    assert_ne!(
        exit_code,
        Some(0),
        "server must not exit 0 with unparseable cert PEM; got {exit_code:?}"
    );
    assert!(
        stderr.contains("EMBYR_TLS_CERT_PATH") || stderr.to_lowercase().contains("pem"),
        "stderr must indicate a PEM parse failure; got: {stderr}"
    );
    assert!(
        !ServerProcess::port_is_bound(server.grpc_port),
        "no listener may fall back to plaintext when TLS config is invalid"
    );
}
