# Evolution — user-admin-ui

**Date:** 2026-07-24
**Feature ID:** `user-admin-ui`
**Waves completed:** DISCUSS → DESIGN → DISTILL → DELIVER
**Status:** FINALIZED

---

## Feature Summary

Leptos 0.8 CSR WASM admin console served by `embyr-admin` (:9090) as a `ServeDir`-mounted SPA.
Implements The Elm Architecture (TEA): single `AppModel` + exhaustive `Msg` enum + pure `update()` function.
Covers 11 user stories across 7 delivery slices, delivering self-service account management without requiring customers to use the raw admin API or contact the embyr SaaS operator.

**Shipped artifacts:**
- `crates/embyr-admin-ui/` — new workspace crate (Leptos 0.8 CSR WASM SPA)
- `crates/embyr-admin/static/` — trunk build output (`dist/`)
- `tests/user_admin_ui/` — acceptance test suite (TEA state + per-slice + walking skeleton)
- 7 views, 8 primitive components, 4 chart types, 1 pure `update()` function

**Quality gate results:**

| Gate | Result |
|------|--------|
| Tests passing | 66 (all 14 delivery steps GREEN) |
| WASM bundle size | 564 KB (limit: 5 MB) |
| Mutation kill rate | 97% (32/33 mutants killed; target ≥80%) |
| Phase 3 L1-L6 refactoring | PASS |
| Phase 4 adversarial review | APPROVED |
| DES integrity (all 14 steps with traces) | PASS |

---

## Business Context

**Primary job:** JOB-10 (`account-admin`) — self-service account management via browser

Before this feature, embyr-rs customers had no web UI. All database creation, team management, SDK key rotation, billing review, and OIDC configuration required raw `curl` calls to the admin API or direct operator involvement.

**Opportunity score:** 16 (importance: 9/10, satisfaction: 0/10 — no UI existed)

**Target outcome KPIs:**
- Self-service database creation rate >90% (UI vs. raw API within 60 days of launch)
- Operator escalation tickets <5/month for database/access management tasks
- Mean time to SDK key (MTTK) <3 minutes from login to first key copied
- WASM bundle <5 MB (hard constraint — validated at 564 KB)
- Auth completion rate >95% of login attempts reach Dashboard

**Primary persona:** P5 Chris (Account Admin / Platform Engineer) — manages 2–20 databases, small team, pays the bill.

**Secondary jobs served:** JOB-04 (credential-isolation) and JOB-05 (cloud-secret) via the Connections panel backend config.

---

## Key Decisions

### D-UI-01: Frontend Paradigm — Leptos 0.8 CSR WASM (ADR-005)

Leptos 0.8 CSR WASM SPA compiled with `trunk`. Rejected: React/TypeScript (JS toolchain, type duplication), HTMX (inadequate for SVG charts + clipboard API), Yew/Dioxus (weaker TEA ergonomics than Leptos 0.8 RwSignal/Callback/Resource/Action), SSR in V1 (coupling deferred; CSR→SSR migration is additive).

### D-UI-02: TEA State Management — `RwSignal<AppModel>` + `Callback<Msg>` via Leptos Context (ADR-006)

Single `RwSignal<AppModel>` owned by `app.rs`. `Callback<Msg>` dispatches to pure `update()`. Provided via `leptos::provide_context()`. Rejected: prop drilling (fragile at 3+ levels), external state library (bundle weight), per-domain signals (cross-domain cascade like database-delete-cascades-sdk-keys requires atomic coordination across multiple signals).

Key guarantee: `fn update(&mut AppModel, Msg)` is testable with `cargo test` — no browser, no WASM, no Leptos runtime.

### D-UI-03: Mock-First Data Layer (ADR-007)

V1 `Resource`/`Action` async blocks call `mock::*()` from `src/data.rs`. V2 replaces those calls with `#[server]` functions. Migration contract: only the async block body changes; no component file changes between V1 and V2.

### D-UI-04: Crate Structure — Separate Workspace Member (ADR-008)

`crates/embyr-admin-ui/` is a distinct Cargo workspace member. `trunk build` compiles WASM; `cargo build` does not (WASM target invisible to host builds). Rejected: inline in `embyr-admin` (build flag coupling), outside workspace (loses `cargo deny`/audit coverage and deduplication).

### D-UI-05: Walking Skeleton Strategy B — Thin E2E Slice

Auth gate + dashboard with mock data; proves Leptos→embyr-admin pipeline. Primary risk validated: WASM bundle size. Actual result: 564 KB (well under 5 MB limit). Strategy A (hardcoded HTML) rejected — wouldn't validate the build pipeline. Strategy C (feature-complete) rejected — too large for a WS.

### D-UI-06: Test Strategy — Direct `update()` Calls (no browser automation)

All TEA state tests call `fn update(&mut AppModel, Msg)` directly. Walking skeleton uses `reqwest` against an in-process Axum server. No Playwright, no WASM runtime in tests. Motivation: the TEA seam makes browser automation redundant at 100× the infrastructure cost.

### D-UI-07: Pure SVG Charts (no JS chart library)

Sparkline, LatencyChart, BarChart, Donut as pure Rust SVG path functions. Zero bundle impact, no npm dependency.

---

## Steps Completed

14 delivery steps across 7 phases, all executed on 2026-07-24:

| Step | Name | Duration | Result |
|------|------|----------|--------|
| 01-01 | Wire embyr-admin ServeDir + WASM entry + trunk build | 06:57–07:06 | PASS |
| 02-01 | Auth TEA state: SignIn, SignOut, TotpFailure, TotpSuccess | 07:11–07:13 | PASS |
| 02-02 | Dashboard + Database CRUD TEA state (US-002–004) | 07:15–07:19 | PASS |
| 03-01 | Connections backend config + SDK key create/revoke | 07:24–07:27 | PASS |
| 04-01 | Members + sole-Owner invariant (US-009) | 07:28–07:31 | PASS |
| 04-02 | Service Accounts + Admin API Keys (US-010) | 07:33–07:35 | PASS |
| 05-01 | OIDC + Toast + Billing + Logs TEA state (US-007, 008, 011) | 07:38–07:40 | PASS |
| 06-01 | Auth view + Dashboard view + primitives (US-001, 002) | 07:43–07:56 | PASS |
| 06-02 | Databases view + DB Detail Overview (US-003, 004) | 08:01–08:02 | PASS |
| 06-03 | Connections + Keys views (US-005, 006) | 08:15–08:18 | PASS |
| 06-04 | Identities + Admin Keys views (US-009, 010) | 08:28–08:29 | PASS |
| 06-05 | Billing + Logs + Settings views (US-007, 008, 011) | 08:33–08:36 | PASS |
| 06-06 | Settings view: OIDC providers + Danger Zone (US-011) | 08:39–08:43 | PASS |
| 07-01 | trunk build --release + bundle size gate < 5 MB | 08:46–08:48 | PASS |

Total elapsed: ~2 hours (06:57–08:48 UTC)

---

## Lessons Learned

1. **DES log `--data` must be exactly `"PASS"` — not descriptive strings.** Using phrases like `"all tests pass"` instead of the canonical `"PASS"` breaks DES phase integrity checks. The correct value is the literal string `"PASS"`.

2. **Leptos 0.8 `Callback<T>` uses `.run()` not `.call()`.** The API changed from Leptos 0.7. Using `.call()` produces a type error. All dispatch patterns must use `dispatch.run(Msg::XYZ)`.

3. **Slice acceptance tests call `update()` directly — they pass immediately once `#[ignore]` is removed.** No additional test infrastructure is needed. The TEA pure-function design means the acceptance test surface is the same as the unit surface for state transitions.

4. **`cargo-mutants` revealed that proptest sole-owner tests lacked blocking-path coverage.** The `sole_owner_invariant_holds_after_member_removal` and `sole_owner_invariant_holds_after_role_change` tests used property-based strategies that did not reliably generate the blocking condition (only one owner present). Adding an explicit test case with exactly one owner ensured the invariant guard was mutation-tested.

5. **Bundle size was not the risk it appeared.** The WASM bundle landed at 564 KB against a 5 MB limit. The concern about `web-sys` feature over-request proved unfounded with careful feature selection. Future features can safely add components without approaching the limit.

6. **mock-first V1 decoupling worked exactly as designed.** Every `update()` call pattern in V1 is identical to what V2 will use — only the async block bodies in `Resource`/`Action` change. Zero component changes expected when wiring real API routes in Slice 07.

---

## Issues Encountered

| Issue | Resolution | Impact |
|-------|------------|--------|
| `Callback<T>.call()` API removed in Leptos 0.8 | Replaced all `.call()` with `.run()` | Step 06-01 GREEN blocked until fixed |
| Step 02-02 required two RED→GREEN iterations | First RED pass had 6 semantic failures in match arms (SetDatabases/DatabaseCreated/DeleteDatabase/SetDbStatus/SetDbLogging); second iteration cleared all 9 target tests | Extra ~4 minutes; not a blocking issue |
| proptest strategies under-generating sole-owner scenarios | Added explicit non-arbitrary test case alongside proptest; both forms retained | Mutation kill rate improved from ~92% to 97% |

---

## Migrated Permanent Artifacts

- Architecture docs: `docs/architecture/user-admin-ui/` — feature-delta.md, wave-decisions.md
- Acceptance scenarios: `docs/scenarios/user-admin-ui/` — spec.md, red-classification.md, slice-01 through slice-07
- ADRs: `docs/product/architecture/adr-005.md` through `adr-008.md` (pre-existing; authored during DESIGN wave)

---

## V2 Handoff Notes

Slice 07 (Backend Wiring) remains as V2 work:
- Replace `mock::*()` calls in `Resource`/`Action` async blocks with `#[server]` functions
- Wire `embyr-admin` real axum routes for databases, members, keys, billing, logs, OIDC
- No component file changes expected (by design — verified by ADR-007 contract)
- New database tables required: `accounts`, `account_members`, `users`, `oidc_providers`, `service_accounts`, `admin_api_keys`, `sdk_api_keys`, `sessions`, `invitations`, `query_logs` (see spec.md Data Model section)
- Add `leptos_router` for URL-based deep linking (deferred from V1)
