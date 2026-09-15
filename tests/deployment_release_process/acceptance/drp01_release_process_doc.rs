// @US-DRP-01
//! US-DRP-01 — `docs/operations/release-process.md` documents the bump/tag/
//! CHANGELOG/exemption/rollback convention.
//!
//! UAT scenarios covered (feature-delta.md, US-DRP-01):
//!   "A documentation-only change does not require a new release"
//!     Then no version bump, tag, or CHANGELOG entry is required
//!   "Sam rolls back to a known-good release"
//!     When Sam checks out the vX.Y.Z tag and rebuilds the Docker image
//!
//! Structural/prose-presence check only (grep for required keywords/sections)
//! — appropriate for a docs artifact, not a deep content test. Matches the
//! repo's `docs/operations/` convention (sibling: backup-disaster-recovery.md).
//!
//! Driving port: none — direct file-content assertion.
//!
//! #[ignore] — DELIVER unskips after writing the doc per DESIGN's exact
//! procedure (feature-delta.md § US-DRP-01 Design, item 2).
//!
//! Scaffold classification target: RED (file not found) until DELIVER
//! creates docs/operations/release-process.md.

use crate::common::workspace_root;

fn read_release_process_doc() -> String {
    let path = workspace_root().join("docs/operations/release-process.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("docs/operations/release-process.md must exist at repository root: {path:?}: {e}")
    })
}

/// The doc states which changes require a bump (crates/, migrations/,
/// either Dockerfile) and which are exempt (docs/-only) — US-DRP-01 AC item 2,
/// and the "documentation-only change does not require a release" UAT scenario.
///
/// @US-DRP-01
#[test]
fn documents_which_changes_require_a_version_bump() {
    let doc = read_release_process_doc();

    for required_trigger in ["crates/", "migrations", "Dockerfile"] {
        assert!(
            doc.contains(required_trigger),
            "release-process.md must name '{required_trigger}' as a version-bump \
             trigger path"
        );
    }
    assert!(
        doc.to_lowercase().contains("docs/") && doc.to_lowercase().contains("exempt"),
        "release-process.md must explicitly exempt docs/-only changes from the \
         bump/tag/CHANGELOG requirement"
    );
}

/// The doc states the git tag format (`vMAJOR.MINOR.PATCH`) and that it is
/// created on the merge commit that performs the bump — US-DRP-01 AC item 3.
///
/// Chained from `documents_which_changes_require_a_version_bump`: reuses the
/// same `read_release_process_doc()` fact.
///
/// @US-DRP-01
#[test]
fn documents_the_git_tag_convention() {
    let doc = read_release_process_doc();

    assert!(
        doc.contains('v') && doc.to_uppercase().contains("MAJOR") && doc.to_uppercase().contains("MINOR") && doc.to_uppercase().contains("PATCH"),
        "release-process.md must state the vMAJOR.MINOR.PATCH tag format"
    );
    assert!(
        doc.to_lowercase().contains("merge commit"),
        "release-process.md must state the tag is created on the merge commit"
    );
}

/// The doc describes a rollback procedure (checkout the prior tag, rebuild)
/// — the "Sam rolls back to a known-good release" UAT scenario.
///
/// @US-DRP-01
#[test]
fn documents_the_rollback_procedure() {
    let doc = read_release_process_doc().to_lowercase();

    assert!(
        doc.contains("rollback") || doc.contains("roll back"),
        "release-process.md must describe a rollback procedure"
    );
    assert!(
        doc.contains("checkout") || doc.contains("tag"),
        "release-process.md's rollback procedure must reference checking out a \
         prior git tag"
    );
}
