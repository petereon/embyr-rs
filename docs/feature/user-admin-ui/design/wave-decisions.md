# DESIGN Wave Decisions — user-admin-ui

**Feature:** `user-admin-ui`
**Wave:** DESIGN
**Date:** 2026-06-14
**Architect:** Morgan (solution-architect)
**Mode:** Propose — autonomous analysis from approved design spec + DISCUSS artifacts

---

## Interaction Mode

**Propose** — the design spec (`docs/superpowers/specs/2026-06-05-embyr-admin-ui-design.md`) was authored and approved prior to this DESIGN wave. The architect's role was to:

1. Confirm no contradictions with DISCUSS ACs or existing architecture constraints
2. Produce the ADR paper trail for the 4 key decisions
3. Produce C4 diagrams for navigability
4. Produce reuse analysis for the DELIVER agent
5. Flag open questions

No new architectural decisions were made. All 4 ADRs formalise decisions that were locked in the design spec.

---

## Architecture Summary

The `embyr-admin-ui` is a **Leptos 0.8 CSR WASM SPA** implementing The Elm Architecture (TEA):

```
Single AppModel (RwSignal<AppModel>)
    ↓
Msg enum (30+ variants, exhaustive)
    ↓
Pure update() function (no IO, cargo-testable)
    ↓
Leptos fine-grained reactive re-render (only changed signals)
    ↓
Resource/Action data layer (V1: data.rs mock; V2: #[server] fns)
```

Served by `embyr-admin` (Axum `:9090`) via `ServeDir` at `/admin/`. Built by `trunk build --release` (separate from `cargo build`). New workspace crate `crates/embyr-admin-ui/`.

---

## Key Decisions

### D-UI-01: Frontend Paradigm — Leptos 0.8 CSR WASM (ADR-005)

**Decision:** Leptos 0.8 CSR WASM SPA compiled with `trunk`.

**Why not React/TypeScript:** npm toolchain, manual type duplication, two CI pipelines.
**Why not HTMX:** SVG charts need `mousemove` events; clipboard API needs JS regardless; HTMX is a poor fit for a component-based SPA with the existing design prototype.
**Why not Yew/Dioxus:** Leptos 0.8 `RwSignal`/`Callback`/`Resource`/`Action` map 1:1 to the TEA pattern in the design spec.
**Why not SSR in V1:** `leptos_axum` coupling deferred until V2 real data exists; CSR → SSR migration is additive (no component changes).

**Risk:** WASM bundle size is unvalidated. Slice 01 CI gate: `du -sm dist/ | cut -f1 < 5`.

---

### D-UI-02: State Management — RwSignal<AppModel> + Callback<Msg> via Leptos Context (ADR-006)

**Decision:** Single `RwSignal<AppModel>` owned by root `app.rs`. `Callback<Msg>` dispatches to pure `update()`. Both provided via `leptos::provide_context()`.

**Rejected: prop drilling** — 3+ levels deep; any model field addition requires updating every intermediate component signature.
**Rejected: external state library** — `leptos-use` doesn't provide TEA; `Arc<Mutex>` can't be held across `await`; any library adds bundle weight and versioning surface.
**Rejected: per-domain signals** — cross-domain transitions (e.g., delete database cascades to revoke SDK keys) require atomic coordination across multiple signals; `update()` handles this in one match arm.

**Key guarantee:** `fn update(&mut AppModel, Msg)` is testable with `cargo test` — no browser, no WASM, no Leptos.

---

### D-UI-03: Mock-First Data Layer (ADR-007)

**Decision:** V1 `Resource`/`Action` async blocks call `mock::*()` functions from `src/data.rs`. V2 replaces those calls with `#[server]` functions. Component code is identical.

**Migration contract:** `dispatch(Msg::SetDatabases(dbs))` is identical in V1 and V2. Only the async block body changes. No component files change between V1 and V2.

**Rejected: backend-first** — delays UI validation by 3–5 sprints; highest-risk assumption (bundle size) not validated until late.
**Rejected: parallel with OpenAPI contract** — Rust type system is the contract; `embyr-admin-ui` types and `embyr-admin` routes sharing `embyr-core` types is stronger than an OpenAPI doc.

---

### D-UI-04: Crate Structure — Separate Workspace Member (ADR-008)

**Decision:** `crates/embyr-admin-ui/` is a new Cargo workspace member. `trunk build` compiles it; `cargo build` does not (WASM target is invisible to host-target builds).

**Rejected: inline in embyr-admin** — `leptos`/`wasm-bindgen`/`web-sys` as dependencies of `embyr-admin` creates build flag coupling; conditional compilation is fragile; SSR migration changes `embyr-admin/Cargo.toml` directly.
**Rejected: outside workspace** — loses workspace deduplication; `cargo deny`/`cargo audit` tooling excludes non-members; future `embyr-core` shared types require path dependency within workspace.

---

## Reuse Analysis Summary

| Source | Classification | Count |
|--------|---------------|-------|
| JSX files → PORT (manual Rust translation) | PORT | 14 files |
| `styles.css` → copy verbatim | REUSE_ASSET | 1 file |
| `tweaks-panel.jsx` → excluded from production | DROP | 1 file |
| `data.js` mock shapes → Rust structs with type guarantees | DERIVE | 1 file |
| `embyr-admin` stub `main.rs` → extend with ServeDir | EXTEND | 1 file |
| `embyr-admin-ui` crate | CREATE NEW | entire crate |

---

## Architecture Constraints Validated

| Constraint | Source | Status |
|-----------|--------|--------|
| WASM bundle < 5 MB | CLAUDE.md | Estimated ~810 KB–1.4 MB; validated at Slice 01 CI |
| No npm/webpack/TypeScript | CLAUDE.md | `trunk` only; no Node.js build step |
| Rust-only workspace | CLAUDE.md | `embyr-admin-ui` is Rust/WASM; no JS/TS source files |
| `embyr-admin` serves at `:9090` | CLAUDE.md + design spec | `ServeDir` at `/admin/` path |
| No SSR in V1 | design spec | CSR only; `leptos/ssr` feature absent from V1 Cargo.toml |
| Mock data only in V1 | design spec | `data.rs` is the sole data source; no real API routes |
| Per-feature mutation testing (≥80%) | CLAUDE.md | `cargo-mutants` on `update.rs`; pure fn, no browser required |

---

## Contradictions Found with Prior Waves

None. The DISCUSS wave's ACs, locked decisions, and out-of-scope items are fully consistent with the design spec and this architecture.

Specific confirmations:
- AC-001-08 (V1 mock auth) → consistent with ADR-007 mock-first data layer
- AC-002-06, AC-003-07 (V1 mock data) → consistent with `data.rs` mock pattern
- DISCUSS Out of Scope (no SSR, no leptos_sse, no deep linking, no email) → consistent with V1 architecture
- DISCUSS locked decision D5 (WS = thin E2E slice) → consistent with bundle size as primary risk and Slice 01 scope

---

## Quality Gates Passed

- [x] Requirements traced to components (all 11 user stories → specific view files and Msg variants)
- [x] Component boundaries with clear responsibilities (see Component Decomposition table)
- [x] Technology choices in ADRs with alternatives (ADR-005 through ADR-008)
- [x] Quality attributes addressed (bundle size, type safety, testability, maintainability, UX responsiveness)
- [x] Dependency-inversion: `update()` has no IO; views have no direct state mutation
- [x] C4 diagrams: L1 (System Context) + L2 (Container) + L3 (Component) in `c4-diagrams-admin-ui.md`
- [x] Integration patterns specified: TEA dispatch flow + Resource/Action data layer
- [x] OSS preference: all dependencies are MIT or Apache 2.0; no proprietary tooling
- [x] AC is behavioral (what, not how): ACs reference observable outcomes, not method names
- [x] External integrations: V1 has none (mock only); V2 will call `embyr-admin` internal API (not external)
- [x] Architectural enforcement tooling: `cargo-deny`, `cargo-mutants`, `trunk build` CI bundle size gate
- [x] Earned Trust: no external substrate in V1 (mock data is in-process); V2 substrate probes are `embyr-admin`'s concern

---

## Handoff to acceptance-designer

**Feature:** `user-admin-ui`
**Paradigm:** Functional-where-practical Rust (matching `embyr-rs` CLAUDE.md); TEA pattern; pure `update()` function.
**Architecture:** Leptos 0.8 CSR WASM SPA in `crates/embyr-admin-ui/`. Single `RwSignal<AppModel>` + `Callback<Msg>` via context. Mock data in V1; `#[server]` V2.

**Key acceptance criteria boundaries for the crafter:**

- `update()` must be pure: no `tokio::spawn`, no `async`, no `println!` in `update.rs`
- `AppModel` must be `Clone` (compile-time enforced)
- `Msg` enum must be exhaustively matched (compile-time enforced)
- All `unwrap()` in production code paths is forbidden; errors go to `Msg::PushToast`
- WASM bundle size verified `<5 MB` in CI (Slice 01 gate)
- Mutation testing ≥80% kill rate on `update.rs` (per CLAUDE.md `per-feature` strategy)

**Primary SSOT artifacts:**
- Architecture brief: `docs/product/architecture/brief.md` § Application Architecture — user-admin-ui
- ADRs: `adr-005` through `adr-008` in `docs/product/architecture/`
- C4 diagrams: `docs/product/architecture/c4-diagrams-admin-ui.md`
- Feature delta (DESIGN sections): `docs/feature/user-admin-ui/feature-delta.md`

**External integrations:** None in V1. V2 will call `embyr-admin` admin API (internal — no consumer-driven contract testing needed as it is within the same deployment boundary).
