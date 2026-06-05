# embyr Admin UI — Design Spec

**Date:** 2026-06-05  
**Status:** Approved  
**Reference design:** `docs/feature/user-admin-ui/spec.md`  
**Design prototype:** HTML/CSS/JSX prototype (extracted from Claude Design handoff bundle)

---

## Overview

Browser-based administrative console for embyr-rs user-admins. Served by `embyr-admin` on `:9090`. Built as a **Leptos 0.8 client-side WASM SPA** with The Elm Architecture (TEA) using `Resource`/`Action` for the data layer.

---

## Crate Structure

New workspace member: `crates/embyr-admin-ui`

```
crates/embyr-admin-ui/
├── Cargo.toml           # leptos { features = ["csr"] }, wasm-bindgen, web-sys
├── Trunk.toml           # public_url = "/admin/"
├── index.html           # trunk entry: links styles.css, loads .wasm
├── public/
│   ├── styles.css       # verbatim port of design prototype CSS (CSS vars + classes)
│   └── assets/
│       └── embyr-mark.svg
└── src/
    ├── main.rs          # leptos::mount_to_body(App)
    ├── model.rs         # AppModel struct + all domain types
    ├── msg.rs           # Msg enum (all state transitions)
    ├── update.rs        # pure fn update(&mut AppModel, Msg)
    ├── data.rs          # mock data (Rust port of prototype data.js)
    ├── app.rs           # Root component: provides model + dispatch via context
    ├── components/
    │   ├── mod.rs
    │   ├── sidebar.rs
    │   ├── topbar.rs
    │   ├── primitives/  # Button, Badge, Card, Modal, Input, Toggle, Tabs, Menu…
    │   └── charts/      # Sparkline, LatencyChart, BarChart, Donut (pure SVG)
    └── views/
        ├── auth.rs          # AuthGate: login, MFA (TOTP + email OTP), recovery
        ├── dashboard.rs     # KPI grid + database card grid
        ├── databases.rs     # Database list table + CreateDatabaseModal
        ├── db_detail/
        │   ├── mod.rs       # wrapper: header + tabs router
        │   ├── overview.rs  # KPI tiles, latency chart, ops bar chart, logging toggle
        │   ├── connections.rs # backend config panel + live connections (V2 placeholder)
        │   ├── logs.rs      # query log table with filters
        │   └── keys.rs      # SDK API key table
        ├── billing.rs       # usage summary + per-database breakdown table
        ├── identities.rs    # Members tab + Service Accounts tab
        ├── api_keys.rs      # Admin API keys table
        └── settings.rs      # Account, auth methods, security, danger zone
```

### embyr-admin integration

`embyr-admin` axum binary on `:9090` adds:

```rust
Router::new()
    .nest_service("/admin", ServeDir::new("admin-ui/dist"))
    // existing routes unchanged
```

Build: `trunk build --release` produces `dist/` consumed by axum `ServeDir`. Not part of `cargo build` — separate Makefile target:

```makefile
ui:
    cd crates/embyr-admin-ui && trunk build --release
    cp -r crates/embyr-admin-ui/dist admin-ui/dist
```

---

## TEA Architecture

### Model (`model.rs`)

Single struct holds all mutable application state. Cloneable (required by Leptos reactive system).

```rust
#[derive(Clone, Debug)]
pub struct AppModel {
    pub authed: bool,
    pub nav: NavState,
    pub account_id: AccountId,
    pub databases: Vec<Database>,
    pub members: Vec<Member>,
    pub service_accounts: Vec<ServiceAccount>,
    pub admin_keys: Vec<AdminKey>,
    pub sdk_keys: HashMap<DbId, Vec<SdkKey>>,
    pub oidc_providers: Vec<OidcProvider>,
    pub toasts: Vec<Toast>,
}

#[derive(Clone, Debug)]
pub struct NavState {
    pub section: Section,
    pub db_id: Option<DbId>,
    pub db_tab: DbTab,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Section { Dashboard, Databases, Billing, Identities, ApiKeys, Settings }

#[derive(Clone, Debug, PartialEq)]
pub enum DbTab { Overview, Connections, Logs, Keys }
```

All domain types (`Database`, `Member`, `AdminKey`, etc.) mirror the prototype's data model, which matches the Postgres schema defined in `docs/feature/user-admin-ui/spec.md § Data Model`.

### Msg (`msg.rs`)

Exhaustive enum. One variant per discrete user action. Leaf data is `Clone`.

```rust
#[derive(Clone, Debug)]
pub enum Msg {
    // Auth
    SignIn,
    SignOut,

    // Navigation
    Navigate(Section),
    OpenDb(DbId, DbTab),

    // Databases
    SetDatabases(Vec<Database>),       // from Resource on load
    DatabaseCreated(Database),         // from Action on success
    SetDbStatus(DbId, DbStatus),
    DeleteDatabase(DbId),
    PatchDb(DbId, DbPatch),
    SetDbLogging(DbId, bool),

    // SDK keys
    SdkKeyCreated { db_id: DbId, key: SdkKey },
    RevokeSdkKey(DbId, KeyId),

    // Admin keys
    AdminKeyCreated(AdminKey),
    RevokeAdminKey(KeyId),

    // Identities
    MemberInvited(Member),
    SetMemberRole(UserId, Role),
    RemoveMember(UserId),
    ServiceAccountCreated(ServiceAccount),
    DeleteServiceAccount(ServiceAccountId),

    // Settings
    ToggleOidc(OidcId),

    // UI
    PushToast(Toast),
    DismissToast(ToastId),
}
```

### Update (`update.rs`)

Pure function — no IO, no async, no side effects. Deterministic given model + msg.

```rust
pub fn update(model: &mut AppModel, msg: Msg) {
    match msg {
        Msg::SignIn => model.authed = true,
        Msg::SignOut => *model = AppModel::from_mock(),
        Msg::Navigate(s) => { model.nav.section = s; model.nav.db_id = None; },
        Msg::OpenDb(id, tab) => {
            model.nav.section = Section::Databases;
            model.nav.db_id = Some(id);
            model.nav.db_tab = tab;
        },
        Msg::SetDatabases(dbs) => model.databases = dbs,
        // ... all 30+ variants fully implemented
    }
}
```

### Dispatch

Provided at root via Leptos context. All components access via `use_context`.

```rust
// app.rs
let model = RwSignal::new(AppModel::from_mock());
let dispatch = Callback::new(move |msg: Msg| {
    model.update(|m| update(m, msg));
});
provide_context(dispatch);
provide_context(model.read_only());
```

Components read model via `use_context::<ReadSignal<AppModel>>()` and trigger updates via `use_context::<Callback<Msg>>()`. No prop-drilling.

---

## Data Layer

### Resource (data loading)

`Resource` wraps async data loading. V1 bodies return mock data synchronously (wrapped in `async { }`). V2 bodies call `#[server]` functions — no component changes required.

```rust
// Seeds databases into the model on mount
let _db_resource = Resource::new(
    || (),
    move |_| async move {
        let dbs = mock::databases(); // V2: fetch_databases().await
        dispatch(Msg::SetDatabases(dbs));
    },
);
```

### Action (mutations)

`Action` wraps async mutations. V1 constructs results locally; V2 calls server functions.

```rust
let create_db = Action::new(move |input: &NewDatabase| {
    let input = input.clone();
    async move {
        // V2: create_database_server_fn(input).await
        Ok::<Database, String>(mock::make_database(&input))
    }
});

// Wire result back to TEA
Effect::new(move |_| {
    if let Some(Ok(db)) = create_db.value().get() {
        dispatch(Msg::DatabaseCreated(db));
    }
});
```

### Pure UI state

Navigation, modal visibility, form field values, toast queue — these never need a server round-trip. They go directly to `dispatch(Msg::...)` with no `Action` wrapper.

---

## Component Guidelines

### Primitives

Each primitive in `components/primitives/` is a typed Leptos component matching the CSS class contract of the design prototype.

```rust
// Example: Button
#[component]
pub fn Button(
    #[prop(optional)] variant: ButtonVariant,  // Default | Primary | Ghost | Danger
    #[prop(optional)] size: ButtonSize,        // Sm | Md | Lg
    #[prop(optional)] icon: Option<Icon>,
    #[prop(optional)] disabled: bool,
    on_click: Callback<()>,
    children: Children,
) -> impl IntoView { ... }
```

### Charts

Pure SVG — no JavaScript, no canvas. Port of the prototype's SVG path math to Rust functions.

```rust
fn sparkline_path(data: &[f64], w: f64, h: f64, pad: f64) -> String { ... }
```

`Sparkline`, `LatencyChart` (with hover crosshair using `mousemove` event), `BarChart`, `Donut`.

### Modal

Uses Leptos `Portal` to render outside the component tree. ESC key closes via `window_event_listener`.

---

## CSS

`public/styles.css` is the design prototype stylesheet verbatim. No modifications. Served as a static file by trunk. Three theme tokens (`data-look="ember"`, `"graphite"`, `"paper"`) and two density modes (`"comfortable"`, `"compact"`) are preserved.

The Tweaks panel (design prototype's floating controls) is omitted from the Leptos build — it was a design-time tool only. The default theme (`ember`, comfortable) is hardcoded via `data-look="ember"` on `<html>`.

---

## Authentication (V1 scope)

The auth gate renders a login form with:
- Email + password entry
- TOTP 6-digit code entry (CodeInput component with focus management)
- Email OTP flow
- Recovery code entry

V1: any non-empty password + any 6 digits → success (mock). V2: `#[server]` fn validates against Argon2id hash + TOTP secret in DB.

---

## Out of scope (V1)

- SSR / hydration (pure WASM SPA)
- Real server function bodies (mock data only)
- Live connection counts (shown as "—" with V2 badge, matching the design)
- `leptos_sse` / `leptos-server-signal` (V2, once active connections are implemented)
- Deep-linking / URL routing (nav state is in-memory)
- Tweaks panel (design-time tool, not for production)

---

## Key dependencies

```toml
[dependencies]
leptos = { version = "0.8", features = ["csr"] }
leptos_router = { version = "0.8", features = ["browser"] }  # optional, V2
leptos_meta = "0.8"
wasm-bindgen = "0.2"
web-sys = { version = "0.3", features = ["Window", "Document", "Element", "MouseEvent", "KeyboardEvent", "ClipboardItem"] }
js-sys = "0.3"
serde = { version = "1", features = ["derive"] }
uuid = { version = "1", features = ["v4", "js"] }
chrono = { version = "0.4", features = ["wasmbind"] }
```

---

## Migration path to SSR (future)

1. Add `[features] ssr = ["leptos/ssr", "leptos_axum"] / hydrate = ["leptos/hydrate"]` to `Cargo.toml`
2. Wrap server-only imports in `#[cfg(feature = "ssr")]`
3. Add `#[server]` attribute to async functions in the data layer (no component changes)
4. Replace `ServeDir` in `embyr-admin` with full `leptos_axum::LeptosRoutes` handler
5. Add `leptos_axum` to `embyr-admin` Cargo.toml

The TEA model, all Msg variants, and all components are unchanged.
