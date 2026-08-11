# C4 Diagrams — embyr-admin-ui

> Feature: user-admin-ui
> Updated: 2026-06-14

---

## C4 L1 — System Context

```mermaid
C4Context
    title System Context — embyr Admin UI

    Person(adminUser, "Account Admin (P5/Chris)", "Manages databases, team members, API keys, and billing for their embyr account. Accesses UI from a web browser.")
    Person(devSecondary, "SDK Developer (P1/Alex)", "Observes database health and retrieves SDK keys. Read-oriented usage of the console.")
    Person(devOpsSecondary, "Tenant Admin (P3/Morgan)", "Configures backend connection and agent mode secrets via the Connections panel.")

    System(adminUI, "embyr Admin UI", "Leptos 0.8 CSR WASM SPA served at /admin/. Provides self-service database management, identity/access control, billing visibility, and account settings.")

    System_Ext(embyrAdmin, "embyr-admin (Axum :9090)", "Admin HTTP server. Serves the WASM bundle via ServeDir at /admin/. V2: exposes admin API routes consumed by #[server] functions.")
    System_Ext(embyrServer, "embyr-rs SaaS", "Core embyr Firestore-protocol server. Admin API on :9090 manages project lifecycle. Does not interact with the UI directly in V1.")
    System_Ext(sysDB, "System Postgres", "Stores project records, member auth, usage metrics. Consumed by embyr-admin API (V2 only).")

    Rel(adminUser, adminUI, "manages account via", "Browser HTTPS GET /admin/")
    Rel(devSecondary, adminUI, "observes database health and keys via", "Browser HTTPS")
    Rel(devOpsSecondary, adminUI, "configures backend connections via", "Browser HTTPS")
    Rel(adminUI, embyrAdmin, "loads WASM bundle and assets from", "HTTP GET /admin/")
    Rel(embyrAdmin, sysDB, "reads/writes account and project data (V2)", "Postgres SQL")
    Rel(embyrAdmin, embyrServer, "delegates project lifecycle ops to (V2)", "Internal HTTP")
```

---

## C4 L2 — Container Diagram

```mermaid
C4Container
    title Container Diagram — embyr Admin UI

    Person(adminUser, "Account Admin (P5/Chris)")

    System_Boundary(adminUISystem, "embyr Admin Console") {
        Container(browser, "Browser WASM Container", "Leptos 0.8 CSR WASM", "Single-page application. AppModel holds all state. Msg enum drives all transitions via pure update(). Resource/Action own all async IO (V1: mock; V2: #[server] fns). Components read state via use_context::<ReadSignal<AppModel>>.")
        Container(adminServer, "embyr-admin Axum Server", "Rust binary, Axum :9090", "Serves WASM bundle + assets via ServeDir at /admin/. V2: adds admin API routes for project CRUD, member management, key issuance. V1: static file server only.")
        ContainerDb(mockData, "data.rs Mock Layer", "Rust module in embyr-admin-ui", "V1 in-process mock: databases(), members(), sdk_keys(), billing_usage(), query_logs(). Replaced by #[server] fn calls in V2 — no component changes.")
    }

    System_Ext(sysDB, "System Postgres", "accounts, projects, members, api_keys, daily_project_metrics tables (V2)")
    System_Ext(trunk, "trunk (build tool)", "Compiles Leptos WASM to dist/. Not a runtime dependency.")

    Rel(adminUser, browser, "navigates admin console via", "HTTPS browser")
    Rel(browser, adminServer, "loads index.html + WASM + styles.css from", "HTTP GET /admin/")
    Rel(browser, mockData, "reads all data from (V1)", "in-process function calls")
    Rel(trunk, adminServer, "produces dist/ consumed by", "Makefile: make ui")
    Rel(adminServer, sysDB, "queries account and project data (V2 only)", "Postgres SQL")
```

---

## C4 L3 — Component Diagram (Browser WASM Container)

```mermaid
C4Component
    title Component Diagram — Browser WASM Container (embyr-admin-ui)

    Container_Boundary(wasm, "Browser WASM Container") {

        Component(main, "main.rs", "WASM entry point", "leptos::mount_to_body(App). Initialises console_error_panic_hook for WASM panic forwarding to browser console.")

        Component(appRoot, "app.rs — App Root", "Leptos root component", "Creates RwSignal<AppModel>. Creates Callback<Msg> that calls update(). Provides both via Leptos context. Renders AuthGate or Shell depending on AppModel.authed.")

        Component(model, "model.rs — AppModel", "Rust struct, Clone", "Single struct holding all mutable state: authed, nav (NavState), databases, members, service_accounts, admin_keys, sdk_keys, oidc_providers, toasts. All domain types (Database, Member, etc.) defined here.")

        Component(msgEnum, "msg.rs — Msg", "Rust enum, Clone, 30+ variants", "One variant per discrete state transition. Covers Auth, Navigation, Databases, SDK Keys, Admin Keys, Identities, Settings, and UI (Toast) categories. Exhaustive match enforced by compiler.")

        Component(updateFn, "update.rs — update()", "Pure Rust fn, no IO, no async", "fn update(model: &mut AppModel, msg: Msg). Exhaustive match on Msg. Deterministic. Testable with cargo test — no browser, no Leptos runtime required.")

        Component(dataLayer, "data.rs — Mock Layer", "Rust module", "V1: provides mock::databases(), mock::members(), mock::sdk_keys(), mock::billing_usage(), mock::query_logs(). Deterministic (literal values). V2: replaced by #[server] fn calls. This is the ONLY file that changes between V1 and V2.")

        Component(shell, "components/sidebar.rs + topbar.rs", "Leptos components", "Sidebar: nav items, account switcher, database count badge. Topbar: breadcrumb navigation, notifications menu, user avatar menu, sign-out action.")

        Component(primitives, "components/primitives/", "Leptos component library", "Button (variant: Default|Primary|Ghost|Danger), Badge, Card, Modal (Portal-based, ESC closes), Input, Toggle, Tabs, Menu, Avatar, Spinner. Each matches CSS class contract of styles.css.")

        Component(charts, "components/charts/", "Pure SVG Leptos components", "Sparkline (polyline path from f64 slice), LatencyChart (24h + mousemove crosshair via web-sys), BarChart (hourly ops), Donut (percentage ring). No JavaScript chart library — all SVG path math in Rust.")

        Component(viewsAuth, "views/auth.rs — AuthGate", "Leptos component", "Login form: email + password. MFA step: TOTP 6-digit CodeInput (auto-focus management). Email OTP flow. Recovery code input. V1: any non-empty password + 6 digits = granted. Dispatches Msg::SignIn on success.")

        Component(viewsDashboard, "views/dashboard.rs — Dashboard", "Leptos component", "KPI grid (total databases, active count). Database card grid (one per database: name, status badge, P95 label, ops counts). Empty state with 'Create your first database' prompt. Clicking card dispatches Msg::OpenDb.")

        Component(viewsDatabases, "views/databases.rs — Databases List", "Leptos component", "Table: Name, Status, Backend Mode, Created, Actions. 'New Database' modal (name + backend mode form). Inline name-uniqueness validation. Suspend/Activate/Delete with confirm modals. Role-based action visibility.")

        Component(viewsDbDetail, "views/db_detail/ — Database Detail", "Leptos component group", "Header: database name, status badge, back breadcrumb. Tab router: Overview | Connections | Logs | Keys. Tab selection dispatches Msg::OpenDb(id, tab).")

        Component(viewsDbOverview, "views/db_detail/overview.rs", "Leptos component", "P95 latency tile, latency sparkline (24 points), ops bar chart (24h), active connections label ('—'). Logging toggle with retention-period selector (1d/7d/30d) and confirm-before-disable modal.")

        Component(viewsDbConnections, "views/db_detail/connections.rs", "Leptos component", "Backend config panel: mode label + mode-specific fields (DSN masked / agent endpoint + secret backend + secret name). Edit/Save/Cancel. Active connections panel ('—' with V2 badge). Role-based edit visibility.")

        Component(viewsDbLogs, "views/db_detail/logs.rs", "Leptos component", "Query log table: Timestamp, Operation, Collection Path, Duration (ms), Status, Client prefix. Filter controls: operation type, collection path prefix, time range, status. Sort by column header. Export CSV (up to 10k rows).")

        Component(viewsDbKeys, "views/db_detail/keys.rs", "Leptos component", "SDK keys table per database: Name, Created, Last Used ('—'), Prefix (8 chars), Revoke. Create flow: name input → result modal shows full key once with clipboard button + 'I've copied' checkbox. Revoke with confirm modal.")

        Component(viewsBilling, "views/billing.rs — Billing", "Leptos component", "Time range selector (7d/30d/this month/last month). Per-database breakdown table: database name, Read Ops, Write Ops, Delete Ops, Peak Connections ('—'), Log Storage GB. Total row.")

        Component(viewsIdentities, "views/identities.rs — Identities", "Leptos component", "Members tab: table (Email, Name, Role, Auth Method, MFA, Last Login, Actions). Invite modal (email + role). Role change selector with Owner-protection guard. Remove with confirm. Service Accounts tab: table + create modal.")

        Component(viewsApiKeys, "views/api_keys.rs — Admin API Keys", "Leptos component", "Account-level admin keys table: Name, Associated Identity, Role, Created, Last Used, Prefix, Revoke. Create modal: select identity → inherit role → name → submit → show full key once (same copy-once UX as SDK keys).")

        Component(viewsSettings, "views/settings.rs — Settings", "Leptos component", "Account section (display name). Auth Methods (email+password toggle, OIDC per-provider toggle). OIDC providers list (Issuer, Client ID, Enabled, Delete). Add provider modal (issuer + client ID + secret). Danger Zone: Delete Account (name re-entry), Transfer Ownership (email input).")
    }

    System_Ext(adminServer, "embyr-admin Axum :9090", "Serves WASM bundle. V2: admin API routes.")
    System_Ext(browserAPIs, "Browser APIs (web-sys)", "window.navigator.clipboard, KeyboardEvent (ESC), MouseEvent (chart crosshair), ClipboardItem")

    Rel(main, appRoot, "mounts")
    Rel(appRoot, model, "creates and owns RwSignal<>")
    Rel(appRoot, msgEnum, "dispatch routes Msg to")
    Rel(appRoot, updateFn, "calls on every dispatch")
    Rel(appRoot, shell, "renders when authed")
    Rel(appRoot, viewsAuth, "renders when not authed")
    Rel(updateFn, model, "mutates via &mut AppModel")
    Rel(dataLayer, model, "seeds initial AppModel.from_mock() from")
    Rel(shell, primitives, "uses Menu, Avatar, Icon from")
    Rel(viewsDashboard, charts, "renders Sparkline via")
    Rel(viewsDbOverview, charts, "renders LatencyChart + BarChart via")
    Rel(viewsBilling, dataLayer, "reads mock::billing_usage() from")
    Rel(viewsDbLogs, dataLayer, "reads mock::query_logs() from")
    Rel(viewsDbKeys, browserAPIs, "writes to clipboard via")
    Rel(primitives, browserAPIs, "ESC key closes Modal via window_event_listener")
    Rel(main, adminServer, "loads WASM bundle from")
```

---

---

## C4 L3 — Component Diagram (Billing Subsystem — card-payments)

> Feature: card-payments | Updated: 2026-08-10
> Extends the L1/L2 diagrams above unchanged (no new container, no new external system).
> Warranted per the mandatory-C4 "complex subsystem" threshold: 9 stories, 8 slices, 1
> cross-cutting component (`SuspensionBanner`) spanning every other `Section`.

```mermaid
C4Component
    title Component Diagram — Billing Subsystem (embyr-admin-ui)

    Container_Boundary(billing, "views/billing/") {
        Component(billingMod, "views/billing/mod.rs — BillingView", "Leptos component", "Page header 'Plan & Billing'. Routes Overview/Usage/Invoices via existing Tabs primitive (local RwSignal<&'static str> active-tab state).")
        Component(overview, "views/billing/overview.rs", "Leptos component group", "PlanCard, PaymentMethodCard, CapUsageCard (Free plan), NextInvoiceCard (Pro plan), TestClockCard (dev-only). Reads model.subscription + AppModel derivation methods.")
        Component(usage, "views/billing/usage.rs — UsageTab", "Leptos component", "Per-database reads/writes/deletes/storage table, 3-color stacked bar, 5 KPI tiles, time-range selector. Supersedes the old billing.rs placeholder table.")
        Component(invoices, "views/billing/invoices.rs — BillingInvoicesTab", "Leptos component", "Date/Period/Base/Overage/Total/Status table. PDF link on paid rows. Free-plan empty state.")
        Component(modals, "views/billing/modals.rs", "Leptos component group", "CardModal (Stripe-Elements-styled Rust-native form, PCI SAQ-A copy) + UpgradeModal (compare/confirm two-step, mandatory downgrade warning). Rendered at ShellView level, gated by AppModel.card_modal_open / upgrade_modal_open — not view-local (ADR-019) — so reachable from SuspensionBanner regardless of active Section.")
    }

    Component(suspensionBanner, "components/suspension_banner.rs — SuspensionBanner", "Leptos component (NEW)", "Cross-cutting. Rendered in ShellView above all routed Section content. Reads AppModel.effective_status()/read_only(). Amber (free_cap_exceeded) or red (past_due) state. CTA dispatches Msg::OpenUpgradeModal or Msg::OpenCardModal.")

    Component(modelBilling, "model.rs — Subscription/Card/Invoice/Database.usage + impl AppModel", "Rust structs + pure methods", "New domain types: Subscription, Card, CardBrand, Plan, Invoice, InvoiceStatus, UsageStats (on Database), UpgradeModalStep, EffectiveStatus. Pure derivation: usage_totals(), cap_ratios(), cap_exceeded(), effective_status(), read_only() — single-sourced D-6/D-7 logic, never duplicated as stored fields.")

    Component(msgBilling, "msg.rs — Billing Msg variants", "Rust enum variants (NEW)", "SetSubscription, SetInvoices, SetCard, SetPlan, SetPaymentFailure, OpenCardModal, CloseCardModal, OpenUpgradeModal, CloseUpgradeModal, SetUpgradeModalStep.")

    Component(updateBilling, "update.rs — Billing match arms", "Pure fn match arms (NEW)", "Direct field mutation only per Billing Msg variant. No IO. No derived business logic here — that lives in model.rs.")

    Component(dataBilling, "data.rs — FREE_CAPS, PRICING, mock::subscription(), mock::invoices()", "Rust module (EXTEND)", "Mock seed data + business constants. V2: mock::subscription()/invoices() replaced by #[server] fn bodies only (ADR-007 migration contract).")

    Component(segmented, "components/primitives/segmented.rs — Segmented", "Leptos primitive (NEW)", "N-way single-select control. Used by TestClockCard ('Payment succeeds'/'Payment fails').")

    Component_Ext(tabs, "components/primitives/tabs.rs — Tabs", "Existing primitive, reused unmodified")
    Component_Ext(modal, "components/primitives/modal.rs — Modal", "Existing primitive, reused unmodified")
    Component_Ext(icons, "components/icons.rs — Icon", "Existing component, extended: + 'trash'")
    Component_Ext(shellView, "views/mod.rs — ShellView", "Existing shell router, extended: renders SuspensionBanner + CardModal + UpgradeModal above routed content")

    Rel(shellView, suspensionBanner, "renders above routed content")
    Rel(shellView, modals, "renders CardModal/UpgradeModal gated on AppModel open-flags")
    Rel(billingMod, tabs, "renders Overview/Usage/Invoices via")
    Rel(overview, modelBilling, "reads subscription + derivation methods from")
    Rel(overview, msgBilling, "dispatches OpenCardModal/OpenUpgradeModal/SetCard/SetPlan/SetPaymentFailure via")
    Rel(overview, segmented, "TestClockCard uses")
    Rel(usage, modelBilling, "reads Database.usage + usage_totals() from")
    Rel(invoices, modelBilling, "reads model.invoices from")
    Rel(modals, modal, "wraps content in")
    Rel(modals, icons, "renders brand icons via")
    Rel(suspensionBanner, modelBilling, "reads effective_status()/read_only() from")
    Rel(suspensionBanner, msgBilling, "dispatches OpenUpgradeModal/OpenCardModal via")
    Rel(updateBilling, modelBilling, "mutates AppModel fields per")
    Rel(modelBilling, dataBilling, "from_mock() seeds from; derivation methods read FREE_CAPS/PRICING from")
```

---

## TEA Dispatch Flow

```mermaid
sequenceDiagram
    participant User as Browser User
    participant View as View Component
    participant Dispatch as Callback<Msg>
    participant Update as update()
    participant Model as RwSignal<AppModel>
    participant Data as data.rs (V1 mock)
    participant Resource as Resource/Action

    Note over View,Model: Context provided by app.rs at root

    User->>View: user action (click, input, form submit)
    View->>Dispatch: dispatch(Msg::SomeVariant(payload))
    Dispatch->>Model: model.update(|m| update(m, msg))
    Model->>Update: &mut AppModel + Msg
    Update->>Model: writes new state (pure, no IO)
    Model-->>View: ReadSignal fires — only changed signals re-render

    Note over Resource,Data: Async data loading path

    Resource->>Data: mock::databases() (V1)
    Data-->>Resource: Vec<Database>
    Resource->>Dispatch: dispatch(Msg::SetDatabases(dbs))
    Note over Dispatch,Update: Same dispatch path as sync actions

    Note over Resource,Data: V2 — only this line changes in Resource body
    Note over Resource,Data: fetch_databases().await replaces mock::databases()
```
