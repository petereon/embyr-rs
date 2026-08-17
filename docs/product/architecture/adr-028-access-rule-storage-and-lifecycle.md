# ADR-028: Access-Rule Storage and Lifecycle

## Status

Accepted

## Context

DISCUSS locked the observable lifecycle of a per-collection access rule
(`docs/feature/security-rules/feature-delta.md`, Resolution 3): definition is an
**idempotent upsert** — the same admin action defines a rule for the first time and
later fully replaces it, with no separate "rotate" step and no overlap window. This
is an explicit, evidence-forced divergence from `client-auth`'s own US-01/US-03
register-then-rotate pattern (ADR-025), because a rule is edited dozens of times
during iterative authoring (JOB-17's "habit" four-force), unlike a rarely-changed
secret.

DISCUSS's Technical Notes (US-01) explicitly delegate the persistence shape to
DESIGN: "Exact endpoint path, persistence shape, and how 'replace' is implemented
(versioned rows vs. in-place update) are DESIGN's call." This ADR makes that call.

## Decision Drivers

1. **Idempotent-upsert lock (Resolution 3)** — "define" and "redefine" are the
   same operation; the new condition is "immediately and fully active" (AC-17-02)
   with "no blend of old and new, no separate 'old rule still valid for a window'
   behavior" (US-01 Domain Example 2). This *rules out* any versioned-row scheme
   where the previous condition remains readable/active after a redefine.
2. **Per-collection scope, per-project isolation (AC-17-15)** — a rule is scoped to
   exactly one `(project_id, collection_path)` pair; a rule on one collection must
   have zero observable effect on another collection in the same project.
3. **System-DB-scoped, like the project's other configuration state** — a rule
   belongs to the project (like `BackendConfig`, `client_identity_credentials`),
   not to any single document (Customer DB, BC-2's boundary).
4. **Do not import `client-auth`'s rotation-window shape** (§ Handoff Package flag
   2) — that pattern solves a different problem (safe transition for a rarely
   -changed secret) and would add process friction to something Alex does
   deliberately and often.
5. **Simplest solution first** (Principle 8) — no history/versioning/audit-log
   machinery; that is Epic 2e's explicitly named, deferred scope
   (`security-rules-operations`, § Out of Scope).

## Considered Options

### Option A: Versioned rows with an `active` flag (append-only history)

Every define/redefine inserts a new row; a partial unique index or `active`
boolean marks the current row; old rows are retained for history.

**Rejected — for now.** This directly anticipates Epic 2e's explicitly deferred
scope (rule history/versioning/rollback). Building it now violates Principle 8
(simplest solution first) for a capability no story in this DISCUSS pass requires,
and it introduces exactly the kind of "old rule still valid for a window"
ambiguity Resolution 3 locked *against* — a soft-deleted-but-still-selectable
previous row is one query bug away from reintroducing the overlap-window behavior
this feature deliberately excludes. If Epic 2e needs history later, it can be
layered on top of this ADR's schema via a new append-only audit table without
touching the single active-row semantics this ADR establishes.

### Option B: Separate audit-log table, mutated table stays single-row

A single current-state row (as in Option C below) plus a separate, additive-only
`access_rule_history` table that logs every define/redefine event for compliance.

**Rejected — for now, same reasoning as Option A.** This is exactly Epic 2e's
"audit logging" item (§ Out of Scope), named and deferred. Nothing in this
feature's 5 user stories or 19 acceptance criteria requires an audit trail. Adding
it now is unrequested scope, not a simplification.

### Option C: Single row per `(project_id, collection_path)`, in-place UPDATE via `INSERT ... ON CONFLICT ... DO UPDATE` — Accepted

Mirrors `client_identity_credentials`'s existing single-row-per-project shape
(ADR-025), but keyed one level finer: single-row-per-**collection**-per-project,
since a project can have many collections, each independently ruled.

**Accepted.** A composite primary key `(project_id, collection_path)` makes "define"
and "redefine" the *same* SQL statement (`INSERT ... ON CONFLICT (project_id,
collection_path) DO UPDATE SET condition_source = EXCLUDED.condition_source,
updated_at = now()`) — there is no code branch that distinguishes first-time
definition from redefinition, which is exactly what "the same action defines and
redefines" (Resolution 3) means structurally, not just observably. The `UPDATE`
clause is unconditional and atomic (single statement, no read-then-write race),
so "immediately and fully active, no overlap window" (AC-17-02) is a property of
the SQL, not an application-level convention that could be gotten wrong.

## Decision

### Schema — `access_rules` table (`crates/embyr-server/migrations/0022_access_rules.sql`)

| Column | Type | Notes |
|--------|------|-------|
| `project_id` | `TEXT NOT NULL REFERENCES projects(id)` | Part of composite PK |
| `collection_path` | `TEXT NOT NULL` | Part of composite PK. Single-segment collection ID for v1 (Trailmark's domain examples: `journal_entries`, `trail_guides`, `app_config` — no subcollection paths exercised) |
| `condition_source` | `TEXT NOT NULL` | Raw grammar source, validated (`parse_condition`, ADR-027) at write time, stored as text — see § Store Source, Not AST below |
| `created_at` | `TIMESTAMPTZ NOT NULL DEFAULT now()` | Set once, on first insert (`ON CONFLICT DO UPDATE` does not touch it) |
| `updated_at` | `TIMESTAMPTZ NOT NULL DEFAULT now()` | Bumped on every define/redefine |

`PRIMARY KEY (project_id, collection_path)`.

No `active`/`previous`/`version` columns — deliberately, per Decision Driver 1.

### Adapter methods (`embyr-server::adapters::system_db`, extends `SystemDb`)

```
async fn upsert_access_rule(
    &self, project_id: &str, collection_path: &str, condition_source: &str,
) -> Result<(), CoreError>;

async fn get_access_rule(
    &self, project_id: &str, collection_path: &str,
) -> Result<Option<AccessRuleRow>, CoreError>;
```

`upsert_access_rule` is the single write path for both US-01 domain examples
(first-time definition and redefinition) — there is no `insert_access_rule` /
`redefine_access_rule` pair, unlike `client_identity_credentials`'s deliberate
`insert_*`/`rotate_*` split (ADR-025). This asymmetry is intentional: ADR-025's
split exists *because* register-vs-rotate are observably different actions
(register 409s on a second attempt; rotate 404s if nothing is registered yet).
Resolution 3 explicitly locks the opposite observable behavior here — define and
redefine are the same action with the same success response — so a single method
is the correct mirror of ADR-025's own precedent, not a deviation from it.

`get_access_rule` returning `None` is the mechanism that answers § Handoff Package
flag 3 (the structural regression guardrail) — see ADR-029 for how the call site
uses this `None` to short-circuit before any evaluation logic runs.

### Store Source, Not AST

`condition_source` stores the raw, validated grammar text — not a serialized
`Condition` AST. The condition is re-parsed (`parse_condition`, ADR-027) on every
`GetDocument` call that reaches a collection with a rule defined.

**Trade-off, accepted deliberately:** re-parsing costs CPU on every gated read.
Given the grammar's small size (two production families, typically a handful of
tokens per condition), this is expected to be negligible relative to the existing
Postgres round-trip already on this hot path (the document fetch itself). Storing
a serialized AST instead would require `Condition` to have a stable, versioned
wire format that survives future parser code changes without silent
misinterpretation — complexity with no evidenced performance need yet. If
profiling after real traffic volume shows otherwise, caching the parsed AST
(keyed by `(project_id, collection_path, updated_at)`, invalidated on
`upsert_access_rule`) is a pure, additive, single-adapter follow-up — flagged as
**OQ-SR-05**, mirroring `client-auth`'s own OQ-CA-02 ("cache verification results
once real traffic volume is known") precedent for deferring performance-tuning
decisions until data exists.

## Consequences

### Positive

- "Define" and "redefine" are structurally the same code path (Resolution 3's
  lock is enforced by the SQL statement shape, not by application logic that could
  drift).
- Per-collection composite key structurally enforces AC-17-15 (a rule on one
  collection cannot affect another — they are different primary key values,
  different rows, no shared mutable state).
- No new workspace dependency; reuses `sqlx` (existing), the existing `SystemDb`
  connection pool (already probed at startup — see ADR-029 § Driven Ports).

### Negative / Trade-offs

- No audit trail of who changed a rule or when the previous condition was —
  explicitly deferred to Epic 2e, not a gap in this ADR's own scope.
- Re-parsing on every gated read has a (currently unmeasured, believed negligible)
  CPU cost — flagged as OQ-SR-05, not silently assumed to be free.

## Enforcement

Style: Hexagonal (ports-and-adapters), unchanged. No new driven port — `SystemDb`
is reused (see ADR-029 for the explicit "no new probe needed" reasoning).
`cargo-deny`/`deny.toml` unaffected — no new dependency.

## References

- `docs/feature/security-rules/feature-delta.md` § Job Discovery Framing
  Resolution (Resolution 3), § System Constraints, § Handoff Package flag 2.
- `docs/product/architecture/adr-025-client-identity-credential-storage-rotation.md`
  — the register/rotate precedent this ADR deliberately does *not* replicate, and
  the single-row-per-project shape it *does* replicate (one level coarser-grained).
- `crates/embyr-server/src/adapters/system_db.rs:169-281` — direct structural
  precedent for the adapter method shape.
- `crates/embyr-server/migrations/0021_client_identity_credentials.sql` — direct
  migration-numbering and schema-shape precedent.
