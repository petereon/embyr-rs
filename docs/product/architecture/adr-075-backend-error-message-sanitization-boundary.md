# ADR-075: Backend Error Message Sanitization — a Single Reusable Boundary Function, Not Per-Site Fixes

## Status

Accepted

## Context

`docs/product/production-readiness-audit-2026-09-08.md` finding #10 (High): raw Postgres/sqlx driver
error text reaches the client verbatim in gRPC `Status` messages, confirmed to embed literal column
names (`"ecies_encrypted_dsn"`, `"api_key_hash_current"`) via `sqlx::Error::ColumnDecode`'s own
`Display` format, and Postgres's own constraint/table names via `sqlx::Error::Database`. Full
investigation is in `docs/feature/sanitize-backend-error-messages/feature-delta.md`.

DISCUSS found the true leak surface is far larger than the audit's own citation: only 4 real call
sites in `crates/embyr-server/src/grpc/handler.rs` route through the existing `core_error_to_status`
boundary function; the remaining `Status::internal(` sites (59 in that file, 9 in
`realtime/listen_handler.rs`, 9 in `crates/embyr-agent/src/server.rs`) independently reconstruct the
identical inline anti-pattern — `Status::internal(e.to_string())` / `Status::internal(format!("...{e}
..."))` — bypassing `core_error_to_status` entirely. DESIGN's own exhaustive re-classification of all
77 sites (feature-delta.md § DESIGN Decision 3) found 17 MORE genuine leak sites beyond DISCUSS's own
9 confirmed instances — every `system_db.get_*_access_rule`/`list_*_access_rule_pattern*` call site
across the access-rule-evaluation code paths in `RunQuery`/`RunAggregationQuery`/write handlers/
`GetDocument`, all sharing the identical shape (a `CoreError` wrapping a raw `sqlx::Error` from a
`system_db` query).

`CoreError::BackendUnavailable` is the only one of `CoreError`'s 12 variants carrying raw driver text
(confirmed per-variant in DISCUSS's own Investigation Finding 3); one site
(`handler.rs:3556`) wraps a bare `sqlx::Error` that never goes through `CoreError` at all. This ADR
records the mechanism DESIGN chose to close both the 9 DISCUSS-confirmed sites and the 17 newly
classified ones with one reusable pattern instead of 26 independent, differently-shaped fixes.

## Decision

### Decision 1 — One small, per-binary helper function: `sanitize_backend_error`

```rust
/// The ONE conversion point between a raw backend/driver error and a
/// client-facing Status anywhere in this binary. `context` names WHICH
/// operation failed (a static string chosen by the call site, never
/// derived from `e`) so the server-side log stays useful without ever
/// echoing `e`'s own text into anything client-visible.
fn sanitize_backend_error(e: impl std::fmt::Display, context: &'static str) -> Status {
    tracing::error!(error = %e, "{context}");
    Status::internal("internal server error")
}
```

Defined once in `crates/embyr-server/src/grpc/handler.rs` (`pub(crate)`, imported into
`realtime/listen_handler.rs` exactly the way that file already imports
`grpc::handler::query_compliance_rejection` — an established same-crate cross-module reuse pattern,
not a new one) and independently, identically, in `crates/embyr-agent/src/server.rs`. Every one of the
26 confirmed genuine leak sites (feature-delta.md § DESIGN Decision 3's classification table) replaces
its inline `Status::internal(e.to_string())`/`format!(...)` with a call to this function, supplying a
short, site-specific `context` string. `core_error_to_status`'s own catch-all (`embyr-server`) and
explicit `BackendUnavailable` arm (`embyr-agent`) call it too — the SAME function closes both US-01's
boundary-function fix and US-02's direct-bypass fixes; there is no second mechanism.

### Decision 2 — Generic message text: one fixed string, `"internal server error"`, never varied by RPC or context

Considered varying the message per RPC/subsystem (e.g. `"internal error: authentication"` vs.
`"internal error: query"`) to remain "useful without leaking detail." Rejected: it adds a second
axis of behavior to test and maintain for zero operator value (the operator's real signal is the
server-side `tracing::error!` line, which already carries full detail and the `context` string), and
it reopens a narrower version of the same problem this feature closes — a sufficiently patient prober
could use WHICH generic message came back to fingerprint which internal code path failed, information
the client has no legitimate use for (JOB-11's own `fair-multitenancy` framing). One string, reused
everywhere `CoreError::BackendUnavailable`-class failures surface, is both the simplest implementation
and the strictly safer one.

### Decision 3 — Two independent, mirrored helpers, not one shared crate

`embyr-core` must remain IO-free (workspace constraint, `deny.toml`-enforced) and `Status` is a
`tonic` type, so the helper cannot live there. No existing shared, non-generated crate is positioned
to hold cross-binary runtime code — `embyr-proto` is stub-generation only, and inventing a new crate
for a 3-line, twice-duplicated function is unrequested infrastructure (ponytail: no new crate for a
value that never changes). This mirrors the codebase's own existing precedent: `core_error_to_status`
is ALREADY two independent, non-shared functions, one per binary, not one shared function both call.
The two `sanitize_backend_error` copies stay in lockstep by inspection (both are 3 lines) and by this
ADR being the recorded source of truth for the shape both must follow.

### Decision 4 — The one bare-`sqlx::Error` site (`handler.rs:3556`) is sanitized directly, not routed through `CoreError` first

`handle_listen`'s notify-listener-pool `PgPoolOptions::connect()` failure is a bare `sqlx::Error`,
never wrapped in `CoreError`. `sanitize_backend_error` accepts `impl Display`, so it already handles
this uniformly alongside every `CoreError::BackendUnavailable` site — wrapping it in
`CoreError::BackendUnavailable` first would add a construction step that exists only to satisfy an
appearance of consistency, not to change behavior. Smaller diff, same fix, same log content.

### Decision 5 — Scope: exactly the sites where the underlying value can carry raw driver/schema/constraint text

Every one of the 77 `Status::internal(` sites across the three files was individually classified (see
feature-delta.md § DESIGN Decision 3 for the full table). Left unchanged, by category, with the
established reasoning:

| Category | Example | Why safe as-is |
|---|---|---|
| `tokio::task::JoinError` | `handler.rs:256` (`spawn_blocking` join) | Never carries driver/schema text |
| Static string constant | `handler.rs:267` etc. (`"...project missing backend_secret_arn"`) | Fixed, developer-authored, does not vary per request |
| AWS/GCP secret-fetcher error | `handler.rs:275,293` | Different vendor/error-domain than Finding #10 (Postgres/sqlx); named follow-up candidate, not this feature's scope |
| ECIES/crypto validation error | `handler.rs:308,341` | Internal data-corruption signal on the server's own previously-stored ciphertext, not client-triggerable via ordinary use |
| JSON-parse-of-internal-data | `handler.rs:312` | Same category as above |
| Rules-grammar re-parse (`ConditionParseError`) | `handler.rs:775,831,893,...` (16 sites), `listen_handler.rs:182,193,526` | `ConditionParseError` carries only `{detail, construct}` describing the STORED CONDITION TEXT's own grammar problem — confirmed by reading `embyr-core/src/access_control/mod.rs:289-293` — never a column/constraint/DSN |
| Caller-input parse/validation | `handler.rs:760`, `listen_handler.rs:81,82,83,89` | Echoes the CALLER's OWN malformed request back to that same caller (parent path, project_id format) — same safe category as `CoreError::InvalidArgument` (DISCUSS Finding 3); wrong status code in some cases, but that is a separate, out-of-scope correctness issue, not a leak |
| Self-configuration validation | `embyr-agent/server.rs:699,712,753` | `ProjectId::new(&self.project_id)` validates the AGENT'S OWN configured project_id, not caller- or database-supplied data |
| `SystemTime` clock error | `embyr-agent/server.rs:772` | Cannot carry driver/schema text under any input |
| `tonic::Streaming` transport-decode error | `handler.rs:3530,3663` | `e` here is already a `tonic::Status` (the stream item type is `Result<T, Status>`) — re-wrapping it is redundant, not a new leak |
| Static "channel closed" | `listen_handler.rs:259,272,287` | Fixed string, internal `mpsc` state, never driver-derived |
| Static "document not found after creation/update" | `embyr-agent/server.rs:366,428` | Fixed string; arguably a wrong status code (should be `not_found`), out of scope for a message-content-only fix |

## Consequences

### Positive

- One mechanism (`sanitize_backend_error`) closes all 26 confirmed genuine leak sites across both
  binaries and three files — no per-site design decision, matching the mechanical-fix precedent this
  session already established for the crash-elimination arc and the `BackendUnavailable`-creation-site
  count.
- Zero new dependency, zero new `CoreError` variant, zero new port/adapter trait method.
- The exhaustive per-site classification (AC-SBM-10) surfaced 17 genuine leak sites DISCUSS's own
  narrower reading did not name — closing them now, in this same feature, instead of leaving a
  silently-incomplete "done" state.
- Zero existing test regression: confirmed no test in this workspace asserts on `Status::internal`'s
  own message content for an Internal-code response (DISCUSS Investigation Finding 4, a workspace-wide
  grep, not scoped to the originally-named sites — generalizes to the 17 newly found sites too).

### Negative / accepted residuals

- The static "project missing X"/"TLS bundle missing Y" messages in `authenticate()`
  (`handler.rs:267,271,285,289,303,306,315,320,325,339`) name real config-column identifiers
  (`backend_secret_arn`, `ecies_encrypted_dsn`, etc.) in fixed, developer-authored strings. These are
  NOT touched by this feature (DISCUSS's own scope is `CoreError::BackendUnavailable`-class dynamic
  driver text, not static diagnostic constants) — named here as a plausible, smaller follow-up finding,
  not evidenced by the audit and explicitly not built in this feature.
- Several sites route a caller's-own-malformed-input error through `Status::internal` instead of
  `Status::invalid_argument` (wrong status code, not an information leak) — out of scope per this
  feature's own narrow "message content, not status code" framing (feature-delta.md § System
  Constraints).
- No request/project-id correlation field is added to the `tracing::error!` call. Considered and
  deferred: the existing 5-site convention this feature reuses (`main.rs:50,62`,
  the 3 sweepers) does not carry one either, and adding it now means threading an extra parameter
  through 26 call sites for a benefit no AC requires. A future observability feature (JOB-12) adding
  request-scoped tracing spans is the right place to add correlation IDs everywhere at once.

## Alternatives Considered

**For Decision 1 (mechanism shape):**
1. **Hand-write `tracing::error!` + `Status::internal("...")` at each of the 26 call sites
   independently** — rejected. The same 2-line pattern repeating 26 times is exactly the shape ponytail
   discipline calls out as warranting a tiny shared helper, not hand-duplication.
2. **A shared helper function, one per binary (selected)** — smallest abstraction that removes the
   duplication without introducing new cross-crate coupling.

**For Decision 3 (shared vs. mirrored):**
1. **New shared crate (e.g. `embyr-grpc-support`) housing the helper** — rejected. Three lines of code
   duplicated twice does not justify a new workspace crate, a new `Cargo.toml` dependency edge for both
   binaries, and a new compilation unit — over-engineering for the problem size.
2. **Two independent, mirrored functions (selected)** — matches the codebase's own existing
   `core_error_to_status` precedent exactly; zero new crate, zero new dependency edge.

**For Decision 4 (bare-`sqlx::Error` site):**
1. **Wrap in `CoreError::BackendUnavailable` first, then sanitize** — rejected. Adds a construction
   step and a `CoreError` import at a call site that has no other reason to touch `CoreError`, for a
   consistency argument the `impl Display` signature already satisfies without it.
2. **Sanitize the bare `sqlx::Error` directly at the call site (selected)** — smaller diff, identical
   client-facing and log-facing outcome.

## Enforcement

No new automated enforcement tool. This is a message-content-only change at an existing conversion
boundary — no new port/adapter boundary, no new external dependency, no new `CoreError` variant. The
regression guard is the existing gRPC status-CODE assertions (DISCUSS Investigation Finding 4) plus
the new DISTILL-authored scenarios asserting the ABSENCE of driver-specific substrings
(`"error occurred while decoding column"`, `"constraint"`, connection host/port patterns) in every
`Status::internal` message across the 26 fixed sites — see feature-delta.md § DESIGN Handoff Package.
