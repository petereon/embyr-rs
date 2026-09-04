# ADR-066: Cross-Document Reads — Two-Phase Evaluation (Path-Discovery → Fetch → Evaluate)

## Status

Accepted

## Context

`security-rules-cel-cross-document-reads` (JOB-17, 12th realization, "Epic 4d") lets Alex import a
narrowly-scoped, single-level cross-document `get()`/`exists()` role-lookup idiom (e.g.
`get(/databases/$(database)/documents/organizations/$(request.auth.uid)).data.role == "admin"`) —
the single most common real-Firestore idiom needing a cross-document read, evidenced by analogy
(no prior Trailmark domain example existed; grounded in the canonical published pattern and real
Firestore's own documented `get()`/`exists()` semantics, independently web-verified this DISCUSS).
DISCUSS (`docs/feature/security-rules-cel-cross-document-reads/feature-delta.md`, Resolutions 1–5)
locked: single-level only (no chaining), `$(request.auth.uid)`/`$(request.path.<var>)`
substitution only, no artificial read-budget cap, read+write parity (Listen's own per-event
re-check explicitly deferred) — and, the central architectural question this ADR resolves,
confirmed a mechanism design for the finding `security-rules-cel-parity`'s own DISCUSS already
flagged: **`get()`/`exists()` require BC-4 to actively call into BC-2's read path mid-evaluation,
which `evaluate()` cannot itself do — `embyr-core` is IO-forbidden, `deny.toml`-enforced, a hard
CI-tooling boundary, not a style preference.**

Confirmed by direct code read (`crates/embyr-core/src/storage/backend_adapter.rs`,
`crates/embyr-core/src/domain/document.rs`): `BackendAdapter::get_document(&self, path:
&DocumentPath)` ALREADY accepts an arbitrary `DocumentPath` (`{project_id, collection_path,
document_id}`) — not restricted to the request's own current document. **No new port method or
adapter capability is needed** — the fetch step this feature builds is a NEW CALL SITE for an
EXISTING method, never a new capability on the adapter trait itself.

## Decision Drivers

1. **`embyr-core` stays genuinely zero-IO** — the ONLY non-negotiable constraint (DISCUSS
   Resolution 1). Every new pure function this ADR designs performs zero I/O; every I/O step lives
   in `embyr-server`, which already has it.
2. **Reuse the SAME "pre-resolve, thread in as a parameter" pattern 3 prior epics already
   established** (`path_variable_value`, ADR-062; `ancestor_path_variable_values`, ADR-063;
   `request_time`, ADR-065) — never a structurally new kind of `evaluate()` extension.
3. **Deduplicate for free** — real Firestore's own documented "cached calls don't count toward the
   limit" behavior (independently web-verified) is trivially free to implement once path-discovery
   returns a SET, not a list; build it deliberately, not accidentally.
4. **Cheap on the hot path when a condition has zero cross-document operands** — the overwhelmingly
   common case (every pre-existing rule, and most new ones) must incur ZERO extra I/O, ZERO extra
   allocation beyond an empty-map construction. Path-discovery walking a `Condition` tree with no
   `CrossDocumentGet`/`CrossDocumentExists` operands anywhere returns an empty set immediately.
5. **Simplest solution first**: reuse the existing `get_document` port method unchanged; no new
   trait method, no new adapter capability, no batched-fetch optimization (a per-path sequential
   fetch is correct and sufficient — real Firestore's own 10/20-read ceiling confirms this scale
   is small by design; DISCUSS Resolution 4 locks NO artificial cap given the single-level-only
   structural bound already keeps counts small).

## Decision — Types (`access_control/mod.rs`, EXTEND)

### `PathTemplate`, new type

```rust
/// A `get()`/`exists()` path argument, decomposed into alternating literal
/// segments and substitutions — mirrors `rules_file::PathSegment`'s own
/// literal/wildcard-segment design almost exactly (ADR-062's own precedent),
/// just for a differently-shaped consumer (a runtime path-BUILDING
/// template, not an import-time path-MATCHING pattern). The leading
/// `/databases/$(database)/documents/` prefix is REQUIRED syntax
/// (mirrors real Firestore's own fully-specified-path requirement,
/// independently web-verified) but is NEVER stored as segments — `$
/// (database)` always resolves to the current `project_id`, structurally,
/// not via a stored substitution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathTemplate {
    pub segments: Vec<PathTemplateSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathTemplateSegment {
    Literal(String),
    /// `$(request.auth.uid)` (DISCUSS Resolution 3).
    AuthUid,
    /// `$(request.path.<var>)` (DISCUSS Resolution 3) — resolves against
    /// the SAME `path_variable_value`/`ancestor_path_variable_values`
    /// bindings every other `PathVariable`-consuming operand already uses.
    PathVariable(String),
}
```

### `Operand`, 2 new variants

```rust
pub enum Operand {
    // ...existing variants unchanged...
    /// `exists(<path template>)` (Slice 01, US-01).
    CrossDocumentExists(PathTemplate),
    /// `get(<path template>).data.<field>` (Slice 02, US-02) — the `.data.
    /// <field>` suffix is REQUIRED syntax (DISCUSS locked scope: no
    /// arbitrary nested access, no bare `get(...)` operand — mirrors
    /// `OQ-CEG-01`'s own "no nested map traversal" boundary, reapplied).
    CrossDocumentGet(PathTemplate, String),
}
```

## Decision — Path-Discovery (NEW, `access_control/mod.rs`, pure, zero-IO)

```rust
/// security-rules-cel-cross-document-reads (Slice 01, US-01, ADR-066 §
/// Decision Driver 1): walks the parsed `Condition` tree, resolving every
/// `CrossDocumentGet`/`CrossDocumentExists` operand's own `PathTemplate`
/// against the SAME already-known bindings `resolve_field_value` already
/// has at the call site, into a concrete path STRING (e.g.
/// `"organizations/uid123"`) — never a `DocumentPath` (that's a domain
/// type this pure function's own crate boundary must not construct with
/// I/O intent; `embyr-server`'s own fetch step does that translation).
/// Returns the DISTINCT SET of paths to fetch — Decision Driver 3's own
/// free deduplication falls out of using a `BTreeSet`, not a `Vec`.
/// `Ok(BTreeSet::new())` for a condition with zero cross-document operands
/// anywhere (Decision Driver 4 — zero cost on the hot path, checked FIRST
/// via a cheap tree walk before any allocation beyond the empty set
/// itself). `Err` only for a template segment that cannot resolve (e.g. a
/// `PathVariable` name absent from `ancestor_path_variable_values` AND not
/// equal to `path_variable_value`'s own leaf) — mirrors `FieldMissing`'s
/// own fail-closed shape, propagated up through `evaluate()`'s existing
/// `Result<bool, FieldMissing>` internal plumbing, never a panic.
pub fn discover_cross_document_paths(
    condition: &Condition,
    auth: Option<&AuthContext>,
    path_variable_value: Option<&str>,
    ancestor_path_variable_values: &BTreeMap<String, String>,
) -> Result<BTreeSet<String>, FieldMissing>
```

`pub`, not `pub(crate)` — `embyr-server`'s own new fetch step (below) is a DIFFERENT crate and must
call this directly, mirroring how `evaluate()`/`parse_condition` are already `pub` for the exact
same cross-crate reason.

## Decision — Fetch (`embyr-server`, NEW call site, EXISTING port method)

A new, small, shared helper (mirrors `resolve_access_rule_pattern`'s own "one shared implementation
for every call site" discipline — never duplicated per handler):

```rust
/// security-rules-cel-cross-document-reads (Slice 01, ADR-066 § Decision
/// — Fetch): for each distinct path `discover_cross_document_paths`
/// returned, splits it into `DocumentPath { collection_path, document_id
/// }` (the SAME last-segment-is-document-id split `rules_file.rs`'s own
/// ancestor/leaf discipline already uses) and calls the EXISTING
/// `BackendAdapter::get_document` unchanged — zero new port method, zero
/// new adapter capability (ADR-066 § Context). Sequential, not batched
/// (Decision Driver 5) — real Firestore's own 10/20-read ceiling and this
/// feature's own single-level-only structural bound (DISCUSS Resolution 2)
/// both confirm the practical count per evaluation is small.
async fn fetch_cross_document_reads(
    adapter: &SharedBackendAdapter,
    project_id: &ProjectId,
    paths: &BTreeSet<String>,
) -> Result<BTreeMap<String, Option<FirestoreDocument>>, Status>
```

Called ONCE per `evaluate()` call site that opts in (Slices 01/04 — `GetDocument`,
`CreateDocument`, `UpdateDocument`'s own exact-match branches; every other call site passes an
empty map, mechanically, mirroring `request_time`'s own `None`-at-unwired-sites rollout exactly).

## Decision — `evaluate()` Signature (EXTEND, 8th parameter)

```rust
pub fn evaluate(
    condition: &Condition,
    auth: Option<&AuthContext>,
    resource_fields: &BTreeMap<String, FieldValue>,
    request_resource_fields: &BTreeMap<String, FieldValue>,
    path_variable_value: Option<&str>,
    ancestor_path_variable_values: &BTreeMap<String, String>,
    request_time: Option<&FieldValue>,
    // NEW (security-rules-cel-cross-document-reads, Slice 01, ADR-066):
    // the pre-fetched cross-document read results, keyed by the SAME
    // concrete path strings discover_cross_document_paths() produced.
    // `&BTreeMap::new()` at every call site not yet wired (mirrors every
    // prior new-parameter rollout exactly) fails CrossDocumentExists/Get
    // closed via the existing FieldMissing short-circuit — a condition
    // with no cross-document operands is completely unaffected either way.
    cross_document_reads: &BTreeMap<String, Option<FirestoreDocument>>,
) -> EvaluationOutcome
```

`resolve_field_value`'s new arms: `CrossDocumentExists(template)` resolves the template (reusing
the SAME resolution logic `discover_cross_document_paths` already has — a shared private helper,
never duplicated) to a path, looks it up, `FieldValue::Boolean(map.get(path).is_some())` — never
`FieldMissing` (a resolved template always produces a definite true/false, mirroring real
Firestore's own `exists()` always returning a clean boolean). `CrossDocumentGet(template, field)`
resolves the template, looks up the path; `None` (or `Some(doc)` lacking `field`) is
`Err(FieldMissing)` — the IDENTICAL mechanism AC-17-09 already uses, zero new control-flow shape.

## Decision — Simulation (`admin/handlers/access_rules.rs`, EXTEND, Slice 05)

`SimulateAccessRuleBody` gains `cross_document_reads: BTreeMap<String, BTreeMap<String,
serde_json::Value>>` (concrete path → synthetic field map, translated via the EXISTING
`json_value_to_field_value` — reused, not duplicated, mirrors `resource`/`request_resource`'s own
identical translation). `simulate_access_rule` builds its own `BTreeMap<String,
Option<FirestoreDocument>>` DIRECTLY from this synthetic input — no call to
`discover_cross_document_paths`/`fetch_cross_document_reads` at all, no real I/O, mirrors
`request_time`'s own synthetic-input precedent (ADR-065) exactly, generalized from a single value
to a map.

## Decision — Path-Template Parser (`access_control/mod.rs`, EXTEND)

`word_to_operand`-adjacent: `exists(` / `get(` are recognized via the SAME call-syntax detection
`detect_unsupported_construct` already has (currently unconditionally rejecting them as
`CrossDocumentRead`) — this feature REMOVES that unconditional rejection for exactly the `exists`/
`get` idents, replacing it with real parsing. The path-template's own tokenizing reuses `duration.
value`'s own recent precedent (4c, ADR-065): a dedicated scan for `$(...)` substitutions inside the
already-tokenized path-argument text, recognizing exactly `$(database)` (structural, always
`project_id`, never stored), `$(request.auth.uid)`, `$(request.path.<var>)` — any other `$(...)`
content is a NAMED `UNSUPPORTED_EXPRESSION_GRAMMAR` rejection (Resolution 3), reusing 4c's own
shared variant.

## Consequences

**Positive**: zero new adapter/port capability (confirmed by direct code read before this ADR
locked); `embyr-core` stays genuinely zero-IO, confirmed by construction (path-discovery is pure,
the fetch step lives entirely in `embyr-server`); a condition with no cross-document operands
incurs zero extra cost (Decision Driver 4, checked structurally); deduplication is free
(`BTreeSet`, not incidental); simulation needs no real I/O (synthetic map, mirrors 3 prior
precedents).

**Negative / accepted trade-offs**: `evaluate()`'s parameter list grows to 8 (from 7) — named,
accepted, the SAME growth pattern every prior grammar-extension epic has produced. Sequential
(non-batched) fetches when a condition references 2+ distinct paths — accepted given the
single-level-only structural bound keeps this count small (Decision Driver 5); a future feature
revisiting chaining (DISCUSS's own deferred Resolution 2) would need to revisit this choice
too. `discover_cross_document_paths` and `resolve_field_value`'s own template-resolution logic
share a resolution helper (never duplicated) — but the SPLIT between "discover paths" (pure,
`embyr-core`) and "fetch" (I/O, `embyr-server`) means the path-template is resolved TWICE per
evaluation in principle (once to discover, once to actually look up during `evaluate()`) — accepted
as a known, minor, non-load-bearing inefficiency (pure string-building, not I/O), not optimized
away this feature (a future cache-the-resolved-template optimization is possible but unevidenced
as needed).

## Enforcement

- `deny.toml` (unchanged) continues to enforce `embyr-core`'s zero-IO boundary — `discover_cross_
  document_paths` and every new `Operand`/`PathTemplate` type are pure data + pure computation,
  confirmed by construction, not merely asserted.
- Mutation testing (per project CLAUDE.md, per-feature, AND per `security-rules-cel-expression-
  grammar`'s own QUALITY_GATE lesson): `embyr-core`-level unit tests for `discover_cross_document_
  paths` and the new `resolve_field_value` arms are written DURING DELIVER, not deferred to a
  post-hoc fix — every slice's own commit includes unit-test coverage for its own new pure logic,
  alongside the acceptance-test suite.

## References

- `docs/feature/security-rules-cel-cross-document-reads/feature-delta.md` (DISCUSS, all 5
  Resolutions, live web-verified real-Firestore semantics)
- `docs/product/architecture/adr-002-bounded-contexts.md` § BC-4 Access Control (the CURRENT,
  narrower BC-4→BC-2 dependency shape this ADR extends)
- `docs/product/architecture/adr-029-access-control-composition-and-bounded-context.md` (BC-4's
  own original placement decision, whose "read-only, non-transactional" dependency-shape
  characterization this ADR re-verifies and preserves, never violates)
- `docs/product/architecture/adr-062-rules-file-import-parser-path-variable-and-decomposition.md`
  (`path_variable_value` precedent, reused for path-template resolution)
- `docs/product/architecture/adr-065-expression-grammar-numeric-in-list-timestamp-duration.md`
  (`request_time`/`duration.value` precedent — the two most direct structural predecessors:
  "pre-resolve, thread as a parameter" and "a dedicated sub-grammar scan for a bracketed/
  parenthesized substitution shape")
