# Feature Delta: firestore-is-null-filter-support

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml`, JOB-01 (`sdk-compat`, persona P1 Alex) — read in full.
✓ `docs/product/known-gaps.md` #4 — `IS_NULL`/`IS_NOT_NULL` unary filter rejected
(`crates/embyr-server/src/grpc/handler.rs:4015-4019` at scan time), flagged "likely reachable —
some SDKs lower `where(f,'==',null)` to this shape; unconfirmed, needs live-SDK verification."
✓ `proto/google/firestore/v1/query.proto` lines 111-128 — confirmed `UnaryFilter.Operator` is a
real, GA proto enum with 4 members: `IS_NAN`(2), `IS_NULL`(3), `IS_NOT_NAN`(4), `IS_NOT_NULL`(5).
This is NOT a hypothetical shape — it is the SAME message already partially handled (`IS_NAN`/
`IS_NOT_NAN` shipped in an earlier feature); `IS_NULL`/`IS_NOT_NULL` are its two siblings, currently
falling into `translate_filter`'s own `_ => return Some(Err(...))` catch-all
(`crates/embyr-server/src/grpc/handler.rs:4094-4098`).
✓ `crates/embyr-core/src/domain/query.rs:89-102` — confirmed `FilterOp` already has `IsNan`/
`IsNotNan` variants; `IsNull`/`IsNotNull` do not exist yet.
✓ `crates/embyr-pg-storage/src/encoding/field_value.rs` — confirmed `FieldValue::Null` encodes as
`{"t": "N"}` (no `v` key at all, unlike every other variant) — this is the structural fact
`IS_NULL`'s own SQL predicate depends on.
✓ Blast-radius confirmation: grepped every existing `FilterOp::IsNan`/`IsNotNan` reference across
the workspace — exactly 4 call sites in 3 files (`handler.rs`'s `translate_filter`,
`agent_backend.rs`'s `domain_filter_op_to_agent`, `encoding/query.rs`'s `append_field_filter`
special-case block), plus the `FilterOp` enum definition itself = 5 required touch points total.
`filter_binds_field_to_uid` (the OR-filter feature's own security-critical function) needs ZERO
changes: it matches on `QueryFilter::Field(ff)` and checks `ff.op == FilterOp::Equal` inline, not
an exhaustive match over `FilterOp` — `IsNull`/`IsNotNull` correctly fall through to `false`
(neither establishes an ownership-equality binding) with no code change required. `embyr-agent`'s
own `proto_filter_to_domain` (the reverse direction) needs ZERO changes: the agent's own internal
`FieldFilterOp` proto (`proto/embyr/agent/v1/storage_agent.proto:148-160`) has no `IS_NAN`-shaped
member at all, so it structurally can never produce a `FilterOp::IsNull`/`IsNotNull` in the first
place — same reasoning already established for `IsNan`/`IsNotNan` and for `firestore-or-filter-
support`'s own `CompositeOr`.

**No live web verification needed this DISCUSS.** Real Firestore's documented semantics for
`.where(field, '==', null)` (lowered by every official SDK to `IS_NULL`) and
`.where(field, '!=', null)` (lowered to `IS_NOT_NULL`) are uncontroversial, ordinary Firestore
behavior, not a contested edge case: `IS_NULL` matches only documents where the field is
EXPLICITLY present with value `null` (a document missing the field entirely does not match);
`IS_NOT_NULL` matches documents where the field is present AND not `null` (a document missing the
field does not match either). This resolves known-gaps.md #4's own "unconfirmed, needs
verification" framing — it IS reachable: every official Firebase SDK (`this.where('field', '==',
null)` in the JS/Node Admin SDK, equivalently in Python/Go/Java) generates exactly this proto
shape, since a raw `==`/`!=` comparison against `null` cannot be expressed as an ordinary
`FieldFilter` (Firestore has no total ordering that includes `null`, so equality-to-null is
special-cased into a unary operator at the SDK layer, mirroring how `NaN` comparisons work — the
SAME reason `IS_NAN`/`IS_NOT_NAN` exist as unary ops rather than `FieldFilter.EQUAL` against a NaN
value).

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** (Decision 1) — a small, mechanical addition confined to 3 files, no
  new domain concepts.
- JTBD: **reuse JOB-01** (Decision 4 = "Yes", existing job) — the same "close a documented gap in
  ordinary SDK compatibility" realization as every filter-support feature this session.
- Walking Skeleton: **Yes** (Decision 2) — `IS_NULL` proven end-to-end first (the more common of
  the two operators in real client code — `where(field, '==', null)` is far more idiomatic than
  `where(field, '!=', null)`), then `IS_NOT_NULL` as the second half of the same slice.
- UX Research Depth: **Lightweight** (Decision 3) — closing a documented, narrowly-scoped
  compatibility gap, one persona, no new emotional arc, no security dimension (confirmed above).

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P1 Alex (SDK Developer), unchanged.

**Job**: JOB-01 `sdk-compat`, unchanged job_story. This feature's own realization: any of Alex's
real Firebase app's queries using `.where(field, '=='|'!=', null)` — an ordinary, idiomatic way to
filter for/against explicitly-null fields — is rejected today with a clean `INVALID_ARGUMENT`
instead of executing. After this feature, both operators execute with real Firestore's own
documented semantics.

## Wave: DISCUSS / [REF] Scope Assessment: PASS

Single bounded-context touch (query filter translation + SQL generation), 5 files total, no new
abstractions, effort well under 1 day. Not oversized — no split needed.

## Wave: DISCUSS / [REF] User Stories

### US-01: `IS_NULL` and `IS_NOT_NULL` filters execute with correct semantics

**job_id**: JOB-01

**Elevator Pitch**
- **Before**: `.where('deletedAt', '==', null)` (or the `!=` equivalent) against embyr returns a
  clean `INVALID_ARGUMENT` — the query never runs.
- **After**: the same query runs and returns exactly the documents real Firestore would: `IS_NULL`
  matches documents with the field explicitly present and `null`; `IS_NOT_NULL` matches documents
  with the field present and not `null`. Neither matches documents missing the field entirely.
- **Decision enabled**: Alex can port an app that filters for explicitly-null/non-null fields
  (a common soft-delete or optional-field pattern) without hitting an unsupported-operator error.

**Acceptance Criteria**
- **AC-IN-01**: A query with `IS_NULL` on a field returns only documents where that field is
  present and its value is `null`.
- **AC-IN-02**: A query with `IS_NULL` does NOT return documents missing the field entirely.
- **AC-IN-03**: A query with `IS_NOT_NULL` returns only documents where the field is present and
  its value is NOT `null`.
- **AC-IN-04**: A query with `IS_NOT_NULL` does NOT return documents missing the field entirely.
- **AC-IN-05**: `backend_mode=agent` cleanly rejects both operators (the agent's own internal
  proto has no equivalent), mirroring the existing `IS_NAN`/`IS_NOT_NAN` rejection pattern exactly
  — not a silent misrepresentation.
- **AC-IN-06**: Security-rule ownership-compliance checking (`filter_binds_field_to_uid`) correctly
  treats `IS_NULL`/`IS_NOT_NULL` filters as NOT establishing an ownership binding (neither can ever
  prove `field == callerUid`) — verified by an existing-shape test, not new logic.

## Wave: DISCUSS / [REF] Definition of Done

1. Both operators translate from proto to domain without error.
2. Both operators produce correct SQL predicates matching real Firestore's documented semantics
   (field present + matching null-ness; missing field never matches either).
3. `backend_mode=agent` rejects cleanly (AC-IN-05).
4. Security compliance checking unaffected (AC-IN-06) — no code change expected, verified by test.
5. Composite-index-requirement checking (`collect_filter_fields`) needs no new logic — `IsNull`/
   `IsNotNull` flow through the existing generic field-collection walk unchanged, and neither
   operator participates in the `IN`+range composite-index trigger rule (matches real Firestore:
   an `IS_NULL`/`IS_NOT_NULL` filter alone never requires a composite index).
6. Full regression suite clean (pre-existing flakes excepted, triaged not assumed).
7. Mutation testing: 0 missed on all viable mutants, especially the new SQL predicates and the
   proto-to-domain translation arms.
8. Evolution doc written, `known-gaps.md` #4 updated to CLOSED.
9. Memory updated.

## Wave: DISCUSS / [REF] Out of Scope

- Any OTHER unary filter shape — the proto's own `Operator` enum has exactly these 4 members
  (`IS_NAN`/`IS_NULL`/`IS_NOT_NAN`/`IS_NOT_NULL`); this feature closes the last 2, nothing further
  remains in this shape.
- `backend_mode=agent` execution of `IS_NULL`/`IS_NOT_NULL` — deferred, same reasoning as
  `IS_NAN`/`IS_NOT_NAN` and `firestore-or-filter-support`'s own agent-mode OR deferral: requires a
  proto change to the agent's own internal wire format, a separate concern.
- Composite `IS_NULL`/`IS_NOT_NULL` interaction with `IN`+range composite-index rules beyond "it
  flows through unchanged" — real Firestore's own composite-index requirements for unary filters
  combined with other filter types are not investigated further here; no evidence this feature's
  own scope needs it.

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy A** (direct implementation, no scaffolding) — this is a same-shape extension of an
already-proven mechanism (`IS_NAN`/`IS_NOT_NAN`), not new architecture.

## Wave: DISCUSS / [REF] Driving Ports

Unchanged — `RunQuery`'s existing gRPC surface (`:8080`) and REST/gRPC-Web surface (`:8081`).

## Wave: DISCUSS / [REF] Pre-requisites

None beyond what already exists — `UnaryFilter` translation, `FieldFilter`'s JSONB storage
encoding, and `append_field_filter`'s special-case dispatch pattern are all already in place from
`IS_NAN`/`IS_NOT_NAN`.

---

## Wave: DESIGN

### D1 — `FilterOp` gains 2 new variants

```rust
// crates/embyr-core/src/domain/query.rs
pub enum FilterOp {
    // ...existing...
    IsNan,
    IsNotNan,
    IsNull,
    IsNotNull,
}
```

### D2 — `translate_filter`'s `UnaryFilter` arm gains 2 new match arms

```rust
// crates/embyr-server/src/grpc/handler.rs
let op = match UnaryOp::try_from(uf.op).ok()? {
    UnaryOp::IsNan => FilterOp::IsNan,
    UnaryOp::IsNotNan => FilterOp::IsNotNan,
    UnaryOp::IsNull => FilterOp::IsNull,
    UnaryOp::IsNotNull => FilterOp::IsNotNull,
    _ => return Some(Err(format!("unsupported unary filter op: {}", uf.op))),
};
```

(The existing `// IS_NAN and IS_NOT_NAN use no value; provide a sentinel Null value.` comment
above the constructed `FieldFilter` already correctly describes `IsNull`/`IsNotNull` too — no
change needed there, both share the same "no value" shape.)

### D3 — `append_field_filter` gains 2 new special-case SQL arms

Storage fact from Reading Confirmation: `FieldValue::Null` encodes as `{"t": "N"}` — no `v` key.
`IS_NULL` needs the field PRESENT with type tag `N`; `IS_NOT_NULL` needs the field PRESENT with a
type tag that is NOT `N` (a missing field matches neither, per real Firestore semantics — this is
why both arms explicitly require presence, unlike `IS_NOT_NAN`'s own existing arm which treats a
missing field as passing "not nan," a pre-existing decision from a different feature not
relitigated here).

```rust
// crates/embyr-pg-storage/src/encoding/query.rs, inside append_field_filter's
// existing IS_NAN/IS_NOT_NAN special-case match:
FilterOp::IsNull => {
    qb.push(format!("fields->'{}'->>'t' = 'N'", f.field_path));
    return;
}
FilterOp::IsNotNull => {
    qb.push(format!(
        "(fields->'{fp}' IS NOT NULL AND fields->'{fp}'->>'t' != 'N')",
        fp = f.field_path
    ));
    return;
}
```

### D4 — `domain_filter_op_to_agent` rejects cleanly, mirroring `IsNan`/`IsNotNan`

```rust
// crates/embyr-server/src/adapters/agent_backend.rs
FilterOp::IsNan | FilterOp::IsNotNan | FilterOp::IsNull | FilterOp::IsNotNull => {
    return Err(CoreError::InvalidArgument(
        "IS_NAN/IS_NOT_NAN/IS_NULL/IS_NOT_NULL filters are not supported in backend_mode=agent"
            .into(),
    ));
}
```

### D5 — No changes needed (confirmed, not assumed)

- `filter_binds_field_to_uid` — inline `.op == FilterOp::Equal` check, not exhaustive.
- `collect_filter_fields` — generic `(field_path, op)` tuple collection, no per-op branching.
- `requires_composite_index`'s `IN`+range rule — only matches `FilterOp::In` and the 4 range
  comparison ops; `IsNull`/`IsNotNull` never participate.
- `embyr-agent`'s own `proto_filter_to_domain` — agent proto structurally cannot produce these ops.

## Wave: DESIGN / Handoff Package

5 files, 5 required edits (enum + 4 call sites), all confirmed via direct reading, zero
speculative touch points. Compiler exhaustiveness checking on `FilterOp` (used in 2 of the 4
match sites — `append_field_filter`'s inner match and `domain_filter_op_to_agent`'s match; the
other 2 sites are non-exhaustive `match ... { X => .., _ => .. }` shapes) will catch any missed
arm at `cargo check` time for those 2 sites.
