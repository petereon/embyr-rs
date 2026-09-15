// @real-io @finding-22
//! tls-startup-warning (finding #22, Medium/Security-Ops) — a startup log
//! nudge when TLS is off while all 3 listeners bind `0.0.0.0` in plaintext.
//!
//! Full context: docs/feature/tls-startup-warning/feature-delta.md
//!
//! Acceptance criteria verified here:
//!   AC-1: TLS disabled (no `EMBYR_TLS_CERT_PATH`/`EMBYR_TLS_KEY_PATH`) —
//!     startup stderr contains a warning naming both env vars.
//!   AC-2: TLS configured (both vars point at a valid PEM pair) — the
//!     warning does NOT appear.
//!
//! Driving port: `embyr-server` binary subprocess (same mechanism as
//! `tests/deployment_release_process/acceptance/drp01_startup_version_log.rs`).
//! Driven port boundary: captured stdout+stderr. No Postgres testcontainer
//! needed — see `common::ServerProcess` doc comment for why.

use std::io::Write;
use std::time::Duration;

use crate::common::ServerProcess;

const WARNING_VARS: &str = "EMBYR_TLS_CERT_PATH, EMBYR_TLS_KEY_PATH";

/// Self-signed test cert/key PEM pair (server-only, no mTLS) — same
/// generation approach as
/// `tests/production_readiness/acceptance/pr05_tls_support.rs::generate_self_signed_test_cert`.
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

#[tokio::test]
async fn warns_on_startup_when_tls_is_disabled() {
    let mut server = ServerProcess::start(&[]);
    let output = server
        .wait_for_exit_and_drain(Duration::from_secs(15))
        .await;

    assert!(
        output.contains(WARNING_VARS),
        "startup log must warn naming both TLS env vars when TLS is disabled; \
         captured output:\n{output}"
    );
}

#[tokio::test]
async fn does_not_warn_on_startup_when_tls_is_configured() {
    let (cert_pem, key_pem) = generate_self_signed_test_cert();
    let dir = tempfile::tempdir().expect("create tmp dir for test TLS material");
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");
    std::fs::File::create(&cert_path)
        .and_then(|mut f| f.write_all(&cert_pem))
        .expect("write test cert PEM");
    std::fs::File::create(&key_path)
        .and_then(|mut f| f.write_all(&key_pem))
        .expect("write test key PEM");

    let mut server = ServerProcess::start(&[
        ("EMBYR_TLS_CERT_PATH", cert_path.to_str().unwrap()),
        ("EMBYR_TLS_KEY_PATH", key_path.to_str().unwrap()),
    ]);
    let output = server
        .wait_for_exit_and_drain(Duration::from_secs(15))
        .await;

    assert!(
        !output.contains(WARNING_VARS),
        "startup log must NOT warn about disabled TLS when TLS is configured; \
         captured output:\n{output}"
    );
}
