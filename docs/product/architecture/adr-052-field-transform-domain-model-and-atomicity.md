# ADR-052: Field Transform Domain Model (`Write`/`FieldTransform`/`WriteResult`) and Atomicity Mechanism

## Status

Accepted

## Context

`firestore-field-transforms` (JOB-01) restores field-transform computation
across `Commit`, `Write`, `BatchWrite` — currently `translate_one_write_for_commit`
(`crates/embyr-server/src/grpc/handler.rs`) parses transform sentinels off the
wire and then unconditionally discards them (`docs/feature/firestore-field-transforms/feature-delta.md`
§ Reading Confirmation, re-confirmed here at current line numbers,
`handler.rs:687-771`, unchanged from DISCUSS's own citation).

DISCUSS's own Reading Confirmation surfaced the single most load-bearing
finding this ADR resolves: a `FieldTransform` reaches the wire via **two
distinct, structurally different paths** on `message Write`
(`proto/google/firestore/v1/document.proto:90-110`):

1. `Write.operation = transform(DocumentTransform)` (field 6) — a standalone
   transform-only write, no `update`/`delete` set. Already has a domain-model
   home: `Write::Transform { path, transforms }` (`embyr-core::storage::backend_adapter`),
   currently always constructed with `transforms: vec![]`.
2. `Write.update_transforms` (field 7, `repeated DocumentTransform.FieldTransform`)
   — "the transforms to perform after update," attached **alongside** a
   regular `update` operation on the SAME `Write` message. Confirmed by this
   DESIGN pass (re-reading `translate_one_write_for_commit`'s `Update` arm,
   `handler.rs:696-717`): `proto_write.update_transforms` is never read at
   all, no comment, no partial handling — a second, previously-undocumented
   instance of the discard bug, and the shape a real SDK call like
   `docRef.update({viewCount: FieldValue.increment(1)})` compiles to.

Three further ground-truth facts, re-confirmed directly (not trusted from
DISCUSS's own citation alone), are decisive for this ADR's shape:

- **`Write::Update`'s existing apply semantics are a full-document overwrite,
  not a partial merge.** `commit_transaction`'s `Write::Update` arm
  (`crates/embyr-pg-storage/src/backend_adapter.rs:1062-1085`) does
  `INSERT ... ON CONFLICT DO UPDATE SET fields = $4::jsonb, ...` — the
  entire `fields` JSONB column is replaced with whatever the write's own
  `fields` map contains. No `update_mask`-driven partial merge exists
  anywhere in this codebase (confirmed by DISCUSS's own grep, re-confirmed
  here). This is a pre-existing, narrower-than-real-Firestore gap, out of
  this feature's scope to close — but it is the reference semantics any
  combined update+transform design must stay honest with, not silently
  widen.
- **`commit_transaction`'s `Vec<Write> → Vec<WriteResult>` mapping is
  positional and 1:1, and this positional alignment is a hard invariant**,
  not a convenience — `docs/SPEC.md`'s own explicit contract for both
  `Commit`'s per-transaction result list and `BatchWrite`'s own
  `write_results[i]`/`status[i]` pairing (ADR-048 § Decision Driver 2,
  reused verbatim here). Any domain-model shape that turns ONE input `Write`
  into TWO `DomainWrite`s breaks this invariant outright.
- **`verify_versions` (`crates/embyr-pg-storage/src/transactions/occ.rs`)
  already proves the exact row-locking idiom this feature needs, one layer
  short of what it needs today**: `SELECT version FROM documents WHERE ...
  FOR UPDATE` inside `commit_transaction`'s own single `pg_txn`. It reads
  `version` only; this feature needs the `fields` column under the same
  lock.
- **`crates/embyr-server/src/encoding/firestore_proto.rs::proto_value_to_field_value`
  and `field_value_to_proto` already exist and are the exact reuse targets**
  for translating `FieldTransform` operands (`Value`/`ArrayValue`) in and
  `WriteResult.transform_results` values out — zero new proto↔domain
  translation logic needed at the wire-shape layer.
- **`WriteResult` (`embyr_core::domain::document::WriteResult`) has exactly
  two fields today** (`update_time`, `create_time`) and is constructed at
  **9 domain-layer call sites**, not just the 3 inside `commit_transaction`:
  `crates/embyr-pg-storage/src/backend_adapter.rs:319,369,400,454,484,1081,1101,1105`
  and `crates/embyr-server/src/adapters/agent_backend.rs:344,375,575`. The
  proto-response layer (`embyr_proto::firestore::WriteResult`) is constructed
  at **6 call sites**, not 5 — DISCUSS's own Reading Confirmation found 5 in
  `handler.rs` (lines 1851-56, 1949-51, 1968-70, 1982-2002) but did not check
  `crates/embyr-server/src/grpc/write_stream.rs:165` (the `Write` RPC's own
  streaming response), which hardcodes `transform_results: vec![]`
  identically. Corrected here, not silently left at DISCUSS's count.

## Decision Drivers

1. **The `Vec<Write> → Vec<WriteResult>` positional invariant is non-negotiable**
   (above) — it eliminates any design that maps one `Write` to more than one
   `DomainWrite`.
2. **Stay honest with the existing (narrower) `Write::Update` semantics** —
   this feature restores transform COMPUTATION; it does not implicitly grow
   `Update` a real `update_mask`-driven partial-merge capability it does not
   have today. A combined update+transform write's "current value" for a
   transform must be defined in terms of what `Write::Update` ALREADY does
   (full replace), not what real Firestore's fuller semantics would imply.
3. **Reuse over invention (standing session practice)** — `verify_versions`'s
   row-locking idiom, `proto_value_to_field_value`/`field_value_to_proto`,
   and `commit_transaction`'s existing single-`pg_txn`/single-UPSERT-per-write
   shape are all extended, not replaced or duplicated.
4. **Pure computation belongs in `embyr-core`, IO belongs in
   `embyr-pg-storage`** — `embyr-core` already bans IO crates by construction
   (`deny.toml`, CLAUDE.md). Type-preservation arithmetic, structural-equality
   filtering, and missing-field semantics are pure functions of
   `(current: Option<&FieldValue>, transform: &FieldTransform, now: (i64,i32))
   -> Result<TransformOutcome, CoreError>` — zero reason to live inside the
   SQL-adjacent adapter crate, and putting them in `embyr-core` makes them
   independently unit-testable without a Postgres fixture (mirrors this
   project's own "functional-where-practical" paradigm, CLAUDE.md).
5. **Fail fast on operand-only validation, before touching Postgres** —
   `docs/SPEC.md` documents some validations that depend only on the
   transform's own operand (`set_to_server_value` must be `REQUEST_TIME`;
   `increment`'s delta must itself be numeric), independent of the document's
   current state. These are checked at translation time
   (`handler.rs`, before any DB round trip), mirroring this codebase's own
   existing convention of validating shape before dispatching to a backend
   (`proto_fields_to_domain`'s own `None`-on-invalid-value pattern).
   Validations that genuinely require the current persisted value (non-numeric
   EXISTING field, integer overflow) can only be checked at apply time, inside
   the locked read-compute-write.

## Decision

### 1. `FieldTransform` grows from 1 variant to 6 (`embyr-core::storage::backend_adapter`)

```rust
#[derive(Debug, Clone)]
pub enum FieldTransform {
    ServerTimestamp(String),
    Increment(String, FieldValue),
    Maximum(String, FieldValue),
    Minimum(String, FieldValue),
    AppendMissingElements(String, Vec<FieldValue>),
    RemoveAllFromArray(String, Vec<FieldValue>),
}
```

Tuple-variant style, matching the existing (dead-code) `ServerTimestamp(String)`
shape unchanged — `String` is always the field path, matching
`DocumentTransform.FieldTransform.field_path`. `Increment`/`Maximum`/`Minimum`
carry the operand as `FieldValue` (validated at translation time to be
`Integer`/`Double`, never any other variant — see § Decision 4).
`AppendMissingElements`/`RemoveAllFromArray` carry `Vec<FieldValue>` (from
`ArrayValue.values`).

### 2. `Write::Update` gains a `transforms` field; `Write::Transform` unchanged in shape

```rust
Update {
    path: DocumentPath,
    fields: BTreeMap<String, FieldValue>,
    transforms: Vec<FieldTransform>,   // NEW — empty for every existing/transform-free write
    version: Option<i64>,
    precondition: Option<WritePrecondition>,
},
```

`Write::Transform { path, transforms }` is unchanged — it already models the
standalone case correctly. This is the ONE architecturally consequential
decision all three slices depend on (§ Decision Drivers 1, 2): a combined
update+transform `Write` message translates to exactly ONE `DomainWrite`
(`Write::Update` with a non-empty `transforms`), preserving the 1:1
`Vec<Write> → Vec<WriteResult>` contract every RPC (`Commit`, `Write`,
`BatchWrite`) already depends on.

**Every existing construction site of `Write::Update` needs `transforms: vec![]`
added** — a mechanical, zero-behavior-change compile fix, not new logic:
`crates/embyr-server/src/adapters/agent_backend.rs:198-201` (the
`embyr-agent` binary's own local translation of `AgentWrite` → `DomainWrite`;
the internal agent proto has zero transform message shape, per DISCUSS's own
confirmed finding, so this site's `transforms` is always empty — a permanent,
not temporary, `vec![]`, consistent with `backend_mode=agent` being out of
v1 scope for transforms).

### 3. `WriteResult` gains `transform_results: Vec<FieldValue>`

```rust
pub struct WriteResult {
    pub update_time: (i64, i32),
    pub create_time: Option<(i64, i32)>,
    pub transform_results: Vec<FieldValue>,   // NEW
}
```

Populated ONLY for value-producing transform kinds (`ServerTimestamp`,
`Increment`, `Maximum`, `Minimum`), in the relative order those kinds appear
within the write's own transform list — array-kind transforms
(`AppendMissingElements`/`RemoveAllFromArray`) never contribute an entry (see
ADR-053 § Escalation 2 Resolution for the full reasoning and the
`docs/SPEC.md` correction this depends on). Empty `vec![]` for every
transform-free write (the overwhelming majority).

**All 9 domain-layer construction sites need `transform_results` added.** 6
are mechanical (`transform_results: vec![]`, zero behavior change — these
paths never carry a transform): `crates/embyr-pg-storage/src/backend_adapter.rs:319,369,400,454,484`
(single-document `create_document`/`update_document`, used by
`CreateDocument`/`UpdateDocument`/`SetDocument` RPCs, which have no wire
representation for transforms at all — out of this feature's scope, same as
`backend_mode=agent`) and `crates/embyr-server/src/adapters/agent_backend.rs:344,375,575`
(the agent adapter's own `create_document`/`update_document`, plus its
`commit_transaction`'s response mapping — the agent's own proto response
never carries `transform_results` either). 3 carry REAL computed values —
`commit_transaction`'s own `Write::Update`/`Write::Transform` apply arms (§
Decision 5).

**All 6 proto-response call sites replace `transform_results: vec![]` with a
real mapping**: `wr.transform_results.iter().map(field_value_to_proto).collect()`
(reusing `crates/embyr-server/src/encoding/firestore_proto.rs::field_value_to_proto`
unchanged) — `handler.rs:1851-56` (`Commit`), `handler.rs:1949-51,1968-70,1982-2002`
(`BatchWrite`, 3 sites: translation failure, `begin_transaction` failure,
`commit_transaction` result — the first two stay `vec![]` since there is no
`WriteResult` to draw from on those failure paths; only the `Ok` arm at
~1986-1992 gets the real mapping), `write_stream.rs:165` (`Write` — the 6th
site DISCUSS's own count missed, corrected here).

### 4. Translation-time validation (`handler.rs`, before any Postgres round trip)

A single shared helper, called from BOTH the `Update` arm (for
`proto_write.update_transforms`) and the `Transform` arm (for
`dt.field_transforms`) — zero write-semantics logic duplicated, mirroring
`translate_one_write_for_commit`'s own established shared-helper discipline
(ADR-048 § Decision 4):

```rust
fn translate_field_transforms(
    field_transforms: &[embyr_proto::firestore::document_transform::FieldTransform],
) -> Result<Vec<FieldTransform>, Status>
```

Per entry, `transform_type` oneof match:
- `set_to_server_value`: value other than `REQUEST_TIME` (including
  `SERVER_VALUE_UNSPECIFIED`) → `Status::invalid_argument` immediately (AC-01-05).
  `REQUEST_TIME` → `FieldTransform::ServerTimestamp(field_path)`.
- `increment`/`maximum`/`minimum`: decode the operand via
  `proto_value_to_field_value` (reused unchanged); if the result is not
  `FieldValue::Integer`/`FieldValue::Double` → `Status::invalid_argument`
  (SPEC.md's own documented non-numeric-delta rule, extended by direct
  analogy to `maximum`/`minimum` per DISCUSS's own § Numeric
  Type-Preservation Findings). Otherwise `FieldTransform::Increment/Maximum/Minimum(field_path, value)`.
- `append_missing_elements`/`remove_all_from_array`: decode each `ArrayValue.values`
  entry via `proto_value_to_field_value`; any decode failure →
  `Status::invalid_argument("invalid field value in write")` (matches
  `proto_fields_to_domain`'s own existing error message for the identical
  failure mode). Otherwise `FieldTransform::AppendMissingElements/RemoveAllFromArray(field_path, values)`.

This is the ONLY validation performed before Postgres. Existing-value-dependent
validation (non-numeric existing field, integer overflow) happens at apply
time (§ Decision 5) — it cannot happen earlier, since it needs the FOR
UPDATE-locked read.

### 5. Atomicity mechanism — confirms DISCUSS's own recommendation, refines the exact shape

**Confirmed: extend `verify_versions`'s `FOR UPDATE` row-locking idiom to
read `fields`, compute in Rust, re-encode — not raw JSONB SQL arithmetic.**
DISCUSS's own § Atomicity Mechanism Investigation reasoning is sound and not
re-litigated (type-preservation and structural equality are natural Rust
match arms on the already-existing `FieldValue` enum, not natural `jsonb`
expressions). This ADR adds the exact shape DISCUSS left to DESIGN:

**5a. Pure compute function — new module, `embyr-core::domain::field_transform`**
(zero IO, matches CLAUDE.md's `embyr-core` constraint and this ADR's own
Decision Driver 4):

```rust
/// Applies one FieldTransform to `fields` in place. `current` is the value
/// already in `fields` at `field_path`, if any (the caller decides what
/// "current" means — see § 5b/5c for the two different bases used).
/// Returns `Some(value)` for value-producing kinds (ServerTimestamp/
/// Increment/Maximum/Minimum) — the entry to push onto WriteResult's own
/// transform_results, in call order. Returns `None` for array kinds (never
/// pushed — ADR-053 § Escalation 2 Resolution).
pub fn apply_field_transform(
    fields: &mut BTreeMap<String, FieldValue>,
    transform: &FieldTransform,
    now: (i64, i32),
) -> Result<Option<FieldValue>, CoreError>
```

Match arms (all pure, all independently unit-testable without a Postgres
fixture):
- `ServerTimestamp(path)`: always `FieldValue::Timestamp(now.0, now.1)` —
  unconditional, creates or overwrites. `Some(value)`.
- `Increment(path, delta)`: `fields.get(path)` — `None`/absent → base is
  `Integer(0)`/`Double(0.0)` matching `delta`'s own type (SPEC.md); `Some(Integer(_))`/`Some(Double(_))`
  → type-preserving add, promoting to `Double` if either side is `Double`
  (SPEC.md); `Some(anything else)` → `CoreError::InvalidArgument("increment target
  is not numeric")`. Integer+Integer overflow → `i64::checked_add`, `None` →
  `CoreError::InvalidArgument("increment overflow")` (ADR-053 § Escalation 1
  Resolution). `Some(value)`.
- `Maximum`/`Minimum(path, value)`: `fields.get(path)` — absent → set
  directly to `value` (ADR-053's own `docs/SPEC.md` addition, not the
  `increment`-style 0/0.0 baseline); present and numeric → type-preserving
  comparison (promote to `Double` for the comparison if either side is
  `Double`; result keeps the WINNING side's own original type, per SPEC.md's
  general type-preservation rule); present and non-numeric →
  `CoreError::InvalidArgument`. `Some(value)`.
- `AppendMissingElements(path, values)`: `fields.get(path)` — absent → the
  field becomes `values` as given, in order; present as `Array(existing)` →
  append each of `values` not already present in `existing`
  (`FieldValue::PartialEq`, already derived — zero new equality logic,
  DISCUSS's own confirmed reuse), preserving `values`'s own input order for
  the appended tail; present as anything else → `CoreError::InvalidArgument`
  (by direct analogy to the numeric non-numeric-target rule — not documented
  by SPEC.md for this kind, flagged as a residual below). `None` (never
  contributes to `transform_results`).
- `RemoveAllFromArray(path, values)`: `fields.get(path)` — absent → true
  no-op, `fields` unchanged, field NOT created (SPEC.md, explicit); present
  as `Array(existing)` → retain only elements not structurally equal
  (`PartialEq`) to any element of `values`; present as anything else →
  `CoreError::InvalidArgument` (same residual as above). `None`.

**5b. Standalone `Write::Transform { path, transforms }` — genuine partial merge onto the persisted document**

Before the main apply loop, for every `Write::Transform` (and every
`Write::Update` with non-empty `transforms`, § 5c), extend the existing
precondition-lock loop (`crates/embyr-pg-storage/src/backend_adapter.rs:1009-1047`,
alongside `verify_versions`) with one additional locked read per such write:

```sql
SELECT fields FROM documents
WHERE project_id = $1 AND collection_path = $2 AND document_id = $3
  AND NOT deleted
FOR UPDATE
```

(`sqlx::query_scalar::<_, Option<serde_json::Value>>().fetch_optional(...)`,
identical connection/transaction plumbing to `verify_versions`, decoded via
the already-existing `json_to_fields`; `None` row = document missing or
soft-deleted, base = `BTreeMap::new()`.) This is the ONE genuinely new SQL
statement this feature adds — everything else reuses the existing
`INSERT ... ON CONFLICT` idiom unchanged.

In the main apply loop's `Write::Transform` arm: `let mut base = locked_fields.unwrap_or_default();`
then `apply_field_transform(&mut base, t, now)?` for each transform in
order, collecting `Some` results into `transform_results`. `base` (now the
full, correctly-merged field map — untouched fields preserved, exactly the
partial-merge semantics a transform-only write requires) is written via the
SAME `INSERT ... ON CONFLICT DO UPDATE SET fields = $4::jsonb, ...`
statement `Write::Update` already uses — reused, not duplicated. A
transform-only write against a nonexistent document therefore upserts
(creates) it, identical to `Write::Update`'s own existing behavior for a new
document — consistent by construction, not a new special case.

**5c. Combined `Write::Update { fields, transforms, .. }` — transforms read against the PRE-existing persisted value, not against `fields`**

The proto's own wording ("the transforms to perform after update") and
US-02's own Domain Example 1 (`viewCount: 41`, already persisted from a
PRIOR write, then `increment(1)` → `42`) together fix the correct base: a
transform's "current value" must be the field's PERSISTED value before this
write, not the value (if any) the write's own `fields` map happens to set
for the same path — the realistic case is that `fields` does not mention the
transformed path at all (the SDK does not literally serialize a sentinel
into `fields`), so reading `fields` alone would silently treat an existing
counter as always-missing. Getting this wrong is not a hypothetical: it
would compute `increment(1)` against `viewCount:41` as `1`, not `42`.

Implementation: reuse the SAME locked read as § 5b (extend the lock-loop's
own write-selection filter to include `Write::Update` with non-empty
`transforms`, in addition to `Write::Transform`). Then, per write:

```rust
let mut final_fields = fields.clone();           // the regular update's own full-replacement map
let locked = locked_fields.unwrap_or_default();  // the pre-existing persisted state
for t in transforms {
    // apply_field_transform reads/writes final_fields, but the "current"
    // value it sees for `t`'s own field_path must come from `locked` if
    // `final_fields` doesn't already carry that path from the regular
    // update itself (the uncommon but not-impossible case where a client
    // writes and transforms the SAME path in one call).
    if !final_fields.contains_key(t.field_path()) {
        if let Some(v) = locked.get(t.field_path()) {
            final_fields.insert(t.field_path().to_string(), v.clone());
        }
    }
    let result = apply_field_transform(&mut final_fields, t, now)?;
    if let Some(v) = result { transform_results.push(v); }
}
```

(`field_path()` is a small accessor added to `FieldTransform`, one match
per variant — trivial, not worth a separate ADR decision.) `final_fields` —
the regular update fields, with each transformed path corrected to read
against the true persisted value first — is written via the EXISTING single
`INSERT ... ON CONFLICT` statement, unchanged in shape, now carrying a
Rust-computed map instead of the write's own raw `fields` unchanged. **Zero
new SQL statement for the combined case beyond the one shared locked read
(§ 5b) — the write itself is still exactly one round trip.**

**5d. Concurrency (AC-02-02)**: the `FOR UPDATE` lock (§ 5b/5c) serializes
any two `commit_transaction` calls targeting the same document row for the
duration of each call's own `pg_txn` — identical guarantee `verify_versions`
already provides for OCC, extended to a read-compute-write shape instead of
read-compare-abort. Two concurrent `increment(1)` calls against
`viewCount:41` each acquire the lock in turn, compute against the value the
PRIOR committer left, and both land — no lost update, matching KPI #2
without a new locking primitive or isolation level.

## Alternatives Considered

### Domain-model shape for the combined update+transform write

**A. Split one `Write` into two `DomainWrite`s** (a `Write::Update` for
`fields`, a `Write::Transform` for `transforms`, both pushed into
`commit_transaction`'s own `writes` vec). Rejected outright: violates §
Decision Driver 1 — `commit_transaction` would return 2 `WriteResult`s for 1
input `Write`, breaking the positional `write_results[i]`/`status[i]`
alignment `Commit` and `BatchWrite` both depend on (ADR-048 § Decision
Driver 2). Not a viable design regardless of any other merit.

**B. Add `transforms: Vec<FieldTransform>` to `Write::Update` (chosen).**
Preserves the 1:1 invariant exactly; mirrors the wire shape directly
(`update_transforms` is literally a field on the same `Write` message as
`update`); zero change to `Write::Transform`'s own shape (already correct
for the standalone case); every existing `Write::Update` construction site
needs one mechanical field addition (`transforms: vec![]`), not a
restructure.

**C. Unify `Write::Update`/`Write::Transform` into one variant carrying
`Option<BTreeMap<...>>` fields plus `transforms`.** Rejected: conflates two
genuinely different semantics behind one `Option` — `fields: None` (no
regular update, standalone transform, partial-merge apply) vs.
`fields: Some(BTreeMap::new())` (an explicit empty-fields update, full
overwrite to nothing) would need to mean different things to the apply
layer, an ambiguity the CURRENT two-variant design already avoids for free.
Achieves nothing `B` doesn't already achieve, at the cost of a real merge
risk.

### Where the locked `fields` read lives

**A. A new `BackendAdapter` trait method** (e.g.
`read_fields_for_transform`), called by `handler.rs` before
`commit_transaction`. Rejected: would require a SEPARATE, un-locked
round trip outside `commit_transaction`'s own `pg_txn` — the lock would not
hold across the gap between the read and the eventual apply, defeating the
entire atomicity purpose. Also a new port method for `PostgresBackendAdapter`/`AgentBackendAdapter`
both to implement, when `AgentBackendAdapter` has no transform support to
give it meaning (agent-mode out of scope).

**B. Inside `commit_transaction`'s own single `pg_txn`, alongside the
existing precondition-lock loop (chosen).** Same transaction, same lock,
zero new trait method — a direct extension of `verify_versions`'s own
already-proven idiom, per DISCUSS's own recommendation.

### Pure-compute function's crate placement

**A. Inside `embyr-pg-storage`, next to `commit_transaction`.** Considered:
simpler in the short term (no cross-crate function). Rejected: the
type-preservation/structural-equality logic has zero dependency on
Postgres or IO of any kind — placing it in the IO-bearing crate makes it
untestable without a database fixture and violates this project's own
"functional-where-practical" paradigm (CLAUDE.md) for no benefit.

**B. Inside `embyr-core::domain::field_transform` (chosen).** Zero IO
(`embyr-core`'s own hard constraint, `deny.toml`-enforced); independently
unit-testable; `embyr-pg-storage` depends on `embyr-core` already, zero new
crate dependency.

## Consequences

**Positive**: the domain-model shape preserves every existing positional
contract (`Commit`/`BatchWrite`/`Write` all keep working unchanged for
transform-free writes, AC-04 guardrail); the standalone-transform and
combined-update-transform cases share one pure compute function and one SQL
statement shape (`INSERT ... ON CONFLICT`), reused not duplicated; the
compute logic is independently unit-testable in `embyr-core` without a
Postgres fixture; zero new `BackendAdapter` trait method; zero new
`CoreError` variant (`InvalidArgument`, already present, covers every new
error case).

**Negative, named explicitly**: every transform-carrying write (standalone
or combined) now costs one additional `SELECT ... FOR UPDATE` round trip
inside `commit_transaction`'s own `pg_txn`, beyond what a transform-free
write costs — an accepted latency cost mirroring `BatchWrite`'s own accepted
per-write round-trip cost precedent (ADR-048 § Consequences), not evidenced
against a numeric target (DISCUSS's own DoR note: no NFR latency target set
for this feature).

**Negative, named explicitly**: this feature builds directly on top of
`Write::Update`'s existing full-overwrite (no `update_mask`) semantics
rather than closing that pre-existing gap — a combined update+transform
write's "current value" base is defined relative to that known-narrower
behavior (§ Decision 5c), which is internally consistent but means a future
fix to add real `update_mask` support will need to re-examine this ADR's own
§ 5c reasoning, not just the plain-`Update` path.

**Negative, flagged, deferred (not one of the two DISCUSS escalations, found
during this DESIGN pass)**: `Write::Transform` has no `precondition` field at
all, unlike `Write::Update`/`Write::Delete` — `translate_one_write_for_commit`'s
`Transform` arm already computes a `precondition` from `proto_write.current_document`
(line 694) but has nowhere to put it for a standalone transform write. A
`must_exist`/`must_not_exist`/`update_time` precondition attached to a
standalone transform write is silently discarded, before and after this
feature. Not fixed here — no AC in any of the three slices requires
precondition enforcement on transforms, and adding the field expands scope
beyond what DISCUSS defined. Named as a candidate follow-up, same treatment
as ADR-048's own transaction-row-hygiene finding.

**Residual, non-blocking**: `AppendMissingElements`/`RemoveAllFromArray`
against an existing NON-array value (e.g. a string) rejecting with
`InvalidArgument` is this architect's own reasonable extension by direct
analogy to the numeric non-numeric-target rule — `docs/SPEC.md` does not
document this case explicitly either before or after this feature's own
SPEC.md corrections (ADR-053). Same residual-finding treatment as
ADR-038/046/048's own precedents — confirm against real Firestore if
higher-confidence evidence surfaces; not blocking.
