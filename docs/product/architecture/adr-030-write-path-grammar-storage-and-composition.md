# ADR-030: Write-Path Access Rule Grammar Extension, Independent Storage, and Handler Composition

## Status

Accepted

## Context

`security-rules` (Epic 2a, ADR-027/028/029) gave Alex per-collection read-path
enforcement on `GetDocument` only. `security-rules-write-path` (Epic 2b) closes the
gap DISCUSS named explicitly: `CreateDocument`/`UpdateDocument`/`DeleteDocument`
never consult `access_rules` at all today (confirmed by direct code read,
`grpc/handler.rs:618-760` — none of the three write handlers calls
`attach_client_identity_if_present` or any rule-lookup method).

DISCUSS (`docs/feature/security-rules-write-path/feature-delta.md`, Resolution 1 +
Resolution 2) locked two things this ADR must implement, not re-litigate:

1. **Two independent condition slots per collection** — the existing read
   condition (`security-rules`, unchanged) and a new write condition. A
   collection's read rule must have **zero** effect on writes unless a separate
   write rule is explicitly defined (AC-17-43, this feature's single
   highest-consequence guardrail). DISCUSS explicitly left the storage shape
   (new column vs. new table) to DESIGN, with the requirement that independence
   be "genuinely structural, not just conventional."
2. **`request.resource` as a new grammar operand family** — `request.resource.data.<field>`
   (proposed new state) coexists with the existing `resource.data.<field>`
   (pre-write state), reusing ADR-027's fail-closed-on-missing-field mechanism
   verbatim. No new null-sentinel operand.

This ADR combines what would otherwise be three ADRs (mirroring ADR-027/028/029's
split) into one, per the smaller decision surface DISCUSS's own Handoff Package
anticipated: each of the three axes below is a bounded extension of an existing,
already-accepted ADR, not a fresh architectural choice with a wide option space.

## Decision Drivers

1. **AC-17-43 structurally, not conventionally, true** — the single most
   consequence-heavy requirement in this feature. Any design where a write-rule
   redefinition could, through a shared column, shared row, or shared code path,
   possibly perturb read-rule behavior (or vice versa) is unacceptable regardless
   of test coverage.
2. **No modification to `security-rules`' already-shipped, review-approved code**
   (`access_rules` table, `get_access_rule`/`upsert_access_rule`,
   `handle_get_document`'s existing guardrail logic) — per this feature's own
   scope boundary ("Do NOT re-open any part of `security-rules`' already-shipped
   read-path scope").
3. **`evaluate()`/`parse_condition()` remain BC-4's sole shared evaluation
   routine** (ADR-029 DDD-SR-8) — write-path enforcement (US-02/03/04) and
   write-rule simulation (US-07) must call the identical extended function, not a
   second, independently-maintained copy. This is also this feature's own
   mutation-testing surface (per-feature mutation strategy, CLAUDE.md) — a single
   extended `evaluate()` concentrates that surface in one place instead of
   duplicating it.
4. **No regression to collections/traffic with no write rule defined** (AC-17-42)
   — mirrors ADR-029's `get_access_rule() -> None` short-circuit shape exactly,
   now applied to the new write-condition store.
5. **Existence non-leakage extends to update/delete** (AC-17-34/38) — reuses
   ADR-029's exact "evaluate unconditionally against real-or-empty fields, `Deny`
   always identical" mechanism, not a second non-leakage mechanism.
6. **Simplest solution first** (Principle 8) — no new crate, no new dependency, no
   speculative whole-object presence sentinel (OQ-SRW-01, deferred).

## Decision — Grammar Extension

### `Operand` enum (extends ADR-027, `crates/embyr-core/src/access_control/mod.rs`)

```rust
pub enum Operand {
    AuthUid,
    AuthNullSentinel,
    ResourceField(String),
    RequestResourceField(String),   // NEW
    BoolLiteral(bool),
    NullLiteral,
}
```

No tokenizer change is required: `tokenize()`'s existing word-character class
(`is_ascii_alphanumeric() || c == '_' || c == '.'`) already consumes
`request.resource.data.owner_id` as a single `Word` token — identical to how it
already consumes `resource.data.owner_id`. Only `word_to_operand()` gains one new
match arm:

```rust
w if w.starts_with("request.resource.data.") => {
    let field = &w["request.resource.data.".len()..];
    if field.is_empty() { return Err(syntax_error(...)); }
    Ok(Operand::RequestResourceField(field.to_string()))
}
```

Ordering relative to the existing `w.starts_with("resource.data.")` arm is
irrelevant — `"request.resource.data."` and `"resource.data."` are non-overlapping
prefixes; both arms coexist with zero risk of misclassification.

`detect_unsupported_construct()` (the `**`/`{`/call-syntax pre-tokenize check) is
unaffected — it operates on structural markers unrelated to the new operand text.

**Explicitly not added** (per DISCUSS's own scoping): a bare `request.resource`
sentinel (whole-object presence check). Only the `.data.<field>` form is
grammar-legal, mirroring `resource.data.<field>`'s own restriction. Flagged as
OQ-SRW-01 (carried from DISCUSS), not built speculatively.

### `evaluate()` signature (extends ADR-027)

```rust
pub fn evaluate(
    condition: &Condition,
    auth: Option<&AuthContext>,
    resource_fields: &BTreeMap<String, FieldValue>,
    request_resource_fields: &BTreeMap<String, FieldValue>,   // NEW parameter
) -> EvaluationOutcome
```

This is a **signature extension of the existing function**, not a new function —
the single decision this ADR treats as most load-bearing for Decision Driver 3.
`eval_bool`/`compare_operands`/`resolve_field_value` thread the new
`request_resource_fields` map alongside the existing `resource_fields` map, with
two additions:

- `resolve_field_value` gains `Operand::RequestResourceField(name) =>
  request_resource_fields.get(name).cloned().ok_or(FieldMissing)` — the identical
  fail-closed shape `ResourceField` already has, against the other map.
- `compare_operands` gains a `(Operand::AuthUid, Operand::RequestResourceField(name))`
  / reverse pairing, mirroring the existing `AuthUid`/`ResourceField` owner-check
  pairing exactly (needed by US-02's `request.resource.data.owner_id ==
  request.auth.uid` domain example).

**No special-casing needed for `resource.data.<field> == request.resource.data.<field>`**
(the immutable-field pattern, US-03): this pairing is NOT one of the named special
pairs above, so it falls through to the existing generic arm — both operands
resolved independently via `resolve_field_value` against their respective maps,
then compared via `FieldValue::PartialEq`. This is the exact "compose cleanly with
the existing `ResourceField` variant, no new evaluator branch type" property
Resolution 2 requires, confirmed structurally rather than asserted.

**Fail-closed semantics, unchanged and now doing double duty:**

- **Create**: `resource_fields` = empty map (no document exists yet) →
  ANY reference to `resource.data.<field>` denies via the existing
  `FieldMissing` short-circuit (AC-17-28). `request_resource_fields` = the
  already-in-memory proposed fields — no I/O.
- **Update**: both maps populated from real data (pre-write fetch +
  in-memory proposed fields) — the two-value comparison (AC-17-30/31/32).
- **Delete**: `request_resource_fields` = empty map (no proposed new state) →
  any reference to `request.resource.data.<field>` denies via the identical
  mechanism (AC-17-37). `resource_fields` populated from the pre-write fetch.
- **Read** (`handle_get_document`, unchanged behavior required): passes its
  existing `resource_fields` argument unchanged, and an **empty map** for the new
  `request_resource_fields` parameter — a `GetDocument` call has no "proposed new
  document" concept at all. Any read rule that happened to reference
  `request.resource.data.<field>` (grammar-legal but semantically nonsensical for
  a read) denies via the same fail-closed mechanism — never a crash, never a
  behavior change for any rule that (as all `security-rules`-era rules do) never
  references the new operand family at all.

This is the **one, minimal, mechanical touch** to `handle_get_document`'s existing
`evaluate()` call site this feature makes: one new argument (`&EMPTY_FIELDS`,
reusing the same empty-map sentinel pattern ADR-029 already established for
non-existent documents), zero change to any branch, condition, or response shape
in that function. `handle_get_document` continues to work exactly as
`security-rules` left it, per this feature's own scope boundary.

**Rejected alternative: a second function (`evaluate_write()`).** Duplicates the
entire fail-closed evaluation tree. Directly violates Decision Driver 3
(ADR-029 DDD-SR-8's no-duplication guarantee) and concentrates this feature's
mutation-testing surface in two places instead of one, doubling the risk of
undetected drift between real enforcement and simulation.

## Decision — Storage Shape

### Considered Options

**Option A: Add nullable `write_condition_source`/`write_created_at`/`write_updated_at` columns to the existing `access_rules` table.**

Rejected. A collection may legitimately have a write rule with **no** pre-existing
read rule (e.g., a first-time write-only protection on `app_config`, which never
had a `security-rules` read rule at all — not demonstrated in DISCUSS's own
domain examples but structurally required by AC-17-20's general "first-time
write-rule definition… for the named collection," which does not require a
pre-existing read rule). Supporting this case within Option A requires
`upsert_write_access_rule` to be able to `INSERT` a new `access_rules` row when
none exists — which means the existing `condition_source TEXT NOT NULL` column
must either (a) become nullable, requiring `AccessRuleRow.condition_source` to
become `Option<String>` and `handle_get_document`'s existing `rule_row.is_some()`
short-circuit to be rewritten as a check on `condition_source.is_some()`
specifically — a direct modification to the exact, already-shipped,
review-approved AC-17-14/15/16 structural guardrail Decision Driver 2 forbids
touching — or (b) be seeded with a placeholder sentinel value on write-only
insert, which is precisely the kind of contamination Resolution 1 exists to
prevent (a write-only insert would have to write *something* meaningful-looking
into the read-condition column). Either sub-option fails Decision Driver 2 or
Decision Driver 1. Rejected.

**Option B: A new, independent table — Accepted.**

`write_access_rules`, schema-identical in shape to `access_rules` (ADR-028),
keyed on the same `(project_id, collection_path)` composite but living in a
wholly separate table with its own primary key, its own rows, and its own
adapter methods.

**Accepted.** This is the strongest available structural independence guarantee:

- A collection can have a row in `access_rules` only (read-only, `trail_guides`'
  US-06 case), a row in `write_access_rules` only (write-only, no pre-existing
  read rule — the case Option A cannot cleanly support), rows in both
  (`journal_entries`), or neither (`app_config`, fully untouched, AC-17-42).
- Redefining a write rule (`upsert_write_access_rule`) is a statement against
  `write_access_rules` **only** — it has no `access_rules` in its `FROM`/`INTO`
  clause at all, structurally, not conventionally. The reverse holds for
  `upsert_access_rule`/`access_rules`, entirely unmodified by this feature.
- `access_rules`, `AccessRuleRow`, `get_access_rule`, `upsert_access_rule`, and
  `handle_get_document`'s existing guardrail branch require **zero** code
  changes — satisfying Decision Driver 2 completely, not just approximately.
- Mirrors ADR-028's own accepted schema shape almost verbatim (Reuse Analysis:
  EXTEND on the adapter-method *pattern*, CREATE NEW on the table itself — the
  identical classification ADR-028 gave `access_rules` relative to
  `client_identity_credentials`).

### Schema — `write_access_rules` table (`migrations/0023_write_access_rules.sql`)

| Column | Type | Notes |
|--------|------|-------|
| `project_id` | `TEXT NOT NULL REFERENCES projects(id)` | Part of composite PK |
| `collection_path` | `TEXT NOT NULL` | Part of composite PK. Same single-segment v1 scope as `access_rules` |
| `condition_source` | `TEXT NOT NULL` | Raw grammar source, validated (`parse_condition`, extended per § Decision — Grammar Extension) at write time |
| `created_at` | `TIMESTAMPTZ NOT NULL DEFAULT now()` | Set once, on first insert |
| `updated_at` | `TIMESTAMPTZ NOT NULL DEFAULT now()` | Bumped on every define/redefine |

`PRIMARY KEY (project_id, collection_path)`. No `active`/`previous`/`version`
columns, no shared columns with `access_rules` — same "no history/versioning
machinery" discipline ADR-028 Decision Driver 5 already established, applied
fresh.

### Adapter methods (`embyr-server::adapters::system_db`, extends `SystemDb`)

```rust
pub struct WriteAccessRuleRow {
    pub condition_source: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

async fn upsert_write_access_rule(
    &self, project_id: &str, collection_path: &str, condition_source: &str,
) -> Result<(), CoreError>;

async fn get_write_access_rule(
    &self, project_id: &str, collection_path: &str,
) -> Result<Option<WriteAccessRuleRow>, CoreError>;
```

Identical shape to `upsert_access_rule`/`get_access_rule` (`INSERT ... ON CONFLICT
(project_id, collection_path) DO UPDATE SET condition_source = EXCLUDED.condition_source,
updated_at = now()`), operating against `write_access_rules` exclusively.
`get_write_access_rule` returning `None` is this feature's own version of
ADR-029's structural no-rule-defined guardrail (see § Decision — Composition).

## Decision — Composition

### Three new call sites, one shared pattern

`handle_create_document`, `handle_update_document`, `handle_delete_document`
(`grpc/handler.rs:618-760`) each gain the following sequence, inserted after the
existing `authenticate()`/rate-limit checks and before the existing
`adapter.{create,update,delete}_document(...)` call:

1. **Identity attach (NEW call site, function unchanged)**:
   `let verified_identity = self.attach_client_identity_if_present(&request, &project_id_str).await;`
   — identical call shape to `handle_get_document`'s own (US-05). Zero
   modification to `attach_client_identity_if_present` itself.
2. **Write-rule lookup (NEW, queries `write_access_rules` only)**:
   `let write_rule_row = self.system_db.get_write_access_rule(&project_id_str, &path.collection_path).await.map_err(...)?;`
   — a single indexed PK lookup, the identical cost/shape as
   `get_access_rule`'s existing call in `handle_get_document`. **`None`** →
   proceed to the existing `adapter.*_document(...)` call, **completely
   unmodified** (AC-17-42). This is the structural mechanism behind AC-17-43:
   this lookup never reads `access_rules`, so a collection's read rule is
   structurally invisible to this code path regardless of whether it exists.
3. **Pre-write state (only when `write_rule_row` is `Some`, and only for
   update/delete)**:
   - **Create**: `resource_fields` = empty `BTreeMap` (no fetch, no I/O — the
     document does not exist yet).
   - **Update**: `let doc_opt = adapter.get_document(&path).await.map_err(...)?;`
     — reuses the existing `BackendAdapter::get_document` trait method (already
     used by `handle_get_document`, already probed at composition root — no new
     port). `resource_fields = doc_opt.as_ref().map(|d| &d.fields).unwrap_or(&EMPTY_FIELDS)`.
   - **Delete**: identical fetch to Update.
   This fetch is paid **only** when a write rule is defined for the target
   collection (gated behind step 2's cheap existence check) — collections with no
   write rule pay zero additional I/O, preserving AC-17-42's "exactly as before"
   guarantee at the performance level, not just the correctness level.
4. **Proposed new state**:
   - **Create/Update**: `request_resource_fields = fields.clone()` — the proposed
     document fields already parsed from the request body earlier in the
     handler; no new I/O.
   - **Delete**: `request_resource_fields` = empty `BTreeMap` (no proposed new
     state on delete).
5. **Evaluate and gate**:
   `let auth_ctx = verified_identity.as_ref().map(|v| AuthContext { uid: v.end_user_id.clone() });`
   `evaluate(&condition, auth_ctx.as_ref(), &resource_fields, &request_resource_fields)`
   — the SAME extended function § Decision — Grammar Extension defines.
   - **`Allow`** → proceed to the existing `adapter.{create,update,delete}_document(...)`
     call, unmodified.
   - **`Deny`** → `return Err(Status::permission_denied("access denied by write rule"))`
     **before** the adapter write call is ever reached — the write must never be
     attempted, unlike `handle_get_document` where the "action" being gated
     (returning an already-fetched document) is inseparable from the fetch
     itself.

### Existence non-leakage, extended (AC-17-34/38)

Same mechanism as ADR-029 § Existence non-leakage, reused verbatim: the pre-write
fetch happens before the `Allow`/`Deny` decision, `resource_fields` falls back to
`EMPTY_FIELDS` when the document does not exist, and `evaluate()` is called
unconditionally. For a content-referencing write rule, a non-owner's update/delete
and an update/delete of a nonexistent document both deny via the identical
fail-closed path, before either ever reaches `adapter.update_document`/
`adapter.delete_document`. Scoped identically to OQ-SR-06's read-path precedent: a
content-**blind** write rule (e.g., `request.auth != null`) has nothing to hide
and may still incidentally reveal existence via the underlying write operation's
own not-found/no-op semantics — this feature does not attempt to mask that,
matching DISCUSS's own explicit scoping.

### Admin handler: `define_write_access_rule` (distinct action, not a `rule_type` field)

**Accepted**: a new handler function `define_write_access_rule` in the same
`admin/handlers/access_rules.rs` file, mirroring `define_access_rule`'s shape
exactly (Owner/Admin role gate, `parse_condition` validation before storage, same
`ConditionRejectionResponse`/`SYNTAX_ERROR`/`UNSUPPORTED_CONSTRUCT` taxonomy,
response shape `WriteAccessRuleResponse` identical to `AccessRuleResponse`), bound
to a new route `POST /admin/v1/projects/:project_id/write_access_rules` and
calling `upsert_write_access_rule`.

**Rejected: extending `define_access_rule` with a `rule_type: "read"|"write"`
body field.** Rejected for three reasons: (1) it reintroduces exactly the
silent-inference risk DISCUSS's own Technical Note for US-01 flagged; (2) it adds
a runtime branch selecting between two storage tables inside the single highest
-consequence handler in BC-4, directly increasing the surface area for an
AC-17-43-class regression (a branching bug could upsert into the wrong table);
(3) it breaks the "one route, one body shape, one storage target" simplicity the
existing admin router already establishes for every other resource in this file —
two single-purpose routes are easier to reason about, review, and mutation-test
than one route with a discriminated body and a table-selection branch.

### Simulation: extend `simulate_access_rule`, not a new endpoint

Per US-07's explicit Technical Note steer, `SimulateAccessRuleBody` gains two new,
backward-compatible (`#[serde(default)]`) fields:

```rust
pub struct SimulateAccessRuleBody {
    pub condition: String,
    pub auth: Option<SimulatedAuth>,
    #[serde(default)]
    pub operation: Option<String>,          // NEW, documentation-only: "create"|"update"|"delete"
    #[serde(default)]
    pub resource: BTreeMap<String, serde_json::Value>,          // existing — pre-write state
    #[serde(default)]
    pub request_resource: BTreeMap<String, serde_json::Value>,  // NEW — proposed new state
}
```

`operation` is **not consumed by `evaluate()`** — it is a request-level annotation
only, present so Alex's simulation payload is self-documenting about which of the
three write shapes he intends to test (matching the Elevator Pitch's literal
"operation type" language). The actual evaluation semantics are driven entirely by
which of `resource`/`request_resource` are populated vs. empty — the same
natural create/update/delete differentiation real enforcement uses, with no
`operation`-based branch anywhere in the evaluation path. `simulate_access_rule`
itself requires no new branch to support both read-rule and write-rule
simulation: it never reads from `access_rules` or `write_access_rules` at all (the
candidate condition is always caller-supplied, never stored), so the single
existing handler already works unmodified for either rule type — only the two
-field-map `evaluate()` call needs updating, identically to real enforcement.

## Consequences

### Positive

- AC-17-43 is enforced structurally: `write_access_rules` and `access_rules` are
  disjoint tables with disjoint adapter methods and disjoint call sites; there is
  no code path, shared column, or shared row through which one could perturb the
  other.
- `evaluate()`/`parse_condition()` remain BC-4's single evaluation routine —
  real enforcement (3 new call sites) and simulation (extended) share the
  identical extended function, preserving ADR-029 DDD-SR-8 and concentrating this
  feature's mutation-testing surface in one place.
- `handle_get_document`, `access_rules`, `get_access_rule`/`upsert_access_rule`
  require zero code changes — the read path this feature must not regress is
  untouched at the source level, not merely retested.
- Collections with no write rule pay zero additional I/O (create: always; update/
  delete: gated behind the cheap PK lookup before any fetch).

### Negative / Trade-offs

- `handle_update_document`/`handle_delete_document` gain a new Postgres round-trip
  (the pre-write fetch) whenever a write rule is defined for the target
  collection — an accepted, evidenced-necessary cost (the two-value comparison,
  US-03, cannot be evaluated without it), mitigated by being gated behind the
  cheap write-rule-existence check.
- A benign TOCTOU window exists between the pre-write fetch and the actual write
  (another concurrent write could change the document in between) — the same
  class of race the existing `WritePrecondition`/OCC (`version`) mechanism already
  exists to handle at the backend layer for other reasons; not a new problem this
  feature introduces, flagged as OQ-SRW-02 for DISTILL/DELIVER to confirm no
  stronger guarantee is required for v1.
- Two schema-identical tables (`access_rules`, `write_access_rules`) exist side by
  side rather than one unified table — accepted deliberately as the cost of
  genuine structural independence (Decision Driver 1 outweighs minor schema
  duplication).

## Enforcement

Style: Hexagonal (ports-and-adapters), unchanged. BC-4 remains the single bounded
context (ADR-029) — this feature adds no new bounded context, only extends BC-4's
existing aggregate surface with a second, independent `WriteAccessRule` aggregate
alongside the existing `AccessRule`.

**No new driven port, no new probe (Principle 12 discipline, explicit reasoning
required, mirroring ADR-029 § Enforcement):**

- `upsert_write_access_rule`/`get_write_access_rule` execute through the existing,
  already-probed `SystemDb` connection pool — the identical substrate every other
  System DB read/write in this codebase already uses.
- The new pre-write fetch calls (`adapter.get_document`) reuse the existing,
  already-probed `BackendAdapter` trait method from 2 new call sites — no new
  port, no new adapter, no new substrate-lie scenario.
- `evaluate()`/`parse_condition()` remain pure, deterministic CPU computation with
  no partial-trust surface — the identical "no environment can lie to a pure
  function" reasoning ADR-029 § Enforcement already established applies
  unmodified to the extended signature.

`cargo-deny`/`deny.toml` unaffected — no new workspace dependency.
`embyr_core::access_control`'s existing zero-IO enforcement covers the extended
module unchanged.

## References

- `docs/feature/security-rules-write-path/feature-delta.md` § Job Discovery
  Framing Resolution (Resolutions 1–2), § System Constraints, § Handoff Package.
- `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md`,
  `adr-028-access-rule-storage-and-lifecycle.md`,
  `adr-029-access-control-composition-and-bounded-context.md` — the machinery this
  ADR extends, not replaces.
- `crates/embyr-core/src/access_control/mod.rs` (full, read during DESIGN) — exact
  current `Operand`/`evaluate()`/`word_to_operand`/`compare_operands`/
  `resolve_field_value` shapes confirmed before designing this extension.
- `crates/embyr-server/src/adapters/system_db.rs:28-38,299-363` — exact current
  `AccessRuleRow`/`upsert_access_rule`/`get_access_rule` shape, the direct
  structural precedent for `WriteAccessRuleRow`/`upsert_write_access_rule`/
  `get_write_access_rule`.
- `crates/embyr-server/src/grpc/handler.rs:358-383` (`attach_client_identity_if_present`),
  `:506-616` (`handle_get_document`), `:618-760` (`handle_create_document`/
  `handle_update_document`/`handle_delete_document`) — read in full during DESIGN;
  confirmed the three write handlers call neither `attach_client_identity_if_present`
  nor any rule-lookup method today.
- `crates/embyr-core/src/storage/backend_adapter.rs:63-66` — `BackendAdapter::get_document`
  trait method signature, reused unchanged for the new pre-write fetch.
- `crates/embyr-server/src/admin/handlers/access_rules.rs`,
  `crates/embyr-server/src/admin/router.rs:190-201` — exact current admin handler
  and route-registration shape, the direct precedent for `define_write_access_rule`
  and its route.
