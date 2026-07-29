# ADR-007: Mock-First Data Layer — Resource/Action with Zero-Component-Change V2 Migration

## Status

Accepted

## Context

The embyr admin UI requires data from two categories of operations:
1. **Read operations** — load databases, members, admin keys, usage metrics, query logs
2. **Write operations** — create database, invite member, revoke key, patch connection config, etc.

In V1, no admin API endpoints exist on `embyr-admin` that the UI can call. The `embyr-admin` binary is currently a stub (prints `"embyr-admin starting on :9090"`). Building the real API routes requires schema design, SQL queries, auth middleware, and integration testing — a separate workstream from building the UI itself.

There are three approaches to this sequencing problem:

1. **Backend-first**: build all admin API routes, then build the UI against them
2. **Mock-first V1, real V2**: build the UI against mock data; swap in real server functions in V2
3. **Parallel streams with a contract**: define the API contract first, then develop both in parallel

The highest-uncertainty assumption for the UI (Leptos 0.8 bundle size, TEA pattern, trunk build pipeline) is independent of whether real data exists. Validating it at Walking Skeleton (Slice 01, 2 days) is lower cost than discovering a problem at Slice 06 (9 days in).

The design spec (`docs/superpowers/specs/2026-06-05-embyr-admin-ui-design.md`) established the `Resource`/`Action` data layer pattern. This ADR formalises the migration guarantee.

## Decision

**V1: All `Resource` and `Action` bodies return mock data from `src/data.rs`. V2: bodies call `#[server]` functions. Component code is identical in both versions.**

### The migration contract

A `Resource` in V1:

```rust
// V1 — app.rs or dashboard.rs
let _db_resource = Resource::new(
    || (),
    move |_| async move {
        let dbs = crate::data::mock::databases();   // V1 body
        dispatch(Msg::SetDatabases(dbs));
    },
);
```

The same `Resource` in V2:

```rust
// V2 — only this line changes
let dbs = fetch_databases().await?;               // V2 body
dispatch(Msg::SetDatabases(dbs));
```

The `dispatch(Msg::SetDatabases(dbs))` call and the entire component tree that reads `model.with(|m| &m.databases)` are **unchanged**. The migration is a one-line substitution inside each `Resource` or `Action` async block.

An `Action` in V1:

```rust
// V1
let create_db = Action::new(move |input: &NewDatabase| {
    let input = input.clone();
    async move {
        Ok::<Database, String>(crate::data::mock::make_database(&input))   // V1
    }
});
```

The same `Action` in V2:

```rust
// V2
async move {
    create_database_server_fn(input).await   // V2
}
```

### What `data.rs` contains

`src/data.rs` is the Rust port of the JSX prototype's `data.js`. It provides:

- `mock::databases() -> Vec<Database>` — 3–5 representative database records with varied status, backend mode, and KPI values
- `mock::make_database(input: &NewDatabase) -> Database` — constructs a new database record from form input
- `mock::members() -> Vec<Member>` — 3 members with different roles (Owner, Admin, Viewer)
- `mock::service_accounts() -> Vec<ServiceAccount>` — 2 service accounts
- `mock::admin_keys() -> Vec<AdminKey>` — keys linked to members and service accounts
- `mock::sdk_keys(db_id: &DbId) -> Vec<SdkKey>` — 2–3 per database
- `mock::oidc_providers() -> Vec<OidcProvider>` — 1 configured provider (GitHub)
- `mock::billing_usage(range: BillingRange) -> Vec<BillingRow>` — usage by database for the selected range
- `mock::query_logs(db_id: &DbId) -> Vec<LogEntry>` — 50 synthetic log entries with varied operation types

All mock data is deterministic and stable across reloads (seeded from literal values, not from random). This enables screenshot-based regression tests in CI.

### Pure UI state is exempt

Navigation, modal visibility, form field values, toast queue, and other transient UI state never need a `Resource` or `Action` — they go directly to `dispatch(Msg::...)`. This category of state is complete in V1 and requires no V2 migration.

### V2 migration verification

The zero-component-change guarantee is verified by a V2 acceptance test:

- Before V2 migration: record a git diff of all `src/views/**/*.rs` and `src/components/**/*.rs` files
- After V2 migration: git diff must show zero changes to any file outside `src/data.rs` and the `Resource`/`Action` async blocks in `src/app.rs` and view files

This test is run as part of the Slice 07 (Backend Wiring) acceptance criteria.

## Consequences

**Benefits:**

- UI development is decoupled from backend API development. Both workstreams can proceed in parallel without blocking on each other.
- Walking Skeleton (Slice 01) validates the Leptos WASM pipeline in 2 days without any backend work.
- The mock data layer (`data.rs`) serves as the first executable specification of the domain types. When V2 backend routes are built, the Rust types in `embyr-admin` must match the types already defined in `embyr-admin-ui`'s `model.rs`. This creates a cross-crate type contract.
- UI slices 01–06 can be deployed to a staging environment and reviewed by stakeholders before any real data exists. This enables early feedback on interaction design, layout, and flows.
- No feature flags, no environment variables to switch between mock and real. V1 is mock; V2 is a code change in the `Resource`/`Action` bodies. The distinction is version control, not configuration.

**Trade-offs and costs:**

- `src/data.rs` must be kept consistent with the `model.rs` types. If a domain type changes (e.g., a new field added to `Database`), `data.rs` mock constructors must be updated. The Rust compiler enforces this: mock constructors are struct literals; missing fields are compile errors.
- Mock data is deterministic/static. It does not exercise error paths (e.g., network timeouts, 500 responses). Error state in V1 is modelled via `Msg::PushToast` with explicit error messages written into `update.rs`, but they are not triggered by real failures.
- V2 server functions require `#[server]` attribute and `leptos_server` feature on `embyr-admin-ui` + `leptos_axum` on `embyr-admin`. The migration step adds dependencies to both crates. This is the only irreversible V2 change (adding dependencies cannot break V1 components).

## Alternatives Considered

### Alternative A: Build backend API first, then UI

Define and implement admin API routes in `embyr-admin`, then build the UI against real endpoints.

**Rejected because:**
- Delays UI validation and stakeholder feedback by 3–5 sprints.
- The highest-risk UI assumption (Leptos WASM bundle size) is not validated until the backend is complete.
- Backend API design decisions (JSON field names, pagination conventions, error envelope format) are influenced by the UI's needs. Building the backend without the UI means guessing what the UI will need, then retrofitting.
- Any backend API design error discovered after the UI is built requires changes to both layers simultaneously.

### Alternative B: Parallel streams with API contract (OpenAPI spec first)

Write an OpenAPI spec, stub the server with a mock (Prism or similar), build the UI against the stub.

**Rejected because:**
- Introduces a non-Rust toolchain (OpenAPI generator, Prism) into the workspace.
- The Rust type system is the contract. `embyr-admin-ui`'s `model.rs` types and `embyr-admin`'s route handlers sharing the same `embyr-core` domain types is a stronger contract than an OpenAPI doc that can drift from the implementation.
- With Leptos `#[server]` functions, the server-client contract is enforced at compile time: the function signature defined in the UI crate is the same function implemented in the server crate. No separate spec is needed.

### Alternative C: V1 uses `#[server]` with mock bodies in embyr-admin

Implement `#[server]` functions immediately in V1, but have them return hardcoded data.

**Rejected because:**
- `#[server]` requires `leptos_axum` integration in `embyr-admin` from day one, coupling the admin server build to the UI build and adding axum route registration complexity before any real data exists.
- The `leptos_axum` integration requires `embyr-admin` to become a full Leptos application server (replacing `ServeDir` with `LeptosRoutes`). This is the SSR migration path (explicitly deferred to V2).
- The migration guarantee still applies in V2 even without this overhead: `#[server]` in V2 is a transparent swap.
