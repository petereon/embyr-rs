// @real-io @US-01 @US-02
//! Architecture enforcement (ADR-022): `migrations/customer/` has exactly
//! one embed point workspace-wide.
//!
//! Journey (structural, not a user journey — an architecture invariant per
//! ADR-022 § Enforcement: "a DISTILL-wave regression test asserting the
//! literal string `sqlx::migrate!("../../migrations/customer")` (or the
//! macro's resolved path) appears exactly once in the workspace source
//! tree"):
//!   Given: the workspace source tree.
//!   When:  every `.rs` file under `crates/` is scanned for the literal
//!          `migrations/customer` migration-embed macro invocation.
//!   Then:  exactly one occurrence exists (inside
//!          `PostgresBackendAdapter::migrate()`,
//!          `crates/embyr-pg-storage/src/backend_adapter.rs`) — not the 5
//!          occurrences present before this feature (3 in `provision.rs`'s
//!          branches + 2 in `backend_adapter.rs`'s `migrate()`/
//!          `run_migrations()`), and not a 6th independent embed in the new
//!          `embyr-db-prep` binary.
//!
//! This test is RED-by-design until DELIVER performs the consolidation
//! refactor ADR-022 requires (`provision.rs`'s 3 branches switch from
//! inline `sqlx::migrate!` to calling `PostgresBackendAdapter::migrate()`;
//! `run_migrations()`'s convenience wrapper is refactored to delegate to
//! `migrate()` rather than independently invoking the macro) — the
//! refactor itself, not new scaffolding, per feature-delta.md's own framing.
//!
//! Real I/O: reads real files from the real workspace source tree (no
//! synthetic/mocked file list).

use std::fs;
use std::path::Path;

/// The exact literal every migration-embed macro invocation for the
/// customer schema set must contain, resolved relative to the crate that
/// embeds it (sqlx's `migrate!` macro path argument is relative to the
/// invoking file's `CARGO_MANIFEST_DIR`).
const EMBED_NEEDLE: &str = r#"sqlx::migrate!("../../migrations/customer")"#;

#[test]
fn migrations_customer_has_exactly_one_embed_point_workspace_wide() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve workspace root from CARGO_MANIFEST_DIR");
    let crates_dir = workspace_root.join("crates");

    let mut occurrences: Vec<String> = Vec::new();
    visit_rs_files(&crates_dir, &mut |path, contents| {
        if contents.contains(EMBED_NEEDLE) {
            let count = contents.matches(EMBED_NEEDLE).count();
            for _ in 0..count {
                occurrences.push(path.display().to_string());
            }
        }
    });

    assert_eq!(
        occurrences.len(),
        1,
        "ADR-022: expected exactly 1 embed point for migrations/customer/ \
         workspace-wide, found {}: {occurrences:#?}",
        occurrences.len()
    );
    assert!(
        occurrences[0].ends_with("embyr-pg-storage/src/backend_adapter.rs"),
        "ADR-022: the sole embed point must be \
         crates/embyr-pg-storage/src/backend_adapter.rs (inside migrate()); \
         found it in: {}",
        occurrences[0]
    );
}

fn visit_rs_files(dir: &Path, visitor: &mut impl FnMut(&Path, &str)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip build artifacts.
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            visit_rs_files(&path, visitor);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            if let Ok(contents) = fs::read_to_string(&path) {
                visitor(&path, &contents);
            }
        }
    }
}
