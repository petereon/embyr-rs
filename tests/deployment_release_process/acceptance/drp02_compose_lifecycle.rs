// @walking_skeleton @driving_port @real-io @adapter-integration @US-DRP-02
//! US-DRP-02 — a real `docker compose up` / restart / `down -v` round trip.
//!
//! Covers 3 chained UAT scenarios (feature-delta.md, US-DRP-02) in ONE
//! lifecycle test to minimize container churn on this machine's constrained
//! resources (one compose up/down/up/down -v/up cycle instead of three
//! separate stacks — Pillar 2, chained narrative: each step's `Given` reuses
//! the previous step's `Given + When`):
//!   1. "Sam starts the full stack with one command"
//!      -> GET :9090/healthz returns 200 within 2 minutes
//!   2. "Data persists across restarts"
//!      -> a provisioned project survives `down` + `up`
//!   3. "Sam resets to a clean environment"
//!      -> `down -v` + `up` starts from an empty, freshly migrated database
//!
//! Walking skeleton for US-DRP-02: proves the full local-evaluation journey
//! end-to-end through the one driving port a new contributor actually uses
//! (`docker compose up`), not through any Rust API directly.
//!
//! RESOURCE CONSTRAINT (8GB-RAM machine, Docker Desktop VM reserves 4GB
//! fixed): #[ignore] — real Docker I/O, 2 containers. Run explicitly, NEVER
//! alongside another active testcontainers-based test run. Always tears
//! down via `ComposeStack`'s `Drop` impl, even on assertion failure/panic.
//!
//! Scaffold classification target: RED ("no configuration file provided")
//! until DELIVER creates docker-compose.yml. Cheap to RED-verify: compose
//! refuses to start with the file absent — no containers are ever spun up
//! for this RED check.

use std::process::Command;
use std::time::Duration;

use crate::common::workspace_root;

const ADMIN_URL: &str = "http://127.0.0.1:9090";
const DEV_ADMIN_KEY: &str = "dev-only-admin-key-do-not-use-in-production";

/// RAII guard: runs `docker compose down -v` on drop, so a mid-test panic
/// still tears down both containers instead of leaking them.
struct ComposeStack;

impl ComposeStack {
    fn up() -> Self {
        let status = Command::new("docker")
            .args(["compose", "-f", "docker-compose.yml", "up", "-d", "--wait"])
            .current_dir(workspace_root())
            .status()
            .expect("docker compose up failed to spawn");
        assert!(status.success(), "docker compose up must exit 0");
        ComposeStack
    }

    fn down(&self) {
        let _ = Command::new("docker")
            .args(["compose", "-f", "docker-compose.yml", "down"])
            .current_dir(workspace_root())
            .status();
    }

    fn down_v(&self) {
        let _ = Command::new("docker")
            .args(["compose", "-f", "docker-compose.yml", "down", "-v"])
            .current_dir(workspace_root())
            .status();
    }

    fn up_again() {
        let status = Command::new("docker")
            .args(["compose", "-f", "docker-compose.yml", "up", "-d", "--wait"])
            .current_dir(workspace_root())
            .status()
            .expect("docker compose up failed to spawn");
        assert!(status.success(), "docker compose up (restart) must exit 0");
    }
}

impl Drop for ComposeStack {
    fn drop(&mut self) {
        self.down_v();
    }
}

async fn wait_for_healthz(timeout: Duration) -> bool {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        match client.get(format!("{ADMIN_URL}/healthz")).send().await {
            Ok(resp) if resp.status().as_u16() == 200 => return true,
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn provision_project(project_id: &str) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{ADMIN_URL}/admin/v1/projects"))
        .header("Authorization", format!("Bearer {DEV_ADMIN_KEY}"))
        .json(&serde_json::json!({
            "project_id": project_id,
            "dsn": "postgres://postgres:postgres@postgres:5432/embyr",
            "backend_mode": "direct_pg",
        }))
        .send()
        .await
        .expect("POST /admin/v1/projects request failed");
    assert!(
        resp.status().is_success(),
        "provisioning project {project_id} must succeed; status={}",
        resp.status()
    );
}

async fn project_exists(project_id: &str) -> bool {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{ADMIN_URL}/admin/v1/projects/{project_id}"))
        .header("Authorization", format!("Bearer {DEV_ADMIN_KEY}"))
        .send()
        .await
        .expect("GET /admin/v1/projects/{project_id} request failed");
    resp.status().is_success()
}

/// @walking_skeleton @driving_port @real-io @US-DRP-02
#[tokio::test]
#[ignore]
async fn compose_up_restart_and_reset_round_trip() {
    let project_id = "compose-lifecycle-smoke";

    // ── Scenario 1: "Sam starts the full stack with one command" ──────────
    let stack = ComposeStack::up();
    let healthy = wait_for_healthz(Duration::from_secs(120)).await;
    assert!(
        healthy,
        "GET :9090/healthz must return 200 within 2 minutes of docker compose up"
    );

    // ── Scenario 2: "Data persists across restarts" ───────────────────────
    provision_project(project_id).await;
    stack.down();
    ComposeStack::up_again();
    assert!(
        wait_for_healthz(Duration::from_secs(120)).await,
        "server must become healthy again after docker compose down + up"
    );
    assert!(
        project_exists(project_id).await,
        "the project provisioned before 'docker compose down' must still \
         exist after 'docker compose up' — data must persist via the named volume"
    );

    // ── Scenario 3: "Sam resets to a clean environment" ───────────────────
    stack.down_v();
    ComposeStack::up_again();
    assert!(
        wait_for_healthz(Duration::from_secs(120)).await,
        "server must become healthy on a freshly migrated, empty database \
         after 'docker compose down -v' + 'up'"
    );
    assert!(
        !project_exists(project_id).await,
        "the previously provisioned project must NOT exist after 'down -v' \
         reset — the database must be empty and freshly migrated"
    );

    // `stack` dropped here — final `down -v` teardown via Drop, leaving no
    // containers running regardless of the assertions above.
}
