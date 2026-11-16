//! Static serving of the built admin SPA (admin-ui-api-wiring, ADR-082).
//!
//! `static_ui_router(dir)` probes a Trunk `dist/` directory (wire -> probe -> use) and returns a
//! router that serves it under `/admin`. The caller merges it onto the admin app; explicit
//! `/admin/v1/*` routes win over the static service, and there is deliberately no `index.html`
//! fallback (an unknown `/admin/v1/x` must be a 404, never `200 text/html`).

use std::path::{Path, PathBuf};

use axum::{http::header, http::HeaderValue, Router};
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use tower_http::{
    services::ServeDir,
    set_header::SetResponseHeaderLayer,
};

/// Why the configured UI directory cannot be served. Each variant names the problem so the
/// operator sees an unmounted volume, an empty dir, or a partial copy at startup.
#[derive(Debug, thiserror::Error)]
pub enum StaticUiError {
    #[error("EMBYR_ADMIN_UI_DIR is not a readable directory: {}", .0.display())]
    DirMissing(PathBuf),
    #[error("EMBYR_ADMIN_UI_DIR has no readable index.html: {}", .0.display())]
    IndexMissing(PathBuf),
    #[error("index.html must contain exactly one inline bootstrap <script>, found {0}")]
    InlineBootstrapCount(usize),
    #[error("index.html references /admin/{0} but that file is missing from EMBYR_ADMIN_UI_DIR")]
    AssetMissing(String),
}

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const CSP_TEMPLATE: &str = "default-src 'self'; script-src 'self' 'sha256-{HASH}' 'wasm-unsafe-eval'; \
style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; \
base-uri 'self'; form-action 'self'";

fn is_html<B>(resp: &axum::http::Response<B>) -> bool {
    resp.headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html"))
}

/// Probe `dir` and build the router serving it under `/admin`.
pub fn static_ui_router(dir: &Path) -> Result<Router, StaticUiError> {
    if !dir.is_dir() {
        return Err(StaticUiError::DirMissing(dir.to_path_buf()));
    }
    let index = std::fs::read_to_string(dir.join("index.html"))
        .map_err(|_| StaticUiError::IndexMissing(dir.to_path_buf()))?;
    let bootstrap = inline_scripts(&index);
    let [script] = bootstrap.as_slice() else {
        return Err(StaticUiError::InlineBootstrapCount(bootstrap.len()));
    };
    for file in referenced_assets(&index) {
        if file.contains("..") || !dir.join(&file).is_file() {
            return Err(StaticUiError::AssetMissing(file));
        }
    }

    let hash = STANDARD.encode(Sha256::digest(script.as_bytes()));
    let csp = HeaderValue::from_str(&CSP_TEMPLATE.replace("{HASH}", &hash))
        .expect("CSP is ASCII built from a base64 hash");

    let service = tower::ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            |resp: &axum::http::Response<_>| {
                if !resp.status().is_success() {
                    return None;
                }
                Some(HeaderValue::from_static(if is_html(resp) { "no-cache" } else { IMMUTABLE }))
            },
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            move |resp: &axum::http::Response<_>| is_html(resp).then(|| csp.clone()),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .service(ServeDir::new(dir));
    Ok(Router::new().nest_service("/admin", service))
}

/// Bodies of `<script>` tags that have no `src` attribute.
fn inline_scripts(html: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = html;
    while let Some(open) = rest.find("<script") {
        let after = &rest[open..];
        let Some(tag_end) = after.find('>') else { break };
        let Some(close) = after.find("</script>") else { break };
        if !after[..tag_end].contains("src=") {
            found.push(&after[tag_end + 1..close]);
        }
        rest = &after[close + "</script>".len()..];
    }
    found
}

/// Every `/admin/<file>` mentioned anywhere in the document (links, scripts, import specifiers).
fn referenced_assets(html: &str) -> Vec<String> {
    html.split("/admin/")
        .skip(1)
        .map(|tail| {
            tail.chars()
                .take_while(|c| !matches!(c, '"' | '\'' | ' ' | ')' | '<' | '>' | '?' | '#' | '\n'))
                .collect::<String>()
        })
        .filter(|file| !file.is_empty())
        .collect()
}
