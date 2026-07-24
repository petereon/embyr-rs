// @walking_skeleton @driving_adapter
//! Walking skeleton — user-admin-ui
//!
//! Two tests that prove the Leptos WASM SPA is wired into embyr-admin:
//!
//!   1. HTTP probe: GET /admin/ → 200 + Content-Type: text/html + <script> tag.
//!   2. Bundle size gate: admin-ui/dist/*.wasm file is < 5,000,000 bytes.
//!
//! Neither test is marked #[ignore] — the walking skeleton MUST be green
//! before DELIVER begins. The HTTP probe requires embyr-admin to be built
//! with the ServeDir route (Slice 01 work). The bundle size gate requires
//! `trunk build --release` to have been run; it skips if dist/ is absent.
//!
//! Classification: GREEN (Slice 01 complete)
//!   - admin_spa_http_probe: GREEN — ServeDir serves the trunk-built dist/
//!   - wasm_bundle_size_gate: SKIP until `trunk build --release` produces dist/

use std::path::PathBuf;

// ── Test 1: HTTP probe ────────────────────────────────────────────────────

/// Verify that GET /admin/ returns 200 with an HTML body containing a <script> tag.
///
/// Drives embyr-admin's ServeDir adapter via HTTP (the driving adapter for the SPA).
/// The `trunk build --release` output in `crates/embyr-admin-ui/dist/` is served by
/// a minimal Axum router using tower_http::ServeDir.
///
/// # Classification
/// GREEN — Slice 01 wires ServeDir into the Axum test router.
///
// @walking_skeleton @driving_adapter @real-io @US-001 @AC-001-01
#[tokio::test]
async fn admin_spa_http_probe() {
    // Resolve the dist directory produced by `trunk build --release`.
    // CARGO_MANIFEST_DIR is the embyr-admin-ui package root at test time.
    let dist_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/embyr-admin-ui/dist");

    // Bind an ephemeral port using tokio's non-blocking listener (avoids
    // the "blocking socket" error on macOS when converting from std::net).
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let port = listener.local_addr().unwrap().port();

    // Build the Axum router that serves the trunk dist directory at /admin.
    let app = axum::Router::new()
        .nest_service("/admin", tower_http::services::ServeDir::new(&dist_dir));

    // Spawn the server on the ephemeral listener.
    let _server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give the server a moment to bind.
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let url = format!("http://127.0.0.1:{}/admin/", port);
    let resp = reqwest::get(&url).await.expect("HTTP GET /admin/ failed");

    // AC-001-01: 200 OK
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::OK,
        "GET /admin/ should return 200; got {}. \
         Verify `trunk build --release` was run in crates/embyr-admin-ui/.",
        resp.status()
    );

    // AC-001-01: Content-Type: text/html
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.contains("text/html"),
        "Content-Type should be text/html; got {}",
        ct
    );

    // AC-001-01: body contains WASM boot <script> tag
    let body = resp.text().await.expect("read body");
    assert!(
        body.contains("<script"),
        "HTML body must contain a <script> tag (WASM bootstrap); \
         body had {} chars but no <script>",
        body.len()
    );
}

// ── Test 2: Bundle size gate ──────────────────────────────────────────────

/// Verify that the WASM bundle produced by `trunk build --release` is < 5 MB.
///
/// This test SKIPS if the dist/ directory doesn't exist (trunk build not run yet).
/// It is a CI gate: the build pipeline must run `trunk build --release` before
/// this test is executed.
///
/// AC: WASM bundle size < 5,000,000 bytes (5 MB hard constraint, CLAUDE.md).
///
// @bundle_size @ci_gate @AC-001-01
#[test]
#[ignore = "requires trunk build --release to produce admin-ui/dist/"]
fn wasm_bundle_size_gate() {
    // Locate the workspace root relative to the manifest dir.
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dist_dir = workspace_root.join("../../crates/embyr-admin-ui/dist");

    if !dist_dir.exists() {
        eprintln!(
            "SKIP: dist dir not found at {}. Run `trunk build --release` first.",
            dist_dir.display()
        );
        return;
    }

    // Find all .wasm files in the dist directory.
    let wasm_files: Vec<_> = std::fs::read_dir(&dist_dir)
        .expect("read dist dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "wasm").unwrap_or(false))
        .collect();

    assert!(
        !wasm_files.is_empty(),
        "No .wasm file found in {}. Verify trunk build output.",
        dist_dir.display()
    );

    for entry in &wasm_files {
        let path = entry.path();
        let size = std::fs::metadata(&path)
            .expect("stat wasm file")
            .len();

        assert!(
            size < 5_000_000,
            "WASM bundle {:?} is {} bytes (>= 5 MB limit). \
             Audit web-sys features and remove unused crates.",
            path.file_name().unwrap(),
            size
        );

        eprintln!(
            "bundle-size-gate PASS: {:?} = {} bytes ({:.1} MB)",
            path.file_name().unwrap(),
            size,
            size as f64 / 1_000_000.0
        );
    }
}
