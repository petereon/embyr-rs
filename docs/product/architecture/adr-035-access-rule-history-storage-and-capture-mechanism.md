# ADR-035: Access-Rule History Storage and Capture Mechanism

## Status

Accepted

## Context

`security-rules` (ADR-028), `security-rules-write-path` (ADR-030), and
`security-rules-collection-group-rules` (ADR-032) each independently
deferred rule history/versioning/rollback to a named, deferred follow-up
epic, in nearly identical language: ADR-028 Decision Driver 5 ("no
history/versioning/audit-log machinery; that is Epic 2e's explicitly named,
deferred scope"), ADR-030 § Decision — Schema ("same 'no history/versioning
machinery' discipline ADR-028 Decision Driver 5 already established, applied
fresh"), ADR-032 § Consequences — Negative ("Alex has no way to audit when a
group rule was previously different — unchanged, deliberate scope boundary
carried from ADR-028... `security-rules-operations`, Epic 2e, remains the
deferred home for rule history/versioning"). `security-rules-operations`
(this feature) is that epic.

DISCUSS (`docs/feature/security-rules-operations/feature-delta.md`,
Resolutions 1-4) locked the observable outcome — every rule a Trailmark
admin has ever defined, across all 3 rule tables, has a complete,
attributable, chronologically-ordered history and can be restored to any
prior state — and locked two mechanism constraints (audit attribution
embedded on the history row, not a separate table; append-only, no `UPDATE`/
`DELETE` ever). DISCUSS explicitly left the exact table shape and capture
-mechanism placement to DESIGN, flagging Resolution 1's own "3
independently-stored, schema-identical tables" conclusion as a strong
recommendation, not a hard lock (§ Handoff Package flag 1). This ADR makes
those two calls, plus the admin-surface and ordering-mechanism decisions
DISCUSS's own Technical Notes left open.

## Decision Drivers

1. **North Star KPI #1: 100% of redefine events, across all 3 rule tables,
   produce a retrievable, correctly-attributed history entry.** A design
   where history capture is possible to skip — even rarely, even only under
   a crash between two statements — fails this KPI structurally, not just
   statistically.
2. **Structural, not conventional, independence between the 3 rule types'
   own histories** (DISCUSS System Constraints) — the identical discipline
   ADR-030 Decision Driver 1 and ADR-032 Decision Driver 2 already
   established for the CURRENT-state rule tables applies without weakening
   to their HISTORY tables: a bug that conflates one rule type's history
   with another's is the same class of risk regardless of which table it
   corrupts.
3. **Append-only, no `UPDATE`/`DELETE`, ever** (DISCUSS System Constraints,
   locked) — a restore is a new forward-moving history entry, never a
   destructive rewrite.
4. **No modification to any of the 3 rule tables' own existing upsert SQL
   statement text** (DISCUSS System Constraints, locked) — `upsert_access_rule`/
   `upsert_write_access_rule`/`upsert_group_access_rule`'s `INSERT ... ON
   CONFLICT ... DO UPDATE` statements remain byte-for-byte unchanged;
   history capture is additive, never a rewrite of that statement.
5. **Audit attribution reuses `session.account_id`, already in scope — no
   new identity mechanism** (Resolution 2, locked).
6. **Correct chronological ordering under rapid successive redefinition**
   (AC-17-158, US-01 Domain Example 3) — two redefinitions "in quick
   succession" must produce two distinct, correctly-ordered entries; the
   ordering mechanism must hold this guarantee structurally, not as a
   probabilistic property of timestamp resolution.
7. **Simplest solution first** (Principle 8) — no new crate, no new
   dependency, no speculative pagination/retention/export machinery
   (§ Out of Scope, DISCUSS), no dedicated restore-by-id endpoint unless the
   two-step retrieve-then-redefine flow is shown to be insufficient.

## Decision — Schema Shape: 3 Independently-Stored Tables, Independently Verified

### Considered Options

**Option A: History for `access_rules` only, in v1.** Rejected by DISCUSS
(Resolution 1) before this ADR — the gap is symmetric and equally severe
across all 3 rule tables, evidenced by 3 independent, already-Accepted ADRs
using nearly identical deferred-scope language. Not re-litigated here.

**Option B: One shared `access_rule_history` table with a `rule_type`
discriminator column, keyed `(project_id, collection_key, rule_type)`.**

Re-examined independently against the ACTUAL schema this ADR designs, not
accepted on DISCUSS's recommendation alone. The re-examination confirms
DISCUSS's own reasoning holds, for a reason that only becomes visible once
the real column shapes are on the table: `access_rules`/`write_access_rules`
key on `collection_path`; `group_access_rules` keys on `collection_id` — a
deliberately different column name (ADR-032 § Decision — Schema, "the first
time this initiative enforces its own 'bare identifier, not a path'
invariant at the DB layer"), carrying its own `CHECK (collection_id NOT LIKE
'%/%')` constraint that `collection_path` does not have. A single
`collection_key` column discriminated by `rule_type` would have to either
(a) drop `group_access_rule_history`'s own `CHECK` constraint (silently
weakening the exact defense-in-depth guarantee ADR-032 added), or (b) make
the `CHECK` conditional on `rule_type`, which is precisely the kind of
`rule_type`-branching logic ADR-030 DDD-SRW-6 rejected for the *current-state*
tables, now reintroduced one layer down, in the history tables. The
history-retrieval query itself would also need a `WHERE rule_type = $N`
predicate on every call — a single missed or wrong-value predicate (e.g. a
copy-pasted query missing the `rule_type` filter) silently returns another
rule type's history alongside or instead of the intended one, exactly the
AC-17-169/172 non-interference guarantee this feature must hold. **Rejected**,
independently confirmed, not merely inherited.

**Option C: 3 independently-stored, schema-identical history tables —
Accepted.** `access_rule_history`, `write_access_rule_history`,
`group_access_rule_history`, each keyed to and column-shaped after its own
parent table (`collection_path` for the first two, `collection_id` +
`CHECK` for the third). No shared table, no discriminator, no cross-table
query ever needed to answer "what did this rule say, and who set it."

**Accepted**, mirroring ADR-030/032's own already-twice-validated
structural-independence pattern for the CURRENT-state tables, now applied
symmetrically to the HISTORY tables — not a blind copy of the
recommendation, but the same reasoning re-derived directly from this ADR's
own schema design and found to still hold.

## Decision — Schema

### Ordering mechanism: monotonic identity column, not `captured_at` alone

`captured_at TIMESTAMPTZ NOT NULL DEFAULT now()` is retained on every history
row (it answers "when," per US-01/US-02's own domain language), but it is
**not** the column `ORDER BY` relies on for "newest first" retrieval
(AC-17-160, AC-17-158's correct-ordering guarantee). The authoritative
ordering key is `id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY`, a
Postgres-native monotonic sequence.

**Investigated precedent, and why it does not transfer**: `query_logs`
(migration 0014, the closest existing append-only, actor-attributed log
table in this codebase) uses `id UUID DEFAULT gen_random_uuid()` with
`PRIMARY KEY (id, created_at)` and is queried by `created_at` range only —
no monotonic sequence exists anywhere in this codebase today (confirmed:
zero `SERIAL`/`BIGSERIAL`/`GENERATED ALWAYS AS IDENTITY` columns in any of
the 24 existing migrations). `query_logs` can rely on timestamp-range
querying alone because it carries no per-row strict-ordering correctness
requirement — it is a metering/billing log, queried in bulk ranges, where a
same-microsecond tie between two unrelated requests is inconsequential.
This feature is different: AC-17-158 requires that two redefinitions "in
quick succession" produce two entries in **correct** chronological order,
individually retrievable and distinguishable. Two sequential `INSERT`
statements issued moments apart by two different HTTP requests will, in
practice, almost always receive distinct `TIMESTAMPTZ` values (microsecond
resolution, plus unavoidable network/parse/role-check latency between
requests) — but "almost always" is a probabilistic property of clock
resolution and request timing, not a structural guarantee, and this
project's own repeated discipline (Principle 11/12: enforceable, not
conventional) sets a higher bar for a requirement with its own explicit AC.
A `GENERATED ALWAYS AS IDENTITY` column is assigned a strictly increasing
value per `INSERT`, by Postgres's own sequence semantics — two rows can
never receive the same `id`, regardless of clock resolution or how close in
time the two `INSERT`s occur. Cost: one extra 8-byte column and index per
table; no new dependency, no new tooling — an ordinary Postgres primary-key
choice, not novel machinery.

### `access_rule_history` (`migrations/0025_access_rule_history.sql`)

| Column | Type | Notes |
|--------|------|-------|
| `id` | `BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY` | Authoritative "newest first" ordering key (see above) |
| `project_id` | `TEXT NOT NULL REFERENCES projects(id)` | Mirrors `access_rules.project_id`'s own FK style (no cascade) |
| `collection_path` | `TEXT NOT NULL` | Mirrors `access_rules.collection_path` exactly — same column name, same "single-segment in v1" scope |
| `condition_source` | `TEXT NOT NULL` | The condition just made active (the NEW value at define/redefine time, per US-01's own Elevator Pitch — history is a forward log of every value a rule has held, not a "previous value" diff) |
| `actor_account_id` | `UUID NOT NULL` | The acting admin's `account_id`, sourced from `SessionContext.account_id` — no FK (see § Decision — Actor Attribution below) |
| `captured_at` | `TIMESTAMPTZ NOT NULL DEFAULT now()` | Display/audit timestamp only — not the ordering key |

Index: `CREATE INDEX idx_access_rule_history_lookup ON access_rule_history
(project_id, collection_path, id DESC)` — the exact index shape US-02's
retrieval query needs, mirroring every prior epic's own "no scan on the
unarmed/read path" NFR discipline (ADR-032 Decision Driver 6).

No `UPDATE`/`DELETE` grant path exists anywhere in this feature's own code —
the append-only invariant (DISCUSS, locked) is enforced by the absence of
any adapter method that issues either statement against this table, mirrored
identically for the other 2 history tables below.

### `write_access_rule_history` (`migrations/0026_write_access_rule_history.sql`)

Schema-identical to `access_rule_history` (same 6 columns, same index shape,
same `collection_path` naming — mirrors `write_access_rules.collection_path`
exactly), living in a wholly separate table with its own primary key, own
rows, own adapter methods — no shared column, no shared row, no shared query
with `access_rule_history` (AC-17-169's structural-independence guarantee).

### `group_access_rule_history` (`migrations/0027_group_access_rule_history.sql`)

Schema-identical to the above with 2 deliberate departures, mirrored from
`group_access_rules`' own ADR-032 departures, for the identical reasons:

- Column named `collection_id`, not `collection_path` (a collection-group
  identifier is structurally never a path).
- `CHECK (collection_id NOT LIKE '%/%')` — the same DB-level defense-in-depth
  layer ADR-032 added to `group_access_rules` itself, now mirrored onto its
  history table. The admin handler's own `validate_bare_collection_id` check
  already runs before either the `group_access_rules` upsert or the history
  capture (§ Decision — Capture Mechanism Placement), so this `CHECK` is a
  second, redundant-by-design layer for the identical invariant, consistent
  with ADR-032's own two-layer reasoning (a future direct-SQL bypass of the
  handler must still be caught).
- `project_id` FK carries `ON DELETE CASCADE`, mirroring
  `group_access_rules.project_id`'s own choice exactly (ADR-032), for
  consistency with its parent table's own deletion policy.

## Decision — Actor Attribution: No FK to `accounts(id)`

`actor_account_id UUID NOT NULL` carries no `REFERENCES accounts(id)`
constraint, mirroring `query_logs.account_id`'s own precedent (migration
0014) exactly — the closest existing analog of "a log-shaped table storing
an acting account's id for attribution." This is a genuine choice, not an
oversight: an FK would force a decision DISCUSS never locked (what happens
to history rows if an account is ever hard-deleted — no such deletion path
exists anywhere in this codebase today, only project soft-delete via
`deleted_at`) for no evidenced benefit; `query_logs` made the identical
choice for the identical reason. Flagged as OQ-SRO-03 if a future feature
ever needs referential-integrity guarantees stronger than this precedent
provides.

## Decision — Capture Mechanism Placement: Fused Into the Existing Upsert, Not a Second Call

DISCUSS's own (non-binding) Technical Notes for US-01 describe history
capture as "a new `INSERT` ... executed alongside (not instead of) the
existing, unmodified `upsert_access_rule` call in `define_access_rule`" —
read literally, two separate adapter calls from the handler. This ADR
departs from that literal phrasing, per the task's own explicit invitation
to pick whichever placement makes history loss **structurally**, not
conventionally, impossible (DISCUSS's own repeated "structural, not
conventional" discipline, e.g. ADR-030 Decision Driver 1, ADR-032 Decision
Driver 5).

### Considered Options

**Option A: Two separate adapter calls from each handler** — `upsert_access_rule`'s
signature stays unchanged; each handler additionally calls a new
`insert_access_rule_history(...)` method after the upsert succeeds.

Rejected. Nothing at compile time forces every current or future caller of
`upsert_access_rule` to also call the history-insert method — a future
handler edit, or a new call site added by a later feature (e.g. a bulk
-import admin action), could upsert a rule without capturing history, and
the compiler would not catch the omission. Worse: the two statements are not
guaranteed to execute in the same transaction, so a crash or connection
failure between them can leave a rule successfully redefined with **no**
matching history row — a silent violation of this feature's own North Star
KPI (100% of redefine events produce a retrievable history entry), possible
precisely because the coupling is conventional (two calls placed next to
each other in the handler body) rather than structural.

**Option B: Fold the history `INSERT` into the SAME adapter method, executed
in one DB transaction with the existing upsert; extend the method signature
to require `actor_account_id: Uuid` — Accepted.**

The Rust compiler enforces the coupling: every existing and future call site
of `upsert_access_rule`/`upsert_write_access_rule`/`upsert_group_access_rule`
must supply an `actor_account_id` argument or fail to compile — a
compile-time-visible, structural forcing function, mirroring `evaluate()`'s
own signature-extension precedent (ADR-030 Decision Driver 3: a signature
extension of an existing function, not a second function). The transaction
guarantees atomicity: if the history `INSERT` fails for any reason, the
rule-redefinition `UPDATE`/`INSERT` itself rolls back and the call returns an
error — there is no code path that can leave the rule changed with no
matching history entry, and no code path that can leave a history entry
capturing a condition that was never actually made active.

**Why this does not violate the "no modification to the existing upsert
statement" constraint**: the constraint (DISCUSS System Constraints, locked)
is about the SQL statement TEXT — `INSERT INTO access_rules (...) VALUES
(...) ON CONFLICT (...) DO UPDATE SET condition_source = EXCLUDED.condition_source,
updated_at = now()` remains byte-for-byte unchanged, still the single,
unconditional upsert it always was. What changes is the Rust function's
calling convention (one new parameter) and the transaction boundary
surrounding it — the history `INSERT` is executed "alongside" the existing
statement in the sense DISCUSS's own Technical Notes intended (a new,
additive statement, never a rewrite of the existing one), just co-located in
the same function and the same transaction rather than in two separate
handler-level calls. This is the stronger, not weaker, reading of
"additive": both statements succeed together or fail together, rather than
succeeding-or-failing independently.

### Adapter method shape (illustrative; final SQL text unchanged from ADR-028/030/032)

```rust
pub async fn upsert_access_rule(
    &self,
    project_id: &str,
    collection_path: &str,
    condition_source: &str,
    actor_account_id: Uuid,               // NEW parameter
) -> Result<(), CoreError> {
    let mut tx = self.pool.begin().await
        .map_err(|e| CoreError::BackendUnavailable(format!("tx begin failed: {e}")))?;

    // EXISTING statement, byte-for-byte unchanged from ADR-028.
    sqlx::query(
        "INSERT INTO access_rules (project_id, collection_path, condition_source) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (project_id, collection_path) \
         DO UPDATE SET condition_source = EXCLUDED.condition_source, updated_at = now()",
    )
    .bind(project_id).bind(collection_path).bind(condition_source)
    .execute(&mut *tx).await
    .map_err(|e| CoreError::BackendUnavailable(format!("upsert_access_rule failed: {e}")))?;

    // NEW statement, additive, same transaction.
    sqlx::query(
        "INSERT INTO access_rule_history \
         (project_id, collection_path, condition_source, actor_account_id) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(project_id).bind(collection_path).bind(condition_source).bind(actor_account_id)
    .execute(&mut *tx).await
    .map_err(|e| CoreError::BackendUnavailable(format!("access_rule_history insert failed: {e}")))?;

    tx.commit().await
        .map_err(|e| CoreError::BackendUnavailable(format!("tx commit failed: {e}")))?;
    Ok(())
}
```

`upsert_write_access_rule`/`upsert_group_access_rule` gain the identical
`actor_account_id: Uuid` parameter and identical transaction shape, against
`write_access_rule_history`/`group_access_rule_history` respectively.

**AC-17-166 (no-op restore still produces a new entry) holds structurally**:
neither the upsert statement nor the history insert ever compares the new
condition against the currently-active one — every successful call produces
exactly one new history row, regardless of whether the submitted condition
text is identical to the current value. No special case exists to remove.

**AC-17-157 (first-ever definition produces exactly one entry) holds
structurally**: the history `INSERT` executes on every successful call,
first-time or redefine alike — there is no branch distinguishing the two,
mirroring the parent upsert statement's own no-branch shape (ADR-028
Decision Driver 1).

### Handler call-site changes (mechanical)

`define_access_rule`/`define_write_access_rule`/`define_group_access_rule`
each gain one additional argument at their existing `upsert_*` call site:
`session.account_id` (already in scope, `Uuid`, confirmed by direct read of
`SessionContext`, `crates/embyr-server/src/admin/extractors/session_context.rs:29`).
No other change to any of the 3 handlers' existing logic, role gates, or
response shapes (AC-17-159/168/171: response shape and role gate
unmodified).

## Decision — Admin Surface: Retrieval and Restore

### Retrieval: 3 new GET routes, any authenticated role

```
GET /admin/v1/projects/:project_id/access_rules/:collection_path/history
GET /admin/v1/projects/:project_id/write_access_rules/:collection_path/history
GET /admin/v1/projects/:project_id/group_access_rules/:collection_id/history
```

Follows the existing `resource-name-matches-table-name` route convention
(`.../access_rules`, `.../write_access_rules`, `.../group_access_rules`)
with a natural `/history` sub-resource nested under the identifying path
segment — `collection_path`/`collection_id` is safe as a URL path segment
because both are, by construction, single-segment identifiers (no `/`) in
this codebase's own v1 scope.

Handler shape mirrors `simulate_access_rule`'s any-role, read-only precedent
exactly: `verify_project_ownership` only, no `session.role < Role::Admin`
gate (Handoff Package flag 6, locked observable behavior). Empty history
(a collection with no rule ever defined, or defined but with no history
yet — cannot occur once any rule is ever defined, since capture is now
structurally fused to every successful define) returns `200` with an empty
list, never an error (AC-17-161) — a natural consequence of `SELECT ...
ORDER BY id DESC` against zero matching rows, not a special case. Missing/
invalid session returns `401` automatically via `SessionContext`'s existing
`FromRequestParts` rejection (AC-17-163) — zero new code required for that
AC.

Response types (3 distinct, schema-identical structs — not a shared generic
type, mirroring `AccessRuleResponse`/`WriteAccessRuleResponse`/
`GroupAccessRuleResponse`'s own precedent of independently-evolvable,
per-rule-type response shapes):

```rust
#[derive(Serialize)]
pub struct AccessRuleHistoryEntry {
    pub id: i64,
    pub condition: String,
    pub actor_account_id: Uuid,
    pub captured_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct AccessRuleHistoryResponse {
    pub project_id: String,
    pub collection_path: String,
    pub history: Vec<AccessRuleHistoryEntry>,
}
```

(`WriteAccessRuleHistoryResponse`/`GroupAccessRuleHistoryResponse` mirror
this shape exactly, the latter with `collection_id` in place of
`collection_path`.) `id` is exposed — not strictly required by any locked
AC, but a near-zero-cost addition (the primary key is already selected for
ordering) giving Alex a stable per-entry reference for his own tooling, and
keeping a future restore-by-id convenience endpoint (§ Decision — Restore,
below) a pure additive change if evidence ever warrants it.

### Restore (US-03): zero new endpoint, zero new mechanism — confirmed, not assumed

DISCUSS's own Technical Notes framed US-03 as a proof obligation: "restore"
is exactly "call the existing define action with a previously-seen condition
value." This ADR confirms that claim holds, by design: Alex retrieves a
collection's history (`GET .../history`, above), reads the `condition` field
of the entry he wants to restore, and calls the existing `POST
.../access_rules` (or `.../write_access_rules`/`.../group_access_rules`)
endpoint with that exact text as the request body's `condition` — the SAME
endpoint, SAME handler, SAME role gate (Owner/Admin, inherited automatically
because restore IS a define call, never a new check) US-01 already uses. The
restoration is captured as a new history entry via § Decision — Capture
Mechanism Placement's own fused mechanism, with no special-casing —
satisfying AC-17-164/165/166/167 with zero new production code beyond
retrieval.

**Considered and rejected: a dedicated `POST .../history/:id/restore`
convenience endpoint** (fetch-by-id + redefine in one call). Rejected for
v1 under Principle 8 (simplest solution first) — no evidence in DISCUSS's
own 5 user stories that the 2-step client flow (retrieve, then redefine
with the retrieved text) is insufficient; DISCUSS's own Journey narrative
literally describes Alex performing exactly this 2-step flow. `id` is
exposed in the retrieval response specifically so this endpoint remains a
pure additive future change (fetch the history row server-side by `id`,
then call the same fused-upsert path) if real usage ever shows the 2-step
flow is friction, not a redesign. Flagged as OQ-SRO-02.

## Decision — Generalization to Write and Group Rules (US-04/US-05)

Confirmed, not merely assumed, that no table-specific complication exists
beyond mirroring the pattern established for `access_rule_history`:

- `write_access_rules`' own peculiarity (ADR-030: read and write conditions
  live in 2 separate tables, never 2 columns in one table) requires nothing
  extra of its history table — `write_access_rule_history` is
  schema-identical to `access_rule_history`, referencing a different parent
  table's own domain, nothing more.
- `group_access_rules`' own peculiarity (ADR-032: `collection_id` naming +
  `CHECK` constraint) is mirrored 1:1 onto `group_access_rule_history` (§
  Decision — Schema, above) — the only genuine per-table variation across
  all 3 history tables, already accounted for.

Both slices (04/05) require the identical schema/adapter-signature-extension/
handler-argument/route pattern established for `access_rule_history`,
applied to a second and third parent table — confirming, not merely
asserting, Resolution 1's central generalization hypothesis.

## Consequences

### Positive

- History loss is structurally impossible, not conventionally unlikely: the
  compiler enforces `actor_account_id` at every call site, and the
  transaction enforces that the rule change and its history entry succeed
  or fail together.
- The 3 history tables are as structurally independent from each other as
  the 3 rule tables they extend — re-verified against the actual schema,
  not inherited from DISCUSS's recommendation unexamined.
- Chronological ordering is a structural guarantee (a monotonic identity
  column), not a probabilistic property of timestamp resolution — closing a
  real, if narrow, correctness gap this codebase's own closest precedent
  (`query_logs`) does not need to close for its own, different, use case.
- Restore requires zero new endpoint and zero new storage mechanism —
  confirmed by design, not merely claimed.
- `access_rules`/`write_access_rules`/`group_access_rules`' own SQL
  statement text, response shapes, and role gates are completely unmodified
  — verifiable by diff.

### Negative / Trade-offs

- `upsert_access_rule`/`upsert_write_access_rule`/`upsert_group_access_rule`'s
  Rust signatures change (one new required parameter each) — every existing
  call site and test using the 3-argument form requires a mechanical update
  at DELIVER time. Accepted: the alternative (Option A) makes history loss
  possible, which this feature exists specifically to prevent.
- Each `upsert_*` call now costs one additional `INSERT` inside the same
  transaction — a small, constant per-redefine cost (rule authoring is an
  infrequent, deliberate admin action, not a hot data-plane path; no
  regression to `GetDocument`/writes/`RunQuery`/`Listen`, none of which call
  `upsert_*`).
- `actor_account_id` carries no FK — an account row could theoretically be
  deleted while history rows referencing it remain, though no account
  -deletion path exists anywhere in this codebase today. Flagged as
  OQ-SRO-03.

## Enforcement

Style: Hexagonal (ports-and-adapters), unchanged. No new crate, no new
bounded context — BC-4 Access Control (ADR-029) is extended with 3 new
append-only child tables of its existing `AccessRule`/`WriteAccessRule`/
`GroupAccessRule` aggregates, not a new aggregate.

**No new driven port, no new Earned Trust probe (Principle 12 discipline,
explicit reasoning required, mirroring ADR-028/030/032 § Enforcement
verbatim):**

- The new transaction (upsert + history insert) executes through the
  existing, already-probed `SystemDb` connection pool — the identical
  substrate every other System DB read/write in this codebase already uses.
  No new substrate, no new substrate-lie scenario.
- The 3 new `get_*_history` retrieval methods are ordinary indexed `SELECT`
  statements against the same pool.

`cargo-deny`/`deny.toml` unaffected — no new workspace dependency.
`embyr_core::access_control` receives zero code changes (Resolution 3,
locked — no touch to `Operand`/`Condition`/the tokenizer/evaluation logic
anywhere in this feature).

## References

- `docs/feature/security-rules-operations/feature-delta.md` § Job Discovery
  Framing Resolution (Resolutions 1-4), § System Constraints, § User Stories
  (US-01 through US-05), § Handoff Package.
- `docs/product/architecture/adr-028-access-rule-storage-and-lifecycle.md`,
  `adr-030-write-path-grammar-storage-and-composition.md`,
  `adr-032-collection-group-rule-storage-and-composition.md` — the 3
  current-state rule tables this ADR adds history to; their own identical
  "deferred to Epic 2e" language and their own structural-independence
  precedent, re-verified rather than blindly mirrored.
- `crates/embyr-server/src/admin/handlers/access_rules.rs` (full) — exact
  current `define_access_rule`/`define_write_access_rule`/
  `define_group_access_rule` shapes, confirmed to call `upsert_*`
  unconditionally with no history side effect on the success path;
  `SessionContext`/`session.account_id` confirmed directly accessible at
  every call site.
- `crates/embyr-server/src/adapters/system_db.rs` (full) — exact current
  `upsert_access_rule`/`upsert_write_access_rule`/`upsert_group_access_rule`
  shapes this ADR extends.
- `crates/embyr-server/src/admin/extractors/session_context.rs` — confirms
  `SessionContext { account_id: Uuid, .. }`'s exact shape.
- `migrations/0014_admin_query_logs.sql` — the closest existing
  append-only, actor-attributed log table; its `id UUID` +
  timestamp-range-only ordering and its unconstrained `account_id` are both
  investigated as precedent, one adopted (no FK) and one deliberately not
  adopted (ordering key), with reasoning given for each.
- `migrations/0008_admin_accounts.sql` — confirms `accounts(id)`'s exact
  shape (investigated, not referenced by FK — see § Decision — Actor
  Attribution).
