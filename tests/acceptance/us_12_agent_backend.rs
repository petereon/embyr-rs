// SCAFFOLD: true
//! US-12 — Deploy embyr agent for credential isolation
//!
//! As Riley (CISO), I want to deploy the embyr agent binary in my VPC and register
//! it as the project backend, so that my DB credentials never cross the network
//! boundary to embyr's SaaS.
//!
//! Driving port: Admin HTTP port (:9090) — POST /admin/v1/projects (agent registration)
//!               gRPC data port (:8080) — SDK writes forwarded through agent
//!               Agent gRPC (:9191) — mTLS inbound to MockAgentServer
//! Red classification: MISSING_FUNCTIONALITY

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::thread;

use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
use tempfile::TempDir;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::runners::AsyncRunner,
};

/// Install the ring crypto provider for rustls.
/// Must be called once per test process before any TLS operations.
/// Silently ignores the error if already installed (idempotent).
fn install_ring_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Find a free TCP port by binding to port 0 and returning the assigned port.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind to find free port");
    listener.local_addr().unwrap().port()
}

/// Generate a CA + server cert + key, write to tmp dir, return (dir, ca_pem, cert_pem, key_pem).
fn generate_mtls_certs() -> (TempDir, std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().expect("create tempdir for certs");

    // CA cert
    let mut ca_params = CertificateParams::new(vec!["embyr-ca".to_string()]).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();

    // Server cert signed by CA
    let server_params = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    let server_key = KeyPair::generate().unwrap();
    let server_cert = server_params.signed_by(&server_key, &ca_cert, &ca_key).unwrap();

    let ca_path = tmp.path().join("ca.pem");
    let cert_path = tmp.path().join("server.pem");
    let key_path = tmp.path().join("server.key");

    std::fs::write(&ca_path, ca_cert.pem()).unwrap();
    std::fs::write(&cert_path, server_cert.pem()).unwrap();
    std::fs::write(&key_path, server_key.serialize_pem()).unwrap();

    (tmp, ca_path, cert_path, key_path)
}

/// Drain a child process stream into an Arc<Mutex<String>> in a background thread.
fn drain_stream<R: std::io::Read + Send + 'static>(
    stream: R,
    buf: Arc<Mutex<String>>,
) {
    thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            if let Ok(line) = line {
                let mut guard = buf.lock().unwrap();
                guard.push_str(&line);
                guard.push('\n');
            }
        }
    });
}

/// Wait until either buffer contains an expected string, with timeout.
fn wait_for_log_combined(
    stdout: &Arc<Mutex<String>>,
    stderr: &Arc<Mutex<String>>,
    needle: &str,
    timeout: Duration,
) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        let combined = {
            let o = stdout.lock().unwrap().clone();
            let e = stderr.lock().unwrap().clone();
            format!("{}{}", o, e)
        };
        if combined.contains(needle) {
            return true;
        }
        thread::sleep(Duration::from_millis(200));
    }
    false
}

/// Find the embyr-agent binary path relative to the test manifest directory.
/// CARGO_MANIFEST_DIR for embyr-server is crates/embyr-server; walk up to workspace root.
fn agent_binary_path() -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crates/embyr-server -> workspace root
    let workspace_root = manifest_dir
        .parent()
        .expect("crates dir")
        .parent()
        .expect("workspace root");
    workspace_root.join("target").join("debug").join("embyr-agent")
}

// ---------------------------------------------------------------------------
// AC-12a: agent binary starts with required env vars and logs readiness messages
// ---------------------------------------------------------------------------

/// AC-12a: agent binary starts with required env vars and logs readiness messages
///
/// Given:  EMBYR_AGENT_DB_DSN, EMBYR_AGENT_CERT, EMBYR_AGENT_KEY, EMBYR_AGENT_CA are set
/// And:    the DSN points to a real Postgres instance
/// When:   the embyr-agent binary is started
/// Then:   the process logs "listening on :9191"
/// And:    the process logs "connected to Postgres"
/// And:    the process stays alive (does not exit)
///
/// Tags: @real_io @adapter_integration
#[tokio::test]
async fn agent_starts_with_required_env_vars_and_logs_readiness() {
    // Build embyr-agent before running
    let workspace_root = {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .parent()
            .expect("crates dir")
            .parent()
            .expect("workspace root")
            .to_path_buf()
    };
    let build_status = Command::new("cargo")
        .args(["build", "-p", "embyr-agent"])
        .current_dir(&workspace_root)
        .status()
        .expect("cargo build -p embyr-agent");
    assert!(build_status.success(), "cargo build -p embyr-agent failed");

    // Start Postgres via testcontainers
    use testcontainers_modules::testcontainers::ContainerAsync;
    let container: ContainerAsync<Postgres> = Postgres::default().start().await.expect("start postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("get pg port");
    let dsn = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    // Generate mTLS certs
    let (_tmp, ca_path, cert_path, key_path) = generate_mtls_certs();

    // Pick a free port for the agent
    let agent_port = free_port();
    let agent_addr = format!("127.0.0.1:{}", agent_port);

    let binary = agent_binary_path();
    let mut child = Command::new(&binary)
        .env("EMBYR_AGENT_DB_DSN", &dsn)
        .env("EMBYR_AGENT_CERT", cert_path.to_str().unwrap())
        .env("EMBYR_AGENT_KEY", key_path.to_str().unwrap())
        .env("EMBYR_AGENT_CA", ca_path.to_str().unwrap())
        .env("EMBYR_AGENT_LISTEN_ADDR", &agent_addr)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn embyr-agent");

    let stdout_buf: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let stderr_buf: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    if let Some(stdout) = child.stdout.take() {
        drain_stream(stdout, Arc::clone(&stdout_buf));
    }
    if let Some(stderr) = child.stderr.take() {
        drain_stream(stderr, Arc::clone(&stderr_buf));
    }

    let timeout = Duration::from_secs(30);

    // Wait for "connected to Postgres"
    let postgres_ok = wait_for_log_combined(
        &stdout_buf,
        &stderr_buf,
        "connected to Postgres",
        timeout,
    );

    // Wait for "listening on" (agent stays alive after connecting to PG)
    let listen_ok = wait_for_log_combined(
        &stdout_buf,
        &stderr_buf,
        "listening on",
        Duration::from_secs(10),
    );

    // Verify process is still alive
    let still_alive = child.try_wait().expect("try_wait").is_none();

    // Kill the agent process before assertions to avoid leaving it running
    let _ = child.kill();
    let _ = child.wait();

    let logs = {
        let o = stdout_buf.lock().unwrap().clone();
        let e = stderr_buf.lock().unwrap().clone();
        format!("{}{}", o, e)
    };
    assert!(
        postgres_ok,
        "Expected 'connected to Postgres' in logs within 30s. Logs:\n{}",
        logs
    );
    assert!(
        listen_ok,
        "Expected 'listening on' in logs within 30s. Logs:\n{}",
        logs
    );
    assert!(
        still_alive,
        "Agent process should stay alive after startup. Logs:\n{}",
        logs
    );
}

// ---------------------------------------------------------------------------
// AC-12d: connection without valid client cert causes TLS handshake failure
// ---------------------------------------------------------------------------

/// AC-12d (error path): connection without valid client cert causes TLS handshake failure
///
/// Given:  an embyr agent is running with mTLS required
/// When:   a gRPC connection attempt is made without a client certificate
/// Then:   the TLS handshake fails
/// And:    no data is transmitted over the connection
#[tokio::test]
async fn connection_without_client_cert_fails_tls_handshake() {
    install_ring_provider();

    // Build embyr-agent before running
    let workspace_root = {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .parent()
            .expect("crates dir")
            .parent()
            .expect("workspace root")
            .to_path_buf()
    };
    let build_status = Command::new("cargo")
        .args(["build", "-p", "embyr-agent"])
        .current_dir(&workspace_root)
        .status()
        .expect("cargo build -p embyr-agent");
    assert!(build_status.success(), "cargo build -p embyr-agent failed");

    // Start Postgres
    use testcontainers_modules::testcontainers::ContainerAsync;
    let container: ContainerAsync<Postgres> = Postgres::default().start().await.expect("start postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("get pg port");
    let dsn = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    // Generate mTLS certs
    let (_tmp, ca_path, cert_path, key_path) = generate_mtls_certs();

    // Pick a free port for the agent
    let agent_port = free_port();
    let agent_addr = format!("127.0.0.1:{}", agent_port);

    let binary = agent_binary_path();
    let mut child = Command::new(&binary)
        .env("EMBYR_AGENT_DB_DSN", &dsn)
        .env("EMBYR_AGENT_CERT", cert_path.to_str().unwrap())
        .env("EMBYR_AGENT_KEY", key_path.to_str().unwrap())
        .env("EMBYR_AGENT_CA", ca_path.to_str().unwrap())
        .env("EMBYR_AGENT_LISTEN_ADDR", &agent_addr)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn embyr-agent");

    let stdout_buf: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let stderr_buf: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    if let Some(stdout) = child.stdout.take() {
        drain_stream(stdout, Arc::clone(&stdout_buf));
    }
    if let Some(stderr) = child.stderr.take() {
        drain_stream(stderr, Arc::clone(&stderr_buf));
    }

    // Wait for agent to start listening — agent logs to stderr via tracing
    let started = wait_for_log_combined(
        &stdout_buf,
        &stderr_buf,
        "listening on",
        Duration::from_secs(30),
    );

    if !started {
        let _ = child.kill();
        let _ = child.wait();
        let o = stdout_buf.lock().unwrap().clone();
        let e = stderr_buf.lock().unwrap().clone();
        panic!("Agent never logged 'listening on' within 30s. stdout:\n{}\nstderr:\n{}", o, e);
    }

    // Attempt a TLS handshake WITHOUT a client certificate using tonic's own Channel.
    // The agent requires mTLS (client_ca_root set), so it must reject a connection
    // that presents no client certificate. We also test with a raw tonic channel
    // and verify the gRPC call fails at the TLS/transport level.
    use rustls::pki_types::{CertificateDer, ServerName};
    use rustls::ClientConfig;
    use tokio::net::TcpStream;
    use tokio_rustls::TlsConnector;
    use std::sync::Arc as StdArc;

    // Build a rustls client config that trusts our CA but presents NO client cert.
    let ca_pem = std::fs::read_to_string(&ca_path).unwrap();
    let ca_cert_der = {
        use rustls_pemfile::certs;
        let mut reader = std::io::BufReader::new(ca_pem.as_bytes());
        certs(&mut reader)
            .collect::<Result<Vec<CertificateDer<'static>>, _>>()
            .expect("parse CA cert")
    };
    let mut root_store = rustls::RootCertStore::empty();
    for cert in ca_cert_der {
        root_store.add(cert).expect("add CA to root store");
    }
    let tls_config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(StdArc::new(tls_config));

    let server_name = ServerName::try_from("localhost").expect("valid server name");
    let addr = format!("127.0.0.1:{}", agent_port);
    let tcp_stream = TcpStream::connect(&addr).await.expect("TCP connect");

    let tls_result = connector.connect(server_name, tcp_stream).await;

    // Kill agent before assertions
    let _ = child.kill();
    let _ = child.wait();

    // The server MUST either:
    // (a) reject the TLS handshake with an error (preferred — mTLS enforced),
    // (b) or close the connection immediately after the handshake (also acceptable).
    // A successful TLS handshake that proceeds to serve gRPC is NOT acceptable.
    //
    // Note: some TLS 1.3 implementations complete the handshake with an empty client
    // cert and then fail at the application layer. We verify both paths.
    match tls_result {
        Err(_) => {
            // Ideal: TLS rejected at handshake level
        }
        Ok(mut tls_stream) => {
            // TLS handshake completed; verify the gRPC call fails immediately
            // (the server should close the connection or return a TLS error
            // after verifying the empty client cert at the application layer).
            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            // Send an HTTP/2 preface
            let _ = tls_stream.write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n").await;
            tokio::time::sleep(Duration::from_millis(500)).await;
            let mut buf = [0u8; 128];
            let read_result = tls_stream.read(&mut buf).await;
            // Accept either EOF (connection closed) or read error as rejection
            let rejected = match read_result {
                Ok(0) => true,
                Err(_) => true,
                Ok(_n) => {
                    // Got some bytes back — this likely means the server ACCEPTED
                    // the connection. Check if it's a TLS alert (fatal alert).
                    // TLS alert record starts with 0x15 (alert type)
                    buf[0] == 0x15
                }
            };
            assert!(
                rejected,
                "Expected server to reject connection without client cert, but it accepted gRPC traffic"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// AC-12e: agent exits non-zero if EMBYR_AGENT_DB_DSN is missing
// ---------------------------------------------------------------------------

/// AC-12e (error path): agent exits non-zero if EMBYR_AGENT_DB_DSN is missing
///
/// Given:  EMBYR_AGENT_CERT, EMBYR_AGENT_KEY, EMBYR_AGENT_CA are set
/// And:    EMBYR_AGENT_DB_DSN is NOT set
/// When:   the embyr-agent binary is started
/// Then:   the process exits with a non-zero exit code
/// And:    the stderr output indicates the missing env var
#[tokio::test]
async fn agent_exits_nonzero_when_db_dsn_missing() {
    // Build embyr-agent before running
    let workspace_root = {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .parent()
            .expect("crates dir")
            .parent()
            .expect("workspace root")
            .to_path_buf()
    };
    let build_status = Command::new("cargo")
        .args(["build", "-p", "embyr-agent"])
        .current_dir(&workspace_root)
        .status()
        .expect("cargo build -p embyr-agent");
    assert!(build_status.success(), "cargo build -p embyr-agent failed");

    // Generate mTLS certs
    let (_tmp, ca_path, cert_path, key_path) = generate_mtls_certs();

    let binary = agent_binary_path();

    // Spawn without EMBYR_AGENT_DB_DSN
    // Use env::vars() to get a clean environment but keep PATH for the binary to execute
    let output = Command::new(&binary)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("EMBYR_AGENT_CERT", cert_path.to_str().unwrap())
        .env("EMBYR_AGENT_KEY", key_path.to_str().unwrap())
        .env("EMBYR_AGENT_CA", ca_path.to_str().unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run embyr-agent without DB_DSN");

    let exit_code = output.status.code().unwrap_or(1);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_ne!(
        exit_code, 0,
        "Agent should exit non-zero when EMBYR_AGENT_DB_DSN is missing. stderr:\n{}",
        stderr
    );
    assert!(
        stderr.contains("EMBYR_AGENT_DB_DSN"),
        "stderr should name the missing env var EMBYR_AGENT_DB_DSN. stderr:\n{}",
        stderr
    );
}

// ---------------------------------------------------------------------------
// Remaining US-12 tests — stay #[ignore] until step 09-02+
// ---------------------------------------------------------------------------

/// AC-12b: POST /admin/v1/projects with backend_mode=agent returns 201; system DB has no DSN
///
/// Given:  a MockAgentServer is running on an ephemeral mTLS port
/// When:   POST /admin/v1/projects with backend_mode=agent and the agent endpoint
/// Then:   the response status is 201 Created
/// And:    the system DB project record has no DSN (backend_pg_creds_enc is NULL)
/// And:    the backend_agent_endpoint column contains the agent address
///
/// Tags: @real_io @kpi
#[tokio::test]
#[ignore = "us-12 AC-12b — RED scaffold, not yet implemented"]
async fn provision_with_agent_stores_endpoint_not_dsn() {
    panic!("RED scaffold — not yet implemented");
}

/// AC-12c: SDK write is forwarded through the agent; data persists in customer DB
///
/// Given:  a project registered with backend_mode=agent pointing to a MockAgentServer
/// And:    the MockAgentServer is backed by a real Testcontainers Postgres
/// When:   a gRPC CreateDocument SDK call is made for the project
/// Then:   the MockAgentServer logs show the gRPC forwarded call
/// And:    the document appears in the customer Postgres accessed by the agent
///
/// Tags: @real_io @adapter_integration
#[tokio::test]
#[ignore = "us-12 AC-12c — RED scaffold, not yet implemented"]
async fn sdk_write_forwarded_through_agent_persists_in_customer_db() {
    panic!("RED scaffold — not yet implemented");
}

/// AC-12f: rolling cert rotation — SDK requests succeed throughout with zero downtime
///
/// Given:  an agent project is receiving SDK calls
/// When:   the agent TLS certificate is rotated (new cert issued, old cert still valid briefly)
/// And:    the embyr SaaS reconnects to the agent with the new CA cert
/// Then:   SDK calls succeed throughout the rotation window without error
///
/// Tags: @real_io
#[tokio::test]
#[ignore = "us-12 AC-12f — RED scaffold, not yet implemented"]
async fn rolling_cert_rotation_has_zero_downtime() {
    panic!("RED scaffold — not yet implemented");
}

/// Error path: agent credential isolation — zero DSN rows in system DB for agent projects
///
/// Given:  multiple projects provisioned with backend_mode=agent
/// When:   the system DB projects table is queried for all agent-mode projects
/// Then:   every row has backend_pg_creds_enc = NULL
/// And:    every row has backend_agent_endpoint set to a non-empty value
///
/// Tags: @kpi
#[tokio::test]
#[ignore = "us-12 kpi-isolation — RED scaffold, not yet implemented"]
async fn agent_projects_have_zero_dsn_rows_in_system_db() {
    panic!("RED scaffold — not yet implemented");
}
