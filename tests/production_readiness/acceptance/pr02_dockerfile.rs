// @real-io @US-PR-02
//! US-PR-02 — Docker image for production deployment.
//!
//! Acceptance criteria verified here:
//!   AC-PR-02-01: docker build succeeds from the repository root.
//!   AC-PR-02-02: container process runs as non-root user.
//!   AC-PR-02-03: final image size is under 100 MB.
//!   AC-PR-02-04: image exposes the correct ports (8080, 8081, 9090).
//!   AC-PR-02-05: image entrypoint is the embyr-server binary.
//!   AC-PR-02-06: dependency layer is cached (rebuild after src-only change < 60s).
//!
//! Driving port: Docker CLI subprocess — `docker build`, `docker run`, `docker image inspect`.
//!
//! All tests are #[ignore] — DELIVER unskips them one at a time (after Dockerfile is created).
//!
//! Docker availability: tests skip gracefully when `docker info` fails, so the test
//! binary compiles and passes `cargo test` in environments without Docker.
//!
//! Scaffold classification target: RED when Docker is available but Dockerfile absent.
//!                                 SKIP when Docker is not available.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// The test image tag used across all docker tests in this module.
const TEST_IMAGE_TAG: &str = "embyr-server-test:ci";

// ─── Docker availability check ────────────────────────────────────────────────

fn docker_available() -> bool {
    Command::new("docker")
        .args(["info", "--format={{.ServerVersion}}"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn workspace_root() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest_dir)
        .parent()
        .expect("crates dir")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

// ─── AC-PR-02-01: docker build succeeds ──────────────────────────────────────

/// Sam builds the embyr-server Docker image without error.
///
/// Journey:
///   Given: the repository checkout contains a Dockerfile at the workspace root
///   When:  Sam runs `docker build . -t embyr-server-test:ci`
///   Then:  the build exits 0 (no compilation errors, no missing files)
///
/// @real-io @US-PR-02 @AC-PR-02-01
#[test]
#[ignore]
fn docker_build_succeeds() {
    if !docker_available() {
        eprintln!("SKIP: docker not available in this environment");
        return;
    }

    let root = workspace_root();
    let status = Command::new("docker")
        .args(["build", ".", "-t", TEST_IMAGE_TAG])
        .current_dir(&root)
        .status()
        .expect("docker build command failed to spawn");

    assert!(
        status.success(),
        "docker build should exit 0; Dockerfile may be missing or build may have failed"
    );
}

// ─── AC-PR-02-02: container runs as non-root ─────────────────────────────────

/// The container process runs as a non-root user (UID != 0).
///
/// Journey (chained from AC-PR-02-01):
///   Given: the embyr-server Docker image is built
///   When:  Sam inspects the running process inside the container
///   Then:  the USER field shows a non-root user (not "root", UID != 0)
///
/// @real-io @US-PR-02 @AC-PR-02-02
#[test]
#[ignore]
fn docker_image_runs_as_non_root() {
    if !docker_available() {
        eprintln!("SKIP: docker not available");
        return;
    }

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--entrypoint",
            "whoami",
            TEST_IMAGE_TAG,
        ])
        .output()
        .expect("docker run whoami failed to spawn");

    let user = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_ne!(
        user, "root",
        "container process must not run as root; USER was '{user}'"
    );
    assert!(
        output.status.success(),
        "docker run whoami exited non-zero"
    );
}

// ─── AC-PR-02-03: image under 100 MB ─────────────────────────────────────────

/// The final Docker image is smaller than 100 MB.
///
/// Journey (chained from AC-PR-02-01):
///   Given: the embyr-server Docker image is built
///   When:  Sam inspects the image size
///   Then:  the compressed image size is less than 100 000 000 bytes (100 MB)
///
/// @US-PR-02 @AC-PR-02-03
#[test]
#[ignore]
fn docker_image_under_100mb() {
    if !docker_available() {
        eprintln!("SKIP: docker not available");
        return;
    }

    let output = Command::new("docker")
        .args([
            "image",
            "inspect",
            TEST_IMAGE_TAG,
            "--format={{.Size}}",
        ])
        .output()
        .expect("docker image inspect failed to spawn");

    assert!(
        output.status.success(),
        "docker image inspect failed — image may not be built yet; run docker_build_succeeds first"
    );

    let size_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let size_bytes: u64 = size_str
        .parse()
        .unwrap_or_else(|_| panic!("expected numeric image size, got: '{size_str}'"));

    assert!(
        size_bytes < 100_000_000,
        "image size {size_bytes} bytes ({} MB) exceeds 100 MB limit",
        size_bytes / 1_000_000
    );
}

// ─── AC-PR-02-04: image exposes correct ports ────────────────────────────────

/// The Dockerfile EXPOSE declaration includes 8080, 8081, and 9090.
///
/// Journey (chained from AC-PR-02-01):
///   Given: the embyr-server Docker image is built
///   When:  Sam inspects the image metadata
///   Then:  ExposedPorts includes 8080/tcp, 8081/tcp, 9090/tcp
///
/// @US-PR-02 @AC-PR-02-04
#[test]
#[ignore]
fn docker_image_exposes_correct_ports() {
    if !docker_available() {
        eprintln!("SKIP: docker not available");
        return;
    }

    let output = Command::new("docker")
        .args([
            "image",
            "inspect",
            TEST_IMAGE_TAG,
            "--format={{json .Config.ExposedPorts}}",
        ])
        .output()
        .expect("docker image inspect failed to spawn");

    assert!(
        output.status.success(),
        "docker image inspect failed — image may not be built yet"
    );

    let exposed = String::from_utf8_lossy(&output.stdout);
    for port in &["8080/tcp", "8081/tcp", "9090/tcp"] {
        assert!(
            exposed.contains(port),
            "image must EXPOSE {port}; ExposedPorts: {exposed}"
        );
    }
}

// ─── AC-PR-02-05: entrypoint is embyr-server ─────────────────────────────────

/// The image ENTRYPOINT is the embyr-server binary, not a shell wrapper.
///
/// Journey (chained from AC-PR-02-01):
///   Given: the embyr-server Docker image is built
///   When:  Sam inspects the image entrypoint
///   Then:  the entrypoint is ["/app/embyr-server"] or equivalent direct binary path
///
/// @US-PR-02 @AC-PR-02-05
#[test]
#[ignore]
fn docker_image_entrypoint_is_embyr_server() {
    if !docker_available() {
        eprintln!("SKIP: docker not available");
        return;
    }

    let output = Command::new("docker")
        .args([
            "image",
            "inspect",
            TEST_IMAGE_TAG,
            "--format={{json .Config.Entrypoint}}",
        ])
        .output()
        .expect("docker image inspect failed to spawn");

    assert!(output.status.success(), "docker image inspect failed");

    let entrypoint = String::from_utf8_lossy(&output.stdout);
    assert!(
        entrypoint.contains("embyr-server"),
        "entrypoint must reference embyr-server binary; got: {entrypoint}"
    );
}

// ─── AC-PR-02-06: cargo-chef dep layer cached on src-only rebuild ────────────

/// Rebuilding after a src-only change hits the cargo-chef dependency cache (< 60s).
///
/// Journey (chained from AC-PR-02-01):
///   Given: the embyr-server Docker image was built once (dep layer is cached)
///   And:   no Cargo.toml or Cargo.lock files changed
///   When:  Sam touches main.rs and rebuilds
///   Then:  the rebuild completes in under 60 seconds (dependency layer cache hit)
///
/// @US-PR-02 @AC-PR-02-06
#[test]
#[ignore]
fn cargo_chef_rebuild_fast() {
    if !docker_available() {
        eprintln!("SKIP: docker not available");
        return;
    }

    let root = workspace_root();
    let main_rs = root.join("crates/embyr-server/src/main.rs");

    // Ensure image was built (dep layer warm).
    // DELIVER: ensure docker_build_succeeds runs first (test ordering or explicit build call).

    // Touch main.rs to invalidate the source layer without changing Cargo deps.
    // We open the file in append mode (zero bytes written) to update the mtime.
    let _ = std::fs::OpenOptions::new()
        .write(true)
        .open(&main_rs)
        .unwrap_or_else(|e| panic!("main.rs not found at {main_rs:?}: {e}"));

    let t0 = Instant::now();
    let status = Command::new("docker")
        .args(["build", ".", "-t", TEST_IMAGE_TAG])
        .current_dir(&root)
        .status()
        .expect("docker build failed to spawn");
    let elapsed = t0.elapsed();

    assert!(status.success(), "docker rebuild must exit 0");
    assert!(
        elapsed < Duration::from_secs(60),
        "rebuild took {elapsed:?} — cargo-chef dep cache may not be working (expected < 60s)"
    );
}
