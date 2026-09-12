# Feature Delta: agent-field-path-validation

## Wave: DISCUSS / [REF] Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` — read in full. Finding #11 confirmed
verbatim: *"`embyr-agent`'s own field-path validator only rejects consecutive dots — doesn't call
`embyr_core::validate_field_path` (the real charset guard the server uses). Feeds the same
raw-interpolating SQL builder. SQL injection reachable via a client-cert holder into the *customer's
own* Postgres, defeating the agent's entire reason to exist (vendor cannot read customer data).
mTLS-gated, so not remote-unauthenticated."* Category: Security. Severity: **High**. Location cited:
`crates/embyr-agent/src/server.rs:117-126,149`; sink at `crates/embyr-pg-storage/src/encoding/
query.rs:326` and siblings. Status before this DISCUSS: "Not started."

✓ `crates/embyr-agent/src/server.rs` read in full. Confirmed the exact weak validator (lines
128-138):
```rust
fn validate_field_path(path: &str) -> Result<(), Status> {
    if path.contains("..") {
        Err(Status::invalid_argument(format!(
            "invalid field path '{}': consecutive dots not allowed", path
        )))
    } else {
        Ok(())
    }
}
```
Any character other than consecutive dots — single quotes, semicolons, whitespace, parens,
backslashes, `=` — passes through unvalidated. Confirmed exactly **one call site** in the entire
crate (`server.rs:161`, inside `proto_filter_to_domain`, itself called from both `run_query` and
`run_aggregation_query`) — see § Investigation Finding 1 for the full blast-radius trace.

✓ `crates/embyr-core/src/domain/query.rs` read in full. Confirmed the real guard:
`pub fn validate_field_path(path: &str) -> Result<(), CoreError>`, backed by
`is_valid_field_path`, enforcing `^[a-zA-Z_][a-zA-Z0-9_.]*$` exactly as `docs/SPEC.md`'s own
"Field Paths" section documents (`docs/SPEC.md:538`: *"Field path segments match
`^[a-zA-Z_][a-zA-Z0-9_.]*$`. Invalid field paths are rejected with `InvalidArgument`."*) — this is
not merely "the server's own stricter choice," it is the literal spec-mandated shape. The file's own
doc comment names it "the single root-cause guard... the only thing standing between a crafted field
path and SQL injection." Already has its own proptest + named-payload unit test suite (4 tests,
including the literal payloads `"x'); DROP TABLE documents; --"` and `"x' OR '1'='1"` from a prior
security brief).

✓ `crates/embyr-pg-storage/src/encoding/query.rs` read in full. Confirmed the sink: `field_path` is
raw-`format!`-interpolated (never `push_bind`) into SQL text at 7 sites in `append_field_filter`/
`push_scalar_comparison`/`push_array_contains`/`push_value_equality` (e.g. line 326:
`qb.push(format!("fields->'{field_path}' {op} "))`), plus 2 more in `backend_adapter.rs`'s own
`order_by_expr`/cursor-comparison blocks. Every one of these functions is backend-agnostic — shared,
unmodified, by both `embyr-server` and `embyr-agent` via the same `PostgresBackendAdapter`. Values
are always `push_bind`-parameterized and never at risk; only the field-path STRING itself is
vulnerable, exactly as the audit states.

✓ `crates/embyr-agent/Cargo.toml` read in full. Confirmed `embyr-core.workspace = true` is already a
direct dependency (line 16) — this session's own prior work on finding #9
(`occ-precondition-validation`) already established this; re-confirmed directly here, not assumed
from memory.

✓ `crates/embyr-pg-storage/src/backend_adapter.rs` (`create_document`/`update_document`/`commit`
write paths, lines 319-520+, 1234-1302) read for the write-path SQL shape. Confirmed: document
field values are serialized to a single JSON blob (`fields_to_json`) and bound as one
`$N::jsonb` parameter (e.g. line 331: `VALUES ($1, $2, $3, $4::jsonb, ...)`, `.bind(&fields_json)`)
— field NAMES supplied on a write never reach raw SQL text at all; they become ordinary JSON object
keys inside a bound JSONB value. This directly narrows the confirmed injection surface: **writes are
not vulnerable regardless of field-path content** — only the query/filter path (`RunQuery`/
`RunAggregationQuery`'s `FieldFilter.field_path`, raw-interpolated into `fields->'{field_path}'`) is,
exactly matching the audit's own sink citation (`query.rs:326`) and confirming task framing item 2's
own "write or query path, whichever is more reachable" question has a definitive answer: **query
path only** — see § Investigation Finding 1.

✓ `docs/product/architecture/adr-001-process-topology.md` read (relevant sections). Confirmed:
*"A separately-deployed customer binary (`embyr-agent`) is required for `backend_mode=agent` to
satisfy Riley's credential isolation requirement (US-12): DB credentials must never cross the VPC
boundary to embyr SaaS... Riley's requirement (US-12, JOB-04) is that DB credentials never cross the
network to embyr SaaS... The agent binary's security model depends on physical process separation."*
This directly ties US-12 (the "agent's entire reason to exist" framing the audit itself echoes) to
**JOB-04**, not JOB-05 or JOB-11 — see § Persona & Job.

✓ `docs/product/jobs.yaml` read in full (JOB-01 through JOB-16+). Confirmed JOB-04
(`credential-isolation`, P4) is the job ADR-001 itself names for the agent's own core purpose; JOB-09
(`agent-auditproof`, P4) is about auditor-facing evidence of zero credential egress specifically;
JOB-11 (`fair-multitenancy`, P2 Sam Chen) is about per-project rate fairness across a shared,
multi-tenant embyr SaaS deployment. See § Persona & Job for why JOB-04 is the correct fit and the
other two are rejected.

✓ Grepped every `fn validate_`/`fn.*sanitize`/`fn.*escape` and every `format!(` call across all 8
source files in `crates/embyr-agent/src/` (`server.rs`, `probe.rs`, `main.rs`, `encoding.rs`,
`notify_bridge.rs`, `lib.rs`, `config.rs`, `sweeper.rs`) — see § Investigation Finding 2 for the
full-crate answer to task item 4.

## Wave: DISCUSS / [REF] Investigation Findings

### Finding 1 — The confirmed blast radius is exactly ONE call site, serving TWO RPCs, and the write path is provably not at risk at all

`validate_field_path` (`server.rs:129`) has exactly one caller in the entire crate:
`proto_filter_to_domain` (`server.rs:161`), which converts a proto `Filter` (`FieldFilter` or
recursively-nested `CompositeFilter`) into a domain `QueryFilter`. `proto_filter_to_domain` itself
has exactly two callers:

| Caller | RPC | Reachability |
|---|---|---|
| `run_query` (`server.rs:494`: `sq.filter.map(proto_filter_to_domain).transpose()?`) | `RunQuery` | Every `db.collection(...).where(...)` / `onSnapshot` query with a filter, forwarded by `embyr-server` on behalf of any real client, for any `backend_mode=agent` project |
| `run_aggregation_query` (`server.rs:569`) | `RunAggregationQuery` | Every `.count()` (the only aggregation kind `embyr-agent` implements — Sum/Avg are hardcoded absent, per `jobs.yaml` JOB-01's own agent-mode-aggregation deferral note) with a filter |

Both RPCs share the identical vulnerable path: `proto_filter_to_domain` → `validate_field_path`
(weak) → `QueryFilter::Field(FieldFilter { field_path, .. })` → `PostgresBackendAdapter::run_query`
→ `append_filter`/`append_field_filter` (`embyr-pg-storage/src/encoding/query.rs`) → raw
`format!("fields->'{field_path}' ...")`.

**Write paths are NOT part of the blast radius.** `CreateDocument`, `UpdateDocument`, and `Commit`'s
`Transform`/`Update` writes never call `validate_field_path` at all — not even the weak version —
because they don't need to: field names supplied on a write become JSON object keys inside a single
`$N::jsonb`-bound parameter (confirmed directly in `backend_adapter.rs`, § Reading Confirmation),
never raw-interpolated SQL text. `order_by` is also not part of the blast radius specifically for
`embyr-agent`: both `run_query` and `run_aggregation_query` hardcode `order_by: vec![]` on the
domain `StructuredQuery` regardless of what the proto request contains (confirmed by direct
reading — `server.rs:510`, `:581`) — `embyr-agent` silently ignores any `orderBy` clause today (a
separate, unscoped correctness gap, not a security one, noted but not fixed here).

**Conclusion**: this is a single, narrow, mechanical fix. Swap the ONE call site
(`proto_filter_to_domain`'s call to the local `validate_field_path`) to delegate to
`embyr_core::domain::query::validate_field_path`, or delete the local function and directly call the
core one with a thin `CoreError -> Status` conversion wrapper (the exact shape — inline `.map_err`
vs. a renamed local wrapper function — is a DESIGN choice, not decided here). Zero other production
code path is touched.

### Finding 2 — No other independently-duplicated validator or raw-SQL-building function exists in `embyr-agent`; this is an isolated incident, confirmed by exhaustive grep, not by absence-of-evidence alone

Per task item 4, grepped `fn validate_`, `fn.*sanitize`, `fn.*escape` across all 8 `.rs` files in
`crates/embyr-agent/src/` (the crate's own complete source tree, confirmed via `Glob`): the only two
hits are `validate_field_path` (this finding's own subject) and `sanitize_backend_error`
(`server.rs:91`) — already fixed correctly in the prior `sanitize-backend-error-messages` feature
(FINALIZED 2026-09-12, confirmed in `docs/product/production-readiness-audit-2026-09-08.md`'s own
row #10). A further grep for every `format!(` call site across the same 8 files found zero SQL-shaped
string building (`SELECT`/`INSERT`/`UPDATE`/`WHERE`/`->'`) anywhere outside `server.rs`'s own
`validate_field_path`/`proto_filter_to_domain` — every other `format!(` call in the crate builds an
error message, a page token, or a resource-name string, none of them destined for raw SQL
interpolation (all real SQL building for both binaries lives exclusively in the shared,
already-safe `embyr-pg-storage` query builder). **Verdict: isolated incident, not a pattern** — no
further instance to name for DESIGN.

A related but explicitly DIFFERENT and already-tracked finding exists at the architecture level:
audit finding #26 (Medium, not started) already names the broader defense-in-depth gap this finding
is a live instance of — *"Server-side field-path SQL interpolation... injection-safe only because
every entry point is funneled through one central `validate_field_path` gate — zero
defense-in-depth... Directly demonstrated as a real gap by #11 (the agent's own divergent, weaker
copy)."* This feature closes #11's own concrete instance; #26's own broader defense-in-depth
architectural question (e.g., should `append_field_filter` itself re-validate defensively, rather
than trusting every caller) is out of scope here — see § Out of Scope.

### Finding 3 — The stricter charset does not reject any real, legitimate field-path shape; the one behavioral delta it introduces is a convergence toward spec-correctness, not a new gap

`docs/SPEC.md:538` documents the field-path charset as spec-mandated policy, not merely
`embyr-server`'s own internal stricter choice: *"Field path segments match
`^[a-zA-Z_][a-zA-Z0-9_.]*$`. Invalid field paths are rejected with `InvalidArgument`."* This is the
exact same validator `embyr-server`'s own `translate_filter` has used, unchanged, across every
finalized query-related feature this session (`firestore-query-filter-operator-support`,
`firestore-or-filter-support`, `composite-index-requirement-rules`, `firestore-is-null-filter-
support`, and others) — every one of those features' own acceptance tests uses ordinary field names
(`"status"`, `"category"`, `"age"`, `"tags"`, `"createdAt"`, `"address.city"`-shaped dotted paths)
and none has ever failed against this exact charset. Since a `backend_mode=agent` project and a
`backend_mode=direct_pg` project both serve the identical Firestore wire protocol to the identical
SDK, any field-path shape a real SDK can produce for one must already be proven to pass the other's
identical validator — there is no SDK-producible shape unique to agent-mode.

One genuine, narrow behavioral delta was found and is explicitly resolved, not glossed over: the
old, weak `embyr-agent`-local validator rejected consecutive dots (`"a..b"`) as its ONLY check; core
`is_valid_field_path`'s charset regex does **not** independently forbid consecutive dots — a string
like `"a..b"` consists entirely of characters in the allowed class (`[a-zA-Z0-9_.]`) and would pass
`validate_field_path` under the new, stricter-on-CHARSET-but-not-on-SEQUENCE guard. Confirmed
directly against the core file's own proptest (`spec_compliant_paths_always_accepted`, which samples
`rest` from `"[a-zA-Z0-9_.]{0,20}"` — a generator that can and does produce consecutive dots — and
asserts `is_ok()` for all such strings). This is not a new permissiveness gap introduced by this
feature: it is `embyr-server`'s own EXISTING, already-live, already-spec-conformant behavior today
(the core validator embyr-server already uses has never forbidden consecutive dots) — adopting it in
`embyr-agent` makes the two binaries' behavior IDENTICAL and spec-conformant, rather than
introducing new agent-specific permissiveness. `"a..b"` itself carries no SQL-injection risk (it
stays entirely within the safe charset); at worst it addresses a JSONB key that structurally cannot
match any real stored field (a query that correctly returns zero rows, not a security or correctness
regression). **Resolution: safe to adopt as-is; no additional consecutive-dot check needs to be
preserved or reintroduced.**

### Finding 4 — Zero existing test in `embyr-agent`'s own acceptance suite asserts on the OLD weak validator's specific consecutive-dot message text, so swapping validators breaks no existing test

Grepped `tests/` for `consecutive dots` and for direct calls to `embyr-agent`'s own
`validate_field_path` — no acceptance test references the OLD validator's specific rejection
message or its narrower consecutive-dots-only behavior. `tests/acceptance/us_12_agent_backend.rs`
(the agent's own primary acceptance suite) exercises `RunQuery`/filters with ordinary, spec-compliant
field paths only (e.g. `field_path: "owner_uid".to_string()` at line 1343) — none construct a
malformed or consecutive-dot field path today. This confirms swapping the validator introduces zero
test regressions and that this feature's own UAT scenarios (below) are the FIRST test coverage of
malformed field-path rejection on the agent's own gRPC surface.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** — a security-hardening fix closing an SQL-injection gap at the
  customer-VPC agent's own query-filter validation boundary.
- JTBD: reuse an existing job — see § Persona & Job for the resolved choice (JOB-04, not JOB-05,
  JOB-09, or JOB-11).
- Walking Skeleton: **Yes** — a real SQL-injection-probe field path submitted via a real `RunQuery`
  call against a real running `embyr-agent` and a real Postgres backend, proven rejected with a
  clean `INVALID_ARGUMENT` rather than reaching the SQL builder.
- UX Research Depth: **Lightweight** — a narrow validator-swap hardening fix on an existing,
  already-correctly-wired RPC surface; no new emotional arc, no new journey artifact (mirrors
  `occ-precondition-validation`, `firestore-malformed-filter-shape-validation`, and
  `sanitize-backend-error-messages` precedent for this class of finding).

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P4 Riley Nakamura (DevSecOps Lead)** — the operator who deploys `embyr-agent` inside
a customer's own VPC specifically because their security policy requires it (mirrors JOB-04's and
JOB-09's own established persona for every `embyr-agent`-touching feature this session). Secondary:
**P1 Alex** as the unwitting beneficiary — Alex's own well-formed queries are completely unaffected;
Alex never needs to know the agent's own internal validator changed.

**Job**: **JOB-04 `credential-isolation`**, reused, EXTENDED to also cover: the agent's own
query-filter validation must enforce the identical spec-mandated field-path charset
`embyr-server` already enforces, so that no caller — not even one holding a valid client
certificate for that deployment — can use a malformed field path to reach past the agent's own
query builder and execute arbitrary SQL against the customer's own Postgres. This is the sharpest
possible violation of JOB-04's own founding promise: JOB-04 exists specifically so that the
customer's own database is never exposed to more than the Firestore protocol's own intended
semantics permit, from ANY party — not only "embyr SaaS never receives the DSN" (the credential-egress
half JOB-04's `functional` dimension already names) but also "the agent itself never becomes a
confused-deputy SQL-injection gateway into that same protected database" (the data-integrity half
this finding closes). ADR-001 itself ties this exact "agent's entire reason to exist" framing
directly to JOB-04 via US-12 (§ Reading Confirmation) — the audit's own chosen wording for this
finding is not a coincidence; it is the same wording ADR-001 already uses for JOB-04's own founding
rationale.

**Candidates considered and rejected**:
- **JOB-05 (`cloud-secret`, P3)** — about AWS/GCP secret-manager credential *fetching*, an entirely
  different mechanism (`backend_mode` in `{aws_secret, gcp_secret}`) than `backend_mode=agent`'s own
  separate-binary model. No functional overlap with this finding at all.
- **JOB-09 (`agent-auditproof`, P4)** — about producing auditor-facing EVIDENCE that zero
  credentials egress (network logs, system-DB rows, mTLS handshake logs). This finding is about
  preventing a data-integrity/confidentiality violation from happening in the first place, not about
  proving after the fact that credentials didn't leave — a different dimension of the same overall
  trust boundary. Rejected for the same reason `sanitize-backend-error-messages` rejected JOB-12: the
  job whose own founding dimension is DIRECTLY realized by this fix is JOB-04's ("DB credentials
  never leave the VPC... embyr SaaS never receives the DSN," the data-protection promise the SQL
  injection would defeat), not JOB-09's own after-the-fact evidence-production dimension.
- **JOB-11 (`fair-multitenancy`, P2 Sam Chen)** — used for the last three `embyr-agent`-touching
  security findings this session (#7, #9's cross-binary half, #10). Rejected here specifically: JOB-11
  is about per-project RATE fairness across MULTIPLE tenants sharing ONE embyr SaaS deployment
  (`RESOURCE_EXHAUSTED` when a noisy tenant exceeds its share). `backend_mode=agent`'s own attack
  surface is fundamentally SINGLE-tenant — one customer's own agent, one customer's own Postgres; the
  threat here is not cross-tenant fairness, it's a confused-deputy SQL-injection risk INSIDE that one
  customer's own trust boundary. JOB-04, not JOB-11, is the job whose dimension text this finding
  actually realizes.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — one function, one
call site, two RPCs sharing that one call site, confirmed exhaustively (§ Investigation Finding 1).
Walking skeleton >5 integration points? No (1: a real `RunQuery` call against a real running agent
and Postgres, proven end-to-end). Estimated effort >2 weeks? No — this is a delete-one-function,
call-one-existing-tested-function change with zero new logic. Multiple independent user outcomes?
No — one outcome ("the agent's own query-filter validator rejects exactly what `embyr-server`'s own
validator rejects, closing the SQL-injection gap, with zero regression for legitimate field
paths"), demonstrable in a single session.

**Verdict: PASS.** This is one of the smallest, most mechanical fixes in this session's own
production-readiness backlog — reuse an already-correct, already-tested function; delete a weaker
duplicate; zero new `CoreError` variant, zero new port/adapter method, zero new proto field.

## Wave: DISCUSS / [REF] System Constraints

- `embyr-core` remains IO-free — this fix only changes which validator function `embyr-agent`'s own
  `proto_filter_to_domain` calls; `embyr_core::domain::query::validate_field_path`'s own definition,
  already IO-free and already used by `embyr-server`, is completely unchanged.
- Zero new `CoreError` variant, zero new proto field, zero new port/adapter trait method — this is a
  call-site substitution plus an error-type conversion (`CoreError::InvalidArgument` →
  `Status::invalid_argument`), a pattern this codebase already uses at dozens of other sites
  (`core_error_to_status`, `embyr-server`'s own `translate_filter`).
- The gRPC status CODE for a malformed field path is unchanged: `Status::invalid_argument` both
  before and after this fix — only the SET of rejected inputs widens (from "contains `..`" only, to
  the full spec-mandated charset).
- `embyr-agent`'s own domain `StructuredQuery.order_by` is hardcoded empty regardless of proto
  content (§ Investigation Finding 1) — `order_by`-based injection is not reachable via
  `embyr-agent` today and is explicitly out of scope for this feature (a separate, unscoped
  correctness gap: `embyr-agent` silently ignores `orderBy` entirely).
- Write paths (`CreateDocument`, `UpdateDocument`, `Commit`) are confirmed not part of this fix's
  scope — field names on a write are never raw-SQL-interpolated (§ Investigation Finding 1).
- Exact wiring shape (delete-and-call-directly vs. a renamed local wrapper performing the
  `CoreError -> Status` conversion) is a DESIGN choice; DISCUSS only requires that
  `embyr_core::domain::query::validate_field_path` becomes the sole source of truth for this check
  in `embyr-agent`, mirroring `embyr-server`'s own existing pattern exactly.

## Wave: DISCUSS / [REF] User Stories

### US-01: embyr-agent's Own Query-Filter Field-Path Validator Enforces the Same Spec-Mandated Charset embyr-server Already Does

**job_id**: JOB-04 | **Release**: 1 (Walking Skeleton) | **Persona**: P4 Riley Nakamura

#### Elevator Pitch
**Before**: Riley Nakamura deploys `embyr-agent` inside Meridian Health's own VPC specifically so
that neither Meridian Health's database credentials NOR their raw patient data ever need to be
trusted to embyr SaaS's own infrastructure. But `embyr-agent`'s own `RunQuery`/`RunAggregationQuery`
handlers validate an incoming filter's `field_path` with a check that only rejects consecutive dots
— a single quote, semicolon, or SQL operator sails through untouched, straight into
`embyr-pg-storage`'s own raw-interpolating `fields->'{field_path}'` SQL builder (the same builder
`embyr-server` uses safely, because IT calls the real charset guard first). Anyone who holds a valid
mTLS client certificate for that deployment — ordinarily `embyr-server` itself relaying a real
client's request, but also a compromised internal service or a bug in the customer's own
cert-issuance process — can submit a crafted `field_path` and execute arbitrary SQL against
Meridian Health's own Postgres.
**After**: the identical crafted `field_path`, submitted via a real `RunQuery` call against
`embyr-agent`'s own `:9191` gRPC surface, is rejected with a clean `INVALID_ARGUMENT` before it ever
reaches the SQL builder — the exact same rejection `embyr-server` already gives for the identical
input on a `backend_mode=direct_pg` project, because both binaries now call the identical,
already-tested `embyr_core::domain::query::validate_field_path`.
**Decision enabled**: Riley Nakamura can tell Meridian Health's own security team that
`backend_mode=agent` deployments enforce the identical field-path security guarantee
`backend_mode=direct_pg` deployments already do — closing the one confirmed asymmetry between the
two backend modes' own query-validation boundary — without needing a compensating control or a
noted exception in their own audit evidence.

#### Who
- Riley Nakamura (P4) | DevSecOps Lead who deploys and operates `embyr-agent` inside a customer VPC
  specifically to satisfy a security policy prohibiting DB-credential egress | Needs the agent's own
  query-validation surface to be exactly as safe as `embyr-server`'s own, since the agent is the
  ONLY thing standing between any mTLS-authenticated caller and the customer's own raw Postgres.
- Alex (P1) | Real Firestore SDK app developer whose queries are transparently forwarded through
  `embyr-server` to Meridian Health's own agent | Needs every legitimate `where()` clause
  (simple field, dotted nested-map field) to keep working exactly as before — zero awareness that
  the underlying validator changed.

#### Solution
Delete `embyr-agent`'s own local `validate_field_path` (`server.rs:129-138`). Change its one caller
(`proto_filter_to_domain`, `server.rs:161`) to call `embyr_core::domain::query::validate_field_path`
instead, converting its `Result<(), CoreError>` to `Result<(), Status>` (the exact conversion
shape — inline `.map_err(...)` vs. a small named wrapper — is a DESIGN choice). Zero new logic: this
is calling an existing, already-tested function instead of a bespoke, weaker one, exactly mirroring
`embyr-server`'s own `translate_filter` call site.

#### Domain Examples

**Example 1 (Happy Path — regression guard, a simple field name)**: Riley Nakamura's Meridian
Health deployment serves a `RunQuery` for `collection("patients").where("status", "==", "active")`,
forwarded by `embyr-server` on behalf of Alex's app. `field_path` is `"status"` — passes both the
old and the new validator identically; the query executes and returns matching documents exactly as
before this feature.

**Example 2 (Happy Path — regression guard, a dotted nested-map field, the real-SDK shape SPEC.md
documents)**: The same deployment serves `.where("address.city", "==", "Boston")`. `field_path` is
`"address.city"` — spec-compliant (`^[a-zA-Z_][a-zA-Z0-9_.]*$`), passes the new validator exactly as
it already passes `embyr-server`'s own identical validator today; the query behavior is unchanged.

**Example 3 (Error/Boundary — the classic SQL-injection probe)**: A holder of a valid client
certificate for Meridian Health's own agent deployment (ordinarily `embyr-server` itself; in an
attack scenario, a compromised internal service or a misissued certificate) submits a `RunQuery`
whose filter's `field_path` is `"age' OR '1'='1"`. Before this fix: the weak validator sees no
consecutive dots and passes it straight to `fields->'age' OR '1'='1'->>'v' = $1`, corrupting the
intended WHERE clause. After this fix: `INVALID_ARGUMENT` is returned immediately, and the query
never reaches the SQL builder — Meridian Health's own Postgres never sees the crafted string.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A simple field-path filter continues to work exactly as before this feature
  Given Riley Nakamura's embyr agent for project "meridian-health" is running against a healthy Postgres
  And a "patients" collection has a document with field "status" set to "active"
  When Alex's app queries that collection with a filter on field "status" equal to "active"
  Then the RunQuery call returns the matching document
  And the response is identical to the response before this feature

Scenario: A dotted nested-map field-path filter continues to work exactly as before this feature
  Given Riley Nakamura's embyr agent for project "meridian-health" is running against a healthy Postgres
  And a "patients" collection has a document with a nested field addressable as "address.city" set to "Boston"
  When Alex's app queries that collection with a filter on field "address.city" equal to "Boston"
  Then the RunQuery call returns the matching document
  And the response is identical to the response before this feature

Scenario: A field path containing a single quote is rejected before reaching the SQL builder
  Given Riley Nakamura's embyr agent for project "meridian-health" is running against a healthy Postgres
  When a RunQuery call submits a filter whose field_path is "age' OR '1'='1"
  Then the RPC returns INVALID_ARGUMENT
  And no SQL statement referencing "age' OR '1'='1" is ever sent to Postgres

Scenario: A known SQL-injection-shaped payload is rejected, mirroring embyr-core's own named test payload
  Given Riley Nakamura's embyr agent for project "meridian-health" is running against a healthy Postgres
  When a RunQuery call submits a filter whose field_path is "x'); DROP TABLE documents; --"
  Then the RPC returns INVALID_ARGUMENT
  And no SQL statement referencing that payload is ever sent to Postgres

Scenario: A RunAggregationQuery filter is rejected identically to RunQuery, since both share the same validator
  Given Riley Nakamura's embyr agent for project "meridian-health" is running against a healthy Postgres
  When a RunAggregationQuery count() call submits a filter whose field_path is "status'; --"
  Then the RPC returns INVALID_ARGUMENT
  And no SQL statement referencing that payload is ever sent to Postgres

Scenario: The agent's own rejection now matches embyr-server's own rejection for the identical malformed field path
  Given a "backend_mode=direct_pg" project and a "backend_mode=agent" project both receive
    a RunQuery filter with field_path "name;DROP TABLE documents;"
  When each request is evaluated by its own backend
  Then both RPCs return INVALID_ARGUMENT
  And both rejection messages report the same spec-mandated charset violation
```

#### Acceptance Criteria
- [ ] AC-AFP-01: `embyr-agent`'s own `validate_field_path` (`server.rs:129-138`) is removed; its one
      caller (`proto_filter_to_domain`) calls `embyr_core::domain::query::validate_field_path`
      instead — the same function `embyr-server`'s own `translate_filter` already calls.
- [ ] AC-AFP-02: a `RunQuery` filter whose `field_path` contains any character outside
      `^[a-zA-Z_][a-zA-Z0-9_.]*$` is rejected with `Status::invalid_argument`, including at minimum
      a single quote, a semicolon, and the two named exploit-shaped payloads from
      `embyr-core`'s own existing test suite (`"x'); DROP TABLE documents; --"`,
      `"x' OR '1'='1"`).
- [ ] AC-AFP-03: a `RunAggregationQuery` filter with the identical malformed `field_path` shapes is
      rejected identically — both RPCs route through the same one call site (§ Investigation
      Finding 1), so both must be proven, not only `RunQuery`.
- [ ] AC-AFP-04 (regression guard): a simple field name (e.g. `"status"`) and a dotted nested-map
      field path (e.g. `"address.city"`) continue to be accepted and continue to return correct
      query results, unchanged from before this feature.
- [ ] AC-AFP-05 (regression guard): `CompositeFilter`-nested field filters (AND-composed) are
      validated per-leaf exactly as before — `proto_filter_to_domain`'s own existing recursion is
      untouched, only the leaf-level `validate_field_path` call target changes.
- [ ] AC-AFP-06 (parity guard): the identical malformed `field_path` submitted to a
      `backend_mode=direct_pg` project (via `embyr-server`) and a `backend_mode=agent` project (via
      `embyr-agent`) both return `INVALID_ARGUMENT` — closing the confirmed asymmetry this finding
      named.

#### Outcome KPIs
- **Who**: Riley Nakamura (P4), and by extension every `backend_mode=agent` customer's own security/
  audit posture (mirrors JOB-04's and JOB-09's own established persona for this deployment mode).
- **Does what**: `embyr-agent`'s own `RunQuery`/`RunAggregationQuery` filter validation rejects
  every field path outside the spec-mandated charset, instead of only rejecting consecutive dots.
- **By how much**: from 1 confirmed SQL-injection-reachable call site (accepting arbitrary
  non-dot-pair characters) to 0 — full charset parity with `embyr-server`'s own identical validator.
- **Measured by**: AC-AFP-02/03 (direct positive proof across both affected RPCs), AC-AFP-04/05
  (regression proof), AC-AFP-06 (cross-binary parity proof).
- **Baseline**: `embyr-agent`'s own validator accepts every character except a literal `".."`
  substring today, confirmed by direct code reading (§ Reading Confirmation).

#### Technical Notes
- Exact `CoreError -> Status` conversion shape at the one call site (inline `.map_err(|e|
  Status::invalid_argument(e.to_string()))` vs. a small named wrapper mirroring
  `embyr-server`'s own `translate_filter` pattern) is a DESIGN choice.
- Zero new `CoreError` variant, zero new proto field, zero new port/adapter trait method.
- `embyr-agent`'s own hardcoded-empty `order_by` (§ Investigation Finding 1) means `orderBy`-based
  field-path injection is not reachable via this binary today — explicitly out of scope, not
  silently assumed safe (see § Out of Scope for why it is named rather than fixed here).
- The one behavioral delta this fix introduces (consecutive-dot paths like `"a..b"` become
  ACCEPTED, where the old validator rejected them) is investigated and resolved as safe in
  § Investigation Finding 3 — no compensating logic is needed.

## Wave: DISCUSS / [REF] Out of Scope

- **Audit finding #26's own broader defense-in-depth question** (should
  `append_field_filter`/`push_scalar_comparison` in `embyr-pg-storage` re-validate the charset
  defensively at the SQL-builder layer itself, rather than trusting every caller to have already
  validated) — a real, separately-tracked, Medium-severity architectural finding this feature's own
  fix is ONE confirmed instance of, not the finding itself. Named here as directly relevant context,
  not silently folded into this feature's own narrow scope (task item 1's own "narrow fix" framing).
- **`embyr-agent`'s own silent discarding of `orderBy` clauses** (§ Investigation Finding 1,
  `order_by: vec![]` hardcoded regardless of proto content) — a real, separate, functional
  completeness gap (a client's `orderBy` is silently ignored rather than honored or rejected), not a
  security gap, and not evidenced as reachable for field-path injection specifically. Noted as a
  plausible follow-up candidate, not built here.
- **`backend_mode=agent` Sum/Avg aggregation field-path validation** — not reachable today because
  `embyr-agent`'s own `RunAggregationQuery` handler only implements `Count` (confirmed:
  `RunAggregationQueryResponse { count: docs.len() as i64 }`, hardcoded); Sum/Avg are deferred
  per `jobs.yaml`'s own JOB-01 agent-mode-aggregation note. Nothing to validate until that gap
  closes.
- **A general re-audit of every other agent RPC handler for unrelated correctness gaps** — this
  feature is narrowly about the one confirmed field-path-validator asymmetry named by finding #11;
  § Investigation Finding 2 already confirms no sibling duplicated-validator pattern exists
  elsewhere in this crate.

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy A** (real, minimal, end-to-end) — US-01's own single slice is a real SQL-injection-probe
`field_path` (a single-quote payload, at minimum), submitted via a real `RunQuery` call against a
real running `embyr-agent` process and a real Postgres backend, proven to return `INVALID_ARGUMENT`
without any SQL statement referencing the payload ever reaching Postgres — not a unit-test-only or
mocked proof. A second, identically-shaped proof against `RunAggregationQuery` closes the
confirmed two-RPC blast radius (§ Investigation Finding 1) in the same slice, since both share one
call site and one fix.

## Wave: DISCUSS / [REF] Driving Ports

gRPC `:9191` `StorageAgent` on `embyr-agent` (existing routes, zero new RPC) — specifically
`RunQuery` and `RunAggregationQuery`, the two confirmed callers of the one changed call site.

## Wave: DISCUSS / [REF] Pre-requisites

- None. `embyr_core::domain::query::validate_field_path` already exists, is already tested, and is
  already depended upon by `embyr-agent`'s own `Cargo.toml` (§ Reading Confirmation) — this is a
  call-site substitution onto existing, already-correct infrastructure, not new construction.

## Wave: DISCUSS / [REF] Definition of Ready Validation

### DoR Checklist (9-item hard gate)
1. [x] Story traces to a job_id (JOB-04) — reused, not new, with two candidate alternatives (JOB-05,
   JOB-09, JOB-11) explicitly reasoned against (§ Persona & Job).
2. [x] Elevator Pitch complete (Before / After / Decision enabled), naming a real user-invocable
   entry point (`RunQuery`/`RunAggregationQuery` on the agent's own real gRPC surface, `:9191`).
3. [x] 3+ domain examples with real, concrete data (real personas — Riley Nakamura, Alex — a real
   customer name — Meridian Health — and real field-path/payload strings, not generic placeholders).
4. [x] UAT scenarios in Given/When/Then — 6 scenarios (within the 3-7 right-sized range).
5. [x] Acceptance criteria derived directly from the UAT scenarios (AC-AFP-01 through AC-AFP-06).
6. [x] Right-sized — single call site, single function swap, zero new logic; § Scope Assessment
   confirms PASS with no split needed.
7. [x] Technical notes identify constraints (exact error-conversion shape is a DESIGN choice; the
   consecutive-dots behavioral delta is investigated and resolved as safe, not left open).
8. [x] Outcome KPIs have a numeric target (1 confirmed vulnerable call site → 0) and measurement
   methods (direct AC proof across both affected RPCs plus a cross-binary parity guard).
9. [x] Prior-wave artifacts read and reconciled (audit finding #11, both `validate_field_path`
   implementations, the SQL sink, `embyr-agent`'s own `Cargo.toml` dependency, ADR-001's own US-12/
   JOB-04 framing, and `jobs.yaml` JOB-04/05/09/11 all directly informed this feature's shape).

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Persona/job: **P4 Riley Nakamura / JOB-04 (`credential-isolation`)**, reused — not JOB-05
  (unrelated cloud-secret-fetching mechanism), JOB-09 (after-the-fact audit evidence, not
  prevention), or JOB-11 (cross-tenant fairness on shared embyr SaaS infrastructure, not
  single-tenant agent-boundary integrity) (§ Persona & Job).
- [D2] Fix mechanism: delete `embyr-agent`'s own weaker, independently-written
  `validate_field_path`; call `embyr_core::domain::query::validate_field_path` directly at the one
  confirmed call site (`proto_filter_to_domain`) — zero new logic, reusing already-correct,
  already-tested infrastructure (§ Investigation Finding 1).
- [D3] Blast radius is exactly one call site serving two RPCs (`RunQuery`, `RunAggregationQuery`);
  write paths (`CreateDocument`/`UpdateDocument`/`Commit`) are confirmed NOT part of the blast
  radius — field names on writes are JSON keys inside a bound JSONB parameter, never raw-SQL text
  (§ Investigation Finding 1).
- [D4] No sibling duplicated-validator pattern exists elsewhere in `embyr-agent` — confirmed by
  exhaustive grep across all 8 source files in the crate, not by absence-of-evidence alone
  (§ Investigation Finding 2). This is an isolated incident.
- [D5] The stricter charset introduces exactly one behavioral delta (consecutive-dot paths like
  `"a..b"` become accepted rather than rejected) — investigated and resolved as safe: this is
  `embyr-server`'s own existing, already-live, spec-conformant behavior, not new agent-specific
  permissiveness (§ Investigation Finding 3).
- [D6] Audit finding #26's own broader defense-in-depth architectural question (should the SQL
  builder itself re-validate defensively) is named as directly relevant context but explicitly NOT
  part of this feature's own narrow scope (§ Out of Scope).

### Requirements Summary
- Primary need: `embyr-agent`'s own query-filter field-path validator enforces the identical
  spec-mandated charset `embyr-server`'s own validator already enforces, closing the one confirmed
  SQL-injection-reachable asymmetry between the two backend modes' query-validation boundary, with
  zero regression for any real, legitimate Firestore field-path shape.
- Constraint: mTLS-gated attack surface (`:9191`, not remote-unauthenticated) — confirmed matching
  the audit's own framing; the threat model is a valid-certificate holder (ordinarily embyr-server
  itself, or a compromised/misissued certificate), not an anonymous remote attacker.
- Success looks like: AC-AFP-01 through AC-AFP-06 all passing against a real running `embyr-agent`
  and a real Postgres backend, plus zero regression in `embyr-agent`'s own existing acceptance
  suite (`tests/acceptance/us_12_agent_backend.rs` and siblings).

### Handoff Package (to DESIGN — solution-architect)
- This feature-delta.md (DISCUSS section) — job grounding, blast-radius trace, charset-safety
  investigation, 1 user story with 6 UAT scenarios and 6 acceptance criteria.
- Confirmed exact file/line targets for DESIGN: delete `crates/embyr-agent/src/server.rs:128-138`;
  change the call at `crates/embyr-agent/src/server.rs:161` to invoke
  `embyr_core::domain::query::validate_field_path`.
- Open DESIGN-level choice: exact `CoreError -> Status` conversion shape at the one call site.
- Flagged, out-of-scope, related findings for DESIGN's own awareness (not this feature's build
  scope): audit finding #26 (defense-in-depth at the SQL-builder layer), `embyr-agent`'s own
  silently-discarded `orderBy` clauses.

---

## Wave: DESIGN / [REF] Reading Confirmation

✓ `crates/embyr-core/src/domain/query.rs` read in full (lines 1-80+). Confirmed
`pub fn validate_field_path(path: &str) -> Result<(), CoreError>` (line 16), backed by private
`is_valid_field_path` (line 26, regex-equivalent hand-rolled char scan for
`^[a-zA-Z_][a-zA-Z0-9_.]*$`). Confirmed `domain/mod.rs:5` declares `pub mod query;`, so the function
is reachable at `embyr_core::domain::query::validate_field_path` exactly as DISCUSS cited. Confirmed
its error text: `CoreError::InvalidArgument(format!("field path must match
^[a-zA-Z_][a-zA-Z0-9_.]*$, got: {path}"))` — this exact string is the load-bearing fact for
§ Error Message Format Consistency below.

✓ `crates/embyr-agent/src/server.rs` read in full (lines 1-180). Confirmed the exact deletion target
(lines 128-138, the weak local `validate_field_path`) and the exact one-line call site to change
(line 161, inside `proto_filter_to_domain`). **Also found a load-bearing fact DISCUSS's own
Technical Notes didn't name**: this same file already has a general-purpose `CoreError -> Status`
converter, `core_error_to_status` (lines 96-114), already used elsewhere in this exact file for
every other `CoreError` variant this binary produces (`DocumentNotFound`, `AlreadyExists`,
`OccConflict`, `FailedPrecondition`, `InvalidArgument`, `BackendUnavailable`, `TransactionNotFound`,
`TransactionAborted`, `ProjectNotFound`, `PermissionDenied`, `Unauthenticated`,
`ResourceExhausted`). Its `InvalidArgument` arm already exists and is exactly the conversion this
feature needs: `CoreError::InvalidArgument(msg) => Status::invalid_argument(msg)`. This changes the
DESIGN outcome from "write a small named wrapper or an inline `.map_err(|e|
Status::invalid_argument(e.to_string()))` closure" (DISCUSS's own illustrative sketch) to "reuse the
converter that is already sitting 30 lines above the call site, in the same file, already handling
this exact `CoreError` variant" — ladder rung 2 (reuse what's already in the codebase) beats rung 6
(write one line) beats rung 7 (write a named wrapper function). See § Decision below.

✓ `crates/embyr-agent/Cargo.toml` read in full. Confirmed `embyr-core.workspace = true` (line 16, a
direct, non-dev dependency) — DISCUSS's claim verified directly, not assumed from memory.

✓ Confirmed `proto_filter_to_domain`'s own signature (`server.rs:158`):
`fn proto_filter_to_domain(filter: ProtoFilter) -> Result<QueryFilter, Status>` — its error type is
already `Status`, so `.map_err(core_error_to_status)` on `validate_field_path`'s
`Result<(), CoreError>` return produces the exact `Result<(), Status>` the `?` operator at that call
site already needs. No further type-conversion machinery is required.

✓ `crates/embyr-server/src/grpc/handler.rs` read (relevant lines: 19, 3067, 3344, 3353, 3988-3996,
4164). Confirmed `embyr-server`'s own established pattern for this exact validation failure, for
§ Error Message Format Consistency below.

## Wave: DESIGN / [REF] Blast-Radius Re-Verification (independent re-check of DISCUSS's own claim)

Per this session's own established lesson (always re-verify a prior wave's blast-radius claim, not
just trust it — see `firestore-composite-indexes-admin-api` and others), grepped for
`validate_field_path` across the entire `crates/embyr-agent/` directory (not scoped to `server.rs`
alone, to also rule out a second copy in `probe.rs`, `main.rs`, `encoding.rs`, `notify_bridge.rs`,
`lib.rs`, `config.rs`, or `sweeper.rs`):

```
crates/embyr-agent/src/server.rs:129   fn validate_field_path(path: &str) -> Result<(), Status> {
crates/embyr-agent/src/server.rs:161       validate_field_path(&ff.field_path)?;
```

**Exactly 2 matches, both in `server.rs`: one definition (line 129), one caller (line 161).**
DISCUSS's "exactly one call site" claim is confirmed independently, not merely re-stated. No other
file in the crate references this symbol. **Blast radius: 1 file, 1 deletion, 1 call-site edit.**

## Wave: DESIGN / [REF] Decision — Exact Code Change (no wrapper function; direct reuse of two already-existing functions)

**Decision: delete the local `validate_field_path` entirely. Import `embyr_core`'s real one under
the identical name (zero call-site rename). Convert its error via the already-existing
`core_error_to_status`, not a new wrapper.**

No new ADR is warranted for this decision — see § ADR Decision below for why.

### The exact diff

**1. Delete `crates/embyr-agent/src/server.rs:128-138`** (the weak local validator and its doc
comment) in full:
```rust
/// Validate that a field path does not contain consecutive dots.
fn validate_field_path(path: &str) -> Result<(), Status> {
    if path.contains("..") {
        Err(Status::invalid_argument(format!(
            "invalid field path '{}': consecutive dots not allowed",
            path
        )))
    } else {
        Ok(())
    }
}
```

**2. Extend the existing `embyr_core` import at `server.rs:23`** to bring the real function into
scope under the same bare name the deleted function used (so the call site at line 161 needs no
rename, only a `.map_err(...)` addition):
```rust
use embyr_core::domain::{
    document::CollectionPath,
    query::{
        FieldFilter, FilterOp, QueryFilter, StructuredQuery as DomainStructuredQuery,
        validate_field_path,
    },
};
```

**3. Change the one call site at `server.rs:161`** from:
```rust
validate_field_path(&ff.field_path)?;
```
to:
```rust
validate_field_path(&ff.field_path).map_err(core_error_to_status)?;
```

That is the entire production-code diff: one 11-line deletion, one import-list extension (one new
name), one `.map_err(core_error_to_status)` insertion. Net effect is negative LOC. Zero new
functions, zero new types, zero new `CoreError` variant (confirmed already zero-new by DISCUSS),
zero touch to `proto_filter_to_domain`'s own recursion for `CompositeFilter` (AC-AFP-05) — the
recursion calls itself unchanged and hits the same, single, now-fixed leaf call site on every
recursive descent.

### Why no wrapper function (ponytail ladder applied)

DISCUSS's own illustrative sketch proposed either an inline `.map_err(|e|
Status::invalid_argument(e.to_string()))` closure or a small renamed wrapper function performing the
same conversion, and left the choice to DESIGN. Neither is needed: `core_error_to_status`
(`server.rs:97`) already exists in this exact file, is already the established, single, general
`CoreError -> Status` boundary this binary uses for every other domain error it produces, and its
`InvalidArgument` arm already does precisely the conversion this call site needs
(`CoreError::InvalidArgument(msg) => Status::invalid_argument(msg)`). Writing a second, one-variant
conversion function — or an inline closure that reimplements one arm of an already-existing match —
would be the exact “unrequested abstraction” the ladder forbids: a second, narrower path to the same
gRPC status the codebase already has a general path for. Rung 2 (reuse what's already in the
codebase) wins outright over rung 6 (write one line) and rung 7 (write a named wrapper).

This also directly answers DISCUSS's own open question ("keep the wrapper or inline it") with a
third option DISCUSS hadn't spotted: don't write *any* new conversion code at all, of either shape —
call the one that's already there.

## Wave: DESIGN / [REF] Error Message Format Consistency (task item 3)

**Checked**: does `embyr-server` have an established message-format convention for this exact
validation failure that the agent-side fix should match?

**Finding**: yes, a convention exists, and it is subtly different from what reusing
`core_error_to_status` produces — this is a real, confirmed, and consciously accepted trade-off, not
an oversight.

- `embyr-server`'s own call sites (`handler.rs:3344`, `:3353`, and `translate_filter`'s own
  `handler.rs:3994`/`:4091`, whose `Err(String)` is later turned into a `Status` via
  `.map_err(Status::invalid_argument)` at `handler.rs:3048`) all convert the `CoreError` via
  **`e.to_string()`** — i.e., through `CoreError`'s own `#[error("invalid argument: {0}")]` `Display`
  impl (`embyr-core/src/error.rs:22`). The resulting client-visible text is: `"invalid argument:
  field path must match ^[a-zA-Z_][a-zA-Z0-9_.]*$, got: <path>"` (the `Display` impl's own
  `"invalid argument: "` prefix, THEN the `validate_field_path` message).
- `embyr-agent`'s own `core_error_to_status` (the function this design reuses) instead matches on the
  `CoreError` variant directly and forwards the **bare inner `msg` string**, never the
  `Display`-prefixed form: `CoreError::InvalidArgument(msg) => Status::invalid_argument(msg)`. The
  resulting text is: `"field path must match ^[a-zA-Z_][a-zA-Z0-9_.]*$, got: <path>"` — identical
  content, minus the `"invalid argument: "` prefix.

**Decision: keep reusing `core_error_to_status` as-is; do not special-case this one call site to add
the prefix.** Three reasons:
1. **Local consistency beats cross-binary wording parity.** Every other `CoreError` variant
   `core_error_to_status` already converts in this exact file (`DocumentNotFound`, `AlreadyExists`,
   `FailedPrecondition`, etc.) uses the bare `msg`, never `e.to_string()`. Special-casing
   `InvalidArgument` alone to add a `Display`-derived prefix would make this one function internally
   inconsistent — one arm behaving differently from its eleven siblings — for a benefit that AC-AFP-06
   does not actually require (below).
2. **AC-AFP-06 is satisfied either way.** Its exact wording is *"both rejection messages report the
   same spec-mandated charset violation"* — not "byte-identical strings." The load-bearing substring,
   `"field path must match ^[a-zA-Z_][a-zA-Z0-9_.]*$, got: <path>"`, is byte-identical in both
   binaries' output; only the optional `"invalid argument: "` `Display` prefix differs, and gRPC's own
   `INVALID_ARGUMENT` status code already carries that same semantic information redundantly.
3. **No test depends on either wording.** DISCUSS's own Finding 4 already confirmed zero existing
   test asserts on the old validator's specific message text; this design confirms the same is true
   in the other direction — no test in either binary's suite asserts on the `Display` prefix being
   present or absent.

This trade-off is recorded here explicitly (rather than silently decided) so DISTILL's own
AC-AFP-06 test asserts on the shared substring, not on exact string equality between the two
binaries' error text.

## Wave: DESIGN / [REF] ADR Decision — no new ADR

**Decision: no new ADR for this feature.** Rationale, evaluated against this project's own ADR
bar (contrast with `sanitize-backend-error-messages`'s ADR-075, which DID warrant one):
- Zero new component, port, adapter, or cross-cutting pattern is introduced — this is a call-site
  substitution onto two functions that already exist, are already tested, and are already each
  other's established neighbors in the same file.
- Zero new `CoreError` variant, proto field, or port/adapter trait method (DISCUSS already confirmed
  this; DESIGN introduces nothing that would change that).
- The one genuine judgment call this design makes (§ Error Message Format Consistency) is narrow,
  fully reversible, non-cross-cutting, and recorded in-line above with its rejected alternative
  (matching `embyr-server`'s `e.to_string()` prefix) and rationale — an ADR's own value (surviving as
  a durable, standalone record future maintainers consult before making a conflicting change) doesn't
  apply to a decision this narrowly scoped to one call site in one file.
- Matches this session's own established precedent: `rate-limiter-project-id-validation` and
  `stripe-webhook-secret-required` (both narrow, single-file security-hardening fixes) also produced
  no new ADR and no C4 diagram; `sanitize-backend-error-messages` DID produce one because it
  established a NEW sanitization-boundary convention spanning 2 crates — a materially different kind
  of decision than this feature's own single-call-site substitution.

## Wave: DESIGN / [REF] C4 Diagrams — none produced (scope justification)

No new container, component, or integration boundary is introduced, moved, or removed. The gRPC
`:9191` `StorageAgent` service, its `RunQuery`/`RunAggregationQuery` handlers, the
`PostgresBackendAdapter`, and the customer's own Postgres are all pre-existing and structurally
unchanged by this fix — only the character-set strictness of one already-existing internal
validation call changes. Producing a System Context/Container diagram would repeat
`adr-001-process-topology.md`'s own existing diagram unchanged. Matches session precedent
(`rate-limiter-project-id-validation`, `stripe-webhook-secret-required` — neither produced a C4
diagram for the same reason).

## Wave: DESIGN / [REF] Earned Trust / Probe Applicability — not applicable, and why

Principle-12 probing applies to adapters/ports depending on something external (filesystem, time,
subprocess, vendor SDK, config source, network, kernel syscall semantics). This fix touches none of
those: `validate_field_path` is a pure, IO-free, in-memory string-charset check
(`embyr-core` remains IO-free — unaffected, confirmed by DESIGN's own reading), and
`core_error_to_status` is a pure in-memory `match` with no side effects beyond constructing a
`Status` value. No adapter, no driven port, no external dependency is introduced, removed, or
touched. **No probe is needed for this fix — this is a conscious "not applicable" determination, not
a silent omission.** The existing enforcement for the shared validator's own correctness is
`embyr-core`'s own pre-existing proptest suite (`spec_compliant_paths_always_accepted`,
`path_with_any_disallowed_char_is_rejected`) plus the two named-payload unit tests
(`known_sql_injection_payloads_rejected`) — all already passing, all already reused (not new) by
this fix, exactly as DISCUSS's Reading Confirmation described.

## Wave: DESIGN / [REF] External Integrations / Enforcement Tooling

- **External integrations**: none. This feature touches no third-party API, webhook, or OAuth
  provider — no contract-testing annotation applies.
- **Architectural enforcement**: `deny.toml`'s existing `embyr-core`-must-stay-IO-free rule is
  unaffected (this fix calls an already-IO-free function; no new dependency is added to
  `embyr-core`). No new architectural rule is introduced by this fix, so no new enforcement tooling
  is warranted. The correctness guarantee this fix depends on (the charset regex itself behaving
  correctly) is already enforced by `embyr-core`'s own existing proptest suite, which this fix reuses
  rather than duplicates.

## Wave: DESIGN / Handoff Package (to DISTILL — acceptance-designer)

**Files requiring a change:**
1. `crates/embyr-agent/src/server.rs` — the ONLY production-code file requiring a change:
   - Delete lines 128-138 (local `validate_field_path`).
   - Extend the `embyr_core::domain::{...}` import at line 23 to add `validate_field_path`.
   - Change line 161 from `validate_field_path(&ff.field_path)?;` to
     `validate_field_path(&ff.field_path).map_err(core_error_to_status)?;`

**Files confirmed to need NO change (blast radius, grep-verified across the whole crate):**
- Every other file in `crates/embyr-agent/src/` (`probe.rs`, `main.rs`, `encoding.rs`,
  `notify_bridge.rs`, `lib.rs`, `config.rs`, `sweeper.rs`) — none reference `validate_field_path`.
- `crates/embyr-core/src/domain/query.rs` — the real validator is unmodified, reused as-is.
- `crates/embyr-pg-storage/*` — the raw-interpolating SQL builder is unmodified; this fix closes the
  gap upstream of it, at the validation boundary, not at the sink.
- `crates/embyr-server/*` — unaffected; already calls the real validator via its own
  `translate_filter`/direct call sites.

**Documentation changes made this wave:**
- No new ADR (§ ADR Decision above).
- No new C4 diagram (§ C4 Diagrams above).
- This DESIGN section, appended to this same `feature-delta.md`.

**Regression guards DISTILL/DELIVER must run:**
- `tests/acceptance/us_12_agent_backend.rs` — the agent's own primary acceptance suite; DISCUSS's own
  Finding 4 confirms zero existing test asserts on the OLD validator's message text, so this suite
  must continue passing unmodified (proves AC-AFP-04's simple-field-name regression path is not
  broken by the swap).
- `embyr-core`'s own `field_path_tests` module (`crates/embyr-core/src/domain/query.rs`) — unmodified,
  already-passing, reused as the shared-validator correctness guard (not re-derived per-binary).
- New acceptance tests DISTILL must author for this feature's own 6 UAT scenarios (AC-AFP-01 through
  AC-AFP-06) — a real `RunQuery`/`RunAggregationQuery` call against a real running `embyr-agent` and a
  real Postgres backend, per the Walking Skeleton Strategy already fixed by DISCUSS. AC-AFP-06's own
  test must assert on the shared substring (`"field path must match ^[a-zA-Z_][a-zA-Z0-9_.]*$, got:
  ..."`), not on exact cross-binary string equality (§ Error Message Format Consistency).
- Full workspace `cargo test` — run once, at the pre-commit gate, per this repo's own token-discipline
  convention (CLAUDE.md).

**Locked DESIGN decisions for DISTILL/DELIVER to build against:**
- [DD1] Delete the local weak validator; import and call
  `embyr_core::domain::query::validate_field_path` directly at the one call site — no rename, no new
  symbol at the call site beyond the import.
- [DD2] No wrapper function of any shape (named or inline closure); convert the error via the
  already-existing `core_error_to_status` (`server.rs:97`), reused as-is.
- [DD3] Blast radius independently re-confirmed as exactly 1 file, 1 definition deleted, 1 call site
  changed — DISCUSS's claim holds.
- [DD4] Message-format delta (missing `"invalid argument: "` `Display` prefix vs. `embyr-server`'s
  own convention) is a conscious, accepted trade-off, not an oversight — AC-AFP-06 must be tested via
  substring, not exact-match.
- [DD5] No new ADR, no new C4 diagram — narrow call-site substitution, no new architectural surface.
- [DD6] No probe required — pure in-memory validation logic, no external dependency touched.

---

## Wave: DISTILL / [REF] Reading Confirmation

+ `docs/feature/agent-field-path-validation/feature-delta.md` (DISCUSS + DESIGN, in full).
+ `nw-test-design-mandates`, `nw-bdd-methodology` skills.
+ `docs/architecture/atdd-infrastructure-policy.md` — existing project policy, mode `inherit`;
  "Agent gRPC (:9191) — in-process tonic mTLS test client" row already covers this feature's driving
  port, no new row needed.
+ `tests/common/state_delta.rs` — Rust state-delta port already bootstrapped (prior feature), no
  bootstrap needed this run.
+ `tests/acceptance/embyr_agent/mod.rs`, `us_a01`–`us_a08` (all 8 existing files) — the established
  mTLS harness (`start_test_agent`, real Postgres testcontainer, real in-process `serve()`).
+ `crates/embyr-agent/src/server.rs`, `crates/embyr-core/src/domain/query.rs`,
  `crates/embyr-pg-storage/src/encoding/query.rs` — confirmed exact current code shape.
+ `tests/security_rules/common/mod.rs` (`SecurityRulesFullContext`) and
  `tests/firestore_query_filter_operator_support/acceptance/qfo01_array_contains.rs` — the
  established `embyr-server` (direct_pg) RunQuery test pattern, reused for the AC-AFP-06 cross-binary
  parity test.
- `docs/product/journeys/*.yaml`, `docs/product/architecture/brief.md`, `docs/product/kpi-contracts.yaml`
  (not found — this project predates the `docs/product/` SSOT layout for this feature; DISCUSS/DESIGN
  already live entirely in this feature's own `feature-delta.md`, which is the authoritative source
  read above in full. No blocking gap: driving ports, domain language, and acceptance criteria are
  all already explicit in DISCUSS/DESIGN.)
- `docs/feature/agent-field-path-validation/{discuss,design,devops}/wave-decisions.md` (not found as
  separate files — this feature uses the single-narrative `feature-delta.md` model; DISCUSS and
  DESIGN sections above ARE the wave-decisions record, already read in full).

## Wave: DISTILL / [REF] Wave-Decision Reconciliation

Read DISCUSS and DESIGN sections of this same `feature-delta.md` in full (no separate per-wave
`wave-decisions.md` files exist for this feature — single-narrative model). Checked every DISCUSS
decision (D1–D6) against DESIGN: DESIGN extends and specializes DISCUSS's decisions (exact diff,
error-conversion mechanism, message-format trade-off) without reversing or contradicting any of
them. **Zero contradictions.**

**Reconciliation passed — 0 contradictions.**

## Wave: DISTILL / [REF] Upstream Issue Found — DISCUSS Finding 4 correction

DISCUSS's own Finding 4 claims: *"Grepped `tests/` for `consecutive dots`... no acceptance test
references the OLD validator's specific rejection message or its narrower consecutive-dots-only
behavior."* This is **not fully accurate** — an existing test,
`query_with_malformed_field_path_rejected_before_data_read`
(`tests/acceptance/embyr_agent/us_a03_query_operations.rs`), used the literal payload
`"order..amount"` and asserted `INVALID_ARGUMENT`. DISCUSS's grep evidently scoped to
`tests/acceptance/us_12_agent_backend.rs` and a literal-string search for "consecutive dots" (the
message text), missing this sibling file's own consecutive-dot payload used for an unrelated
purpose (a generic "malformed field path" example, not specifically testing the consecutive-dot
rule).

**Impact**: this existing test would have SILENTLY REGRESSED once DELIVER lands the fix (consecutive
dots become accepted, not rejected — Finding 3's own confirmed, intentional behavior change) — a
real "found it before DELIVER did" catch, not a hypothetical. **Resolved in this DISTILL wave**: the
test's payload was swapped to `"order amount"` (space) — genuinely malformed under both the old and
new validator — preserving its original, unrelated intent (generic malformed-field-path rejection)
without a silent future break. The consecutive-dot behavioral delta itself now has its OWN explicit,
documented test (`consecutive_dot_field_path_is_now_accepted_and_matches_zero_documents`, per
AC-AFP-05-adjacent DISCUSS requirement item 5).

## Wave: DISTILL / [REF] Scenario List

Test placement: `tests/acceptance/embyr_agent/us_a09_field_path_validation.rs` (registered under
`crates/embyr-agent/Cargo.toml`'s existing `[[test]] name = "embyr_agent"` binary via
`tests/acceptance/embyr_agent.rs`'s module list) — precedent: identical placement to `us_a01`–`us_a08`,
same crate, same harness. One cross-binary test lives separately (see below).

| # | Scenario | Tags | AC | Tier |
|---|---|---|---|---|
| 1 | `run_query_rejects_sql_injection_probe_field_path_with_single_quote` | `@walking_skeleton @driving_port @real_io @error` | AC-AFP-02 | A |
| 2 | `run_aggregation_query_rejects_sql_injection_probe_field_path_with_single_quote` | `@driving_port @real_io @error` | AC-AFP-03 | A |
| 3 | `run_query_rejects_named_drop_table_injection_payload_and_documents_table_survives` | `@driving_port @real_io @error` | AC-AFP-02 | A |
| 4 | `simple_field_name_filter_continues_to_work_on_run_query` | `@driving_port @real_io` | AC-AFP-04 | A |
| 5 | `simple_field_name_filter_continues_to_work_on_run_aggregation_query` | `@driving_port @real_io` | AC-AFP-04 | A |
| 6 | `dotted_nested_field_path_filter_continues_to_work_on_run_query` | `@driving_port @real_io` | AC-AFP-04 | A |
| 7 | `composite_filter_rejects_malformed_leaf_field_path_among_valid_leaves` | `@driving_port @real_io @error` | AC-AFP-05 | A |
| 8 | `create_document_accepts_a_field_name_with_special_characters_write_path_unaffected` | `@driving_port @real_io` | AC-AFP-04 (write-path confirmation) | A |
| 9 | `consecutive_dot_field_path_is_now_accepted_and_matches_zero_documents` | `@driving_port @real_io` | Finding 3 (deliberate delta) | A |
| 10 | `query_with_malformed_field_path_rejected_before_data_read` (modified, us_a03) | `@driving_port @real_io @error` | regression (payload swap) | A |
| 11 | `identical_malformed_field_path_rejected_identically_by_both_backend_modes` | `@driving_port @real_io @error` | AC-AFP-06 | A |

Error/edge-path ratio: 6 of 11 scenarios are error/behavioral-delta paths (55%) — exceeds the 40%
mandate; consistent with this feature's security-hardening nature.

## Wave: DISTILL / [REF] Walking Skeleton Strategy

Strategy A (real, minimal, end-to-end) — confirmed per DISCUSS. Scenario 1
(`run_query_rejects_sql_injection_probe_field_path_with_single_quote`) is the sole
`@walking_skeleton`: real `embyr-agent` composition root (`embyr_agent::server::serve`, the same
function the production binary calls), real Postgres testcontainer, real mTLS client cert, a single
SQL-injection-probe field path submitted via a real `RunQuery` call. No mocks anywhere in the path.

## Wave: DISTILL / [REF] Adapter Coverage Table

| Port / Adapter | `@real_io` scenario | Covered by |
|---|---|---|
| StorageAgent gRPC `RunQuery` (mTLS :9191) | YES | Scenarios 1, 3, 4, 6, 7, 9, 10 |
| StorageAgent gRPC `RunAggregationQuery` (mTLS :9191) | YES | Scenarios 2, 5 |
| StorageAgent gRPC `CreateDocument`/`GetDocument` (write-path confirmation) | YES | Scenario 8 |
| `PostgresBackendAdapter` (customer Postgres, real testcontainer) | YES | All scenarios (via `start_test_agent`'s real Postgres container) |
| Firestore gRPC `RunQuery` on `embyr-server` (direct_pg, cross-binary comparison partner) | YES | Scenario 11 (via `SecurityRulesFullContext`) |

Zero "NO — MISSING" rows. No new adapter is introduced by this feature (DD5/DD6) — coverage reuses
the already-established harness in full.

## Wave: DISTILL / [REF] Scaffolds

None required. This feature introduces zero new production modules or call sites the tests need to
import ahead of implementation — `crates/embyr-agent/src/server.rs`,
`crates/embyr-core/src/domain/query.rs`, and `embyr_proto::agent::*` all already exist, already
compile, and are already exercised by the pre-existing `us_a01`–`us_a08` suite. RED comes from the
CURRENT (wrong) runtime behavior of the already-existing `validate_field_path`/`proto_filter_to_domain`
call site, not from missing code (Mandate 7's scaffold requirement is not triggered — no
`__SCAFFOLD__`/`AssertionError` stub needed; the file-header `// SCAFFOLD: true` marker present on
this and every sibling file in this suite is this project's own established convention marking
"DISTILL-authored test file," not a literal RED-stub marker).

## Wave: DISTILL / [REF] Driving Adapter Coverage

Both RPCs DESIGN names as reachable callers of the changed call site (§ Investigation Finding 1)
are exercised via their real gRPC protocol (not a direct Rust function call to
`proto_filter_to_domain`): `RunQuery` (scenarios 1, 3, 4, 6, 7, 9, 10, 11) and `RunAggregationQuery`
(scenarios 2, 5). Zero uncovered entry points — `order_by` and Sum/Avg aggregation are confirmed
unreachable via `embyr-agent` today (§ Out of Scope) and correctly excluded.

## Wave: DISTILL / [REF] Pre-requisites

- `docs/architecture/atdd-infrastructure-policy.md` — present, `inherit` mode, no new row needed
  (existing "Agent gRPC (:9191)" row already covers the driving port; existing "Driven internal
  (real)" Postgres rows already cover the customer DB).
- `tests/common/state_delta.rs` — present (bootstrapped by a prior feature). Not used by these tests:
  per the Layered Test Discipline table, these are Integration-layer tests (real adapter, real
  Postgres testcontainer, ~1-10s per test) where `assert_state_delta`/Universe is OPTIONAL — matches
  this codebase's own established, unbroken precedent across all 8 prior `embyr_agent` acceptance
  files and every `SecurityRulesFullContext`-based test in this session (plain `assert_eq!`/
  `tonic::Status` assertions throughout).
- Tier B (state-machine PBT): SKIPPED. Journey is a single narrow security-hardening fix (one call
  site), not a ≥3-chained-scenario rich journey; matches this session's own precedent
  (`stripe-webhook-secret-required`, `rate-limiter-project-id-validation`) for identically-shaped
  narrow fixes.

## Wave: DISTILL / [REF] Machine Artifacts

- `tests/acceptance/embyr_agent/us_a09_field_path_validation.rs` — new file, 11 `#[tokio::test]`
  functions (10 in-file + the file also documents the modified sibling in `us_a03`).
- `tests/acceptance/embyr_agent.rs` — extended: `mod us_a09_field_path_validation;` registration.
- `tests/acceptance/embyr_agent/us_a03_query_operations.rs` — modified:
  `query_with_malformed_field_path_rejected_before_data_read`'s payload swapped from `"order..amount"`
  to `"order amount"` (see Upstream Issue Found above).
- `tests/agent_field_path_validation/acceptance/afp06_cross_binary_parity.rs` — new file, 1
  `#[tokio::test]` function (AC-AFP-06).
- `crates/embyr-server/Cargo.toml` — extended: one new `[[test]]` entry
  (`agent_field_path_validation_afp06_cross_binary_parity`).
- `docs/feature/agent-field-path-validation/distill/red-classification.md` — pre-DELIVER
  fail-for-the-right-reason gate evidence, all 6 RED tests empirically confirmed
  MISSING_FUNCTIONALITY.

## Wave: DISTILL / [REF] Mandate Compliance Evidence

- **CM-A** (Mandate 1, hexagonal boundary): every test invokes the `StorageAgentClient`/
  `FirestoreClient` gRPC stub over a real transport (mTLS or plain-TLS-free in-process channel) —
  the driving port — never `proto_filter_to_domain` or `validate_field_path` directly. Zero internal
  component imports.
- **CM-B** (Mandate 2, business language): test/scenario names and doc-comment Given/When/Then use
  domain terms ("query", "field path", "documents", "collection") — technical terms (SQL, JSONB,
  charset regex) are confined to doc-comment RATIONALE paragraphs explaining RED classification, not
  to scenario titles or step logic.
- **CM-C** (Mandate 3, user-journey completeness): each scenario carries a full Given/When/Then in
  its doc comment naming the trigger, the action, and the observable outcome (RPC status +
  message/row-count), not an isolated technical assertion.
- **CM-D** (Mandate 4, pure-function extraction): not applicable — this feature introduces zero new
  business logic; it reuses two already-existing, already-pure functions
  (`validate_field_path`, `core_error_to_status`).
- **CM-E/F/G/H** (Mandates 8-11): Integration-layer tests (real Postgres, real gRPC) — state-delta
  optional (Mandate 8 layer-4+ carve-out, exercised), no PBT machinery used (Mandate 9/11 — matches
  layer-3+ example-only requirement), Tier B correctly skipped (Mandate 10 — journey too narrow).

## Wave: DISTILL / Handoff Package (to DELIVER — software-crafter)

- This feature-delta.md in full (DISCUSS + DESIGN + DISTILL).
- `docs/feature/agent-field-path-validation/distill/red-classification.md` — RED gate evidence.
- Exact production diff already specified by DESIGN (§ Decision — Exact Code Change): delete
  `server.rs:128-138`, extend the import at `server.rs:23`, change the call at `server.rs:161` to
  `.map_err(core_error_to_status)`.
- 11 acceptance scenarios (10 in `tests/acceptance/embyr_agent/us_a09_field_path_validation.rs` + 1
  cross-binary parity test), all confirmed RED for the right reason against current code.
- 1 modified regression-guard test (`us_a03_query_operations.rs`) whose payload was corrected ahead
  of the fix landing, to avoid a silent post-fix regression.
- DELIVER's job: apply DESIGN's exact 3-edit diff, watch all 6 RED tests turn GREEN, run
  `cargo test --test embyr_agent -p embyr-agent` and
  `cargo test --test agent_field_path_validation_afp06_cross_binary_parity -p embyr-server` to
  confirm, then the full-workspace `cargo test` once at the pre-commit gate per this repo's own
  token-discipline convention.
