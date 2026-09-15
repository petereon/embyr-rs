// @US-DRP-01
//! US-DRP-01 — CHANGELOG.md exists, is in Keep a Changelog format, and gains
//! a real dated entry from this feature's own merge.
//!
//! UAT scenarios covered (feature-delta.md, US-DRP-01):
//!   "A merged change to server code is captured in the release history"
//!     Then CHANGELOG.md gains a new dated entry under an incremented version
//!   "Sam finds what changed between two deployments"
//!     Then Sam sees one dated entry per intermediate release, each with a
//!     one-line-or-longer description
//!
//! These are structural/shape checks on a Markdown file, not deep behavioral
//! tests — appropriate for a docs artifact (per this feature's own framing).
//! The "between v0.1.3 and v0.1.5" scenario is a future hypothetical this
//! feature's own merge does not populate; it is validated at the SHAPE level
//! here (every entry present, now or later, is well-formed and dated) rather
//! than by fabricating entries that don't exist yet.
//!
//! Driving port: none (no code path) — direct file-shape assertion, same
//! treatment as `pr02_dockerfile.rs`'s file-presence checks.
//!
//! All tests are #[ignore] — DELIVER unskips them one at a time after
//! creating CHANGELOG.md per DESIGN's exact format.
//!
//! Scaffold classification target: RED (file not found) until DELIVER
//! creates CHANGELOG.md.

use crate::common::workspace_root;

fn read_changelog() -> String {
    let path = workspace_root().join("CHANGELOG.md");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("CHANGELOG.md must exist at repository root: {path:?}: {e}"))
}

/// Extract `(version, date)` for every `## [X.Y.Z] - YYYY-MM-DD` heading,
/// in file order (excludes the `## [Unreleased]` heading).
fn dated_entries(changelog: &str) -> Vec<(String, String)> {
    changelog
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("## [")?;
            let (version, rest) = rest.split_once(']')?;
            if version.eq_ignore_ascii_case("unreleased") {
                return None;
            }
            let date = rest.trim().strip_prefix("- ")?.trim();
            Some((version.to_string(), date.to_string()))
        })
        .collect()
}

fn is_iso_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 3
        && parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 2
        && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()))
}

// ─── AC: CHANGELOG.md exists in Keep a Changelog format with [Unreleased] ────

/// CHANGELOG.md exists, follows Keep a Changelog, and has an [Unreleased] section.
///
/// @US-DRP-01
#[test]
fn changelog_exists_in_keep_a_changelog_format() {
    let changelog = read_changelog();

    assert!(
        changelog.contains("# Changelog"),
        "CHANGELOG.md must start with a '# Changelog' heading"
    );
    assert!(
        changelog.to_lowercase().contains("keep a changelog"),
        "CHANGELOG.md must reference the Keep a Changelog convention"
    );
    assert!(
        changelog.contains("## [Unreleased]"),
        "CHANGELOG.md must have an '## [Unreleased]' section"
    );
}

// ─── UAT: "A merged change to server code is captured in the release history" ─

/// This feature's own merge performs the first real version bump + CHANGELOG
/// entry: `## [0.1.1] - YYYY-MM-DD`, proving the mechanism end-to-end (US-DRP-01
/// AC: "This feature's own merge performs the first real version bump").
///
/// Chained from `changelog_exists_in_keep_a_changelog_format`: reuses the same
/// `read_changelog()` fact, adds the specific-entry assertion.
///
/// @US-DRP-01
#[test]
fn changelog_gains_a_dated_entry_for_this_features_own_bump() {
    let changelog = read_changelog();
    let entries = dated_entries(&changelog);

    let entry = entries.iter().find(|(v, _)| v == "0.1.1");
    assert!(
        entry.is_some(),
        "CHANGELOG.md must contain a '## [0.1.1] - YYYY-MM-DD' entry for this \
         feature's own first real version bump; found entries: {entries:?}"
    );

    let cargo_toml = std::fs::read_to_string(workspace_root().join("Cargo.toml"))
        .expect("root Cargo.toml must exist");
    assert!(
        cargo_toml.contains("version = \"0.1.1\""),
        "root Cargo.toml [workspace.package] version must be bumped to 0.1.1 \
         to match the new CHANGELOG entry"
    );
}

// ─── UAT: "Sam finds what changed between two deployments" ──────────────────

/// Every dated entry is well-formed: `YYYY-MM-DD` date, and at least one
/// non-empty description line before the next heading — the shape Sam's
/// future "read the entries between two versions" workflow depends on.
///
/// @US-DRP-01
#[test]
fn every_dated_entry_has_a_valid_date_and_a_description() {
    let changelog = read_changelog();
    let entries = dated_entries(&changelog);

    assert!(
        !entries.is_empty(),
        "CHANGELOG.md must have at least one dated release entry (this \
         feature's own [0.1.1])"
    );

    for (version, date) in &entries {
        assert!(
            is_iso_date(date),
            "entry [{version}] date '{date}' must be YYYY-MM-DD"
        );
    }

    // Each "## [X.Y.Z] - ..." heading must be followed by at least one
    // non-blank, non-heading line before the next "## " heading (a
    // one-line-or-longer description, per US-DRP-01 UAT wording).
    let lines: Vec<&str> = changelog.lines().collect();
    let mut heading_indices: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("## ["))
        .map(|(i, _)| i)
        .collect();
    heading_indices.push(lines.len());

    for window in heading_indices.windows(2) {
        let (start, end) = (window[0], window[1]);
        let heading = lines[start];
        if heading.to_lowercase().contains("unreleased") {
            continue;
        }
        let has_description = lines[start + 1..end]
            .iter()
            .any(|l| !l.trim().is_empty() && !l.starts_with('#'));
        assert!(
            has_description,
            "entry '{heading}' must have at least a one-line description \
             naming the change and its motivating finding/feature"
        );
    }
}
