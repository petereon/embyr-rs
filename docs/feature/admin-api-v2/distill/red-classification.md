# RED Classification — admin-api-v2 Acceptance Tests

**Author:** Quinn (nw-acceptance-designer)
**Date:** 2026-07-29
**Run:** `cargo test --test admin_api_v2_*`

Classification:
- **RED**: Test compiles, runs, panics at runtime with "Not yet implemented -- RED scaffold". Correct pre-implementation state.
- **GREEN**: Test passes (state_delta helpers only — not feature scenarios).
- **IGNORED**: Test compiled but skipped (`#[ignore]`). Will be unskipped by DELIVER one at a time.
- **BROKEN**: Would not compile or panics for wrong reason. None found after fixes.

---

## Test File Results

| Test Binary | Tests | Result |
|-------------|-------|--------|
| `admin_api_v2_walking_skeleton` | `admin_user_signs_in_views_databases_and_signs_out` | **RED** — panics "AdminTestContext::new requires embyr_server::admin::build_admin_router" |
| `admin_api_v2_walking_skeleton` | `state_delta::tests::*` (×2) | GREEN — state_delta helpers always pass |
| `admin_api_v2_b01_auth_migrations` | 12 scenarios | IGNORED (RED-ready) |
| `admin_api_v2_b01_auth_migrations` | `state_delta::tests::*` (×2) | GREEN |
| `admin_api_v2_b02_project_list` | 8 scenarios | IGNORED (RED-ready) |
| `admin_api_v2_b02_project_list` | `state_delta::tests::*` (×2) | GREEN |
| `admin_api_v2_b03_sdk_keys` | 9 scenarios | IGNORED (RED-ready) |
| `admin_api_v2_b03_sdk_keys` | `state_delta::tests::*` (×2) | GREEN |
| `admin_api_v2_b04_project_patch` | 13 scenarios | IGNORED (RED-ready) |
| `admin_api_v2_b04_project_patch` | `state_delta::tests::*` (×2) | GREEN |
| `admin_api_v2_b05_members` | 21 scenarios | IGNORED (RED-ready) |
| `admin_api_v2_b05_members` | `state_delta::tests::*` (×2) | GREEN |
| `admin_api_v2_b06_oidc_billing` | 13 scenarios | IGNORED (RED-ready) |
| `admin_api_v2_b06_oidc_billing` | `state_delta::tests::*` (×2) | GREEN |

**Walking skeleton: 1 RED (correct). All other tests: IGNORED (RED-ready). 0 BROKEN.**

---

## Scenario Count Summary

| Slice | Scenarios | Error/Edge | Error Ratio | Status |
|-------|-----------|-----------|-------------|--------|
| Walking Skeleton | 1 | 0 | — | RED |
| B-01 Auth Migrations | 12 | 9 | 75% | IGNORED |
| B-02 Project List | 8 | 4 | 50% | IGNORED |
| B-03 SDK Keys | 9 | 5 | 56% | IGNORED |
| B-04 Project Patch | 13 | 6 | 46% | IGNORED |
| B-05 Members | 21 | 10 | 48% | IGNORED |
| B-06 OIDC Billing | 13 | 6 | 46% | IGNORED |
| **Total** | **77** | **40** | **52%** | |

Target: ≥40% error ratio. Achieved: **52%** across all slices.

---

## Production Scaffold Classification

All production scaffold modules compile and panic with "Not yet implemented -- RED scaffold" when called:

| Module | Classification |
|--------|---------------|
| `embyr-core/src/admin/mod.rs` + sub-modules | RED — panics in all functions |
| `embyr-server/src/admin/state.rs` | RED |
| `embyr-server/src/admin/middleware/*.rs` | RED |
| `embyr-server/src/admin/extractors/*.rs` | RED |
| `embyr-server/src/admin/handlers/auth.rs` | RED |
| `embyr-server/src/admin/handlers/projects.rs` | RED |
| `embyr-server/src/admin/handlers/sdk_keys.rs` | RED |
| `embyr-server/src/admin/handlers/metrics.rs` | RED |
| `embyr-server/src/admin/handlers/query_logs.rs` | RED |
| `embyr-server/src/admin/handlers/members.rs` | RED |
| `embyr-server/src/admin/handlers/service_accounts.rs` | RED |
| `embyr-server/src/admin/handlers/admin_keys.rs` | RED |
| `embyr-server/src/admin/handlers/oidc_providers.rs` | RED |
| `embyr-server/src/admin/handlers/billing.rs` | RED |
| `embyr-server/src/adapters/email.rs` | RED |
| `embyr-server/src/adapters/query_log.rs` | RED |

---

## DELIVER Sequence

DELIVER unskips one test at a time (Outside-In TDD):

1. Unskip `admin_user_signs_in_views_databases_and_signs_out` (walking skeleton) — currently RED.
   Implement: migrations, session auth middleware, signin/signout handlers, list_projects handler.
   First commit to make it GREEN.

2. Unskip B-01 tests one at a time (sign_in_with_valid_credentials_returns_session_cookie first).

3. Continue through B-02 → B-03 → B-04 → B-05 → B-06 in slice order.

4. Each `@real-io @adapter-integration` test (DB verification) requires direct sqlx connection
   in `AdminTestContext` — wire these last in each slice after the HTTP tests pass.
