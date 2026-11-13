# ADR-082: Admin UI API Wiring — Static Serving from the Admin Listener and Effects-as-Data Client

## Status

Accepted. **Supersedes ADR-007** (mock-first data layer). Amends ADR-006 (the `update` signature and the async boundary).
Consistent with ADR-005 (Leptos CSR, served under `/admin/`), ADR-009/010 (auth sub-routers, session context).

**Related (revision 2):** server capabilities the console needs are decided separately — ADR-083 (connection secrets: single server-held
store, DSN edit and backend-mode switch) and ADR-084 (hourly metrics rollup). Decisions below are unchanged by them; screens that depend on
them are wired after those features land and hidden until then. SSO, account bootstrap/invitations and account deletion are feature-level
designs under `docs/feature/` without a separate ADR.

## Context

`embyr-admin-ui` runs entirely on `data::mock`; `embyr-server` exposes a real admin API on `:9090` but serves no UI.
The user decided to wire every screen to the real API in one pass and to serve the SPA same-origin from the admin
listener so the `HttpOnly; Secure; SameSite=Strict; Path=/admin` session cookie works without CORS.

Facts that constrain the decision:

- ADR-007 planned the migration via Leptos `#[server]` functions and `leptos_axum`. Since then a complete REST API with
  its own acceptance suites was built (`admin-api-v2` and later). A second, parallel server-function surface would duplicate it.
- The UI crate's `reqwest` is a **dev-dependency** used by host tests; the WASM build has no HTTP client.
  `web-sys`, `wasm-bindgen` and `js-sys` are already dependencies.
- `embyr-server` already depends on `tower-http` (feature `cors`). `dist/` is git-ignored and is not produced by the
  `Dockerfile`; CI's test job runs `trunk build --release` before `cargo test`; the lint and agent jobs do not.
- `embyr-server` has roughly 200 test targets and an 8 GB development machine; anything that makes the server crate's
  build depend on a WASM build is paid on every one of them.
- Quality attributes, ranked: security of the admin session, testability without a browser, operational simplicity, bundle size (< 5 MB).

## Decision

### 1. Serving: `ServeDir` from a configured directory

- New optional `EMBYR_ADMIN_UI_DIR`. Unset → the server is API-only, exactly as today. Set → a static router
  (`tower-http` `ServeDir`, nested at `/admin`) is merged onto the admin app in `main.rs`, next to the existing `healthz_app` merge.
- Explicit `/admin/v1/*` routes take precedence over the static service. No `index.html` fallback while the SPA has no URL router.
- Headers on static responses: hashed assets `Cache-Control: public, max-age=31536000, immutable`; HTML `no-cache`;
  `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `X-Frame-Options: DENY`, and a CSP whose `script-src` is
  `'self'`, `'wasm-unsafe-eval'` and the SHA-256 of Trunk's inline bootstrap, computed once at startup from `index.html`.
- Startup probe: when the variable is set, `index.html` must exist, contain one inline bootstrap, and every `/admin/<file>` it
  references must exist; otherwise the server refuses to start with a named configuration error.
- The server `Dockerfile` gains a UI build stage and sets `EMBYR_ADMIN_UI_DIR=/app/admin-ui`. The agent image is untouched.

### 2. Client: effects as data, one api module, `web-sys` fetch

- `update(&mut AppModel, Msg) -> Vec<Effect>`. `update` remains pure and deterministic; `Effect::Http(ApiCall)` is plain data.
- User intents are dispatched as `Msg::Request(ApiCall)`. The existing mutating and `Set*` messages keep their semantics and
  become the result messages applied after a successful response. No optimistic updates.
- `src/api.rs` compiles on the host and is the single anti-corruption layer: `ApiCall`, `to_request`, `decode`, private DTOs,
  and an explicit `ApiError` type. Views and `update` never see routes or JSON field names.
- The only browser-specific code is a small runtime in `app.rs` (`csr` feature) that executes effects with `web-sys` fetch and
  dispatches `decode`'s output. Dependencies added: four `web-sys` features and workspace `serde_json`. No new crate.
- `data::mock` and `AppModel::from_mock()` remain only as test fixtures; a source-scan test forbids any reference to them
  (and to `display_stats`, which is deleted) from `app.rs`, `views/` and `components/`.

## Alternatives Considered

### Serving

- **Embed `dist/` in the server binary at compile time** (`rust-embed` / `include_dir`). Single-artifact deploys and no runtime
  path to misconfigure. Rejected: adds a crate; makes every `embyr-server` build, every one of its test targets and the clippy job
  require a prior `trunk build` (or a build-script fallback that silently embeds nothing — a worse failure mode than a refused
  start); invalidates the server build cache on every UI change; grows the binary. The single-artifact benefit is already delivered
  by the container image, which is how the server ships.
- **Separate static host / CDN on another origin.** Rejected: needs CORS with credentials on `:9090` and a weaker cookie policy
  than `SameSite=Strict`; contradicts the user's same-origin decision and `cors-origin-policy`'s "no CORS on :9090".
- **`leptos_axum` SSR.** Rejected: couples UI and server compilation, changes the rendering model, no requirement asks for it.

### Client transport

- **`reqwest` (WASM backend).** Works on `wasm32`, but today it is only a host-side dev-dependency; promoting it adds a large
  dependency tree to the bundle for what is one request function. Rejected.
- **`gloo-net`.** Small and pleasant, but a new crate replacing about forty lines over APIs that are already linked. Rejected.
- **Leptos `#[server]` functions (ADR-007's plan).** Rejected: requires `leptos_axum` in `embyr-server`, and duplicates an
  existing, tested REST surface with a second contract.

### Effects

- **Views perform fetches directly (`Resource` / `Action` per view, ADR-006/007's sketch).** Fewer types, but which request an
  intent produces and how each response maps to state would live in `csr`-gated view code that host tests cannot reach. Rejected
  on testability.
- **Optimistic updates with rollback.** Rejected: more states and failure paths for an admin console where round-trips are rare
  and correctness matters more than perceived latency.

## Consequences

**Positive**

- No new crate in either build; existing server tests and the agent build are unaffected by default (variable unset).
- Request construction, response decoding, error mapping and navigation-triggered loads are all testable as pure functions, and
  can be replayed against the real router as consumer contract tests without a browser.
- A misdeployed UI directory fails at startup, not at first page load.
- Session cookie policy and CSRF posture are unchanged; the admin API still needs no CORS.

**Negative / accepted**

- Deployments must ship a directory alongside the binary (the image does this) and can misconfigure the path — mitigated by the probe.
- `update`'s signature changes and `Msg::SignIn`, `TotpFailure`, `TotpSuccess` are replaced, so some existing state tests are rewritten.
- The fetch runtime and the views are not covered by automated tests; a manual browser smoke checklist remains necessary.
- The CSP hash is derived from Trunk's generated `index.html`; a Trunk change that emits more than one inline script fails the
  startup probe and must be handled then. Inline `on…` handler attributes are no longer allowed in views.
- No compression and no deep links yet; both are additive later (`tower-http` compression, `leptos_router` + index fallback).

## Enforcement

- Compiler: exhaustive `match` in `update`; `ApiCall` variants force a `to_request` and a `decode` arm.
- Source-scan tests: no mock symbols in shipped UI code; no `/admin/v1` literal outside `api.rs`.
- Behavioural: startup-probe tests with injected faults (missing directory, missing `index.html`, missing referenced asset);
  consumer contract tests against the real admin router.
