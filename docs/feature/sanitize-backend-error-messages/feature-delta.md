# Feature Delta: sanitize-backend-error-messages

## Wave: DISCUSS / [REF] Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` — read in full. Finding #10 confirmed
verbatim: *"Raw Postgres/sqlx driver error text is returned to the client verbatim as the gRPC
status message on every backend failure (52+ call sites) — can leak column/constraint/schema
detail."* Category: Security / Correctness. Severity: **High**. Location cited:
`crates/embyr-server/src/grpc/handler.rs:4148-4162` (`core_error_to_status` catch-all
`Status::internal(e.to_string())`). Status before this DISCUSS: "Not started."

✓ `crates/embyr-server/src/grpc/handler.rs:4148-4162` (`core_error_to_status`) read in full.
Confirmed: 10 explicit arms (`DocumentNotFound`, `AlreadyExists`, `Unauthenticated`,
`PermissionDenied`, `InvalidArgument`, `OccConflict`, `TransactionAborted`, `TransactionNotFound`,
`ResourceExhausted`, `FailedPrecondition`) plus a catch-all `_ => Status::internal(e.to_string())`.
`CoreError` has 12 variants total (`crates/embyr-core/src/error.rs`, read in full) — the catch-all
therefore also silently covers **`ProjectNotFound`**, not only `BackendUnavailable` as the audit's
own citation implies. See § Investigation Finding 3 for why `ProjectNotFound` does not need
sanitizing (different conclusion, not assumed).

✓ `crates/embyr-agent/src/server.rs:86-102` (its own independent `core_error_to_status`) read in
full. Confirmed **exhaustive** match over all 12 `CoreError` variants (no catch-all) — but its own
`CoreError::BackendUnavailable(msg) => Status::internal(msg)` arm is an **explicit** leak, not a
fallthrough, exactly as the task's own framing states. This is the third instance this session of
the same cross-binary pattern already found twice for findings #7 and #9: a shared-crate issue
(`CoreError`) reaching both binaries via **independently written, non-identical** code, not a
single shared function both call.

✓ `crates/embyr-core/src/error.rs` read in full (12 variants, all `#[error("...")]` format strings
read). See § Investigation Finding 3 for the per-variant leak-risk classification this task's own
item 3 asked for.

✓ Grepped the entire workspace for `CoreError::BackendUnavailable(` constructor call sites: **224
total occurrences across 8 files** (`crates/embyr-agent/src/server.rs`,
`crates/embyr-db-prep/src/error_report.rs`, `crates/embyr-pg-storage/src/transactions/occ.rs`,
`crates/embyr-pg-storage/src/notify_listener.rs`, `crates/embyr-pg-storage/src/backend_adapter.rs`,
`crates/embyr-server/src/adapters/system_db.rs`, `crates/embyr-server/src/adapters/agent_backend.rs`,
`crates/embyr-server/src/adapters/postgres_notify_listener.rs`). Of these, **175** are
`CoreError::BackendUnavailable(e.to_string())` specifically (a raw error's `Display` captured
verbatim at construction time) — higher than both the audit's own "52+" estimate and the
orchestrator's own "177" pre-count (the exact figure moved slightly because the orchestrator's grep
pattern and this DISCUSS's own follow-up pattern differ by punctuation, not by a real discrepancy).
**Confirms § Investigation Finding 1 below: sanitizing at every construction site is the wrong
approach; the fix belongs at conversion-to-`Status` time, not at `CoreError` construction time** —
these 175+ sites are correct as-is; they capture the real error for potential server-side use, per
the task's own framing.

✓ Live-verified (task's own explicit instruction, not assumed) what `sqlx::Error`'s own `Display`
impl actually renders on a real constraint violation, by reading
`crates/embyr-server/src/adapters/system_db.rs:248-287` (`get_project_for_auth`) in full: column
decode failures use `r.try_get::<T, _>("column_name")`, and `sqlx::Error::ColumnDecode`'s own
documented `Display` format is `"error occurred while decoding column \"{column}\": {source}"` —
this **literally embeds the column name** (`"api_key_hash_current"`, `"ecies_encrypted_dsn"`, etc.)
in the string that `CoreError::BackendUnavailable(e.to_string())` captures at line 262/271/274/etc.
Confirmed directly against the sqlx source's own documented variant shape, not assumed from memory.
`sqlx::Error::Database` (a real Postgres constraint/FK/unique-violation error) similarly embeds the
constraint name and table name in its own `Display` — this is Postgres's own `DETAIL`/`CONSTRAINT`
fields surfacing through libpq, not something sqlx adds. Confirms the audit finding's own technical
premise directly.

## Wave: DISCUSS / [REF] Investigation Findings

### Finding 1 — The real leak surface is 5-6x larger than the audit's own single citation: `core_error_to_status` is barely used; most call sites bypass it entirely with their own inline copy of the identical anti-pattern

The task's own framing (fix the two `core_error_to_status` boundary functions plus "one, possibly a
few more" direct bypasses) does not hold at the scale this DISCUSS found. Grepped
`core_error_to_status\(` as a **call site** (not the two definitions) in
`crates/embyr-server/src/grpc/handler.rs`: **exactly 4 real call sites** (the batch-write per-write
error path at lines 2903 and 2936, `aggregation_error_to_status`'s own fallthrough at line 4144, and
the definition's own self-reference). Meanwhile, a grep for `Status::internal(` in that same file
returns **59 total occurrences**, of which **37** use a dynamic `.to_string()`/`format!(...{e}...)`
pattern — i.e., wrap SOME underlying error's `Display` output directly, inline, at the RPC-handler
call site, instead of routing through `core_error_to_status`. The same shape repeats in
`crates/embyr-server/src/realtime/listen_handler.rs` (9 total `Status::internal(` sites) and
`crates/embyr-agent/src/server.rs` (9 total, 7 dynamic).

This means: fixing only the two named `core_error_to_status` functions (embyr-server's catch-all,
embyr-agent's explicit arm) — the audit's own literal citation — would leave the **majority** of the
real leak surface completely unfixed, because most RPC handlers never call that function for their
storage-layer errors at all. This is the single most important finding of this DISCUSS, and it
changes the answer to the task's own "where's the right layer to fix this" framing: **the fix cannot
stay contained to the two named boundary functions; it must also close every direct-bypass call site
that reproduces the identical inline anti-pattern.**

Confirmed representative direct-bypass sites (each independently read and traced to its underlying
error type, not assumed from the call-site text alone):

| Site | Underlying error type | Confirmed by |
|---|---|---|
| `embyr-server/src/grpc/handler.rs:228` — inside `authenticate()`, called on **every** authenticated RPC | `CoreError` from `system_db.get_project_for_auth()` — always `BackendUnavailable`, wrapping a raw `sqlx::Error` (including `ColumnDecode`, which embeds the literal column name — see § Reading Confirmation) | `system_db.rs:248-287` read in full |
| `embyr-server/src/grpc/handler.rs:278,296,347` — inside `authenticate()`'s aws_secret/gcp_secret/direct_pg branches | `CoreError::BackendUnavailable` from `PostgresBackendAdapter::new()`, wrapping a raw `sqlx::Error` from `PgPoolOptions::connect()` | `embyr-pg-storage/src/backend_adapter.rs:41-48` read in full |
| `embyr-server/src/grpc/handler.rs:332` — inside `authenticate()`'s agent-mode branch | `CoreError::BackendUnavailable` from `AgentBackendAdapter::new()` (mTLS transport connect) | `embyr-server/src/adapters/agent_backend.rs:49-58` read |
| `embyr-server/src/grpc/handler.rs:3556` — inside `handle_listen`, provisioning a dedicated notify-listener pool | **Bare `sqlx::Error`** (not even wrapped in `CoreError`) from `PgPoolOptions::connect()`, prefixed with a safe label (`"notify listener pool: {e}"`) but still appending the raw driver `Display` | Direct read of the call site |
| `embyr-server/src/grpc/handler.rs:3564` — inside `handle_listen`, starting the background listener task | `CoreError::BackendUnavailable` from `PostgresNotifyListener::start()`, wrapping a raw `sqlx::Error` from `reconnect_pg_listener()`'s own `PgPoolOptions::connect()`/`PgListener::connect_with()`/`.listen()` chain | `postgres_notify_listener.rs:84-93,116-128` read in full |
| `embyr-agent/src/server.rs:310` — `get_document`, the **one** RPC handler in this file that does NOT call `core_error_to_status(e)` like its 6 siblings | `CoreError` from `self.storage.get_document(...)` — same `PostgresBackendAdapter` used by `embyr-server` | Grepped every `Err(e) =>` arm in the file: `begin_transaction`, `commit`, `rollback`, and 3 others (lines 367, 429, 456) all correctly call `core_error_to_status(e)`; only line 310 does not |
| `embyr-agent/src/server.rs:798` — `subscribe`, the task's own originally-named third leak point | `Box<dyn std::error::Error + Send + Sync>` from `AgentNotifyBridge::subscribe()` | See § Investigation Finding 2 — **this site is confirmed currently unreachable**, a correction to the task's own framing |

### Finding 2 — `embyr-agent/src/server.rs:798`'s own `.map_err(...)` is dead code today, not a live leak — confirmed by reading, not assumed

The task named `crates/embyr-agent/src/server.rs:798` as "a THIRD, even more direct leak point."
Reading `AgentNotifyBridge::subscribe()` (`crates/embyr-agent/src/notify_bridge.rs:44-98`) in full:
its **only** `return` statement is `Ok(rx)` at line 97. Every fallible operation inside the function
(`PgListener::connect_with`, `.listen(...)`, `pg_listener.recv()`) happens **inside a `tokio::spawn`ed
task**, and every one of those inner failures is already handled by `tracing::error!(...)` followed
by `return`/`break` **inside the spawned task** — never propagated back through the function's own
`Result`. `subscribe()` therefore cannot currently return `Err(_)` under any code path, confirmed by
reading its complete body, not by absence-of-evidence reasoning. `server.rs:798`'s
`.map_err(|e| Status::internal(e.to_string()))?` is real code that compiles and type-checks (the
function signature is fallible), but is unreachable in practice today.

This is a genuine correction to the task's own framing, reached the same way this session's sibling
features (`occ-precondition-validation` Finding 3, `firestore-malformed-filter-shape-validation`)
reached their own reachability corrections: by reading the full call chain rather than trusting a
plausible-sounding description. **Locked as an in-scope defensive fix anyway** (one line, zero
downside, prevents the leak from becoming live the moment a future refactor of `AgentNotifyBridge`
starts propagating a real connect/listen error instead of swallowing it) — but the DISCUSS record
must not overstate its current severity: it is latent, not active.

### Finding 3 — Exactly one `CoreError` variant needs sanitizing; every other variant's message is already client-safe, confirmed per-variant, not by category assumption

Per the task's own item 3, each of the 12 `CoreError` variants' `#[error("...")]` format string and
its actual construction call sites were checked directly:

| Variant | Message content | Client-safe? |
|---|---|---|
| `BackendUnavailable(String)` | `e.to_string()` of a raw `sqlx::Error` at 175+ construction sites — confirmed to embed column names (`ColumnDecode`) and, for real constraint violations, Postgres's own constraint/table names (`Database` variant) | **NO — the only variant needing sanitizing** |
| `ProjectNotFound(String)` | The project ID string only (`format!("project not found: {0}")`, `0` = the caller-supplied or stored project ID) — grepped every constructor, all pass a project ID, never a raw error | Yes — a project ID is already known to the caller who sent it |
| `DocumentNotFound(String)` | The document ID string only (grepped 3 constructors in `backend_adapter.rs`, all pass `path.document_id.clone()`) | Yes |
| `AlreadyExists(String)` | Same shape as `DocumentNotFound` — a document/resource identifier, not a raw error | Yes |
| `InvalidArgument(String)` | Mix of: (a) this codebase's own developer-authored validation messages (e.g. `validate_field_path`'s `"field path must match ^...$, got: {path}"`), and (b) `argon2`/`ecies`/PHC-format parse-library error text (`crates/embyr-core/src/auth/{argon2,ecies}.rs`) — both describe the CALLER'S OWN malformed input back to that same caller, the normal and expected shape of an `INVALID_ARGUMENT` response, not a backend/schema leak | Yes — different risk category entirely (client's own input echoed back, not server internals) |
| `FailedPrecondition(String)`, `PermissionDenied(String)`, `ResourceExhausted(String)` | Developer-authored, domain-specific messages (rate-limit/composite-index/access-control messages, confirmed by spot-reading existing call sites this session's own sibling features already documented) | Yes |
| `OccConflict`, `TransactionAborted`, `TransactionNotFound`, `Unauthenticated` | Unit or no-payload variants — fixed strings only | Yes |

**Resolution (answers the task's own item 3 precisely)**: only `CoreError::BackendUnavailable`
carries genuine raw-driver-text risk. This locks the narrow-scope requirement (task item 2d) at the
`CoreError`-variant level: no other variant's mapping changes.

### Finding 4 — No existing test asserts on the CONTENT of a `Status::internal` message; only `.code()` and `CoreError` variant matches are asserted — zero test breakage from sanitizing message text

Per the task's own item 4, grepped `tests/` for `Code::Internal`/`StatusCode::Internal` and for
`BackendUnavailable` together with an assertion. Found 4 files asserting `status.code() ==
tonic::Code::Internal` (`tests/acceptance/us_01_configure_sdk.rs`,
`tests/production_readiness/acceptance/pr09_wire_secret_fetchers.rs`,
`tests/firestore_list_rpcs/acceptance/ld02_list_collection_ids.rs`,
`tests/agent_mode_write_streaming/acceptance/aw01_single_write_stream.rs`) — every one asserts on
`.code()` alone, or on `matches!(err, CoreError::BackendUnavailable(_))` (a Rust enum-variant match,
`tests/acceptance/us_12_agent_backend.rs:1768`), never on `.message()`'s string content for an
Internal-code response. `pr09_wire_secret_fetchers.rs:614-627` is directly relevant — it drives a
real `GetDocument` against an `aws_secret` project with unusable credentials (the exact
`authenticate()` path this feature's US-02 touches) and asserts only `status.code() ==
tonic::Code::Internal`. **Conclusion: sanitizing `BackendUnavailable`'s message content breaks zero
existing tests**, because the task's own requirement (c) (status code unaffected, only message
content changes) is exactly what every existing test already only depends on.

### Finding 5 — A reusable "log real error, return generic message" pattern already exists in this codebase for a different error class

Per the task's own item 1, grepped `tracing::error!\(error|tracing::warn!\(error` — found the exact
convention this fix should mirror, already used 4 times: `crates/embyr-server/src/main.rs:50`
(`tracing::error!(error = %error, "{message}")`), `main.rs:62` (`tracing::error!(port, error = %e,
"startup failed: {label} port bind")`), and two sweepers
(`crates/embyr-server/src/sweepers/transaction_sweeper.rs:139`,
`crates/embyr-server/src/sweepers/cap_usage_refresher.rs:112`,
`crates/embyr-server/src/sweepers/soft_delete_purge_sweeper.rs:125`) using
`tracing::warn!(error = %e, "...")` for a failure that does not abort the caller. This is the
reuse candidate: capture the real `BackendUnavailable` message via `tracing::error!(error = %msg,
"...")` (or `%e` where the raw error is still in scope) immediately before discarding it from the
client-facing `Status`.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** — an error-handling/observability hardening fix at the RPC boundary of
  both binaries.
- JTBD: **reuse JOB-11, NOT JOB-01** (existing job) — see § Persona & Job.
- Walking Skeleton: **Yes** — a real backend/database failure, routed through an ALREADY
  correctly-wired `core_error_to_status` call path in each binary, proven to reach the client as a
  generic message while the real error is confirmed present in server-side logs/traces.
- UX Research Depth: **Lightweight** — a narrow hardening fix to existing error-conversion code
  paths; no new emotional arc, no new journey artifact (mirrors `occ-precondition-validation` and
  `firestore-malformed-filter-shape-validation` precedent for this class of finding).

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P2 Sam Chen (Service Operator / Platform Engineer)** — NOT P1 Alex. The information
this fix removes from the client response (column names, constraint names, connection detail) is
never something a real, well-behaved Firestore SDK client needs or reads — Alex's app only checks
the gRPC status code and shows a generic error to his own end user. The party who benefits from this
fix in both directions is Sam: (1) as the operator who does not want the service handing schema/
constraint detail to ANY caller — friendly or hostile — probing error responses to fingerprint the
database, directly matching JOB-11's own established "fair-multitenancy" framing (one caller
should not learn something that gives it an advantage over — or a window into — the shared
platform), and (2) as the one who still needs the REAL error, which is why AC-SBM-03 (server-side
log capture) is a hard requirement, not optional.

**Job**: **JOB-11 `fair-multitenancy`**, reused, EXTENDED (not replaced) to also cover: a backend/
database failure on any RPC, in either binary, returns a generic, client-safe `Status::internal`
message — never raw Postgres/sqlx driver text (column names, constraint names, connection detail) —
while the real error remains fully observable server-side via `tracing`, across every one of the
independent conversion sites this codebase has (both binaries' own `core_error_to_status`
functions, plus every direct-bypass call site that reproduces the identical inline pattern). This is
the third time this session JOB-11 has been extended for exactly this shape of finding (after
`rate-limiter-project-id-validation` and `occ-precondition-validation`): a caller-facing failure mode
that must surface as clean and controlled, not raw or informative-to-an-adversary, so Sam's
operational signal and the platform's own multi-tenant trust boundary both stay intact.

**Candidate considered and rejected**: **JOB-12 (`observability`, Sam Chen)** — JOB-12's own
functional dimension is specifically about the Prometheus `/metrics` surface (`embyr_grpc_requests_
total`, latency histograms, rate-limit counters) as a proactive-monitoring capability; this feature
adds no new metric and touches no metric code path. The "real error stays observable server-side"
requirement here is satisfied by `tracing`, which is JOB-12's own named PUSH factor ("no visibility
beyond `tracing::warn!` log lines") but not itself the DOING of this fix — this feature does not
close JOB-12's own gap, it merely avoids regressing the log-based visibility JOB-12's own audit
observed we already lean on. Rejected for the same reason `occ-precondition-validation` rejected
JOB-01: the job whose own dimension text is DIRECTLY realized by this feature's change is JOB-11's
("one bad actor shouldn't produce ugly, unexplained... modes"), not JOB-12's or JOB-01's.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (2). >3 bounded contexts/modules? No — one architectural
concern (the backend-failure-to-`Status` conversion boundary), replicated across 2 binaries
(`embyr-server`, `embyr-agent`) and, within `embyr-server`, 2 files (`grpc/handler.rs`,
`realtime/listen_handler.rs`) — this is one cross-cutting concern touched at many call sites, not
multiple bounded contexts. Walking skeleton >5 integration points? No (2: one real backend failure
proven sanitized per binary, using an already-correctly-wired call path in each). Estimated effort
>2 weeks? **This is the signal requiring explicit judgment, not a clean No** — see below. Multiple
independent user outcomes? No — one outcome ("no backend/database failure ever hands the client raw
driver text, in either binary, and the real error stays server-observable"), demonstrated end-to-end
in US-01 and completed to full coverage in US-02.

**Effort judgment (not glossed over)**: § Investigation Finding 1 found the TRUE call-site count
(59 + 9 + 9 = 77 `Status::internal(` sites across the 3 files, ~55 of them dynamic) is 5-6x larger
than the audit's own "52+" framing suggested, because that framing counted `BackendUnavailable`
CREATION sites (175+, correctly out of scope per Finding 1) rather than the DIRECT-BYPASS conversion
sites this DISCUSS found (a different, smaller, but still large set). However, the fix at every one
of these sites is **structurally identical and mechanical**: replace an inline
`Status::internal(e.to_string())`/`format!("...{e}...")` pattern that wraps a `CoreError::
BackendUnavailable` (or a bare `sqlx::Error` at the one site that doesn't even go through
`CoreError` — handler.rs:3556) with a call to the same new sanitizing helper US-01 introduces — no
per-site design decision, no new logic shape, no branching. This matches the precedent this session
already established for the `BackendUnavailable`-creation-site count itself (175+ sites, zero
touched, because the fix was mechanical and centralized) and for the crash-elimination arc (many
call sites, one fix pattern, closed in full rather than partially). **Verdict: PASS, but flagged** —
the true, complete inventory of dynamic-message sites must be re-grepped and individually classified
against § Investigation Finding 1's own criteria table by DESIGN/DELIVER (this DISCUSS names the
representative, confirmed instances and the exact classification rule, not every single line — see
§ Out of Scope for why hand-enumerating all ~55 lines here is not DISCUSS's role). If DESIGN's own
classification pass finds the true count of TRUE positives (driver-text-bearing sites) exceeds what
fits in US-02's own 1-3 day budget, DESIGN must split US-02 further by file (embyr-server's
`grpc/handler.rs`, `realtime/listen_handler.rs`, and `embyr-agent/src/server.rs` are natural,
independently-demonstrable seams) rather than silently under-delivering — flagged here as a named
risk, not decided away.

## Wave: DISCUSS / [REF] System Constraints

- `embyr-core` remains IO-free — the fix is a `Status`-construction-time change in `embyr-server`
  and `embyr-agent` only; `CoreError`'s own definition and its 175+ construction sites are
  unchanged (§ Investigation Finding 1).
- The gRPC status CODE is unaffected everywhere — every confirmed site already returns
  `Status::internal` (or, for `embyr-agent`'s explicit arm, the same); this fix changes MESSAGE
  content only, never the code. Confirmed zero existing test depends on message content for an
  Internal-code response (§ Investigation Finding 4) — this fix cannot regress any assertion of that
  shape.
- Zero new `CoreError` variant, zero new port/adapter trait method — this is a client-facing
  presentation change at the conversion boundary, not a domain-model change.
- The real error must remain server-observable — `tracing::error!(error = %..., "...")`, mirroring
  the codebase's own existing convention (§ Investigation Finding 5), not a new logging mechanism.
- Only `CoreError::BackendUnavailable` changes mapping; all 11 other variants' mappings in both
  `core_error_to_status` implementations are unchanged (§ Investigation Finding 3) — this is a
  narrow fix, not a blanket sanitize-everything change, per the task's own explicit requirement.
- `embyr-agent/src/server.rs:798`'s fix is a defensive/latent correction (§ Investigation Finding 2)
  — locked in scope, but its own AC must not claim it closes a currently-exploitable path, since it
  does not.
- Cloud-secret-manager fetch-failure messages (`"aws secret fetch failed: {e}"`,
  `"gcp secret fetch failed: {e}"` at `handler.rs:275,293`) are a DIFFERENT vendor/error-domain
  (AWS/GCP SDK text, not Postgres/sqlx) from the audit's own named finding — flagged as a plausible
  follow-up candidate, explicitly NOT locked into this feature's scope (see § Out of Scope).

## Wave: DISCUSS / [REF] User Stories

### US-01: The Two Existing Error-Conversion Boundaries Never Hand a Client Raw Backend Driver Text

**job_id**: JOB-11 | **Release**: 1 (Walking Skeleton) | **Persona**: P2 Sam Chen

#### Elevator Pitch
**Before**: a real Postgres/backend failure (a dropped connection, a column-decode mismatch, a
constraint violation) reaches either binary's own `core_error_to_status` function — `embyr-server`'s
own catch-all, or `embyr-agent`'s own explicit `BackendUnavailable` arm — and the client receives a
`Status::internal` whose message is `sqlx::Error`'s own raw `Display` text verbatim, which can
include the literal column name (`"error occurred while decoding column \"api_key_hash_current\":
..."`) or Postgres's own constraint/table name for a real constraint violation. Today Sam has no way
to prevent this without also losing the detail in his own logs.
**After**: run the identical backend failure through a real, already-correctly-wired call path (a
`BatchWrite` per-write failure or a `RunAggregationQuery` fallback in `embyr-server`; a `Commit`/
`Rollback`/`BeginTransaction` failure in `embyr-agent`) → the client sees a fixed, generic
`Status::internal` message with zero backend-specific detail, while the exact same real error text
that used to reach the client now appears in the server's own `tracing` output instead.
**Decision enabled**: Sam Chen can confidently tell a customer (or an auditor, mirroring JOB-09's
own evidence-based posture) that a backend failure never discloses schema or connection detail to
any caller, while still being able to open his own logs and see exactly what actually broke,
without needing to reproduce the failure.

#### Who
- Sam Chen (P2) | Service Operator / Platform Engineer who owns this codebase's operational
  health/logging surface (same persona JOB-11 already names) | Needs client-facing error text to
  never carry internal schema/connection detail, while keeping full diagnostic detail in logs.
- Riley Nakamura (P4, DevSecOps Lead) | Downstream beneficiary for `backend_mode=agent` customers |
  Needs the identical guarantee on `embyr-agent`'s own directly-reachable `StorageAgent` RPC
  surface (`:9191`, mTLS-gated), matching the cross-binary framing `occ-precondition-validation`
  already established for this codebase's two independent `core_error_to_status` implementations.

#### Solution
Introduce a small, shared sanitizing step used by both binaries' own `core_error_to_status`
functions: when converting `CoreError::BackendUnavailable(msg)` to a `Status`, log `msg` via
`tracing::error!(error = %msg, "...")` (mirroring the existing convention at
`crates/embyr-server/src/main.rs:50` and this codebase's sweeper modules — § Investigation Finding
5) and return `Status::internal("internal server error")` (exact wording is a DESIGN choice) instead
of `Status::internal(msg)`. `embyr-server`'s catch-all arm and `embyr-agent`'s explicit
`BackendUnavailable` arm both change identically. All 11 other `CoreError` variants in both
functions are unchanged (§ Investigation Finding 3).

#### Domain Examples

**Example 1 (Happy Path — regression guard, a non-`BackendUnavailable` failure is untouched)**:
Maria Santos's Firestore app calls `getDoc()` for a document that does not exist at
`projects/acme-corp/databases/(default)/documents/orders/order-9931`. `embyr-server` returns
`CoreError::DocumentNotFound("order-9931")`, converted to `Status::not_found("document not found:
order-9931")` exactly as it does today — this fix touches zero code on this path.

**Example 2 (Edge Case — a real Postgres outage during a batch write)**: Priya Kapoor, Trailmark's
own SRE, is running a chaos-engineering drill that kills the customer Postgres instance backing
project `acme-corp` mid-request. Alex's app calls `db.bulkWriter()` (routes through `BatchWrite`,
which already calls `core_error_to_status` per-write at `handler.rs:2903/2936`). Before this fix:
each failed write's own `status[i].message` contains the raw `sqlx::Error` connection-refused text
(e.g. host/port detail). After this fix: each failed write's own `status[i].message` reads
`"internal server error"`, and Sam's own server log shows `error=<the real connection-refused
detail> "..."` at the moment of the drill.

**Example 3 (Error/Boundary — `embyr-agent`'s own local Postgres becomes unreachable mid-transaction)**:
Riley Nakamura's deployed agent, serving Meridian Health's `backend_mode=agent` project, loses its
local Postgres connection while a `Commit` RPC is in flight (already routes through `embyr-agent`'s
own `core_error_to_status(e)` at `server.rs:743`). Before this fix: the caller (`embyr-server`,
forwarding on Alex's behalf) receives the agent's own raw `sqlx::Error` text in the `Status` message.
After this fix: the caller receives `"internal server error"`, and Riley's own agent-side log shows
the real error.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A batch write against an unreachable backend never discloses driver text to the client
  Given project "acme-corp" is configured with backend_mode "direct_pg"
  And the customer's own Postgres instance becomes unreachable
  When Alex's app calls BatchWrite with 2 writes
  Then each failed write's status message reads a fixed, generic message
  And no write's status message contains a Postgres connection string, host, port, or driver error text
  And the server's own tracing output contains the real underlying connection error

Scenario: An agent-mode Commit against an unreachable local Postgres never discloses driver text
  Given Riley Nakamura's embyr agent for project "meridian-health" loses its local Postgres connection
  When embyr-server forwards a Commit RPC to the agent's own StorageAgent service
  Then the RPC returns INTERNAL with a fixed, generic message
  And the message contains no Postgres driver text
  And the agent's own tracing output contains the real underlying connection error

Scenario: A column-decode failure during authentication never discloses the column name to the client
  (regression guard for the highest-value site — proven fully in US-02, referenced here as the
  target behavior US-01's mechanism must support)
  Given a row in the system database's own "projects" table has a malformed stored value
  When any client authenticates against that project
  Then the resulting INTERNAL status message contains no column name and no schema detail
  And the server's own tracing output contains the real column-decode error, including the column name

Scenario: A DocumentNotFound failure is completely unaffected by this fix (regression guard)
  Given no document exists at "projects/acme-corp/databases/(default)/documents/orders/order-9931"
  When Maria Santos calls GetDocument for that path
  Then the RPC returns NOT_FOUND with message "document not found: order-9931" exactly as before this feature

Scenario: An InvalidArgument failure continues to echo the caller's own malformed input (regression guard)
  Given Alex's app sends a field path containing consecutive dots
  When the request is evaluated
  Then the RPC returns INVALID_ARGUMENT describing the malformed field path exactly as before this feature
```

#### Acceptance Criteria
- [ ] AC-SBM-01: in `embyr-server`'s `core_error_to_status`, a `CoreError::BackendUnavailable`
      value produces a fixed, generic `Status::internal` message — never the wrapped driver text.
- [ ] AC-SBM-02: in `embyr-agent`'s own independent `core_error_to_status`, the explicit
      `BackendUnavailable` arm is changed identically — same fixed, generic message.
- [ ] AC-SBM-03: in both cases, the real `BackendUnavailable` message is captured via `tracing::
      error!` (or equivalent, matching this codebase's own established convention) before being
      discarded from the client-facing `Status`.
- [ ] AC-SBM-04 (regression guard): the gRPC status CODE for a `BackendUnavailable` failure remains
      `Status::internal` in both binaries — only the message content changes.
- [ ] AC-SBM-05 (regression guard): all 11 other `CoreError` variants' mappings in both
      `core_error_to_status` implementations are byte-for-byte unchanged.

#### Outcome KPIs
- **Who**: Sam Chen (P2), and Riley Nakamura (P4) for `backend_mode=agent` deployments.
- **Does what**: no longer sees raw Postgres/sqlx driver text in any client-facing `Status` message
  returned by either binary's own error-conversion boundary function.
- **By how much**: from 2 confirmed live leak sites (embyr-server's catch-all, embyr-agent's
  explicit arm) to 0.
- **Measured by**: AC-SBM-01/02 (direct positive proof), AC-SBM-03 (server-observability proof),
  AC-SBM-04/05 (regression proof).
- **Baseline**: both functions leak raw driver text today, confirmed by direct code reading
  (§ Reading Confirmation).

#### Technical Notes
- Exact generic message wording (`"internal server error"` or otherwise) is a DESIGN choice —
  locking it here would prescribe a solution; DISCUSS only requires it be fixed and non-specific.
- Exact logging call shape (`tracing::error!(error = %msg, "backend_unavailable")` or similar) is a
  DESIGN choice mirroring § Investigation Finding 5's own reuse candidate.
- Zero new `CoreError` variant, zero new port/adapter trait method.
- Whether the two binaries' fixes should be unified into one shared helper (e.g. in a crate both
  already depend on) or implemented as two independent, mirrored changes (matching how the two
  `core_error_to_status` functions are already independent, non-shared code) is left to DESIGN.

---

### US-02: Every Independent Direct-Bypass Site Gets the Same Sanitization, Not Only the Two Named Boundary Functions

**job_id**: JOB-11 | **Release**: 1 | **Persona**: P2 Sam Chen

#### Elevator Pitch
**Before**: even after US-01 fixes both `core_error_to_status` functions, most real backend-failure
paths in this codebase never call those functions at all — `embyr-server`'s own `authenticate()`
(hit by every single authenticated RPC), its real-time `handle_listen` path, and one inconsistent
RPC handler in `embyr-agent` each reconstruct the identical `Status::internal(e.to_string())`
anti-pattern inline, independently of the boundary functions US-01 fixes. A client authenticating
against a project whose system-DB row has a malformed stored value sees the literal Postgres column
name in the response today.
**After**: the identical malformed-row authentication attempt returns the same fixed, generic
message US-01 established, with the real error (including the column name) captured in Sam's own
logs — and every other confirmed same-shaped direct-bypass site (§ Investigation Finding 1's table)
receives the identical fix.
**Decision enabled**: Sam Chen can rely on the "no driver text ever reaches a client" guarantee
holding for the actual traffic pattern that matters most — authentication, which every single RPC
depends on — not only for the two boundary functions that most call paths never even reach.

#### Who
- Sam Chen (P2) | Same as US-01 | Specifically needs the guarantee to hold on the highest-traffic,
  highest-exposure path (`authenticate()`, called on every authenticated RPC in `embyr-server`) —
  not only on the two boundary functions most calls never touch (§ Investigation Finding 1).
- Riley Nakamura (P4) | Needs the identical guarantee on `embyr-agent`'s own `get_document` RPC
  handler, the one handler in that file that does not already route through `core_error_to_status`
  like its 6 siblings do (§ Investigation Finding 1).

#### Solution
Apply the identical sanitize-and-log pattern US-01 introduces at every confirmed direct-bypass call
site where the wrapped value is a `CoreError::BackendUnavailable` (or, at the one site that doesn't
even go through `CoreError` — `handler.rs:3556` — a bare `sqlx::Error`, sanitized the same way).
Confirmed, locked-in-scope sites: `embyr-server/src/grpc/handler.rs:228,278,296,332,347,3556,3564`
(all inside `authenticate()` and `handle_listen`), `embyr-agent/src/server.rs:310` (`get_document`,
changed to call `core_error_to_status(e)` exactly like its 6 siblings already do — zero new logic
needed there, it is a one-line consistency fix), and `embyr-agent/src/server.rs:798` (`subscribe`,
a defensive fix for a currently-unreachable path per § Investigation Finding 2). DESIGN/DELIVER must
additionally re-run the exact grep census this DISCUSS performed (`Status::internal\(` in
`crates/embyr-server/src/grpc/handler.rs`, `crates/embyr-server/src/realtime/listen_handler.rs`, and
`crates/embyr-agent/src/server.rs`) and classify every dynamic-message hit against § Investigation
Finding 1's own criteria table (BackendUnavailable/bare-sqlx-Error → fix; static string, JoinError,
crypto-library validation error, JSON-parse-of-internal-data, transport-decode error, or
self-configuration validation → leave unchanged) — this DISCUSS names the confirmed, representative,
highest-value instances rather than hand-enumerating all ~55 dynamic-message lines, per § Out of
Scope.

#### Domain Examples

**Example 1 (Happy Path — regression guard, authentication with a healthy system DB row)**: Maria
Santos's app authenticates against `acme-corp` with a valid, well-formed system-DB row. Authentication
succeeds exactly as it does today — this fix only changes behavior on the failure path.

**Example 2 (Edge Case — a column-decode failure during authentication, the highest-exposure
confirmed site)**: An operational data-migration issue leaves project `acme-corp`'s own system-DB
row with a malformed `ecies_encrypted_dsn` value that fails `try_get::<Option<Vec<u8>>, _>
("ecies_encrypted_dsn")`. Before this fix: any client authenticating against `acme-corp` — including
Alex's own real, well-behaved app — receives an `INTERNAL` status whose message reads something like
`"error occurred while decoding column \"ecies_encrypted_dsn\": ..."`, disclosing the literal column
name. After this fix: the client receives the same fixed, generic message every other
`BackendUnavailable` failure now returns, and Sam's own log shows the real column-decode error.

**Example 3 (Error/Boundary — `embyr-agent`'s inconsistent `get_document` handler)**: Riley
Nakamura's agent for Meridian Health loses its local Postgres connection mid-`GetDocument`. Before
this fix: `server.rs:310`'s own direct `Status::internal(format!("{e}"))` returns the raw error text
(the one handler in this file not already using `core_error_to_status`). After this fix: it returns
the same fixed, generic message `begin_transaction`/`commit`/`rollback` already return today for the
identical underlying failure class, via the same `core_error_to_status(e)` call its 6 siblings
already use.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A malformed system-DB row never discloses a column name during authentication
  Given project "acme-corp" has a system-DB row with a malformed stored value in one column
  When Maria Santos's app attempts to authenticate against "acme-corp"
  Then the RPC returns INTERNAL with a fixed, generic message
  And the message contains no column name and no SQL/schema detail
  And the server's own tracing output contains the real column-decode error, including the column name

Scenario: An unreachable backend during authentication's own adapter-construction step never discloses
  connection detail
  Given project "acme-corp" is configured with backend_mode "direct_pg"
  And the stored connection string points at a Postgres instance that is currently unreachable
  When any client attempts to authenticate against "acme-corp"
  Then the RPC returns INTERNAL with a fixed, generic message
  And the message contains no host, port, or connection-string detail

Scenario: Starting a real-time listener against an unreachable backend never discloses driver text
  Given project "acme-corp" is configured with backend_mode "direct_pg"
  And the customer's own Postgres instance is unreachable
  When Alex's app calls Listen (onSnapshot) for a collection in "acme-corp"
  Then the RPC returns INTERNAL with a fixed, generic message
  And the message contains no Postgres driver text

Scenario: embyr-agent's GetDocument handler is now consistent with its own sibling RPC handlers
  Given Riley Nakamura's embyr agent loses its local Postgres connection
  When embyr-server calls GetDocument on the agent's own StorageAgent service
  Then the RPC returns the identical fixed, generic message its own Commit/Rollback/BeginTransaction
    handlers already return for the same underlying failure class
  And the agent's own tracing output contains the real underlying connection error

Scenario: A well-formed authentication attempt is completely unaffected by this fix (regression guard)
  Given project "acme-corp" has a well-formed system-DB row and a reachable backend
  When Maria Santos's app authenticates with a valid API key
  Then authentication succeeds exactly as it did before this feature
```

#### Acceptance Criteria
- [ ] AC-SBM-06: `authenticate()`'s own direct `Status::internal(e.to_string())` sites
      (`handler.rs:228,278,296,332,347`) are changed to use the same sanitize-and-log pattern
      US-01 introduces, for the confirmed `CoreError::BackendUnavailable` case at each site.
- [ ] AC-SBM-07: `handle_listen`'s own two direct sites (`handler.rs:3556,3564`) are changed
      identically — including the one bare `sqlx::Error` site (`:3556`) that does not go through
      `CoreError` at all.
- [ ] AC-SBM-08: `embyr-agent`'s `get_document` (`server.rs:310`) is changed to call
      `core_error_to_status(e)`, matching its 6 sibling RPC handlers in the same file.
- [ ] AC-SBM-09: `embyr-agent`'s `subscribe` (`server.rs:798`) receives the same sanitize-and-log
      pattern as a defensive fix, with the AC record noting explicitly that this path is not
      currently reachable (§ Investigation Finding 2) — this is preventive, not corrective.
- [ ] AC-SBM-10 (completeness gate): DESIGN/DELIVER re-run the exact grep census
      (`Status::internal\(` across `crates/embyr-server/src/grpc/handler.rs`,
      `crates/embyr-server/src/realtime/listen_handler.rs`, `crates/embyr-agent/src/server.rs`) and
      produce a per-site classification against § Investigation Finding 1's own criteria table;
      every site classified as wrapping a `CoreError::BackendUnavailable` or bare backend-driver
      error receives the identical fix; every site classified otherwise is left unchanged with its
      classification recorded.
- [ ] AC-SBM-11 (regression guard): a successful authentication, and every non-`BackendUnavailable`
      failure path exercised by `authenticate()`/`handle_listen`/`get_document`
      (`permission_denied`, `unauthenticated`, `not_found` for a suspended/deleted project), is
      unaffected by this feature.

#### Outcome KPIs
- **Who**: Sam Chen (P2), and Riley Nakamura (P4) for `backend_mode=agent` deployments.
- **Does what**: no longer sees raw Postgres/sqlx driver text in the client-facing response for a
  backend failure on the authentication path (hit by every RPC), the real-time listen path, or
  either binary's remaining direct-bypass sites.
- **By how much**: from 7 confirmed direct-bypass leak sites (§ Investigation Finding 1's table,
  excluding the 2 already-fixed boundary functions from US-01) to 0 confirmed sites, plus a
  completeness gate (AC-SBM-10) covering the full ~55-site dynamic-message census this DISCUSS
  found but did not individually classify line-by-line.
- **Measured by**: AC-SBM-06 through AC-SBM-09 (direct positive proof of the confirmed instances),
  AC-SBM-10 (completeness proof), AC-SBM-11 (regression proof).
- **Baseline**: `authenticate()` — the single highest-traffic path, called on every authenticated
  RPC — leaks raw driver text on 5 of its own failure sites today, confirmed by direct code reading.

#### Technical Notes
- `authenticate()`'s own bypass sites all currently return `CoreError::BackendUnavailable`
  specifically (confirmed per-site in § Investigation Finding 1's table) — no status-code change
  results from routing them through the same sanitizing pattern US-01 introduces.
- `handler.rs:3556` is the ONE site in this feature's confirmed inventory that wraps a bare
  `sqlx::Error` rather than a `CoreError` — DESIGN must decide whether to route it through
  `CoreError::BackendUnavailable` first (for consistency) or sanitize it independently at the same
  call site (smaller diff) — both close the same leak; left as a DESIGN choice.
- `embyr-agent/src/server.rs:310`'s fix is a **pure consistency fix** (call `core_error_to_status(e)`
  instead of a bespoke inline conversion) — zero new logic, matching its 6 sibling handlers exactly.
- AC-SBM-10's completeness gate is deliberately a PROCESS requirement (re-run a named grep, classify
  against a named table), not a hand-enumerated list of ~55 line numbers — DISCUSS's own role is to
  fix the classification RULE and prove it on confirmed representative instances, not to perform
  DESIGN/DELIVER's own exhaustive line-by-line audit (§ Out of Scope explains why).

## Wave: DISCUSS / [REF] Out of Scope

- **Hand-enumerating and classifying all ~55 dynamic-message `Status::internal(...)` sites in this
  DISCUSS** — the true count (§ Investigation Finding 1: 37 in `handler.rs`, up to 9 each in
  `listen_handler.rs` and `embyr-agent/server.rs`) is large enough that a manual, one-by-one DISCUSS
  read of every remaining site would consume disproportionate discovery effort for a fix whose
  CLASSIFICATION RULE is already fully specified (§ Investigation Finding 1's table) and whose
  MECHANISM is already fully specified (US-01/US-02). AC-SBM-10 makes this a hard, checkable gate
  for DESIGN/DELIVER instead of an unverified DISCUSS claim.
- **Cloud-secret-manager (AWS/GCP) fetch-failure message sanitization**
  (`handler.rs:275,293` — `"aws secret fetch failed: {e}"`, `"gcp secret fetch failed: {e}"`) — a
  structurally similar but DIFFERENT vendor/error-domain (AWS/GCP SDK error text, not Postgres/sqlx
  driver text) from the audit's own named finding. Named here as a plausible follow-up candidate,
  not silently ruled out, but not locked into this feature's own narrow scope (task item 2d).
- **Crypto/decoding error sanitization** (`ecies::decrypt` failures, JSON-parse-of-internal-TLS-
  bundle failures, `String::from_utf8` failures on decrypted DSN/bundle bytes) — these describe
  internal DATA-CORRUPTION signals (the server's own previously-stored ciphertext/config failing to
  decode), not raw Postgres/sqlx driver text, and are not client-triggerable via ordinary use;
  different risk category, not evidenced as part of Finding #10.
- **Status-code correctness for RPC handlers that only ever return `Status::internal` today
  regardless of the underlying `CoreError` variant** (a SEPARATE potential correctness gap noticed
  incidentally — e.g. some direct-bypass sites would map a hypothetical future `InvalidArgument`
  from a storage call to `Internal` instead of `InvalidArgument`) — this is a wrong-status-code risk,
  not an information-disclosure risk, and is out of scope for a feature whose task is narrowly
  message-content sanitization (task item 2d's own "narrow fix" instruction).
- **`CoreError::ProjectNotFound`'s own current fallthrough into `embyr-server`'s catch-all** — its
  message is already client-safe (§ Investigation Finding 3), so it is unaffected by this fix by
  construction; whether it deserves its own explicit `Status::not_found` arm (a status-code-
  correctness improvement, not a leak fix) is a separate, unscoped finding, noted but not built here.
- **A general audit of every other `.expect()`/`.unwrap()` or panic-risk site in either binary** —
  this feature is about MESSAGE CONTENT sanitization of already-`Result`-typed error paths, not
  panic elimination (that class of finding is `occ-precondition-validation`'s and
  `firestore-malformed-filter-shape-validation`'s own separate territory).

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy A** (real, minimal, end-to-end) — US-01's own single slice is a real backend failure
(a genuinely unreachable customer Postgres, or a real dropped connection during a chaos-style test),
routed through an ALREADY-correctly-wired `core_error_to_status` call path in EACH binary (e.g. a
`BatchWrite` per-write failure in `embyr-server`; a `Commit`/`Rollback` failure in `embyr-agent`),
proven against a real running server and a real Postgres backend to return a fixed, generic message
while the real error appears in server-side tracing output — not a unit-test-only or mocked proof.

## Wave: DISCUSS / [REF] Driving Ports

gRPC `:8080`/`:8081` (existing routes, zero new RPC) on `embyr-server` — every RPC that reaches
`core_error_to_status`, `authenticate()`, or `handle_listen`. gRPC `:9191` `StorageAgent` (existing
routes, zero new RPC) on `embyr-agent` — every RPC that reaches its own `core_error_to_status`, plus
`get_document` and `subscribe` specifically.

## Wave: DISCUSS / [REF] Pre-requisites

- None. `CoreError::BackendUnavailable`, both `core_error_to_status` implementations, every
  confirmed direct-bypass call site, and the `tracing::error!`/`tracing::warn!` logging convention
  this fix reuses all already exist and are already correct for every other purpose — this is a
  narrow, self-contained message-sanitization fix layered onto existing, working error-conversion
  code.

## Wave: DISCUSS / [REF] Definition of Ready Validation

### DoR Checklist (9-item hard gate)
1. [x] Story traces to a job_id (JOB-11) — reused, not new, with the rejected alternative (JOB-12)
   explicitly reasoned against (§ Persona & Job).
2. [x] Both stories have a complete Elevator Pitch (Before / After / Decision enabled), each naming
   a real user-invocable entry point (real gRPC RPCs on `:8080`/`:8081`/`:9191`).
3. [x] 3+ domain examples per story, with real, concrete data (real personas — Maria Santos, Priya
   Kapoor, Riley Nakamura — and real project/document identifiers, not generic placeholders).
4. [x] UAT scenarios in Given/When/Then (US-01: 5 scenarios; US-02: 5 scenarios — both within the
   3-7 right-sized range).
5. [x] Acceptance criteria derived directly from the UAT scenarios (AC-SBM-01 through AC-SBM-11).
6. [x] Right-sized — see § Scope Assessment for the explicit, non-glossed-over effort judgment: the
   true call-site count is larger than the audit's own citation, but the fix is uniform and
   mechanical, matching this session's own established precedent for "many call sites, one fix
   pattern" features; flagged (not hidden) as a risk DESIGN must re-confirm via AC-SBM-10.
7. [x] Technical notes identify constraints (message wording and logging-call-shape are DESIGN
   choices; the one bare-`sqlx::Error` site's exact routing is a DESIGN choice; both binaries'
   fixes may be shared or independently mirrored).
8. [x] Outcome KPIs have numeric targets (US-01: 2 known leak sites → 0; US-02: 7 confirmed
   direct-bypass sites → 0, plus a completeness gate for the full census) and measurement methods
   (direct AC proof for both).
9. [x] Prior-wave artifacts read and reconciled (audit finding #10, both `core_error_to_status`
   implementations, all 12 `CoreError` variants, `sqlx::Error`'s own documented `Display` shape,
   existing test assertions across 4 files, the existing `tracing::error!`/`warn!` convention, and
   `jobs.yaml` JOB-11/JOB-12 all directly informed this feature's shape).

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Persona/job: **P2 Sam Chen / JOB-11 (`fair-multitenancy`)**, reused — not JOB-12
  (`observability`), which is the job this fix protects the VALUE of (server-side log visibility)
  without being that job's own realization (§ Persona & Job).
- [D2] Fix location: **both existing `core_error_to_status` functions (US-01) AND every confirmed
  direct-bypass site that reproduces the identical inline anti-pattern outside those functions
  (US-02)** — fixing only the two named boundary functions would leave the majority of the real
  leak surface (§ Investigation Finding 1) completely unfixed, since most call sites never route
  through them.
- [D3] Scope is narrowed to exactly one `CoreError` variant, `BackendUnavailable` (§ Investigation
  Finding 3) — every other variant's message is already client-safe and is explicitly unchanged.
- [D4] The task's own "third leak point" (`embyr-agent/src/server.rs:798`) is confirmed currently
  UNREACHABLE (§ Investigation Finding 2) — locked in scope as a defensive fix, but the DISCUSS
  record does not overstate it as a live, currently-exploitable leak.
- [D5] The true dynamic-message call-site count (~55 across 3 files) is materially larger than
  either the audit's own "52+" framing or the orchestrator's own initial "177" framing (both counted
  a DIFFERENT thing — `BackendUnavailable` CREATION sites, correctly left untouched per D3/Finding
  1) — AC-SBM-10 makes full completeness a checkable DESIGN/DELIVER gate rather than an unverified
  DISCUSS claim, given hand-enumerating every line was assessed as disproportionate discovery effort
  for a fully-specified, mechanical fix (§ Out of Scope).
- [D6] Cloud-secret-manager (AWS/GCP) fetch-error sanitization is named as a plausible, structurally
  similar follow-up but explicitly NOT part of this feature's own locked scope (different vendor/
  error-domain than the audit's own Postgres/sqlx-specific finding).

### Requirements Summary
- Primary need: no backend/database failure, in either binary, on any confirmed or yet-to-be-
  classified call path, discloses raw Postgres/sqlx driver text to a client — while the real error
  remains fully observable server-side via `tracing`.
- Walking skeleton scope: US-01 (both boundary functions).
- Full closure scope: US-02 (confirmed direct-bypass sites + a completeness gate for the rest).
- Feature type: Security/correctness hardening fix — Finding #10 (High) from the 2026-09-08
  production-readiness audit, whose own true blast radius this DISCUSS found to be substantially
  larger than its own citation described.

### Constraints Established
- Only `CoreError::BackendUnavailable`'s mapping changes in both `core_error_to_status`
  implementations; all 11 other variants are unchanged.
- Every fix preserves the existing gRPC status CODE; only message content changes.
- The real error is captured via `tracing::error!`/`warn!` (existing convention) before being
  discarded from the client-facing `Status`.
- Zero new `CoreError` variant, zero new port/adapter trait method, zero existing test regression
  (confirmed no test asserts on Internal-code message content — § Investigation Finding 4).

### Upstream Changes
- **`docs/product/jobs.yaml`, JOB-11 entry**: append a dated NOTE (not applied by this DISCUSS —
  flagged for the FINALIZE step, matching this session's own established convention) — "JOB-11 now
  also covers backend/database failures never disclosing raw Postgres/sqlx driver text to a client
  on any RPC in either binary — across both independent `core_error_to_status` implementations AND
  every independent direct-bypass call site that reproduced the identical inline anti-pattern
  outside them (most notably `embyr-server`'s own `authenticate()`, called on every authenticated
  RPC) — while the real error remains server-observable via `tracing`. Same job, same persona (P2
  Sam Chen), not a new job. See
  docs/feature/sanitize-backend-error-messages/feature-delta.md § Investigation Finding 1."

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 6 locked Decisions (D1-D6), a 2-story walking-skeleton-then-
full-closure plan, 11 ACs (AC-SBM-01 through AC-SBM-11) to design executable scenarios against.
DESIGN's own investigation scope: (1) exact generic message wording; (2) exact logging-call shape
(field name, log level, whether to include a request/project identifier alongside the sanitized
error for correlation); (3) whether the two binaries' fixes share a common helper or remain two
independently-mirrored implementations (matching how `core_error_to_status` is already two separate
functions today); (4) how to route the one bare-`sqlx::Error` site (`handler.rs:3556`) — through
`CoreError` first, or sanitized independently at the call site; (5) execute AC-SBM-10's completeness
gate (re-run the named grep census, classify every hit, apply the fix to every TRUE positive) and
report the final, exact site count back into the evolution doc at FINALIZE, since this DISCUSS
explicitly could not (and should not) hand-enumerate all ~55 lines itself.

---

## Wave: DESIGN / [REF] Reading Confirmation

✓ `crates/embyr-server/src/grpc/handler.rs:4148-4162` (`core_error_to_status`) and
`crates/embyr-agent/src/server.rs:86-102` (its own independent copy) re-read at DESIGN depth.
Confirmed both signatures exactly as DISCUSS described — `embyr-server`'s catch-all
`_ => Status::internal(e.to_string())` (line 4160), `embyr-agent`'s explicit
`CoreError::BackendUnavailable(msg) => Status::internal(msg)` (line 94).
✓ `crates/embyr-core/src/error.rs` re-read in full (12 variants) — confirms DISCUSS's Finding 3
per-variant classification exactly; no re-litigation needed.
✓ `crates/embyr-server/src/main.rs:46-65` (`fail_startup`, `bind_or_exit`) and the 3 sweeper
`tracing::warn!(error = %e, "...")` call sites re-read to confirm the EXACT existing convention
this feature's helper must mirror: `tracing::error!(error = %e, "{message}")` at ERROR level for a
failure that aborts/rejects the caller (`fail_startup`, `bind_or_exit` — both process-fatal),
`tracing::warn!(error = %e, "...")` at WARN level for a failure a background loop tolerates and
retries (the 3 sweepers). This feature's failures are per-REQUEST and client-facing (the caller gets
`Status::internal` back), not process-fatal or retried-in-background — closer in shape to
`fail_startup`/`bind_or_exit`'s "this aborts what the caller was trying to do" semantics than to the
sweepers' "this cycle failed, try again next tick" semantics. **Locked: `tracing::error!`, not
`tracing::warn!`.**
✓ `crates/embyr-agent/src/notify_bridge.rs:44-98` (`AgentNotifyBridge::subscribe`) independently
re-read in full (not trusting DISCUSS's own Finding 2 without re-verification, per this session's
established discipline). Confirmed byte-for-byte: the function's only `return`/tail expression
outside the `tokio::spawn`ed task is `Ok(rx)` (line 97); every fallible operation inside the spawned
task (`PgListener::connect_with`, `.listen(...)`, `pg_listener.recv()`) is already handled by
`tracing::error!(...)` + `return`/`break` INSIDE the task, never propagated through the function's
own `Result`. DISCUSS's "currently unreachable, fix defensively anyway" framing for
`server.rs:798` is confirmed correct, independently.
✓ Blast-radius grep independently re-run (not trusted from DISCUSS's own count) per AC-SBM-10's own
completeness-gate instruction: `Status::internal\(` in the three named files. Confirmed EXACT counts:
**59** in `crates/embyr-server/src/grpc/handler.rs`, **9** in
`crates/embyr-server/src/realtime/listen_handler.rs`, **9** in `crates/embyr-agent/src/server.rs` —
**77 total**, matching DISCUSS's own 59+9+9 figure exactly (no discrepancy this time). Every one of
the 77 was individually read with 1-4 lines of surrounding context and traced to its underlying
error-producing expression (not classified from the call-site text alone) — see § DESIGN Decision 3
for the full table. This is the work AC-SBM-10 named as DESIGN/DELIVER's own responsibility, not
DISCUSS's, and it surfaced a materially larger TRUE-positive count than DISCUSS's own confirmed
inventory (see Decision 3).
✓ For every newly-classified `system_db.get_*_access_rule`/`list_*_access_rule_pattern*` call site,
the callee's own signature was read directly in `crates/embyr-server/src/adapters/system_db.rs`
(e.g. `get_access_rule` line 987, `get_write_access_rule` line 1457, `get_group_access_rule` line
1649, `list_access_rule_patterns_by_skeleton` line 1223,
`list_recursive_access_rule_patterns_up_to` line 1282) — every one returns `Result<_, CoreError>`
built directly from a `sqlx::query(...)` call, the identical shape as `get_project_for_auth`
(`authenticate()`'s own already-confirmed leak site). Confirms these are genuine
`CoreError::BackendUnavailable`-class leaks, not assumed from the method name alone.
✓ `embyr_core::access_control::parse_condition`/`parse_path_segments`'s own error type
(`ConditionParseError`, `embyr-core/src/access_control/mod.rs:289-293`) read directly: carries only
`{detail: String, construct: UnsupportedConstruct}` describing the STORED CONDITION TEXT's own
grammar problem (unbalanced parens, unsupported construct) — confirmed it can never carry a
column/constraint/DSN string, closing the question of whether the "stored X failed to re-parse"
family of sites (16 in `handler.rs`, 3 in `listen_handler.rs`) needed sanitizing. They do not.
✓ `listen_handler.rs:81-89` (`collection_id_from_query_target`, `filter_from_query_target`,
`project_id_from_parent`, `ProjectId::new`) read directly: the first three return `Result<_, String>`
describing the CALLER's OWN malformed `QueryTarget`/parent path (e.g. `"invalid parent path:
{parent}"`); `ProjectId::new` (`embyr-core/src/domain/project.rs:8-17`) returns
`CoreError::InvalidArgument` echoing the caller's own supplied project_id. Same safe category
DISCUSS's own Finding 3 already established for `CoreError::InvalidArgument` generally.
✓ `embyr-agent/src/server.rs:698-753`'s three `ProjectId::new(&self.project_id)` sites read in
context: `self.project_id` is the AGENT'S OWN configured project_id (constructor parameter,
`StorageAgentService::new`, line 78), never caller- or database-supplied — a self-configuration
validation, not a backend/database error at all. Confirmed safe by construction.
✓ `listen_handler.rs:3525-3530` and `handler.rs:3658-3663` (`request.get_mut().next().await`) read
in context: for `tonic::Streaming<T>`, `Stream::Item = Result<T, tonic::Status>` — the `e` these two
sites wrap is ALREADY a `tonic::Status` (a transport/decode-level failure), not a raw driver error.
Re-wrapping it in another `Status::internal` is redundant but not a NEW leak; left unchanged.

## Wave: DESIGN / [REF] Decision 1 — Mechanism, Message Text, and Helper Shape

Full reasoning, alternatives, and consequences recorded in
`docs/product/architecture/adr-075-backend-error-message-sanitization-boundary.md` (new ADR).
Summarized here:

- **Generic message**: one fixed string, `"internal server error"`, used at every fixed site with
  zero variation by RPC or context. Ponytail-favored (one simple, reusable string) and, on reflection,
  the SAFER choice too — varying the message by subsystem would itself become a low-bandwidth side
  channel telling a prober which internal code path failed (ADR-075 § Decision 2).
- **Logging shape**: `tracing::error!(error = %e, "{context}")` — mirrors `main.rs`'s
  `fail_startup`/`bind_or_exit` convention exactly (ERROR level, `error = %e` field name), not the
  sweepers' WARN-level convention, because these failures are per-request and client-facing, not
  background-and-retried. No request/project-id correlation field added — considered and deferred
  (ADR-075 § Consequences); the 5 existing convention sites don't carry one either, and it is not
  required by any AC.
- **Helper**: one small function per binary,
  `fn sanitize_backend_error(e: impl std::fmt::Display, context: &'static str) -> Status { tracing::error!(error = %e, "{context}"); Status::internal("internal server error") }`,
  defined in `grpc/handler.rs` (`embyr-server`) and independently, identically, in `server.rs`
  (`embyr-agent`) — two mirrored copies, not one shared crate (ADR-075 § Decision 3: `embyr-core`
  must stay IO-free, and no other shared crate is positioned for cross-binary runtime code; a new
  crate for 3 duplicated lines is unrequested infrastructure). `realtime/listen_handler.rs` imports
  `embyr-server`'s copy exactly the way it already imports `grpc::handler::query_compliance_rejection`
  — reusing an established same-crate cross-module pattern, not inventing a new one.
- **The bare-`sqlx::Error` site** (`handler.rs:3556`): sanitized directly via
  `sanitize_backend_error(e, ...)` — `impl Display` already accepts it uniformly; wrapping it in
  `CoreError::BackendUnavailable` first would add a step that exists only for appearance-of-consistency
  (ADR-075 § Decision 4).
- Both binaries' `core_error_to_status` (US-01) and every direct-bypass site (US-02) call the SAME
  function — there is exactly one mechanism, not two.

### Code shape (illustrative — DELIVER owns exact `context` string wording per call site)

```rust
// crates/embyr-server/src/grpc/handler.rs
pub(crate) fn sanitize_backend_error(e: impl std::fmt::Display, context: &'static str) -> Status {
    tracing::error!(error = %e, "{context}");
    Status::internal("internal server error")
}

pub(crate) fn core_error_to_status(e: CoreError) -> Status {
    match e {
        CoreError::DocumentNotFound(_) => Status::not_found(e.to_string()),
        // ...unchanged arms (AC-SBM-05)...
        _ => sanitize_backend_error(e, "core_error_to_status: unmapped CoreError variant"),
    }
}

// authenticate() — 5 sites (AC-SBM-06), same shape at each:
.map_err(|e| sanitize_backend_error(e, "authenticate: load project row"))?

// handle_listen — the bare-sqlx site + the CoreError site (AC-SBM-07):
.map_err(|e| sanitize_backend_error(e, "handle_listen: provision notify-listener pool"))?
.map_err(|e| sanitize_backend_error(e, "handle_listen: start PostgresNotifyListener"))?
```

```rust
// crates/embyr-agent/src/server.rs — independent, mirrored copy
fn sanitize_backend_error(e: impl std::fmt::Display, context: &'static str) -> Status {
    tracing::error!(error = %e, "{context}");
    Status::internal("internal server error")
}

fn core_error_to_status(e: CoreError) -> Status {
    match e {
        CoreError::BackendUnavailable(msg) => sanitize_backend_error(msg, "core_error_to_status: BackendUnavailable"),
        // ...unchanged arms...
    }
}

// get_document (AC-SBM-08) — pure consistency fix, zero new logic:
match self.storage.get_document(&path, None).await {
    Ok(Some(doc)) => { /* unchanged */ }
    Ok(None) => Err(Status::not_found(format!("document not found: {name}"))),
    Err(e) => Err(core_error_to_status(e)),   // was: Err(Status::internal(format!("{e}")))
}

// subscribe (AC-SBM-09, defensive — confirmed unreachable today):
.map_err(|e| sanitize_backend_error(e, "subscribe: AgentNotifyBridge::subscribe"))?
```

## Wave: DESIGN / [REF] Decision 2 — `authenticate()`/`handle_listen` Sites Confirmed (AC-SBM-06/07)

No change from DISCUSS's own confirmed inventory — all 7 sites confirmed by direct re-reading:
`handler.rs:228,278,296,332,347` (`authenticate()`, all `CoreError::BackendUnavailable` wrapping a
raw `sqlx::Error`), `handler.rs:3556` (bare `sqlx::Error`), `handler.rs:3564`
(`CoreError::BackendUnavailable` wrapping a raw `sqlx::Error`). `embyr-agent/server.rs:310`
(`get_document`, AC-SBM-08) and `:798` (`subscribe`, AC-SBM-09) also confirmed exactly as DISCUSS
described.

## Wave: DESIGN / [REF] Decision 3 — AC-SBM-10 Completeness Gate: Full Per-Site Classification (17 additional genuine leak sites found beyond DISCUSS's own inventory)

Every one of the 77 confirmed `Status::internal(` sites across the three named files was read with
context and classified. **Result: 26 sites need the fix (9 already named by DISCUSS + 17 newly found
in this DESIGN pass), 51 sites are already safe and stay unchanged.** The 17 new sites all share one
identical, previously-unnamed shape: a `system_db.get_*_access_rule`/`list_*_access_rule_pattern*`
call — the SAME `CoreError`-wrapping-raw-`sqlx::Error` pattern already confirmed for
`get_project_for_auth`, just at the access-rule-evaluation call sites inside `GetDocument`,
`RunQuery`, `RunAggregationQuery`, and the write handlers, none of which DISCUSS's own narrower
reading (which focused on `authenticate()`/`handle_listen`) happened to enumerate. This is exactly
the gap AC-SBM-10's completeness gate was designed to catch.

**`crates/embyr-server/src/grpc/handler.rs` — 59 total sites, 21 need fixing, 38 stay unchanged:**

| Sites (line numbers) | Underlying error | Classification | Reasoning |
|---|---|---|---|
| 228,278,296,332,347 | `CoreError::BackendUnavailable` (raw `sqlx::Error`) | **FIX** | `authenticate()` — confirmed by DISCUSS, re-confirmed here |
| 768,812,886,1321,1624,1844,2071,2354,2604,3129,3180,3412,3445 | `CoreError` from `system_db.get_access_rule`/`get_write_access_rule`/`get_group_access_rule`/`list_access_rule_patterns_by_skeleton`/`list_recursive_access_rule_patterns_up_to` — all wrap a raw `sqlx::Error` | **FIX (NEW — 13 sites)** | Identical shape to `get_project_for_auth`; confirmed via direct signature read of every callee (`system_db.rs`) |
| 3556 | Bare `sqlx::Error` (`PgPoolOptions::connect`) | **FIX** | `handle_listen` — confirmed by DISCUSS |
| 3564 | `CoreError::BackendUnavailable` (raw `sqlx::Error`) | **FIX** | `handle_listen` — confirmed by DISCUSS |
| 4160 | `CoreError` catch-all | **FIX** | `core_error_to_status`'s own catch-all — confirmed by DISCUSS |
| 256 | `tokio::task::JoinError` | leave | Never carries driver/schema text |
| 267,271,285,289,303,306,315,320,325,339 | Static string constants | leave | Fixed, developer-authored, never varies per request (ADR-075 names the column-adjacent wording as a smaller, separate, unevidenced follow-up candidate) |
| 275,293 | AWS/GCP secret-fetcher SDK error | leave | Different vendor/error-domain than Finding #10 — DISCUSS's own explicit Out of Scope |
| 308,341 | `ecies::decrypt` validation error | leave | Internal data-corruption signal on previously-stored ciphertext — DISCUSS's own explicit Out of Scope |
| 310,343 | Static "not valid UTF-8" | leave | Fixed string |
| 312 | JSON-parse-of-internal-data (`serde_json::from_str` on decrypted TLS bundle) | leave | Same category as crypto validation — DISCUSS's own explicit Out of Scope |
| 760 | Caller-supplied collection_path parse (`parse_path_segments`) | leave | Echoes the caller's own malformed request back to that caller — safe content, possible wrong-status-code issue is out of scope |
| 775,831,893,1382,1467,1631,1729,1851,1954,2078,2172,2368,2635,3143,3421,3451 | `ConditionParseError` (stored rule re-parse) | leave | Confirmed: carries only `{detail, construct}` describing the STORED grammar text, never schema/driver info |
| 3530,3663 | `tonic::Status` (stream decode error, already a `Status`) | leave | Re-wrapping an existing `Status` is redundant, not a new leak |

**`crates/embyr-server/src/realtime/listen_handler.rs` — 9 total sites, 2 need fixing, 7 stay unchanged:**

| Sites | Underlying error | Classification | Reasoning |
|---|---|---|---|
| 116 | `CoreError` from `system_db.get_access_rule` (raw `sqlx::Error`) | **FIX (NEW)** | Same shape as the `handler.rs` access-rule family |
| 245 | `CoreError` from `adapter.run_query` (raw `sqlx::Error`) | **FIX (NEW)** | Real backend query, same shape |
| 81,82,83 | `Result<_, String>` (caller's own malformed `QueryTarget`) | leave | Caller-input echo, safe content |
| 89 | `CoreError::InvalidArgument` (`ProjectId::new` on caller-supplied project_id) | leave | Caller-input echo, safe content |
| 182,193,526 | `ConditionParseError` (stored rule re-parse) | leave | Same as `handler.rs`'s re-parse family |
| 259,272,287 | Static "channel closed" | leave | Fixed string, internal `mpsc` state |

**`crates/embyr-agent/src/server.rs` — 9 total sites, 3 need fixing, 6 stay unchanged:**

| Sites | Underlying error | Classification | Reasoning |
|---|---|---|---|
| 94 | `CoreError::BackendUnavailable` | **FIX** | `core_error_to_status`'s own explicit arm — confirmed by DISCUSS |
| 310 | `CoreError` from `self.storage.get_document` | **FIX** | `get_document` — pure consistency fix (route via `core_error_to_status`, matching 6 siblings) — confirmed by DISCUSS |
| 798 | `Box<dyn Error>` from `AgentNotifyBridge::subscribe` | **FIX (defensive)** | Confirmed currently unreachable (independently re-verified above); fixed anyway at zero marginal cost |
| 366,428 | Static "document not found after creation/update" | leave | Fixed string; arguably wrong status code, out of scope |
| 699,712,753 | `CoreError::InvalidArgument` (`ProjectId::new` on the AGENT'S OWN configured project_id) | leave | Self-configuration validation, not caller- or database-derived |
| 772 | `SystemTime::duration_since` clock error | leave | Cannot carry driver/schema text under any input |

**DELIVER worklist**: exactly these 26 sites, all using the identical `sanitize_backend_error(e,
context)` call-shape substitution (or, for `server.rs:310`, the identical `core_error_to_status(e)`
consistency substitution) — no per-site design decision remains open.

## Wave: DESIGN / [REF] Regression Guards Carried Forward

- DISCUSS's Investigation Finding 4 (no existing test asserts on `Status::internal`'s own message
  content) was a workspace-wide grep across `tests/`, not scoped to the 9 originally-named sites —
  generalizes to all 26 fixed sites without re-verification.
- AC-SBM-04/05/11 (status CODE unaffected, other 11 `CoreError` variants byte-for-byte unchanged,
  successful auth/non-`BackendUnavailable` paths unaffected) require zero new work — no fixed site
  changes which match arm fires or what status code it maps to, only what `Status::internal`'s own
  message text is.
- New DISTILL-authored scenarios must assert the ABSENCE of driver-specific substrings (column-decode
  phrasing, `"constraint"`, connection host/port patterns, DSN fragments) in the `Status::internal`
  message for each of the 3 confirmed real-failure entry points named in DISCUSS's own UAT scenarios
  (BatchWrite against an unreachable backend, agent-mode Commit against an unreachable local Postgres,
  a column-decode failure during authentication) — plus at least one NEW scenario exercising one of
  the 17 newly found access-rule-lookup sites (e.g. `RunQuery` against a collection with an access
  rule row present but the backend unreachable), since none of DISCUSS's own UAT scenarios happened to
  cover that family.

## Wave: DESIGN / Handoff Package

**Files requiring a change:**
1. `crates/embyr-server/src/grpc/handler.rs` — add `sanitize_backend_error`; swap 21 call sites
   (§ Decision 3 table); `core_error_to_status`'s catch-all (line 4160) calls the helper.
2. `crates/embyr-server/src/realtime/listen_handler.rs` — import `sanitize_backend_error` from
   `grpc::handler`; swap 2 call sites (116, 245).
3. `crates/embyr-agent/src/server.rs` — add its own mirrored `sanitize_backend_error`; swap
   `core_error_to_status`'s `BackendUnavailable` arm (94); route `get_document` (310) through
   `core_error_to_status`; swap `subscribe` (798).

**Files confirmed to need NO change:**
- `crates/embyr-core/*` — zero new `CoreError` variant, zero new port/adapter trait method
  (System Constraints, unchanged from DISCUSS).
- `crates/embyr-pg-storage/*`, `crates/embyr-server/src/adapters/system_db.rs` — every `CoreError`
  construction site (175+) stays exactly as-is; the fix is entirely at `Status`-construction time
  (DISCUSS Investigation Finding 1, unchanged).
- `docs/product/jobs.yaml` — the JOB-11 extension note DISCUSS drafted is applied at FINALIZE, not
  DESIGN, per this session's established convention.

**Documentation changes made this wave:**
- `docs/product/architecture/adr-075-backend-error-message-sanitization-boundary.md` (new).

**Regression guards DISTILL/DELIVER must run:** the 4 existing files asserting `.code() ==
tonic::Code::Internal` (DISCUSS Investigation Finding 4: `tests/acceptance/us_01_configure_sdk.rs`,
`tests/production_readiness/acceptance/pr09_wire_secret_fetchers.rs`,
`tests/firestore_list_rpcs/acceptance/ld02_list_collection_ids.rs`,
`tests/agent_mode_write_streaming/acceptance/aw01_single_write_stream.rs`), plus
`tests/acceptance/us_12_agent_backend.rs:1768`'s `CoreError::BackendUnavailable` variant-match
assertion. Full workspace `cargo test` once, at the pre-commit gate, per this repo's own root
`CLAUDE.md` test-run token-discipline rule.

**New test scenarios DISTILL must design:** UAT scenarios already specified in US-01/US-02 (5+5,
feature-delta.md above) for the 9 DISCUSS-named sites, plus one new scenario for the access-rule-
lookup family (§ Regression Guards Carried Forward) proving `RunQuery`/`GetDocument` against a
collection with an access rule configured, backend unreachable, returns the fixed generic message
with zero driver-specific substrings.

---

## Wave: DISTILL / [REF] Scenario list with tags

| # | File | Test | Tags | Proves |
|---|---|---|---|---|
| 1 | `sbm01_walking_skeleton_authenticate_unreachable_backend.rs` | `authenticate_against_unreachable_direct_pg_backend_never_discloses_driver_text` | `@walking_skeleton @driving_port @real-io @AC-SBM-01 @AC-SBM-03 @AC-SBM-06` | `authenticate()`'s direct_pg branch (handler.rs:347) sanitized + real error server-observable via tracing |
| 2 | same file | `authenticate_against_healthy_direct_pg_backend_still_succeeds` | `@real-io @AC-SBM-11` | Regression: healthy backend auth unaffected |
| 3 | `sbm02_completeness_gate_access_rule_family.rs` | `get_document_access_rule_routing_lookup_sanitized_when_system_db_unreachable` | `@driving_port @real-io @AC-SBM-10` | Read-rule routing lookup (handler.rs:768, one of DESIGN's 13 newly-classified sites) sanitized |
| 4 | same file | `create_document_write_rule_lookup_sanitized_when_system_db_unreachable` | `@driving_port @real-io @AC-SBM-10` | Write-rule lookup (handler.rs:1624, a genuinely different newly-classified site) sanitized |
| 5 | `sbm03_regression_guards.rs` | `document_not_found_message_unchanged_by_this_feature` | `@error @AC-SBM-05 @AC-SBM-11` | `DocumentNotFound` byte-for-byte unchanged |
| 6 | same file | `wrong_api_key_rejection_unchanged_by_this_feature` | `@error @AC-SBM-05 @AC-SBM-11` | `Unauthenticated` rejection unchanged |
| 7 | `sbm04_listen_unreachable_backend.rs` | `listen_against_backend_that_becomes_unreachable_never_discloses_driver_text` | `@driving_port @real-io @AC-SBM-07` | `handle_listen`'s bare-`sqlx::Error` site (handler.rs:3556) + its `CoreError`-wrapped sibling (handler.rs:3564) sanitized |
| 8 | `tests/acceptance/embyr_agent/us_a08_error_sanitization.rs` | `get_document_never_discloses_driver_text_when_local_backend_unreachable` | `@driving_port @real-io @AC-SBM-02 @AC-SBM-08` | `embyr-agent`'s `core_error_to_status` `BackendUnavailable` arm (server.rs:94) + `get_document`'s consistency fix (server.rs:310) sanitized |

`AC-SBM-09` (`subscribe`, server.rs:798) is confirmed unreachable dead code (DISCUSS Finding 2 /
DESIGN independently re-verified) — no driving-port test can exercise it today; DELIVER applies the
one-line defensive fix without an accompanying acceptance test, per its own already-recorded
unreachability.

## Wave: DISTILL / [REF] Adapter coverage

No new driven adapter — this feature is a message-content change at an existing conversion
boundary. Existing adapters exercised with real I/O: System Postgres (testcontainers, both
`sys_pool.close()` and natural unreachability), Customer Postgres (testcontainers, unreachable DSN
and `ContainerAsync::stop()`), gRPC data port (:8080, in-process tonic client), StorageAgent mTLS
port (:9191, in-process tonic client via `agent_common::start_test_agent`).

## Wave: DISTILL / [REF] Scaffolds

None. This feature touches only existing, already-implemented production code paths
(`core_error_to_status`, `authenticate()`, `handle_listen`, `embyr-agent`'s `get_document`) — the
fix is the introduction of `sanitize_backend_error` plus a call-site substitution, DELIVER's own
work per Scope. No new production module needed scaffolding for these tests to compile; all RED
failures are `MISSING_FUNCTIONALITY` (assertion trips on real, empirically-observed raw driver
text), never `ImportError`/`BROKEN`.

## Wave: DISTILL / [REF] Test placement

`tests/sanitize_backend_error_messages/{acceptance,common}/` — new feature directory, following
the established per-feature convention (`tests/client_auth/`, `tests/security_rules/`, etc.),
registered as four new `[[test]]` entries in `crates/embyr-server/Cargo.toml`. The `embyr-agent`
scenario is placed inside the EXISTING `tests/acceptance/embyr_agent/` module tree (new file
`us_a08_error_sanitization.rs`, registered in `tests/acceptance/embyr_agent.rs`) rather than a
parallel directory, since that tree already owns the mTLS `StorageAgentService` test harness
(`agent_common::start_test_agent`) this scenario needs — reusing it, not duplicating it (ponytail).

## Wave: DISTILL / [REF] Driving Adapter coverage

Zero new CLI/endpoint/hook — this feature adds no new RPC. All 8 scenarios enter through EXISTING
gRPC routes (`GetDocument`, `CreateDocument`, `Listen` on `embyr-server` :8080; `GetDocument` on
`embyr-agent`'s `StorageAgent` :9191), exactly matching § Driving Ports (DISCUSS).

## Wave: DISTILL / [REF] Pre-requisites

None beyond what DISCUSS/DESIGN already recorded. No Project Infrastructure Policy row was added —
the gRPC data port and System/Customer Postgres rows already cover this feature's needs unchanged.

## Wave: DISTILL / [REF] RED-state confirmation

All 5 non-regression scenarios run against the CURRENT (unfixed) code and fail on the message-content
assertion (`MISSING_FUNCTIONALITY`, correct RED — never `ImportError`/`FIXTURE_BROKEN`), with the
real raw driver text captured empirically:

| Scenario | Actual message today |
|---|---|
| SBM-01 (`authenticate()`/direct_pg) | `"backend unavailable: pool timed out while waiting for an open connection"` |
| SBM-02 read-rule (`resolve_access_rule_pattern`) | `"backend unavailable: get_access_rule failed: attempted to acquire a connection on a closed pool"` |
| SBM-02 write-rule (`evaluate_write_rule_for_commit`) | `"backend unavailable: get_write_access_rule failed: attempted to acquire a connection on a closed pool"` |
| SBM-04 (`handle_listen`, bare `sqlx::Error`) | `"notify listener pool: pool timed out while waiting for an open connection"` |
| `embyr-agent` `get_document` | `"backend unavailable: attempted to acquire a connection on a closed pool"` |

The 3 regression scenarios (healthy-backend auth, `DocumentNotFound`, wrong-API-key) all PASS today,
unchanged, as expected. Full command output: `docs/feature/sanitize-backend-error-messages/` (not
persisted — see chat transcript / PR description for the `cargo test` invocations and result lines).
