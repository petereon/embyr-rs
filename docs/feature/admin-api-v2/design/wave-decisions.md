# DESIGN Decisions — admin-api-v2

**Wave:** DESIGN
**Date:** 2026-07-29
**Author:** Morgan (nw-solution-architect)
**Mode:** Propose (autonomous analysis)

---

## Interaction Mode

Propose — requirements fully specified in DISCUSS wave (feature-delta.md). No ambiguity gates required before design.

---

## Key Decisions

### Auth Middleware Separation (Problem 1 — contested)

| # | Option | Verdict |
|---|--------|---------|
| A | Sub-router merge with per-router Tower layers | **Selected** (see ADR-009) |
| B | Single router with typed Axum extractors | Rejected — no structural enforcement, compiler does not catch missing auth extractor |
| C | `route_layer` called twice on one router | Rejected — second `route_layer` applies to all routes, causing cross-contamination |

**Decision:** Four sub-routers merged at composition: `operator_router` (Bearer EMBYR_ADMIN_KEY), `session_router` (cookie OR admin_api_key Bearer), `public_router` (no auth), `dual_auth_router` (`GET /admin/v1/projects/:id` only, accepts either).

### Account-Scoping (Problem 2)

**Decision:** `SessionContext` as Axum `FromRequestParts` extractor (see ADR-010). Extractor queries `sessions` or `admin_api_keys` table, returns `SessionContext { user_id, account_id, role }`. If a handler declares `SessionContext` in its parameter list, auth is enforced; if it doesn't, the route is unprotected by design (public or operator routes). Account scoping (`WHERE account_id = $session.account_id`) is explicit in each handler's SQL.

### SDK Key → ECIES Integration (Problem 3)

**Decision:** New pure function `new_sdk_key_material(raw_key: &[u8]) -> SdkKeyMaterial` in `embyr-core::domain::project` (see ADR-014). Returns `{ argon2id_hash, ecies_pubkey, blake3_hash }`. Handler calls it on `spawn_blocking` thread. Single transaction updates `sdk_api_keys` and `projects.api_key_hash_current`. DSN re-encryption uses `backend_pg_dsn_enc` (AES-GCM under `EMBYR_ENCRYPTION_KEY`) as the plaintext source.

### Query Log Write Path (Problem 4)

**Decision:** Fire-and-forget `tokio::spawn` from gRPC handler (see ADR-013). New `IQueryLogWriter` port in `embyr-core::admin::query_log`. `PostgresQueryLogAdapter` in `embyr-server::adapters::query_log`. `logging_enabled` flag read from project record (already loaded in auth middleware); no additional DB query per request. Bounded channel (1000) to prevent unbounded task accumulation.

### RBAC Enforcement (Problem 5)

**Decision:** Pure domain function `check_rbac(actor, action, target_user_id, target_role, is_last_owner) -> Result<(), RbacError>` in `embyr-core::admin::rbac` (see ADR-012). Called explicitly at top of each write handler. `RbacError` variants cover all non-trivial invariants (self-demotion, last-owner, key role cap, Owner-only actions).

### IEmailSender Port Placement (Problem 6)

**Decision:** `IEmailSender` trait in `embyr-core::admin::email`. `NoopEmailSender` (V1) and `SmtpEmailSender` (V2) in `embyr-server::adapters::email` (see ADR-011). Injected into `UserAdminState.email_sender: Arc<dyn IEmailSender>` at composition root.

---

## Requirements Summary

- **Primary job:** JOB-10 — replace all mock data in user-admin-ui Slice 07
- **Secondary jobs:** JOB-04 (Connections patch), JOB-05 (cloud secret fields), JOB-06 (metrics/logging)
- **WS scope (B-01):** 10 DB migrations + session auth endpoints + `GET /admin/v1/projects`
- **Feature type:** Backend — 22 new routes, 10 new DB tables, 2 schema alterations

---

## Architecture Decisions Summary

| ID | Decision | ADR |
|----|----------|-----|
| AA-01 | Sub-router merge with per-router Tower layers for auth separation | ADR-009 |
| AA-02 | `SessionContext` as `FromRequestParts` extractor | ADR-010 |
| AA-03 | `IEmailSender` in `embyr-core::admin::email` | ADR-011 |
| AA-04 | RBAC via pure domain function in `embyr-core::admin::rbac` | ADR-012 |
| AA-05 | Fire-and-forget query log writes; `IQueryLogWriter` port | ADR-013 |
| AA-06 | `new_sdk_key_material` domain function; `backend_pg_dsn_enc` for re-encryption | ADR-014 |
| AA-07 | New `embyr-core::admin` sub-module for admin domain types and ports | This document |
| AA-08 | `UserAdminState` separate from `OperatorState`; both `with_state()`-erased before merge | This document |
| AA-09 | `QueryLogSweeper` Tokio task for partition pruning — Postgres advisory lock prevents multi-instance duplicate drops | This document |
| AA-10 | `SessionCleaner` Tokio task — hourly, deletes sessions expired > 30 days ago and expired unaccepted invitations — advisory lock pattern | This document |

---

## Constraints Carried Forward

- BLAKE3 hash storage: no plaintext credentials anywhere in DB (sessions, SDK keys, admin keys)
- `EMBYR_ENCRYPTION_KEY` required at startup (32-byte); refuse to start if absent when accounts table has rows — validated by a startup probe query
- Operator routes (Bearer EMBYR_ADMIN_KEY) must not be broken by any session-auth middleware
- SDK key generation must use `new_sdk_key_material` domain function (no bypass of ECIES scheme)
- `embyr-core` must not import any IO crate — `IEmailSender`, `IQueryLogWriter`, RBAC function are all trait definitions and pure functions only

---

## Open Questions Carried Into DISTILL

| ID | Question | Impact |
|----|----------|--------|
| OQ-B01 | DSN re-encryption for pre-existing projects: `backend_pg_dsn_enc` cannot be backfilled without the original raw key. Do existing projects simply retain their existing `ecies_encrypted_dsn` without SDK key rotation support? | SDK key rotation for pre-existing projects will silently not re-encrypt the DSN. Acceptance test must verify this edge case. |
| OQ-B02 | `BLAKE3(cookie_value)` length: BLAKE3 produces 32 bytes. Session token is a random 32-byte value formatted as base64url (43 chars). The `sessions.token_hash` column type: `BYTEA` (32 bytes) or `TEXT` (hex string)? | Minor — affects migration DDL and the hash comparison query. Recommend `BYTEA`. |
| OQ-B03 | TOTP validation library: `EMBYR_ENCRYPTION_KEY` encrypts `totp_secret_enc`. V1 signin validates the TOTP code. Which Rust TOTP crate? `totp-rs` (MIT) is the most active. Acceptance-designer needs this confirmed. | Blocks B-01 implementation; must be resolved before crafter starts. |
| OQ-B04 | `admin_api_keys` — dual-authentication with admin API keys (D6): these keys bypass session auth for the same user-facing routes. Do admin API keys have the same idle-expiry behaviour as sessions (24h)? AC-B05-11 says "immediate" revocation only. Recommend no idle expiry for admin keys (long-lived by design for CI/CD). | Needs AC clarification for acceptance-designer. |
