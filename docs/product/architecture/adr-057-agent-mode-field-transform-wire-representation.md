# ADR-057: Agent-Mode Field Transform Wire Representation

## Status

Accepted

## Context

`firestore-field-transforms` (ADR-052/053) restored field-transform
computation for `direct_pg`/`aws_secret`/`gcp_secret` and confirmed, in its
own scoping, that `backend_mode=agent` had zero wire representation for
transforms — a permanent gap, not a temporary one, per `agent_backend.rs`'s
own comment (`crates/embyr-server/src/adapters/agent_backend.rs:583-587`).
This feature closes that gap.

DISCUSS's own Reading Confirmation made one claim load-bearing enough to
verify directly before designing anything: **the locked-read-compute-write
compute path (`apply_field_transform` + `commit_transaction`'s atomicity
mechanism, ADR-052 § Decision 5) is already reachable from `embyr-agent`'s
own `commit()` handler, unchanged.** Confirmed by direct reading, not trusted
from citation:

- `crates/embyr-agent/src/server.rs::commit` (line 613) calls
  `self.storage.commit_transaction(&pid, &txn_id, writes)`, where
  `self.storage: Arc<PostgresBackendAdapter>` — the exact same
  `embyr_pg_storage::backend_adapter::PostgresBackendAdapter` struct
  `direct_pg` uses server-side. Not a lookalike, not a parallel
  implementation — the identical type, imported directly
  (`crates/embyr-agent/src/server.rs:20`).
- `PostgresBackendAdapter::commit_transaction`
  (`crates/embyr-pg-storage/src/backend_adapter.rs:1064-1230`) already has
  live match arms for both `Write::Transform` (line 1186) and
  `Write::Update` with non-empty `transforms` (line 1116), calling
  `apply_field_transform` and populating `transform_results` — ADR-052's own
  delivered shape, confirmed present in the working tree, not merely
  documented in the ADR.

**Conclusion: the claim holds.** No new compute logic, no new
`BackendAdapter` trait method, no new SQL beyond what ADR-052 already added
(shared by construction, since it is the same adapter instance). This
feature's entire scope is: (1) a wire representation for transforms on
`storage_agent.proto`, since `Write` today (`proto/embyr/agent/v1/storage_agent.proto:318-325`)
carries only `oneof operation { Document update = 1; string delete = 2; }`
with no transform shape of any kind; (2) the two translation-layer edits at
the two ends of that wire — decode in `embyr-agent`, encode in
`embyr-server`.

**One correction to DISCUSS's own characterization**, found during this
DESIGN pass: DISCUSS's Technical Notes describe the `agent_backend.rs` edit
as "removing the `.filter_map` drop" (implying something close to a one-line
change). Confirmed directly
(`crates/embyr-server/src/adapters/agent_backend.rs:535-561`): the
`.filter_map` also silently strips `Write::Update`'s own `transforms` field
via its `{ path, fields, precondition, .. }` destructure — a SECOND discard
site DISCUSS's own Reading Confirmation did not separately name (it named
only the standalone `Write::Transform { .. } => None` arm). Both discards
must be fixed, and fixing them requires a genuine new encode function
(`FieldTransform` → agent proto), not a one-line deletion. The "wire-only,
no new compute logic" claim survives this correction intact — the new code
is translation, not computation — but "close to zero new logic" is more
accurate than "delete a filter."

**A second, unscoped finding, flagged not fixed**: `embyr-agent`'s own
`commit()` handler discards `commit_transaction`'s return value entirely
today — `Ok(_results) => Ok(Response::new(CommitResponse { commit_time:
Some(...), ..Default::default() }))` (`server.rs:614-620`). `write_results`
(and therefore `transform_results`) is never populated on the agent's own
`CommitResponse`, for ANY write type, transform or not — a pre-existing gap
orthogonal to this feature (it affects `update_time` confirmation for plain
document writes identically). This feature's own AC are satisfied without
touching it: US-01's AC verify transform effects via a subsequent read, not
via the immediate commit response payload (`docs/feature/agent-mode-field-transforms/slices/slice-01-wire-transforms-through.md`
§ Acceptance Criteria — every AC is phrased "reading ... afterward shows").
Named here as a candidate follow-up, same treatment as ADR-052's own
flagged residuals — not in scope, since fixing it would touch `commit()`'s
response-mapping logic for every write kind, not just transforms.

## Decision Drivers

1. **Wire shape must mirror the client-facing `document.proto`'s own
   `Write`/`DocumentTransform`/`DocumentTransform.FieldTransform`/`ServerValue`
   shape** (`proto/google/firestore/v1/document.proto:90-152`) — same two
   representations (a standalone transform-only write via a third `oneof
   operation` variant; `update_transforms` attached alongside a regular
   `update`), same 6-variant `FieldTransform.transform_type` oneof — so the
   domain-model mapping (ADR-052 § Decision 2) applies with zero
   reinterpretation.
2. **No cross-proto imports** — `storage_agent.proto`'s own file-level
   comment states its message types intentionally mirror Firestore
   semantics without importing `google.firestore.v1` types. New messages are
   authored fresh in `embyr.agent.v1`, not imported.
3. **Match this proto's own existing flattening convention, not Google's
   nesting convention** — `storage_agent.proto` already prefers top-level
   messages over nested ones for structurally identical concepts
   (`FieldFilterProto`/`CompositeFilterProto` are top-level, where
   `document.proto`'s own `StructuredQuery.Filter` nests them). A top-level
   `FieldTransform` message (not nested inside a wrapper), and a top-level
   `Transform` wrapper message (in place of naming a local equivalent of
   `DocumentTransform`) for the standalone-write case, matches the
   established local style and is simpler for `prost` codegen (avoids
   `embyr_proto::agent::transform::FieldTransform`-style nested paths).
4. **Reuse every existing FieldValue↔proto Value translation function
   unchanged** — `crates/embyr-agent/src/encoding.rs::proto_value_to_field_value`
   (decode) and `crates/embyr-server/src/adapters/agent_backend.rs::field_value_to_agent_value`
   (encode) already handle every `FieldValue` variant a transform operand
   can carry (`Integer`/`Double` for `increment`/`maximum`/`minimum`;
   `Array` elements for `append_missing_elements`/`remove_all_from_array`).
   Zero new value-encoding logic — confirmed by direct reading of both
   functions, not assumed from the ADR-052 precedent alone.
5. **Validation semantics reused from ADR-052 § Decision 4, not
   reinvented** — `set_to_server_value` must be `REQUEST_TIME`;
   `increment`/`maximum`/`minimum` operands must decode to
   `FieldValue::Integer`/`Double`. `embyr-agent`'s own
   `proto_value_to_field_value` decodes unconditionally (no `Option`/`Result`
   — unlike `embyr-server`'s equivalent), so the type check happens by
   matching the decoded `FieldValue` variant post-hoc, exactly as ADR-052
   Decision 4 already does for the non-agent path.

## Decision

### 1. `storage_agent.proto` additions

```proto
// A transformation of a single field, computed server-side. Mirrors
// google.firestore.v1.DocumentTransform.FieldTransform's own oneof shape,
// authored fresh per this proto's own no-cross-proto-import convention.
message FieldTransform {
  string field_path = 1;

  oneof transform_type {
    ServerValue set_to_server_value = 2;
    Value increment = 3;
    Value maximum = 4;
    Value minimum = 5;
    ArrayValue append_missing_elements = 6;
    ArrayValue remove_all_from_array = 7;
  }
}

// A value that is calculated by the server.
enum ServerValue {
  SERVER_VALUE_UNSPECIFIED = 0;
  REQUEST_TIME = 1;
}

// A standalone transform-only write on a document — no accompanying field
// update. Mirrors google.firestore.v1.DocumentTransform's role.
message Transform {
  string document = 1;
  repeated FieldTransform field_transforms = 2;
}
```

`Write` grows a third `oneof operation` variant and one new repeated field
(both previously-unused field numbers on the message):

```proto
message Write {
  oneof operation {
    Document update = 1;
    string delete = 2;
    Transform transform = 5;          // NEW — standalone transform-only write
  }
  DocumentMask update_mask = 3;
  Precondition current_document = 4;
  repeated FieldTransform update_transforms = 6;   // NEW — transforms after `update`
}
```

No other message on `storage_agent.proto` changes. `CommitRequest`,
`CommitResponse`, `WriteResult` are untouched by this ADR — this feature does
not address the pre-existing `write_results`/`transform_results` gap on the
agent's own `CommitResponse` (see § Context, flagged not fixed).

### 2. Decode side — `crates/embyr-agent/src/server.rs::proto_write_to_domain`

New shared helper, mirroring `handler.rs`'s own `translate_field_transforms`
(ADR-052 § Decision 4) but adapted to `embyr-agent`'s own infallible
`proto_value_to_field_value`:

```rust
fn translate_field_transforms(
    field_transforms: &[embyr_proto::agent::FieldTransform],
) -> Result<Vec<CoreFieldTransform>, Status>
```

Per entry, match `transform_type`: `set_to_server_value` must decode to
`ServerValue::RequestTime` (any other value, including `Unspecified`, is
`Status::invalid_argument`) → `CoreFieldTransform::ServerTimestamp(field_path)`;
`increment`/`maximum`/`minimum` decode the operand via
`proto_value_to_field_value`, reject (via `Status::invalid_argument`) if the
resulting `FieldValue` is not `Integer`/`Double`; `append_missing_elements`/
`remove_all_from_array` decode each `ArrayValue.values` entry the same way.

`proto_write_to_domain` gains a third match arm and calls the new helper for
both attachment points:

```rust
fn proto_write_to_domain(w: embyr_proto::agent::Write, project_id_str: &str) -> Result<DomainWrite, Status> {
    let precondition = parse_precondition(w.current_document);
    let update_transforms = translate_field_transforms(&w.update_transforms)?;
    match w.operation {
        Some(Operation::Update(doc)) => {
            let path = parse_document_name(&doc.name, project_id_str)?;
            let fields = proto_fields_to_domain(doc.fields);
            Ok(DomainWrite::Update { path, fields, version: None, precondition, transforms: update_transforms })
        }
        Some(Operation::Delete(name)) => {
            let path = parse_document_name(&name, project_id_str)?;
            Ok(DomainWrite::Delete { path, version: None, precondition })
        }
        Some(Operation::Transform(t)) => {
            let path = parse_document_name(&t.document, project_id_str)?;
            let transforms = translate_field_transforms(&t.field_transforms)?;
            Ok(DomainWrite::Transform { path, transforms })
        }
        None => Err(Status::invalid_argument("write operation required")),
    }
}
```

### 3. Encode side — `crates/embyr-server/src/adapters/agent_backend.rs::commit_transaction`

New shared helper, symmetric to the decode side, reusing the existing
`field_value_to_agent_value`:

```rust
fn field_transform_to_agent(t: &CoreFieldTransform) -> AgentFieldTransform
```

One match arm per `CoreFieldTransform` variant (`ServerTimestamp` →
`set_to_server_value: RequestTime`; `Increment`/`Maximum`/`Minimum` →
`field_value_to_agent_value` on the operand; `AppendMissingElements`/
`RemoveAllFromArray` → `field_value_to_agent_value` mapped over the `Vec`,
wrapped in `AgentArrayValue`).

`commit_transaction`'s `.filter_map` becomes a `.map` (no variant is dropped
any more) and gains a `Write::Transform` arm and a `transforms`-aware
`Write::Update` arm:

```rust
let agent_writes: Vec<AgentWrite> = writes
    .into_iter()
    .map(|w| match w {
        Write::Update { path, fields, precondition, transforms, .. } => {
            let name = domain_path_to_agent_name(&path);
            let doc = AgentDocument { name, fields: fields_to_agent_map(&fields), ..Default::default() };
            AgentWrite {
                operation: Some(Operation::Update(doc)),
                current_document: precondition.as_ref().map(precondition_to_agent),
                update_transforms: transforms.iter().map(field_transform_to_agent).collect(),
                ..Default::default()
            }
        }
        Write::Delete { path, precondition, .. } => {
            let name = domain_path_to_agent_name(&path);
            AgentWrite {
                operation: Some(Operation::Delete(name)),
                current_document: precondition.as_ref().map(precondition_to_agent),
                ..Default::default()
            }
        }
        Write::Transform { path, transforms } => {
            let name = domain_path_to_agent_name(&path);
            AgentWrite {
                operation: Some(Operation::Transform(AgentTransform {
                    document: name,
                    field_transforms: transforms.iter().map(field_transform_to_agent).collect(),
                })),
                current_document: None,   // Write::Transform has no precondition field — pre-existing gap, ADR-052 § Consequences, not this feature's scope
                ..Default::default()
            }
        }
    })
    .collect();
```

`transform_results: vec![]` in the response-mapping loop
(`agent_backend.rs:577-588`) is unchanged by this ADR — see § Context for
why (the agent's own `commit()` never populates `write_results` for any
write kind; a separate, unscoped gap).

## Alternatives Considered

### Wire representation shape for the standalone transform-only write

**A. Reuse `Write.update` with an empty `Document` and rely on
`update_transforms` alone (no third oneof variant).** Rejected: conflates
"a transform-only write" with "an update that happens to write zero
fields," which are different operations in the domain model
(`Write::Update` vs `Write::Transform`, ADR-052 § Decision 2) — the decode
side would have no reliable signal to pick the right `DomainWrite` variant,
forcing a fragile "empty fields map means Transform" heuristic. A real SDK
write with a legitimately empty regular-field set (existing, if narrow,
possibility) would be misrouted.

**B. Third `oneof operation` variant (`Transform`), chosen.** Mirrors
`document.proto`'s own `Write.operation`'s `transform` variant exactly (field
6 there, field 5 here — number reused for shape-parity of intent, not
wire-compatibility, since this proto never round-trips with
`google.firestore.v1` bytes). Unambiguous on the decode side: the oneof tag
alone determines which `DomainWrite` variant to construct.

### `FieldTransform` message placement

**A. Nest `FieldTransform` inside `Transform`, mirroring `document.proto`'s
own `DocumentTransform.FieldTransform` nesting exactly.** Considered for
maximal mirroring. Rejected: `storage_agent.proto` already deviates from
Google's own nesting style elsewhere (`FieldFilterProto`/
`CompositeFilterProto` are top-level, not nested under `StructuredQuery`) —
nesting here would be inconsistent with this proto's own established local
convention, and `update_transforms` (attached directly to `Write`, not to a
`Transform`) needs the type to be nameable outside any `Transform` wrapper
regardless, so top-level is required for at least one of the two attachment
points.

**B. Top-level `FieldTransform` message, referenced by both `Write.update_transforms`
and `Transform.field_transforms` (chosen).** One message type, two
attachment points, consistent with local convention, simpler generated Rust
path (`embyr_proto::agent::FieldTransform`, not
`embyr_proto::agent::transform::FieldTransform`).

### Where the agent-side translation validation lives

**A. Validate inside `commit()` before calling `proto_write_to_domain`.**
Rejected: scatters validation across two functions for no reason;
`proto_write_to_domain` is already the single per-write translation
boundary (mirrors `handler.rs`'s own `translate_one_write_for_commit`
precedent, ADR-048 § Decision 4).

**B. Validate inside a new `translate_field_transforms` helper, called from
`proto_write_to_domain`'s `Update` and `Transform` arms (chosen).** Mirrors
`handler.rs`'s own established shared-helper discipline exactly — same
pattern, different crate.

## Consequences

**Positive**: zero new compute logic (confirmed, not assumed, by direct
reading of `PostgresBackendAdapter::commit_transaction`'s live match arms);
zero new `BackendAdapter` trait method; zero new SQL statement (the locked
`fields` read ADR-052 added is already exercised identically, since
`embyr-agent` calls the same adapter type); zero new value-encoding
function (`field_value_to_agent_value`/`proto_value_to_field_value` reused
unchanged in both directions); the wire shape mirrors
`google.firestore.v1.DocumentTransform` closely enough that a future reader
familiar with ADR-052 needs no new mental model, only the proto-level
renaming (`Transform` in place of `DocumentTransform`).

**Negative, named explicitly**: two new, genuinely non-trivial translation
functions are required (`translate_field_transforms` on the decode side,
`field_transform_to_agent` on the encode side) — DISCUSS's own
characterization ("removing the `.filter_map` drop," implying a near-trivial
diff) undersold this by one finding (the `Write::Update`-attached
`transforms` discard, not just the standalone `Write::Transform` discard).
Still the smallest of the four sibling features by a wide margin — two
translation functions plus a proto edit, no new adapter, no new port, no new
SQL.

**Negative, flagged, deferred (not this feature's scope)**: `embyr-agent`'s
own `commit()` handler does not populate `CommitResponse.write_results` for
any write kind today, so `transform_results` will not be visible in the
immediate commit response for agent-mode transforms even after this
feature ships — only via a subsequent read. This feature's own AC do not
require the immediate-response path (§ Context) and do not close this gap.
Named as a candidate follow-up.

**Negative, inherited not re-derived**: cross-version graceful degradation
(an older `embyr-agent` binary receiving a `Write.transform`/
`update_transforms` it does not know about, or a newer binary's response
being consumed by an older `embyr-server`) is the shared, still-open
question the `agent-mode-write-streaming` DESIGN wave is resolving
concurrently (its own Escalation 2). This feature does not build its own
mechanism and defers entirely to that resolution. This feature's own risk
on this axis is confirmed lower than the two sibling features that add
entirely new RPCs: an unrecognized future `Write.operation` oneof variant
already has a safe existing fallback,
`proto_write_to_domain`'s own `None => Err(Status::invalid_argument("write
operation required"))` (`server.rs:207`, unchanged by this ADR) — a
structurally impossible-to-not-hit `None` arm for any oneof value the
`prost`-generated enum doesn't recognize. No new code is required for that
specific case. What is NOT covered by this existing fallback, and remains
open pending the sibling resolution: an OLDER `embyr-server` talking to a
NEWER `embyr-agent` that already computes transforms, or a NEWER
`embyr-server` sending `update_transforms` to an OLDER `embyr-agent` binary
that has this ADR's proto change compiled in but is running rolled-back
application logic — genuine skew scenarios outside a single oneof match.

## Numbering Note

Originally drafted as ADR-056 (highest number at the time this DESIGN wave
began was ADR-055). A concurrent sibling DESIGN wave
(`agent-mode-transaction-sweeper`-adjacent work) claimed ADR-056
(`adr-056-agent-transaction-sweeper-purge-sql-and-retention-config.md`)
before this file was written to disk. Renumbered to ADR-057 per this
session's own standing instruction ("if your chosen number collides when
you go to write, increment and note it"). The stub left at the original
`adr-056-agent-mode-field-transform-wire-representation.md` path points here.
