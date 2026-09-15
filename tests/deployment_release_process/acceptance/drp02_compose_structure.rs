// @US-DRP-02
//! US-DRP-02 — `docker-compose.yml` exists, is syntactically valid, reuses
//! the production Dockerfile (no divergent build definition), and defines
//! both services with the right ports/env/volume.
//!
//! UAT scenarios covered (feature-delta.md, US-DRP-02):
//!   "Compose reuses the production Dockerfile"
//!   "Default dev credentials are clearly non-production"
//!
//! These are static/structural checks — `docker compose config` validates
//! syntax WITHOUT starting containers (cheap, safe on this machine's
//! resource constraints); the rest are plain text/substring checks against
//! the committed YAML (no new YAML-parsing dependency — ponytail: stdlib
//! string ops are enough for a structural presence check).
//!
//! Driving port: Docker CLI subprocess (`docker compose config`) for syntax
//! validation; direct file read for the rest.
//!
//! Docker availability: `docker_available()` checks; tests skip gracefully
//! (not fail) when Docker is not installed, matching
//! `pr02_dockerfile.rs`'s own precedent.
//!
//! All tests are #[ignore] — DELIVER unskips them one at a time after
//! creating docker-compose.yml per DESIGN's exact content.
//!
//! Scaffold classification target: RED (file not found / `docker compose
//! config` fails with "no configuration file provided") until DELIVER
//! creates docker-compose.yml.

use std::process::Command;

use crate::common::workspace_root;

fn docker_available() -> bool {
    Command::new("docker")
        .args(["info", "--format={{.ServerVersion}}"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn read_compose_file() -> String {
    let path = workspace_root().join("docker-compose.yml");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("docker-compose.yml must exist at repository root: {path:?}: {e}"))
}

// ─── AC: docker-compose.yml is syntactically valid ───────────────────────────

/// `docker compose config` validates the file without starting any container.
///
/// @US-DRP-02
#[test]
fn compose_config_validates_without_starting_containers() {
    if !docker_available() {
        eprintln!("SKIP: docker not available in this environment");
        return;
    }

    let root = workspace_root();
    let output = Command::new("docker")
        .args(["compose", "-f", "docker-compose.yml", "config", "--quiet"])
        .current_dir(&root)
        .output()
        .expect("docker compose config failed to spawn");

    assert!(
        output.status.success(),
        "docker compose config must validate cleanly; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ─── UAT: "Compose reuses the production Dockerfile" ─────────────────────────

/// The `embyr-server` service builds via `dockerfile: Dockerfile` /
/// `context: .` — the repository's own root Dockerfile, not a copy or a
/// compose-specific variant. And no second Dockerfile-like file exists
/// anywhere else in the repository (the one explicitly-allowed exception is
/// `crates/embyr-agent/Dockerfile`, a different binary's own build, out of
/// scope for this story per DESIGN).
///
/// @US-DRP-02
#[test]
fn compose_builds_embyr_server_from_the_one_root_dockerfile() {
    let compose = read_compose_file();

    assert!(
        compose.contains("dockerfile: Dockerfile"),
        "docker-compose.yml's embyr-server service must build with \
         'dockerfile: Dockerfile' (the repository's own root Dockerfile)"
    );
    assert!(
        compose.contains("context: ."),
        "docker-compose.yml's embyr-server service must build with 'context: .'"
    );

    let root = workspace_root();
    let find_output = Command::new("find")
        .args([".", "-name", "Dockerfile*", "-not", "-path", "./target/*"])
        .current_dir(&root)
        .output()
        .expect("find failed to spawn");
    let found = String::from_utf8_lossy(&find_output.stdout);
    let dockerfiles: Vec<&str> = found
        .lines()
        .filter(|l| *l != "./Dockerfile" && *l != "./crates/embyr-agent/Dockerfile")
        .collect();

    assert!(
        dockerfiles.is_empty(),
        "no second Dockerfile or compose-specific variant may exist besides \
         ./Dockerfile and ./crates/embyr-agent/Dockerfile; found extra: {dockerfiles:?}"
    );
}

// ─── AC: both services with right ports/env/volume ───────────────────────────

/// Compose defines exactly the two services (`embyr-server`, `postgres`)
/// with the correct ports, required env vars, and a named volume for
/// Postgres data persistence.
///
/// @US-DRP-02
#[test]
fn compose_defines_required_services_ports_and_volume() {
    let compose = read_compose_file();

    for service in ["embyr-server:", "postgres:"] {
        assert!(
            compose.contains(service),
            "docker-compose.yml must define a '{service}' service"
        );
    }
    for port in ["\"8080:8080\"", "\"8081:8081\"", "\"9090:9090\""] {
        assert!(
            compose.contains(port),
            "docker-compose.yml's embyr-server service must map port {port}"
        );
    }
    assert!(
        compose.contains("5433:5432"),
        "docker-compose.yml's postgres service must expose host port 5433 \
         (avoids clashing with a locally-installed Postgres on 5432)"
    );
    for env_var in ["DATABASE_URL", "EMBYR_ADMIN_KEY", "EMBYR_ENCRYPTION_KEY"] {
        assert!(
            compose.contains(env_var),
            "docker-compose.yml's embyr-server service must set {env_var}"
        );
    }
    assert!(
        compose.contains("embyr_pgdata"),
        "docker-compose.yml must declare the named volume 'embyr_pgdata' for \
         Postgres data persistence across restarts"
    );
}

// ─── UAT: "Default dev credentials are clearly non-production" ──────────────

/// `EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY` default values are labeled as
/// dev-only placeholders, with a comment warning against reuse in any
/// shared/production environment.
///
/// @US-DRP-02
#[test]
fn dev_credentials_are_labeled_non_production() {
    let compose = read_compose_file();
    let lower = compose.to_lowercase();

    assert!(
        lower.contains("dev-only"),
        "docker-compose.yml must label its default credentials as dev-only"
    );
    assert!(
        lower.contains("production"),
        "docker-compose.yml must contain a comment warning against reuse in \
         a shared/production environment"
    );
}
