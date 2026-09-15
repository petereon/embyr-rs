# cors-origin-policy — Feature Delta (combined DISCUSS + DESIGN)

Closes finding #21 (Medium, Security) from `docs/product/production-readiness-audit-2026-09-08.md`.
Token-budget mode: single doc, no separate wave-decisions/slices ceremony (Medium severity, one middleware layer).

## 1. DISCUSS-equivalent — current state

**Router confirmed.** `:8081` is `HybridService` (`crates/embyr-server/src/rest/grpc_web.rs`), a per-connection
content-type dispatcher built in `spawn_all_servers` (`crates/embyr-server/src/lib.rs`):

- Requests with `Content-Type: application/grpc-web*` → forwarded straight to the tonic `GrpcWebLayer` service.
  **Never touches axum at all** — a CORS layer on the axum router cannot and does not need to cover this path;
  it's binary protobuf over a custom content-type, not a browser-`fetch`-readable JSON API, and gRPC-Web
  clients that speak this framing are SDK-generated stubs, not raw `fetch()`/`XHR` calls subject to CORS.
- Everything else → `axum_app`: `/channel` (BrowserChannel GET/POST) + `/livez`/`/healthz` +
  `/v1/projects/:project_id/accounts:action` (REST auth bridge: signIn/signUp/sendOobCode/resetPassword/
  signInWithIdp/signInAnonymously). **This is the surface a browser `fetch()` call actually hits**, and where
  CORS headers matter.

**"Fails closed today" — confirmed, not assumed.** Zero `CorsLayer`/`allow_origin` hits repo-wide (`tower-http`
is not even a dependency of `embyr-server` today — see §3). Consequence for `axum_app`:
- Simple cross-origin GET (`/channel` long-poll GET) executes server-side but the browser withholds the
  response body from JS (no `Access-Control-Allow-Origin` echoed back).
- Cross-origin POST with `Content-Type: application/json` (every accounts:action call, BrowserChannel POST)
  is **not simple** — it triggers a CORS preflight `OPTIONS` request. No `OPTIONS` route is registered and no
  CORS layer answers it, so the preflight fails and the browser never sends the real POST at all.

Confirmed fails-closed by construction, not by luck — this is the correct, deliberate baseline the design
below preserves as the *default*, not something it introduces from scratch.

**Who legitimately needs this cross-origin.** `crates/embyr-admin-ui` (Leptos SPA) is **mock-data-only today**
(`docs/product/architecture/adr-007-mock-first-data-layer.md`: "`embyr-admin` is currently a stub... In V1, no
admin API endpoints exist... the UI can call"). It makes zero real HTTP calls to `:8081` or `:9090` right now —
confirmed via grep, no `fetch`/origin/base-URL wiring in `crates/embyr-admin-ui/src`. And per the same ADR, the
**planned V2 integration is Leptos `#[server]` functions via `leptos_axum`**, i.e. the UI and its API share one
axum server/origin (SSR-style) — not a separate-origin browser fetch. So admin-ui is not, and architecturally
is not planned to become, a cross-origin consumer of either `:8081` or `:9090`.

The actual browser-CORS-relevant consumer of `:8081` is **customer web apps** built against the Firestore-SDK-
compatible REST/gRPC-Web/BrowserChannel surface — arbitrary origins the customer's own frontend is served
from, genuinely unknown to embyr-server at build time. That's exactly the shape `EMBYR_CORS_ALLOWED_ORIGINS`
(below) is for.

## 2. DESIGN — origin allowlist

### Scope: `:8081` only. `:9090` (admin API) is OUT of scope.

Checked, not assumed:
- `:9090`'s session auth (`crates/embyr-server/src/admin/middleware/session_auth.rs`) uses a cookie
  (`embyr_session`) set with `HttpOnly; Secure; SameSite=Strict; Path=/admin` (`crates/embyr-server/src/admin/
  handlers/auth.rs:103`). `SameSite=Strict` already prevents the cookie riding along on any cross-site
  request — CSRF is closed independently of CORS, by a mechanism already in place.
- No `ServeDir`/`nest_service`/CORS/origin reference anywhere under `crates/embyr-server/src` wires admin-ui
  into `:9090` today, and the ADR-007 V2 plan (`leptos_axum` `#[server]` functions) is explicitly same-origin.
- `:9090`'s secondary auth path (`Authorization: Bearer <admin_api_key>`) is also not browser-fetch-friendly
  without an explicit CORS grant, and no legitimate browser caller has been identified.

Conclusion: `:9090` needs no CORS layer. If a genuine browser cross-origin need for the admin API appears
later, it gets its own allowlist decision at that time — not bundled into this fix.

### Env var / config (`crates/embyr-server/src/config.rs`, `ServerConfig`)

Follows the established `ServerConfig`/optional-with-default convention (`EMBYR_RATE_LIMIT_RPS`,
`EMBYR_*_INTERVAL_SECS`, pool-sizing vars):

```
EMBYR_CORS_ALLOWED_ORIGINS   comma-separated origin list, e.g.
                              "https://app.customer1.com,https://app.customer2.com"
                              default: unset → empty Vec<String> → DENY ALL (fails closed, byte-for-byte
                              identical to today's no-CORS-layer behavior — see below)
```

`ServerConfig` gains one field:

```rust
/// `EMBYR_CORS_ALLOWED_ORIGINS` — comma-separated browser origins permitted to call
/// the :8081 REST/gRPC-Web/BrowserChannel surface cross-origin. Empty (default,
/// unset) = no origin is allowed, matching today's pre-CORS-layer behavior exactly
/// (ADR: fails closed is the deliberate baseline, not an accident).
pub cors_allowed_origins: Vec<String>,
```

Parse (mirrors `parse_port`/pool-var shape, whitespace-trimmed, empty entries dropped):

```rust
fn parse_cors_allowed_origins() -> Vec<String> {
    std::env::var("EMBYR_CORS_ALLOWED_ORIGINS")
        .ok()
        .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}
```

A malformed origin string (fails to parse as an HTTP header value when building the layer — e.g. contains a
raw newline) is a startup config error, not a silent drop — new `ConfigError::InvalidCorsOrigin { value: String
}` variant, same "fail fast, name the bad value" convention every other `ConfigError` variant already follows.

### `CorsLayer` construction (composition root — `spawn_all_servers`, `crates/embyr-server/src/lib.rs`)

```rust
use tower_http::cors::{AllowOrigin, CorsLayer};
use axum::http::{header, Method};

let origins: Vec<axum::http::HeaderValue> = cfg.cors_allowed_origins
    .iter()
    .map(|o| o.parse())
    .collect::<Result<_, _>>()
    // validated at ServerConfig::from_env() time (ConfigError::InvalidCorsOrigin) —
    // unreachable here in practice; composition root treats a parse failure as the
    // same startup-refusal shape as every other invalid-config case.
    .expect("cors origins validated in ServerConfig::from_env()");

let cors = CorsLayer::new()
    .allow_origin(AllowOrigin::list(origins))   // empty Vec ⇒ never matches ⇒ no ACAO header ever (deny-all)
    .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
    .allow_headers([header::CONTENT_TYPE])
    .allow_credentials(false);                  // :8081 REST bridge is API-key/body-auth, not cookie-auth

let axum_app = axum_app.merge(accounts_bridge_app).layer(cors);
```

Placement: `.layer(cors)` on the **whole merged router** (not `route_layer` on one sub-router) — `tower_http`'s
`CorsLayer` intercepts and answers preflight `OPTIONS` requests itself, before axum routing runs, so it must
sit outermost to cover every route (`/channel`, `/livez`, `/healthz`, `/v1/.../accounts:action`) including any
added later, mirroring how `route_layer(rate_limit)` already sits around the accounts-bridge router but at the
next layer out.

`allow_credentials(false)` deliberately: nothing on `:8081` reads a cookie (confirmed — no `extract_cookie`/
`Set-Cookie` usage anywhere in `crates/embyr-server/src/rest/`), so there's no reason to widen the origin
matcher's semantics or invite `Access-Control-Allow-Credentials: true` + wildcard-adjacent mistakes later.

**Regression guard, not just intent:** with the default empty allowlist, `AllowOrigin::list(vec![])` never
matches any `Origin` header, so the layer never emits `Access-Control-Allow-Origin` — response shape is
byte-identical to today's no-CORS-layer behavior. This is what makes "empty default = fails closed" a provable
property, not a documentation promise, and is exactly the regression test the crafter should write in DELIVER
(no CorsLayer test infra needed — cite this design section for the assertion).

## 3. ADR decision: **no new ADR.**

`tower-http` is an already-established dependency in this codebase (`crates/embyr-admin-ui/Cargo.toml:66`,
`tower-http = { version = "0.5", features = ["fs"] }`, direct non-workspace dep — not promoted to
`[workspace.dependencies]`). Adding `CorsLayer` to `embyr-server` is:

```toml
# crates/embyr-server/Cargo.toml [dependencies]
tower-http = { version = "0.5", features = ["cors"] }
```

— a new **direct** dependency on `embyr-server` (it isn't one today; confirmed via grep, zero hits in that
Cargo.toml), but same crate/version/pattern already proven elsewhere in the workspace, mirroring
`embyr-admin-ui`'s own precedent exactly (direct dep, not workspace-pinned, since only one crate needs it).
Nothing architecturally novel: it's one `tower::Layer` on an existing axum `Router`, following the exact
env-var/`ServerConfig`/composition-root conventions ADR-079 (pool-sizing-and-limits) and ADR-018
(secrets-management) already established. Doesn't meet the bar for a new ADR (no new component, no new
integration pattern, no cross-cutting architectural decision).

## 4. Self-review (DoR-equivalent)

- **Problem clear?** Yes — audit finding is specific (`:8081` has no CORS policy, landmine risk), verified
  independently (zero `CorsLayer` hits, confirmed router shape) rather than taken on faith.
- **Scope right-sized?** Yes — one middleware layer, one new `ServerConfig` field, one new dependency line. No
  redesign of the hybrid dispatcher, no touch to `:9090`, no touch to gRPC-Web's binary-framing path (which
  structurally bypasses axum/CORS entirely — confirmed via `HybridService::call`'s content-type dispatch, not
  assumed).
- **Technical approach sound?** Yes — reuses the exact `ServerConfig` env-var/default/`ConfigError` pattern
  this session already established twice (pool-sizing-and-limits ADR-079, deployment-release-process), and
  the exact `tower-http` dependency/version this workspace already runs in `embyr-admin-ui`. Placement of
  `.layer(cors)` (outermost, not `route_layer`) is deliberate and justified (preflight interception semantics),
  not copy-pasted from the rate-limiter's `route_layer` pattern without checking whether it fits.
- **Security tradeoff explicitly reasoned?** Yes, three-way, not just "permissive is bad":
  - `CorsLayer::permissive()` / wildcard origin — **rejected**: exactly the landmine the audit finding warns
    against; would let any origin read authenticated REST-bridge responses.
  - Hardcoded origin list in source — **rejected**: customer origins are deploy-time, per-operator data, not
    compile-time constants; every other cross-cutting config in this codebase is env-var-driven
    (`ServerConfig`), and CORS origins are exactly that shape.
  - **Chosen**: env-var-driven allowlist, empty-by-default (deny-all), matching today's de facto fails-closed
    behavior exactly when unset, opt-in per deployment when a real browser customer needs it. Provably
    regression-safe (see `AllowOrigin::list(vec![])` argument above), no accidental broadening.

**Verdict: READY.** No blocking gaps. Handoff to acceptance-designer/crafter can proceed directly — small
enough that a full DoR/reviewer dispatch would be scope-inflation for a Medium-severity, single-layer fix.
