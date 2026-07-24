# Slice 01 — Walking Skeleton: Auth Gate + Dashboard

**Feature:** user-admin-ui  
**Slice:** 01 of 07  
**Estimate:** 2 days  
**Stories:** US-001, US-002

---

## Goal

Leptos 0.8 WASM SPA scaffolded, served from `embyr-admin` axum binary at `/admin/`. Login form renders; mock auth completes; Dashboard renders database cards from mock data. End-to-end browser flow works.

## Learning Hypothesis

Disproves: "Leptos WASM bundle exceeds the 5 MB hard limit when served from embyr-admin `ServeDir`."  
Confirms if succeeds: Trunk build pipeline + axum `ServeDir` + WASM load is viable; remaining UI slices can proceed.

## IN Scope

- `crates/embyr-admin-ui` workspace crate scaffolded (Cargo.toml, Trunk.toml, index.html, src/)
- `AppModel`, `Msg`, `update()` stubs for auth + nav + databases
- `data.rs` mock with 3 database entries
- Auth view: email + password + TOTP code input (6-digit CodeInput component)
- Mock auth: any non-empty password + any 6 digits → `Msg::SignIn`
- Dashboard view: database card grid from `model.databases`
- `embyr-admin` router: `.nest_service("/admin", ServeDir::new("admin-ui/dist"))`
- Makefile `ui` target: `trunk build --release` → `cp dist/ admin-ui/dist`
- WASM bundle size CI check: fail if >5 MB
- Sidebar: nav links for Dashboard, Databases, Billing, Identities, API Keys, Settings (non-functional, just render)
- Topbar: embyr logo, "New Database" button (navigates to create form placeholder)

## OUT Scope

- Real auth (any input passes)
- Real KPI data (all "—")
- Any section beyond Dashboard rendering
- TOTP secret validation (mock only)
- OIDC flow (V2)
- Recovery code flow (later slice)

## Acceptance Criteria

From US-001: AC-001-01 (SPA loads <3s), AC-001-02 (mock auth flow), AC-001-08 (mock gate), AC-001-09 (session cookie shape — V1: in-memory signal)  
From US-002: AC-002-01 (card per database), AC-002-02 (card fields with "—" KPIs), AC-002-03 (card click navigates), AC-002-04 ("New Database" button), AC-002-06 (V1 mock data)

Additional: WASM `.wasm` file ≤4.5 MB (leaving headroom for V1 additions).

## Dependencies

- `embyr-admin` crate (exists; only adds `nest_service` route)
- Leptos 0.8, wasm-bindgen, web-sys in workspace Cargo.toml
- Trunk installed in CI environment

## Pre-Slice Spike

None required — Leptos 0.8 CSR is proven technology. Bundle size is the risk; measure on first build and adjust if needed (drop unused web-sys features).
