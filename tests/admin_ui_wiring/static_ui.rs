// @US-AUW-01 @driving_port @real-io
//! admin-ui-api-wiring slice 01 — serving the built SPA from the admin listener
//! (AC-AUW-01..06, ADR-082). Plain axum router over a temp `dist/`; no Postgres, no trunk.
//!
//! AC-AUW-05 (variable unset => API-only) has no test here: it is the `None` arm of the
//! `match &cfg.admin_ui_dir` in `main.rs`, which merges nothing.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    routing::get,
    Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use embyr_server::admin::static_ui::{static_ui_router, StaticUiError};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const BOOTSTRAP: &str = "\nimport init from '/admin/app-1a2b.js';\nawait init({ module_or_path: '/admin/app-1a2b_bg.wasm' });\n";

fn index_html(scripts: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><link rel=\"stylesheet\" href=\"/admin/styles-1a2b.css\"/>\
         <link rel=\"preload\" href=\"/admin/app-1a2b_bg.wasm\" as=\"fetch\"/></head><body>{scripts}</body></html>"
    )
}

/// A complete Trunk-shaped `dist/` in a temp dir.
fn dist() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::write(p.join("index.html"), index_html(&format!("<script type=\"module\">{BOOTSTRAP}</script>"))).unwrap();
    std::fs::write(p.join("app-1a2b.js"), "export default function init(){}").unwrap();
    std::fs::write(p.join("app-1a2b_bg.wasm"), b"\0asm\x01\0\0\0").unwrap();
    std::fs::write(p.join("styles-1a2b.css"), "body{}").unwrap();
    dir
}

async fn get_path(app: &Router, path: &str) -> axum::response::Response {
    app.clone()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

fn header_of(resp: &axum::response::Response, name: header::HeaderName) -> String {
    resp.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string()
}

/// AC-AUW-01 + AC-AUW-03: the document is served with the bootstrap, no-cache and the full header set.
#[tokio::test]
async fn index_is_served_uncached_with_csp_and_security_headers() {
    let dir = dist();
    let app = static_ui_router(dir.path()).expect("complete dist must probe clean");
    let resp = get_path(&app, "/admin/").await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert!(header_of(&resp, header::CONTENT_TYPE).starts_with("text/html"));
    assert_eq!(header_of(&resp, header::CACHE_CONTROL), "no-cache");
    assert_eq!(header_of(&resp, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
    assert_eq!(header_of(&resp, header::REFERRER_POLICY), "no-referrer");
    assert_eq!(header_of(&resp, header::X_FRAME_OPTIONS), "DENY");
    let hash = STANDARD.encode(Sha256::digest(BOOTSTRAP.as_bytes()));
    let csp = header_of(&resp, header::CONTENT_SECURITY_POLICY);
    assert!(csp.contains(&format!("script-src 'self' 'sha256-{hash}' 'wasm-unsafe-eval'")), "csp: {csp}");
    assert!(csp.contains("default-src 'self'") && csp.contains("frame-ancestors 'none'") && csp.contains("connect-src 'self'"));
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("await init("), "bootstrap must be in the body");
}

/// AC-AUW-02: hashed assets are immutable-cached with the right type; misses carry no cache header.
#[tokio::test]
async fn hashed_assets_are_immutable_and_misses_are_uncached_404s() {
    let dir = dist();
    let app = static_ui_router(dir.path()).unwrap();
    for (path, content_type) in [
        ("/admin/app-1a2b_bg.wasm", "application/wasm"),
        ("/admin/app-1a2b.js", "javascript"),
        ("/admin/styles-1a2b.css", "text/css"),
    ] {
        let resp = get_path(&app, path).await;
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        assert_eq!(header_of(&resp, header::CACHE_CONTROL), "public, max-age=31536000, immutable", "{path}");
        assert!(header_of(&resp, header::CONTENT_TYPE).contains(content_type), "{path}");
        assert!(header_of(&resp, header::CONTENT_SECURITY_POLICY).is_empty(), "CSP is for documents only: {path}");
    }
    let miss = get_path(&app, "/admin/nope-1a2b.js").await;
    assert_eq!(miss.status(), StatusCode::NOT_FOUND);
    assert!(header_of(&miss, header::CACHE_CONTROL).is_empty());
}

/// AC-AUW-04: explicit API routes win over the static service; an unknown API path is a plain 404, never HTML.
#[tokio::test]
async fn api_routes_win_and_unknown_api_paths_are_not_html() {
    let dir = dist();
    let api = Router::new().route("/admin/v1/ping", get(|| async { "pong" }));
    let app = api.merge(static_ui_router(dir.path()).unwrap());

    let ping = get_path(&app, "/admin/v1/ping").await;
    assert_eq!(ping.status(), StatusCode::OK);
    assert_eq!(axum::body::to_bytes(ping.into_body(), 1024).await.unwrap(), "pong");
    assert!(header_of(&get_path(&app, "/admin/v1/ping").await, header::CACHE_CONTROL).is_empty());

    let unknown = get_path(&app, "/admin/v1/does-not-exist").await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    assert!(!header_of(&unknown, header::CONTENT_TYPE).starts_with("text/html"));
}

/// AC-AUW-06: a missing dir, missing index.html, missing referenced asset, or a bootstrap count other than
/// one each refuse with a named error.
#[test]
fn startup_probe_refuses_broken_ui_dirs_with_named_errors() {
    let missing = std::env::temp_dir().join("embyr-no-such-ui-dir-4f0c");
    assert!(matches!(static_ui_router(&missing), Err(StaticUiError::DirMissing(_))));

    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(static_ui_router(empty.path()), Err(StaticUiError::IndexMissing(_))));

    let partial = dist();
    std::fs::remove_file(partial.path().join("app-1a2b_bg.wasm")).unwrap();
    match static_ui_router(partial.path()) {
        Err(StaticUiError::AssetMissing(file)) => assert_eq!(file, "app-1a2b_bg.wasm"),
        other => panic!("expected AssetMissing, got {other:?}"),
    }

    let two_scripts = dist();
    let doubled = format!("<script>{BOOTSTRAP}</script><script>alert(1)</script>");
    std::fs::write(two_scripts.path().join("index.html"), index_html(&doubled)).unwrap();
    assert!(matches!(static_ui_router(two_scripts.path()), Err(StaticUiError::InlineBootstrapCount(2))));

    let no_script = dist();
    std::fs::write(no_script.path().join("index.html"), index_html("")).unwrap();
    assert!(matches!(static_ui_router(no_script.path()), Err(StaticUiError::InlineBootstrapCount(0))));
}
