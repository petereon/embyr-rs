# ADR-032: Collection-Group Rule Storage and Composition

## Status

Accepted

## Context

`security-rules` (Epic 2a, ADR-027/028/029) gave Alex per-collection read-path
enforcement on `GetDocument`. `security-rules-write-path` (Epic 2b, ADR-030)
extended the same mechanism to `CreateDocument`/`UpdateDocument`/
`DeleteDocument`. `security-rules-query-path` (Epic 2c, ADR-031) closed the
`RunQuery` bypass for ordinary, single-collection queries via
`check_query_compliance()`, consulting `access_rules` keyed by
`(project_id, collection_path)`.

None of the three touched `all_descendants = true` (collection-group)
queries. DISCUSS (`docs/feature/security-rules-collection-group-rules/
feature-delta.md`, Resolution 1 + Resolution 2) confirmed, by direct code
read, that `handle_run_query` derives `all_descendants` independently
(`sq_proto.from[0].all_descendants`) and threads it into the domain query for
data-plane execution (`embyr-pg-storage::backend_adapter::run_query`'s
already-correct `all_descendants` SQL branch), but the RULE lookup at the
same call site never branches on it — a collection-group query is checked,
if at all, against whatever `access_rules` row exists for the bare
`collection_id`, a row that (per Trailmark's own domain example,
`journal_entries` existing at both the top level and nested under
`expeditions/{id}/journal_entries`) was never written to govern the group as
a whole. This ADR implements DISCUSS's locked resolution, not re-litigates
it:

1. **A collection-group rule is a new, independently-authored,
   independently-stored rule concept** (Resolution 1, Option C) — never
   auto-applied from a same-named exact-path rule (Option A, affirmatively
   unsafe by direct construction — a caller satisfying the top-level rule's
   filter would be admitted into a query whose SQL also returns nested rows
   that rule never governed) and never execute-time-composed from every
   matching nested path's own rule (Option B, structurally intractable — the
   nested-path set is unknowable at query-planning time — and not
   well-defined even if it were, since two nested instances of the same
   collection id may legitimately use different field names for different
   entitlement concepts).
2. **A collection-group query against a collection id with no group rule is
   rejected outright** (Resolution 2, Option C), regardless of whether a
   same-named exact-path rule exists anywhere for that id. Confirmed against
   real Firestore's own documented behavior post-DISCUSS (`§ Handoff
   Package`, escalation resolved 2026-08-25): a regular per-exact-path
   `match` rule never governs a collection-group query in real Firestore
   either.
3. **`check_query_compliance()`/`QueryComplianceOutcome`/
   `UnsatisfiedConjunct` (ADR-031) are reused completely unchanged** — no
   new decidable shape, no new evaluator branch, zero modification to
   `embyr_core::access_control`.
4. **This feature reads/writes a new table only** — never `access_rules` or
   `write_access_rules`. `GetDocument`, write-path, and non-group `RunQuery`
   receive zero code changes (confirmed by DISCUSS's own direct reads: both
   already resolve rule lookups from the actual document/query's own
   fully-derived exact collection path, never a bare leaf id — they have no
   gap of this feature's kind).
5. **No change to `embyr-pg-storage`'s SQL-building layer** — its
   `all_descendants` branch already executes genuine collection-group
   queries correctly; the enforcement point is entirely upstream, in
   `grpc::handler::handle_run_query`'s rule-lookup composition, the
   identical class of finding ADR-031 made about its own feature.

This ADR combines the schema, adapter, composition, and Release-2 simulation
decisions into one ADR, mirroring ADR-030's and ADR-031's own
"smaller, bounded decision surface" precedent — each axis below is a small,
additive extension of an existing mechanism plus one genuinely new, narrow
storage/validation decision, not several independently wide option spaces.

## Decision Drivers

1. **US-04's "no group rule defined" default arm must reject, never
   silently fall back to a same-named exact-path rule and never silently
   allow** — this feature's single highest-consequence design risk.
   Designated mutation-testing surface (per-feature strategy, CLAUDE.md).
2. **Structural, not conventional, independence from `GetDocument`,
   writes, and non-group `RunQuery`** (US-05/06, AC-17-93/94/95/96) — a
   group rule's existence, content, or absence must have zero observable
   effect on any of the three existing surfaces, provable by disjoint
   storage and a disjoint, mutually-exclusive call-site branch — not merely
   by the absence of a test that would have caught interference.
3. **`check_query_compliance()`/`Condition`/`Operand`/`AuthContext` remain
   BC-4's sole grammar/type surface** (ADR-027, extended ADR-030/031) — this
   feature adds no new pure function and no new type to `embyr_core`.
4. **Simplest solution first** (Principle 8) — no new crate, no new
   dependency, no new bounded context, no new evaluation algorithm. The only
   genuinely new artifact is a third disjoint storage table and its
   adapter/admin-handler shape.
5. **Enforceable, not conventional, grammar containment for the new table's
   own structural invariant** — a collection-group id is, by construction, a
   bare identifier, never a path (Resolution 1). ADR-028's own
   `access_rules.collection_path` "single-segment in v1" constraint is a
   documented convention only, NOT code-enforced (confirmed by DISCUSS's
   direct read of `DefineAccessRuleBody` — no validation rejects a
   `/`-containing value). This ADR does not repeat that gap for the new
   table (see § Decision — Schema).
6. **No new overhead on the unarmed path** — a collection id with no group
   rule must cost exactly one indexed PK lookup, mirroring every prior
   epic's own NFR discipline; no scan of `access_rules` to decide the
   ungoverned-group default (AC-17-91).

## Decision — Schema

### New table (`migrations/0024_group_access_rules.sql`)

```sql
CREATE TABLE group_access_rules (
    project_id       TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    collection_id    TEXT NOT NULL CHECK (collection_id NOT LIKE '%/%'),
    condition_source TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, collection_id)
);
```

Schema-identical in column shape to `access_rules`/`write_access_rules`
(ADR-028/030 — `condition_source` stored as raw text, re-parsed per
evaluation, no active/previous/version columns, idempotent-upsert-only) with
two deliberate departures, both justified below, not copy-pasted from
precedent unexamined:

**Column named `collection_id`, not `collection_path`.** A collection-group
identifier is structurally never a path (Resolution 1) — naming the column
to match makes that distinction visible in the schema itself, not only in a
doc comment, the first of two structural (not conventional) enforcements of
this invariant.

**`CHECK (collection_id NOT LIKE '%/%')` — a genuinely new departure from
ADR-028/030's own precedent, not a blind mirror.** `access_rules.
collection_path`'s "single-segment collection id in v1" constraint is,
confirmed by DISCUSS's own direct code read, a documented convention only —
`DefineAccessRuleBody` has no validation rejecting a `/`-containing value.
For `access_rules`/`write_access_rules` that gap is low-consequence (a
multi-segment `collection_path` there is merely an unusual but
still-well-defined exact path). For `group_access_rules` it is not: a
`/`-containing "collection id" is a category error — it would silently
become an unreachable, permanently-ungoverned row (no real `RunQuery` ever
produces a `collection_id` containing `/`, so such a row could never
short-circuit anything, and its presence would misleadingly suggest a
protection that isn't real). This feature's own Earned-Trust discipline
(Principle 11 — enforceable rules, not conventions) is satisfied by a
two-layer defense: the admin handler validates and returns a distinguishable
400 (AC-17-80, the user-facing, friendly rejection); the CHECK constraint is
the second, DB-level layer that survives any future write path that might
bypass the handler (an admin fixup script, a future evolution feature, a
migration backfill) — cheap (a single `LIKE` predicate, evaluated only on
write, never on the hot read path), and it costs nothing on the existing
`access_rules`/`write_access_rules` tables, which are unmodified.

**Considered and rejected**: mirroring `access_rules`/`write_access_rules`
exactly (no CHECK constraint, convention-only) — rejected because it would
silently reproduce the exact code-vs-convention gap DISCUSS's own reading
flagged as evidence for why `DefineAccessRuleBody`'s "single-segment... in
v1" comment needed new, real validation work in the first place (§ Reading
Confirmation, `docs/feature/security-rules-collection-group-rules/
feature-delta.md`); it would have been the cheaper choice but not the
correct one for a table whose entire raison d'être is "bare identifier,
never a path."

### Adapter (`crates/embyr-server/src/adapters/system_db.rs`, extended)

```rust
/// security-rules-collection-group-rules (ADR-032): a project's per-
/// collection-id COLLECTION-GROUP access-control rule, as stored in
/// `group_access_rules` — a table structurally independent of both
/// `access_rules` (read, exact-path) and `write_access_rules` (write,
/// exact-path). Schema-identical shape to both, deliberately a separate
/// type (mirrors `WriteAccessRuleRow`'s own precedent of not sharing a type
/// with `AccessRuleRow`).
pub struct GroupAccessRuleRow {
    pub condition_source: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

`upsert_group_access_rule(project_id, collection_id, condition_source)` and
`get_group_access_rule(project_id, collection_id) -> Option<GroupAccessRuleRow>`
mirror `upsert_write_access_rule`/`get_write_access_rule`'s exact shape —
single `INSERT ... ON CONFLICT (project_id, collection_id) DO UPDATE`,
single indexed `SELECT ... WHERE project_id = $1 AND collection_id = $2`,
identical `CoreError::BackendUnavailable` error mapping, identical `Ok(None)`
short-circuit contract. `get_group_access_rule`'s `Ok(None)` is this
feature's own version of ADR-029's structural no-rule-defined guardrail —
but with the OPPOSITE default from `get_access_rule`/`get_write_access_rule`
(Resolution 2: reject, not "unrestricted") — see § Decision — Composition.

## Decision — Composition (Placement in `handle_run_query`)

### Exact insertion point (`crates/embyr-server/src/grpc/handler.rs::handle_run_query`)

The existing, unmodified rule-lookup block (ADR-031 § Decision —
Composition, step 3 — `get_access_rule` on `access_rules`, `if let Some(...)
{ ... }`) is wrapped in a branch on the ALREADY-COMPUTED `all_descendants`
local (line ~1172 today, extracted from `sq_proto.from[0].all_descendants`,
unchanged extraction logic):

```rust
if all_descendants {
    // security-rules-collection-group-rules (ADR-032): a WHOLLY NEW,
    // mutually-exclusive arm. Reads group_access_rules ONLY — never
    // access_rules — the structural (not conventional) mechanism behind
    // AC-17-93/94/95/96.
    let group_rule_row = self
        .system_db
        .get_group_access_rule(&project_id_str, &collection.collection_path)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    let Some(group_rule_row) = group_rule_row else {
        // US-04, Resolution 2 (CONFIRMED universal fail-closed default):
        // no group rule defined -> reject outright, regardless of any
        // same-named exact-path rule's existence. This feature's single
        // highest-consequence arm (designated mutation-testing surface).
        return Err(group_rule_not_defined_rejection());
    };

    let condition = embyr_core::access_control::parse_condition(&group_rule_row.condition_source)
        .map_err(|e| {
            Status::internal(format!("stored group access rule failed to re-parse: {e:?}"))
        })?;
    let auth_ctx = verified_identity
        .as_ref()
        .map(|v| embyr_core::access_control::AuthContext { uid: v.end_user_id.clone() });

    // SAME check_query_compliance() and SAME query_compliance_rejection()
    // real, non-group enforcement uses (ADR-031) — never a second,
    // independently-maintained shape-compliance path.
    match embyr_core::access_control::check_query_compliance(
        &condition,
        domain_query.filter.as_ref(),
        auth_ctx.as_ref(),
    ) {
        embyr_core::access_control::QueryComplianceOutcome::Admitted => {}
        outcome => return Err(query_compliance_rejection(&outcome)),
    }
} else {
    // EXISTING ARM (ADR-031), PRESERVED UNCHANGED. Reads access_rules
    // ONLY. `None` -> reached completely unmodified, the EXACT
    // pre-security-rules-collection-group-rules code path.
    let rule_row = self
        .system_db
        .get_access_rule(&project_id_str, &collection.collection_path)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    if let Some(rule_row) = rule_row {
        let condition = embyr_core::access_control::parse_condition(&rule_row.condition_source)
            .map_err(|e| {
                Status::internal(format!("stored access rule failed to re-parse: {e:?}"))
            })?;
        let auth_ctx = verified_identity
            .as_ref()
            .map(|v| embyr_core::access_control::AuthContext { uid: v.end_user_id.clone() });

        match embyr_core::access_control::check_query_compliance(
            &condition,
            domain_query.filter.as_ref(),
            auth_ctx.as_ref(),
        ) {
            embyr_core::access_control::QueryComplianceOutcome::Admitted => {}
            outcome => return Err(query_compliance_rejection(&outcome)),
        }
    }
}
```

**Why branch, not a parallel/duplicate check**: `all_descendants` is
mutually exclusive by construction (a `StructuredQuery` either targets a
single collection instance or the whole group, never both) — an `if`/`else`
is the structurally correct shape, not two independent checks that could
both fire or both be skipped.

**Why the `else` arm is the existing block verbatim, not refactored**: the
non-group arm's own behavior (which table it reads, its "no rule ⇒
unrestricted" default) must remain structurally unreachable from any
group-rule state (AC-17-93/94/95/96) — moving it unmodified into the `else`
branch, rather than rewriting it to share logic with the new `if` arm,
keeps that guarantee provable by inspection (disjoint table per arm,
mutually exclusive condition), not by a shared code path's own correctness.

**Left to software-crafter (HOW, not WHAT — Principle 2)**: whether the
~8-line compliance-evaluation sequence common to both arms (parse the stored
condition, build the `AuthContext`, call `check_query_compliance`, map a
non-`Admitted` outcome to a `Status` via `query_compliance_rejection`) is
factored into a small private helper shared by both arms, or left
duplicated per-arm, is an implementation-level decision. Either satisfies
the structural non-interference invariant this ADR requires, because the
invariant is about which TABLE each arm's `Option<Row>` comes from, not
about whether the arms share a helper function once a row has already been
found.

### `GROUP_RULE_NOT_DEFINED` is not a `QueryComplianceOutcome` variant

`check_query_compliance()` only ever receives an ALREADY-PARSED `Condition`
— it has no way to represent "there was no rule at all," and this ADR does
not give it one, per Decision Driver 3 (`embyr_core::access_control` gains
zero new types). The "no group rule" rejection is decided entirely inside
`grpc::handler::handle_run_query`'s composition code, before
`parse_condition`/`check_query_compliance` are ever reached — mirroring how
`get_access_rule`/`get_write_access_rule`'s own `None` short-circuits
already work (ADR-029), just with the opposite default:

```rust
fn group_rule_not_defined_rejection() -> Status {
    Status::permission_denied(
        "query rejected [GROUP_RULE_NOT_DEFINED]: no collection-group rule \
         is defined for this collection id",
    )
}
```

The bracketed `[GROUP_RULE_NOT_DEFINED]` token follows the identical
message-string-only, grep/substring-checkable convention `query_compliance_
rejection` already established (ADR-031 § Decision — Rejection Response
Shape) — distinguishable from `UNSUPPORTED_RULE_SHAPE`/
`OWNERSHIP_FILTER_MISSING`/`AUTH_REQUIRED`/`RULE_DENIES_ALL` (Slice 03's
rejections, AC-17-92) by reason code, and from `authenticate()`-level and
composite-index rejections by gRPC status code, exactly as the non-group
case already is. The literal string is duplicated (not centralized behind a
shared constant) between this function and the Release-2 simulation
handler's own reasons vocabulary — mirrors this codebase's own existing,
unexamined precedent for `"UNSUPPORTED_RULE_SHAPE"` (confirmed by direct
read: it appears as an inline literal in both `query_compliance_rejection`
and `simulate_query_compliance` today, not behind a shared `const`) — this
ADR does not invent a new centralization mechanism the codebase does not
already use for the sibling case.

### Composite-index-check ordering (OQ-SRQ-03 precedent) — confirmed to hold

ADR-031 established that the compliance check must run strictly BEFORE the
existing `requires_composite_index`/`is_index_ready` check, to avoid
revealing index topology to a caller never entitled to query the collection
at all. That check's own code is untouched and remains positioned
immediately after the `if all_descendants { ... } else { ... }` block above.
Both arms either continue past the block (compliant) or return early
(non-compliant, unsupported shape, or no group rule) — by the time
execution would reach the composite-index check, EITHER arm's compliance
decision has already been made, so the ordering guarantee holds identically
for group and non-group queries without any new ordering logic.

## Decision — Admin Surface

### `define_group_access_rule` (US-01)

New handler in `crates/embyr-server/src/admin/handlers/access_rules.rs`
(same file — the module already hosts three sibling rule-definition/
simulation handlers; a fourth, closely-related one belongs alongside them,
not in a new file). Mirrors `define_write_access_rule`'s exact shape (Owner/
Admin role gate, `verify_project_ownership`, `parse_condition` validation
before storage, `upsert_group_access_rule`, same no-branch
first-time-vs-redefine response shape) plus ONE new step, run BEFORE
`parse_condition` (AC-17-80's domain example: an invalid collection id is
rejected independent of the condition's own validity):

```rust
fn validate_bare_collection_id(id: &str) -> Result<(), Response> {
    if id.contains('/') {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ConditionRejectionResponse {
                reason: "INVALID_COLLECTION_ID",
                error: "a collection-group id must be a bare collection \
                        identifier, not a path"
                    .to_string(),
            }),
        )
            .into_response());
    }
    Ok(())
}
```

Reuses the EXISTING `ConditionRejectionResponse` struct verbatim (already
generic — `{reason: &'static str, error: String}`) rather than inventing a
new response type for this one new rejection reason. Confirmed by targeted
search: no existing collection-id validator exists anywhere in
`embyr-server` (`validate_bare_collection_id` is genuinely new, not a
duplicate of anything).

Request/response types (`DefineGroupAccessRuleBody { collection_id,
condition }`, `GroupAccessRuleResponse { project_id, collection_id,
condition, created_at, updated_at }`) mirror `DefineWriteAccessRuleBody`/
`WriteAccessRuleResponse` exactly, field-renamed `collection_path` ->
`collection_id` to match § Decision — Schema's column-naming rationale.

**Route**: `POST /admin/v1/projects/:project_id/group_access_rules` —
follows the existing `access_rules`/`write_access_rules` naming convention
(resource-name-matches-table-name), registered in
`admin::router::build_admin_router`'s session sub-router alongside the other
three.

### `simulate_group_query_compliance` (US-07) — a genuine, evaluated departure from both DISCUSS's Technical Note and ADR-031's own precedent

DISCUSS's own (non-binding) Technical Note suggested extending
`simulate_query_compliance` in place with "an optional 'this is a
group-rule simulation' input." This ADR does not do that, but also does not
blindly re-apply ADR-031's own conclusion (a wholly new request AND
response type) unexamined — the two contracts are evaluated independently,
because they do not, in fact, both differ.

**The RESPONSE contract is identical, and is reused verbatim.**
`SimulateQueryComplianceResponse { compliant: bool, reasons:
Vec<&'static str> }` already expresses everything US-07's own AC require:
admit/reject (AC-17-101), "missing required filter" (AC-17-102), and — once
`GROUP_RULE_NOT_DEFINED` is added to the reasons vocabulary as a plain
string, per § Decision — Composition above — "no collection-group rule
defined" (AC-17-103). No new response struct is created.

**The REQUEST contract is genuinely different, and does need a new type.**
`simulate_query_compliance`'s existing `condition: String` is REQUIRED — the
handler exists specifically to test a caller-authored CANDIDATE rule; there
has never been, and is not now, a scenario where testing "what if no rule
existed" is meaningful for the non-group case, because non-group's own real
default (no rule -> unrestricted) is never a rejection worth simulating.
Collection-group simulation is different: US-04's "ungoverned" default IS
this feature's single highest-consequence, always-relevant scenario
(AC-17-103), so it must be a first-class, directly simulatable case — making
the candidate condition OPTIONAL:

```rust
#[derive(Deserialize)]
pub struct SimulateGroupQueryComplianceBody {
    /// `None` simulates the US-04 "no collection-group rule defined"
    /// default directly — a first-class candidate scenario, not an error.
    /// Reuses `SimulatedAuth`/`SimulatedQueryFilter` verbatim (unchanged).
    pub group_condition: Option<String>,
    pub auth: Option<SimulatedAuth>,
    #[serde(default)]
    pub query_filters: Vec<SimulatedQueryFilter>,
}
```

**Considered and rejected: making `simulate_query_compliance`'s own
`condition` field optional in place, reusing one handler for both.**
Rejected for the same class of reason ADR-030's DDD-SRW-6 rejected a
`rule_type`-discriminated single handler: the `None` case would need a
DIFFERENT resolved meaning depending on which "kind" of simulation the
caller intended (for the existing non-group handler, "no condition
supplied" has never had — and should not need — any defined meaning at all;
for the group case, it is a specific, load-bearing scenario). Overloading
one field's absence with two different semantic universes reintroduces
exactly the runtime-behavior-selection-by-optional-field risk ADR-030
already rejected once in this same module, even though — unlike ADR-031's
own precedent — the RESPONSE type here would not have needed to change to
do it. The response-type identity is real and is honored (by reuse, not
duplication); the request-semantics difference is also real and is honored
(by a new sibling type, not an overloaded optional field on the existing
one).

Handler body: `verify_project_ownership` (any role, no role gate — mirrors
`simulate_query_compliance`'s identical any-role, read-only precedent) ->
if `group_condition` is `None`, return `{compliant: false, reasons:
["GROUP_RULE_NOT_DEFINED"]}` directly, matching US-04's real default,
without ever touching `group_access_rules` (zero storage read, mirroring
`simulate_query_compliance`'s own "never read from or written to
access_rules" discipline for the candidate case) -> else `parse_condition`
(same `condition_parse_error_response` taxonomy) -> `translate_query_filters`
(reused verbatim) -> `check_query_compliance` (reused verbatim) -> same
outcome-to-`(compliant, reasons)` mapping `simulate_query_compliance`
already uses.

**Route**: `POST /admin/v1/projects/:project_id/access_rules/
simulate_group_query` — sibling to `.../access_rules/simulate_query`,
registered alongside it. The `/access_rules/` prefix denotes this
codebase's existing "access-control simulation namespace" (already true of
`simulate_query_compliance`, which simulates `RunQuery` compliance, not a
literal `access_rules` row, despite living under that prefix) — not a claim
about which table the simulated rule is stored in.

## Consequences

### Positive

- The collection-group `RunQuery` bypass is closed using the identical
  compliance mechanism (`check_query_compliance`) real, single-collection
  query enforcement already uses — zero new decidable-shape logic, zero new
  evaluator branch, zero risk of the two mechanisms drifting apart.
- `access_rules`, `write_access_rules`, `get_access_rule`,
  `get_write_access_rule`, `handle_get_document`, and every write-path
  handler receive ZERO code changes — verifiable by diff.
- `embyr-pg-storage::backend_adapter::run_query`'s SQL-building layer
  receives ZERO code changes.
- The "no group rule" reject-default (Decision Driver 1) is reached via an
  explicit `let...else` early return, not an omitted branch — a missing
  case is a compile error, not a silent runtime gap.
- Collections with no group rule pay exactly one new indexed PK lookup on
  the group-query path only (zero added cost on the non-group path, which
  is untouched) — the identical "no overhead when unarmed" guarantee every
  prior epic already established.
- The new table's bare-id invariant is enforced at two independent layers
  (handler validation for a friendly response, DB CHECK for defense in
  depth) — closing, for this table specifically, the code-vs-convention gap
  DISCUSS's own reading found in `access_rules`' equivalent constraint.

### Negative / Trade-offs

- A small amount of duplicated compliance-evaluation logic (~8 lines) now
  exists between the group and non-group arms of `handle_run_query`,
  because refactoring the non-group arm to share a helper — while still
  structurally safe — was deliberately left as a crafter-level (HOW)
  choice rather than mandated here, to avoid this ADR prescribing
  implementation detail beyond its own architectural contract.
- `"GROUP_RULE_NOT_DEFINED"` is a duplicated string literal across two call
  sites (gRPC rejection, simulation JSON), consistent with — but not an
  improvement over — this codebase's existing precedent for
  `"UNSUPPORTED_RULE_SHAPE"`. A future ADR could centralize the whole
  reason-code vocabulary behind shared constants project-wide if evidence
  warrants; this ADR does not unilaterally invent that mechanism for
  itself alone, mirroring ADR-031's own identical reasoning.
- A third table with the identical idempotent-upsert, no-history shape as
  `access_rules`/`write_access_rules` means Alex has no way to audit when a
  group rule was previously different — unchanged, deliberate scope
  boundary carried from ADR-028 (`security-rules-operations`, Epic 2e,
  remains the deferred home for rule history/versioning).

## Enforcement

Style: Hexagonal (ports-and-adapters), unchanged project-wide pattern. No
new crate, no new bounded context — BC-4 Access Control (ADR-029) gains a
third disjoint storage table and one new mutually-exclusive branch in one
existing `grpc::handler` call site (`handle_run_query`), plus two new admin
call sites. `embyr_core::access_control` gains nothing.

Rules enforced (existing, applying unchanged):

- `embyr-core::access_control` retains zero IO imports (`cargo-deny`,
  `deny.toml`) — this feature adds no code to that module at all.
- `embyr-core` defines the value-type/function surface; `embyr-server`
  consumes it — dependency direction inward, unchanged. `group_access_rules`
  and its adapter methods are entirely an `embyr-server` concept;
  `embyr-core` has no knowledge that collection-group rules exist.
- New: `group_access_rules.collection_id`'s `CHECK (collection_id NOT LIKE
  '%/%')` constraint — a DB-level enforcement layer for an invariant the
  admin handler also enforces, per § Decision — Schema's explicit two-layer
  reasoning (mirrors this project's own Earned Trust discipline, Principle
  11/12, applied here to a domain invariant rather than a substrate-lie
  probe — there is no external substrate this feature newly depends on, see
  below).

**No new driven port, no new Earned Trust probe (Principle 12 discipline,
explicit reasoning required, mirroring ADR-029/030/031 § Enforcement
verbatim):**

- `get_group_access_rule`/`upsert_group_access_rule` (new call sites, new
  methods, identical shape to existing ones) execute through the existing,
  already-probed `SystemDb` connection pool — the identical substrate
  `get_access_rule`/`get_write_access_rule` already use.
- `check_query_compliance`/`decompose_decidable`/`filter_binds_field_to_uid`
  are unchanged, pure, deterministic CPU computation — this feature adds no
  new call into them beyond a second call site with a different rule
  source, already covered by ADR-031's own Earned Trust reasoning.
- The substrate this feature adds new reliance on is exactly zero — no
  filesystem, network, subprocess, clock, or vendor-SDK dependency
  anywhere in this feature's own call graph.

`cargo-deny`/`deny.toml` unaffected — no new workspace dependency.

## References

- `docs/feature/security-rules-collection-group-rules/feature-delta.md` §
  Job Discovery — Framing Resolution (Resolutions 1-2), § System
  Constraints, § Handoff Package (including the 2026-08-25 escalation
  resolution confirming Resolution 2's universal reading against real
  Firestore's own documented behavior), § User Stories (US-01 through
  US-07), § Reading Confirmation.
- `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md`,
  `adr-028-access-rule-storage-and-lifecycle.md`,
  `adr-029-access-control-composition-and-bounded-context.md`,
  `adr-030-write-path-grammar-storage-and-composition.md`,
  `adr-031-query-shape-compliance-check.md` — the machinery this ADR
  extends (types, storage pattern, composition/rejection-shape/simulation
  precedent), not replaces.
- `crates/embyr-core/src/access_control/mod.rs` (full, read during DESIGN)
  — confirmed the exact current `check_query_compliance`/
  `QueryComplianceOutcome`/`UnsatisfiedConjunct`/`query_compliance_rejection`
  shapes this ADR reuses verbatim; confirmed zero new type or function is
  needed here.
- `crates/embyr-server/src/grpc/handler.rs::handle_run_query` (targeted full
  read, lines 1131-1290) — the exact current composition (identity attach,
  structured-query translation, `collection`/`domain_query` construction,
  the existing unconditional `get_access_rule` block, the composite-index
  check, `adapter.run_query()`) this ADR's branch is inserted into.
- `crates/embyr-server/src/adapters/system_db.rs` (`AccessRuleRow`/
  `WriteAccessRuleRow`, `get_access_rule`/`upsert_access_rule`,
  `get_write_access_rule`/`upsert_write_access_rule`) — the exact adapter-
  method-shape precedent `GroupAccessRuleRow`/`get_group_access_rule`/
  `upsert_group_access_rule` mirror.
- `crates/embyr-server/src/admin/handlers/access_rules.rs` (full) — exact
  current `define_write_access_rule`/`simulate_query_compliance`/
  `ConditionRejectionResponse`/`translate_query_filters`/
  `json_value_to_field_value` shapes, the direct structural precedent for
  `define_group_access_rule`/`simulate_group_query_compliance`; confirmed no
  existing bare-collection-id validator exists anywhere in this file or
  crate (targeted search, zero matches).
- `crates/embyr-server/src/admin/router.rs` (targeted read, lines 190-234)
  — the exact current route-registration shape and ordering the two new
  routes are added alongside.
- `migrations/0022_access_rules.sql`, `migrations/0023_write_access_rules.sql`
  — the exact schema precedent `migrations/0024_group_access_rules.sql`
  mirrors, with the two deliberate departures documented in § Decision —
  Schema.

## Changed Assumptions

**Original assumption** (§ Consequences — Negative / Trade-offs, above): "A
third table with the identical idempotent-upsert, no-history shape as
`access_rules`/`write_access_rules` means Alex has no way to audit when a
group rule was previously different — unchanged, deliberate scope boundary
carried from ADR-028 (`security-rules-operations`, Epic 2e, remains the
deferred home for rule history/versioning)."

**Resolved by**: `security-rules-operations` (Epic 2e),
`docs/product/architecture/adr-035-access-rule-history-storage-and-capture-mechanism.md`.
A new, independently-stored `group_access_rule_history` table — mirroring
this ADR's own `collection_id`+`CHECK (collection_id NOT LIKE '%/%')`
departure exactly — now captures every condition `group_access_rules` has
ever held, via a history `INSERT` fused into the SAME
`upsert_group_access_rule` transaction. This ADR's own `group_access_rules`
schema, `CHECK` constraint, `handle_run_query` composition, and structural
independence from `access_rules`/`write_access_rules` are all unchanged by
that feature; the deferred gap named above is now closed, not this ADR's own
decision revised.
