# ADR-005: Frontend Paradigm — Leptos 0.8 CSR WASM

## Status

Accepted

## Context

The embyr-rs project requires a browser-based administrative console for account admins (P5/Chris). The console must be served by `embyr-admin` (an Axum binary on `:9090`) and delivered at `GET /admin/`. The project is a **pure Rust workspace** — no npm, no TypeScript toolchain, no Node.js build step exists or is wanted.

Quality attributes that drive this decision (ranked):

1. **Zero JS toolchain** — no npm/webpack/vite in the workspace. Contributors are Rust engineers; context-switching to a TypeScript build system is a friction multiplier.
2. **Shared types** — domain types (`Database`, `Member`, `AdminKey`, etc.) are defined in `embyr-core` and `embyr-admin`. A Rust-native frontend can `use` these types directly; a TypeScript frontend requires manual duplication or codegen.
3. **Type safety** — all form inputs, navigation state, and domain mutations must be statically typed. No `any`, no runtime JSON duck-typing in view logic.
4. **Bundle size** — hard constraint of `<5 MB` total (from CLAUDE.md and KPI `WASM bundle size`). Any framework that adds significant JS runtime weight is a risk.
5. **Maintainability** — a single language across server and browser reduces the number of pattern libraries, linter configurations, and CI toolchains to maintain.
6. **UX responsiveness** — interactive charts (SVG with `mousemove` crosshair), modals, clipboard API, and keyboard navigation require genuine reactivity (not server round-trips on every interaction).

An approved design spec (`docs/superpowers/specs/2026-06-05-embyr-admin-ui-design.md`) was authored prior to this DESIGN wave; this ADR formalises the decision and its rationale.

## Decision

**Leptos 0.8 CSR (client-side rendering) WASM SPA**, compiled with `trunk`, served from `embyr-admin` via `axum::routing::nest_service("/admin", tower_http::services::ServeDir::new("admin-ui/dist"))`.

Key properties of the chosen approach:

- **Crate**: `crates/embyr-admin-ui/` — new workspace member, separate from `crates/embyr-admin/`
- **Build**: `trunk build --release` → `dist/` → `admin-ui/dist/` (Makefile target `make ui`)
- **Entry**: `index.html` → `leptos::mount_to_body(App)`
- **Leptos feature flags**: `csr` only in V1; `ssr` and `hydrate` reserved for V2 migration path
- **Public URL**: `Trunk.toml` sets `public_url = "/admin/"` so all asset references are correctly prefixed
- **V1 scope**: mock data only; `#[server]` functions are a V2 concern (see ADR-007)

## Consequences

**Benefits:**

- No npm, no package.json, no node_modules, no webpack config — `cargo build` + `trunk build` is the complete toolchain.
- Domain types from `embyr-core` (e.g., `Section`, `DbTab`, `NavState`) can be `use`d directly in the UI crate without codegen. A Rust struct `Database` in the UI is structurally identical to the one in the server, enforced by the type system.
- Leptos reactive primitives (`RwSignal`, `ReadSignal`, `Callback`, `Resource`, `Action`, `Effect`) directly implement The Elm Architecture (TEA) pattern: a single `RwSignal<AppModel>` is the entire mutable state; `Callback<Msg>` is the dispatch function. No external state management library is needed.
- Pure SVG charts (Sparkline, LatencyChart, BarChart, Donut) are Rust functions computing SVG paths from `&[f64]` — no JavaScript chart library required. This is safe from the bundle constraint perspective.
- The Leptos 0.8 CSR → SSR migration path is additive: add `ssr/hydrate` feature flags, add `#[server]` to async functions, replace `ServeDir` with `leptos_axum::LeptosRoutes` handler. **No component changes are required.**

**Trade-offs and costs:**

- **WASM bundle size is unvalidated until Walking Skeleton (Slice 01)**. The Leptos 0.8 CSR bundle including `web-sys`, `wasm-bindgen`, and all view components is estimated at 1–3 MB compressed, well under the 5 MB hard limit — but this is the highest-risk assumption and the primary reason Walking Skeleton is Slice 01.
- **Leptos 0.8 is relatively new** (released late 2024). The API surface stabilised in 0.7/0.8; `Resource` and `Action` APIs changed significantly between 0.5 and 0.7. Pinning to `0.8` with lockfile is necessary.
- **No URL-based deep linking in V1** — navigation state is in-memory (`NavState` in `AppModel`). The browser back button does not work. This is deferred to V2 with `leptos_router`. Account admin consoles typically have low navigation frequency; this is an acceptable V1 constraint.
- **WASM debugging** is more complex than JavaScript debugging. `wasm-bindgen` provides source maps; `console_error_panic_hook` forwards panics to the browser console. Leptos provides an `errors` context for runtime error display. This is tooling overhead for contributors unfamiliar with WASM development.

## Alternatives Considered

### Alternative A: React + TypeScript SPA

A React SPA (Vite + TypeScript) is the most familiar pattern for web UI engineers.

**Rejected because:**
- Introduces npm, Vite, TypeScript compiler, ESLint, and a separate CI toolchain to a pure Rust workspace. The contributor base is Rust engineers; every push now requires knowledge of two build systems.
- Shared types require either manual duplication (error-prone) or a codegen step (`schemars` + `json-schema-to-typescript`). Codegen adds a CI gate; manual duplication drifts silently.
- A TypeScript frontend cannot `cargo test` or `cargo clippy`. The project's CLAUDE.md requirement for mutation testing and the `per-feature` strategy apply to Rust; TypeScript would require a parallel Jest/Vitest + Stryker pipeline.

### Alternative B: HTMX + Askama (hypermedia-driven)

HTMX + Askama/Minijinja keeps the UI as server-rendered HTML with HTMX attribute-driven partial updates. Zero JavaScript bundle.

**Rejected because:**
- Interactive SVG charts with `mousemove` crosshairs require JavaScript event handling. HTMX cannot eliminate JS for this use case — it would be HTMX for CRUD sections plus raw JavaScript for charts and modals. The result is two interaction paradigms maintained in parallel.
- Clipboard API (`navigator.clipboard.writeText`) requires JavaScript. The SDK key creation flow (copy-once modal with clipboard button) requires it explicitly (AC-006-02).
- HTMX's partial-update model adds server round-trips for every navigation, every modal open, every filter change. For a single-page admin console that will eventually need real-time data, hypermedia-driven updates are a poor fit. V2 server functions would require a model change.
- The design prototype already implements a component-based SPA model. Translating it to hypermedia would require a full redesign of the interaction model.

### Alternative C: Yew or Dioxus (alternative Rust WASM frameworks)

Yew and Dioxus are Rust WASM UI frameworks with similar goals to Leptos.

**Rejected because:**
- **Yew**: Virtual DOM model (similar to React's reconciler) adds overhead versus Leptos's fine-grained reactivity (no VDOM). Performance is not the primary concern, but the additional complexity of the VDOM diffing layer is not justified when Leptos provides the same TEA-compatible programming model with finer granularity.
- **Dioxus**: Excellent framework, but Dioxus 0.5 changed APIs significantly; 0.6 (2024) is still maturing. `Resource`/`Action` primitives in Dioxus do not map as directly to the TEA pattern established in the design spec.
- **Leptos 0.8 was specifically designed** around `RwSignal<T>` + context + `Resource`/`Action` — this maps 1:1 to the design spec's `AppModel` + `Msg` + `update()` pattern.

### Alternative D: Leptos 0.8 SSR + hydration (immediate, no CSR phase)

Skip the CSR phase and implement SSR from day one with `leptos_axum` integration.

**Rejected for V1 because:**
- SSR requires `embyr-admin` to become a full `leptos_axum` server (not just a `ServeDir` mount). This couples the admin UI build to the admin server compilation — any change to the UI crate requires a full `cargo build` of `embyr-admin`.
- V1 data is mock only. SSR's primary benefit (faster initial paint from pre-rendered HTML) is irrelevant when all data is static mock. The performance benefit accrues only when real server functions exist (V2).
- The migration path from CSR to SSR is additive (feature flags, no component changes) — delaying SSR has zero architectural cost.

### Alternative E: Elm

Elm is a pure functional language with a built-in TEA runtime.

**Rejected because:**
- Introduces a third language (Rust + Elm + possibly a small JS bridge) into a Rust workspace.
- No shared types with the Rust server. The Elm type system is excellent but disjoint from Rust's type system.
- The TEA pattern that makes Elm attractive is fully available in Leptos via `RwSignal<AppModel>` + `Callback<Msg>`. There is no reason to import Elm's runtime when Leptos provides the same discipline in Rust.
