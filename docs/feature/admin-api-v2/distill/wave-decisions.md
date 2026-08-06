# DISTILL Decisions — admin-api-v2

**Author:** Quinn (nw-acceptance-designer)
**Date:** 2026-07-29
**Feature:** admin-api-v2

---

## Reconciliation Gate

Wave-Decision Reconciliation HARD GATE: **PASSED — 0 contradictions**.

DISCUSS decisions D1–D7 and DESIGN decisions AA-01–AA-10 were reviewed.
No contradiction found. Scenarios proceed from this stable foundation.

---

## Walking Skeleton Strategy

**DT-01 — Walking Skeleton scope**

The walking skeleton tests the central invariant of the entire feature:
**two auth stacks coexist on :9090 without collision**.

Journey: POST /auth/signin (session auth) → GET /admin/v1/projects (session-scoped) → POST /auth/signout.

Rationale: This answers "can a user admin sign in and see their databases?" — the most direct stakeholder-demo question. If the two middleware stacks interfere (D2/AA-01), the walking skeleton fails before any other test runs. Proves the sub-router merge (AA-01) at the integration boundary.

**DT-02 — Two walking skeletons considered, one chosen**

A second skeleton (operator Bearer → GET /admin/v1/projects/:id dual-auth route) was considered. Rejected: the dual-auth route is tested in B-02 as a focused scenario. Adding it to the skeleton would exceed "simplest user journey" and obscure the B-01 → B-02 chain.

---

## Test Tier Decisions

**DT-03 — Tier A (production composition root, example-only)**

All B-01 through B-06 scenario tests are Tier A. They use `reqwest::Client` against a real Axum admin server on an ephemeral port, backed by a real Postgres container (testcontainers-rs), with `sqlx::migrate!` applied.

Rationale: Per Project Infrastructure Policy (atdd-infrastructure-policy.md): "Admin port (:9090) | reqwest::Client against Axum admin server on ephemeral port."

**DT-04 — Tier B (state-machine PBT, in-memory) for sole-owner invariant only**

The sole-owner invariant (AC-B05-04) meets the Tier B threshold:
- Journey has 3+ chained scenarios (invite → role-change → remove).
- Input space is domain-rich (N members with varying roles, concurrent operations).

A single `proptest!` scenario in `b05_members.rs` validates the pure domain function `check_rbac` against an N-member state space. This is `@layer-2` (calls pure domain function, no HTTP).

All other properties that involve HTTP (layer 3) use named example-based "Sad_" scenarios per Mandate 11 — no PBT machinery at layer 3+.

---

## OQ Resolutions Recorded

**OQ-B01 — Pre-existing projects skip DSN re-encryption**

Resolution: When `backend_pg_dsn_enc IS NULL`, PATCH skips DSN re-encryption silently. Known limitation documented in B-04 scenario `pre_existing_project_with_null_dsn_enc_skips_re_encryption_silently`.

**OQ-B03 — TOTP library**

Resolution: `totp-rs 5.x` confirmed in DESIGN technology choices. Test uses `ctx.totp_code_now()` (scaffold panics; DELIVER wires real `totp-rs` call from seed secret).

**OQ-B04 — Admin key idle expiry**

Resolution: Admin keys are long-lived. No idle expiry. Immediate revocation only (`revoked_at IS NOT NULL`). Documented in `admin_key_updates_last_used_at_on_successful_auth` scenario with `@OQ-B04` tag.

---

## Infrastructure Decisions

**DT-05 — reqwest cookie jar enabled**

Added `cookies` feature to `reqwest` in workspace `Cargo.toml` so `reqwest::Client` can automatically handle `Set-Cookie` responses in session-auth scenarios. Tests that need raw header inspection use `.headers().get("set-cookie")` directly.

**DT-06 — proptest added to workspace**

`proptest = "1"` added to workspace `[workspace.dependencies]` and to `embyr-server` `[dev-dependencies]`. Used only for the sole-owner invariant PBT in `b05_members.rs`.

---

## Adapter Coverage

| Driven Adapter | Test File | Scenario Tag |
|----------------|-----------|--------------|
| Postgres (testcontainers-rs, sqlx migrations) | b01 | `@real-io @adapter-integration` — `db_migrations_run_cleanly_on_fresh_postgres` |
| BLAKE3 session token storage | b01 | `session_token_stored_as_blake3_hash_not_plaintext` |
| BLAKE3 SDK key storage | b03 | `sdk_key_stored_as_blake3_hash_not_plaintext` |
| ECIES + Argon2id SDK key (RotateAuthKey) | b03 | `sdk_key_authenticates_to_firestore_grpc_rpc` |
| AES-256-GCM DSN encryption | b04 | `backend_pg_dsn_never_stored_plaintext_after_patch` |
| BLAKE3 admin key storage | b05 | `admin_key_stored_as_blake3_not_plaintext` |
| AES-256-GCM OIDC secret encryption | b06 | `owner_creates_oidc_provider_and_secret_encrypted` |
| OIDC JWKS validation (mock) | b06 | `oidc_callback_with_valid_id_token_grants_session` |
| FakeEmailSender (IEmailSender) | b05 | `owner_invites_member_and_invitation_email_is_sent` |
| tokio paused clock | b01 | `session_expires_after_24h_of_inactivity` |

Every driven adapter in scope has at least one `@real-io @adapter-integration` scenario.

---

## Post-Review Fixes (applied after final gate)

**DT-07 — Audit logging strategy (Atlas high issue #2)**

Decision: **OPTION A — defer**. Audit logging for admin operations (member invites, key creation/revocation, OIDC changes) is deferred. The existing `tracing` spans on every Axum handler produce structured log entries that satisfy current compliance review needs. No explicit `admin_audit_log` table or per-story acceptance criterion is added in this feature.

Rationale: Compliance requirements are not yet formalized for this product. Adding explicit ACs would require a new table, an adapter, and significant scope increase (~1 day). The `tracing` span approach (request_id, user_id via SessionContext, resource_id, action verb) is sufficient for initial review. OPTION B (explicit audit ACs) is deferred to a future `audit-trail` feature once compliance requirements are locked.

**DT-08 — Session index DDL requirement (Atlas low issue)**

The B-01 migration MUST include: `CREATE UNIQUE INDEX sessions_token_hash_idx ON sessions(token_hash);`. This index is required to meet the p99 ≤ 100ms target for session lookup on every authenticated request (AC-B02-05, ADR-010). The crafter MUST add this DDL to the B-01 migration file. DELIVER will verify via `b01_auth_migrations::session_lookup_uses_token_hash_index` scenario.

**DT-09 — Dual-auth middleware collision tests added (Forge critical finding)**

Two `#[ignore]` tests added to `b02_project_list.rs`:
- `operator_bearer_route_must_reject_session_cookie_auth` — verifies session cookie rejected on operator-only routes
- `session_auth_route_must_reject_bare_bearer_token` — verifies Bearer token alone rejected on session-only routes

These validate the sub-router merge (ADR-009 AA-01) at the HTTP boundary, not just at compile time. DELIVER must unskip both as part of the B-02 step.
