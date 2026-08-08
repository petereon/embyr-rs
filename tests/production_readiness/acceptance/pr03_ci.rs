// @US-PR-03
//! US-PR-03 — CI pipeline gating every PR.
//!
//! Acceptance criteria verified here:
//!   AC-PR-03-01: CI YAML triggers on push to master.
//!   AC-PR-03-02: CI YAML triggers on pull_request.
//!   AC-PR-03-03: CI YAML has a test job with a Postgres service container.
//!   AC-PR-03-04: CI YAML has a lint job with cargo clippy and cargo deny.
//!   AC-PR-03-05: CI YAML has a docker job that depends on the test job.
//!   AC-PR-03-06: cargo clippy uses -D warnings (deny all warnings).
//!
//! Driving port: filesystem read of `.github/workflows/ci.yml`.
//!               These are YAML-content acceptance tests — no subprocess needed.
//!
//! All tests are #[ignore] — DELIVER unskips them one at a time.
//!
//! RED behaviour: `.github/workflows/ci.yml` does not exist yet. Tests that try
//! to read it will panic with a "file not found" error, which is RED — the CI
//! pipeline file is missing, i.e., the implementation is absent.

use std::path::PathBuf;

// ─── CI YAML helper ───────────────────────────────────────────────────────────

/// Resolve path to `.github/workflows/ci.yml` from the workspace root.
fn ci_yaml_path() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest_dir)
        .parent()
        .expect("crates dir")
        .parent()
        .expect("workspace root")
        .join(".github/workflows/ci.yml")
}

/// Read the CI YAML file. Panics if the file does not exist — RED until DELIVER creates it.
fn read_ci_yaml() -> String {
    let path = ci_yaml_path();
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "ci.yml not found at {path:?}: {e}.\n\
             Create `.github/workflows/ci.yml` in the repository root (US-PR-03 DELIVER)."
        )
    })
}

// ─── AC-PR-03-01: triggers on push to master ─────────────────────────────────

/// The CI workflow triggers on every push to the master branch.
///
/// Journey:
///   Given: .github/workflows/ci.yml exists
///   When:  Sam pushes directly to master
///   Then:  the workflow has a trigger for push events on the master branch
///
/// @US-PR-03 @AC-PR-03-01
#[test]
#[ignore]
fn ci_yaml_triggers_on_push_to_master() {
    let yaml = read_ci_yaml();

    assert!(
        yaml.contains("push:") || yaml.contains("push "),
        "ci.yml must include a 'push' trigger; content:\n{yaml}"
    );
    assert!(
        yaml.contains("master"),
        "ci.yml push trigger must reference 'master' branch; content:\n{yaml}"
    );
}

// ─── AC-PR-03-02: triggers on pull_request ───────────────────────────────────

/// The CI workflow triggers on every pull_request targeting master.
///
/// Journey (chained from AC-PR-03-01):
///   Given: ci.yml exists and triggers on push
///   When:  Sam opens a pull request targeting master
///   Then:  the workflow also has a pull_request trigger
///
/// @US-PR-03 @AC-PR-03-02
#[test]
#[ignore]
fn ci_yaml_triggers_on_pull_request() {
    let yaml = read_ci_yaml();

    assert!(
        yaml.contains("pull_request"),
        "ci.yml must include a 'pull_request' trigger; content:\n{yaml}"
    );
}

// ─── AC-PR-03-03: test job has Postgres service container ────────────────────

/// The `test` job declares a Postgres service container for integration tests.
///
/// Journey (chained from AC-PR-03-02):
///   Given: ci.yml exists with push and pull_request triggers
///   When:  Sam inspects the 'test' job
///   Then:  the job defines a 'postgres' service with a Docker image
///   And:   DATABASE_URL or equivalent env var is set for cargo test
///
/// @US-PR-03 @AC-PR-03-03
#[test]
#[ignore]
fn ci_yaml_has_test_job_with_postgres_service() {
    let yaml = read_ci_yaml();

    assert!(
        yaml.contains("cargo test"),
        "ci.yml 'test' job must run 'cargo test'; content:\n{yaml}"
    );
    // Services block must reference postgres image.
    assert!(
        yaml.contains("postgres:"),
        "ci.yml 'test' job must declare a postgres service container; content:\n{yaml}"
    );
    assert!(
        yaml.contains("DATABASE_URL"),
        "ci.yml 'test' job must set DATABASE_URL for the Postgres service; content:\n{yaml}"
    );
}

// ─── AC-PR-03-04: lint job has clippy and deny ───────────────────────────────

/// The `lint` job runs both cargo clippy and cargo deny check.
///
/// Journey (chained from AC-PR-03-03):
///   Given: ci.yml has a test job with Postgres
///   When:  Sam inspects the 'lint' job
///   Then:  the job runs 'cargo clippy'
///   And:   the job runs 'cargo deny check'
///
/// @US-PR-03 @AC-PR-03-04
#[test]
#[ignore]
fn ci_yaml_has_lint_job_with_clippy_and_deny() {
    let yaml = read_ci_yaml();

    assert!(
        yaml.contains("cargo clippy"),
        "ci.yml 'lint' job must run 'cargo clippy'; content:\n{yaml}"
    );
    assert!(
        yaml.contains("cargo deny check"),
        "ci.yml 'lint' job must run 'cargo deny check'; content:\n{yaml}"
    );
}

// ─── AC-PR-03-05: docker job depends on test job ─────────────────────────────

/// The `docker` job depends on the `test` job passing first.
///
/// Journey (chained from AC-PR-03-04):
///   Given: ci.yml has test and lint jobs
///   When:  Sam inspects the 'docker' job
///   Then:  the job declares a dependency on the test job (needs: [test] or needs: test)
///   And:   the job runs `docker build`
///
/// @US-PR-03 @AC-PR-03-05
#[test]
#[ignore]
fn ci_yaml_has_docker_job_depending_on_test() {
    let yaml = read_ci_yaml();

    assert!(
        yaml.contains("docker build"),
        "ci.yml 'docker' job must run 'docker build'; content:\n{yaml}"
    );
    assert!(
        yaml.contains("needs:") && yaml.contains("test"),
        "ci.yml 'docker' job must declare 'needs: [test]' or 'needs: test'; content:\n{yaml}"
    );
}

// ─── AC-PR-03-06: clippy uses -D warnings ────────────────────────────────────

/// The clippy invocation uses `-D warnings` to treat all warnings as errors.
///
/// Journey (chained from AC-PR-03-05):
///   Given: ci.yml has a lint job with cargo clippy
///   When:  Sam reads the clippy invocation in the lint job
///   Then:  the command includes `-- -D warnings` or `--deny warnings`
///
/// @US-PR-03 @AC-PR-03-06
#[test]
#[ignore]
fn ci_yaml_clippy_uses_deny_warnings() {
    let yaml = read_ci_yaml();

    assert!(
        yaml.contains("-D warnings") || yaml.contains("--deny warnings"),
        "ci.yml clippy must use '-D warnings' to deny all warnings; content:\n{yaml}"
    );
}
