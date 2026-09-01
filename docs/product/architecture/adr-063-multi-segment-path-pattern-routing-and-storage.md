# ADR-063: Multi-Segment Path-Pattern Routing, Overlap Detection, and Storage

## Status

Accepted

## Context

`security-rules-cel-path-matching` (Epic 4b, JOB-17's 9th realization) lets
Alex import fixed-depth, multi-segment/nested-match-block `.rules` patterns
(e.g. `match /expeditions/{expeditionId}/journal_entries/{entryId} { allow
read, write: if request.auth.uid == resource.data.owner_id; }`) — the single
most common real Firestore hierarchical-ownership shape 4a's own locked v1
scope (`security-rules-cel-parity`, ADR-062) left untranslatable. DISCUSS
(`docs/feature/security-rules-cel-path-matching/feature-delta.md`, Resolution
1 Option C, Resolution 2 Option B, Resolution 3) locked the observable
behavior — deterministic single-pattern routing (reject on any structural
overlap, never OR-composition, never most-specific-wins), fixed-depth
patterns only (recursive wildcards deferred), read+write+Listen-per-event
parity — and explicitly handed DESIGN the routing/storage mechanism itself
(§ System Constraints, Handoff Package flag 4) as its own obligation, naming
two candidate directions for evaluation, not locking either.

Confirmed by direct code read (`crates/embyr-core/src/access_control/
rules_file.rs`, full, 669 lines; `mod.rs`, targeted; `crates/embyr-server/src/
adapters/system_db.rs`, targeted; `migrations/0022-0027`, full): the outer
scanner (`parse_path_segments`) is already fully general — arbitrary-length
segment sequences, `{name}` vs `{name=**}` already distinguished. Only
`decompose_block`'s own 2-shape allow-list (`[Literal(coll)]` /
`[Literal(coll), Wildcard(var)]`) narrows scope today. `access_rules`/
`write_access_rules` (ADR-028/030) are `PRIMARY KEY (project_id,
collection_path)` exact-match tables with **zero** "list all rules for a
project" method anywhere in this codebase, for any of the 3 existing rule
tables (`access_rules`, `write_access_rules`, `group_access_rules`) — direct
evidence a routing/matching mechanism has no reusable precedent.
`security-rules-collection-group-rules` (ADR-032) is confirmed, independently
re-verified (not merely re-cited from DISCUSS), to be a flat, disjoint table
keyed by a **bare** `collection_id` alone with zero path/wildcard/precedence
concept — an orthogonal problem (collection-GROUP querying across unknown
nesting depths) whose only reusable precedent is the "new disjoint table,
DB-level CHECK constraint over convention-only invariant" *shape*, not any
matching algorithm.

**The genuinely new architectural fact this ADR turns on** (confirmed by
direct inspection of `grpc::handler.rs::handle_get_document` and
`crates/embyr-core/src/domain/document.rs`): `DocumentPath { collection_path,
document_id }` already separates a document's **ancestor path** (e.g.
`"expeditions/trek-2026/journal_entries"` — always odd-length, always ending
on a literal collection name) from its **leaf document ID** (e.g.
`"entry-042"`). 4a's own `[Literal(coll)]` / `[Literal(coll), Wildcard(var)]`
shapes are the depth-1 special case of this same structure: `collection_path`
is *already* an ancestor-path concept, and the leaf wildcard (if any) is
*already* handled entirely through condition-rewrite + direct `document_id`
threading (ADR-062), never through any routing/matching mechanism at all.
**This means Epic 4b's genuinely new routing problem is scoped narrowly: only
INTERMEDIATE (ancestor) wildcard-or-literal document-ID captures are new.**
The leaf capture mechanism (ADR-062) is reused completely unchanged.

## Decision Drivers

1. **Deterministic single-pattern routing (DISCUSS Resolution 1 Option C,
   hard boundary)** — a concrete request path matches AT MOST ONE stored
   pattern, ever; structural overlap is rejected at import time, never
   resolved by precedence.
2. **Cheap on every read/write/query/Listen call** — the routing lookup must
   not become an unbounded per-request scan; the common case (no
   multi-segment pattern touches this collection at all) must not regress.
3. **One shared matching implementation for both import-time overlap
   detection and request-time routing** (DISCUSS Shared Artifact table,
   HIGH risk) — never two independently-maintained routines.
4. **Zero change to any already-shipped storage shape's own existing rows**
   (DISCUSS System Constraints, US-05 guardrail) — `access_rules`/
   `write_access_rules`/`group_access_rules` and their history tables are
   untouched.
5. **Re-verify, not re-cite, the `decompose_decidable` "zero new code" claim**
   under this feature's own multi-variable extension (Handoff Package flag 5
   / OQ-PM-02).
6. **Simplest solution first** — reuse `rules_file::PathSegment` and the
   existing hand-rolled-scanner discipline rather than a second parser or
   representation; no new workspace dependency.

## Considered Options — Storage/Routing Mechanism

### Option A: Extend `access_rules`/`write_access_rules` unchanged — store the pattern's `{var}`-bearing text directly as `collection_path`

Store `"expeditions/{expeditionId}/journal_entries"` as a literal
`collection_path` value (structurally distinguishable from a real Firestore
collection path, since `{`/`}` can never legally appear in one — confirmed no
CHECK constraint blocks this today).

**Rejected.** Two problems, both confirmed by direct reasoning about the
call-site shape, not assumed: (1) a request's concrete `collection_path`
(e.g. `"expeditions/trek-2026/journal_entries"`) can **never** be found via
the existing exact-match `WHERE collection_path = $2` query against a
template row — routing still requires a second, differently-shaped query
(narrow-by-skeleton, then in-memory wildcard/literal compatibility), so
co-locating gains nothing on the "cheap lookup" driver; the two shapes always
cost two different query *kinds* regardless of which table holds them. (2) it
overloads `collection_path`'s semantics — every existing consumer
(`get_access_rule_history`, admin listing, the direct hand-authoring JSON API
that today accepts *any* string with zero validation) would need `{`/`}`
-sniffing logic to know which "kind" of row it has, exactly the
convention-only-invariant risk `security-rules-collection-group-rules`'s own
ADR-032 explicitly moved away from ("first time this initiative enforces its
own... invariant at the DB layer rather than by convention alone").

### Option B: A new disjoint table, structured for indexed routing — Accepted

A new table, `access_rule_patterns`, storing each pattern's **ancestor**
shape (the genuinely new part) in a form the database can index directly:
segment count and a literal-collection-name skeleton, alongside the full
canonical pattern text (for the primary key / idempotent re-import) and the
leaf variable name (if any, for continuity with ADR-062's existing leaf
mechanism).

**Accepted.** Directly mirrors `group_access_rules`' own precedent (ADR-032:
new disjoint table, DB-level invariant, `check_query_compliance()` reused
unmodified where applicable) while going one step further: because the
ancestor shape is stored in a way the database can narrow on
(`segment_count`, `literal_skeleton`), the routing query is a genuine indexed
lookup — typically 0 or 1 candidate row — not a per-project scan. Keeps
`access_rules`/`write_access_rules` semantically pure (every row an exact,
concrete collection path) and every existing consumer of those two tables
untouched (Decision Driver 4).

## Decision — The Ancestor/Leaf Split (Decision Driver 2's real basis)

A stored pattern's segments (`Vec<PathSegment>`, reused unmodified from
`rules_file.rs`) split as:

- **Ancestor** = `segments[..len-1]` if `len` is even (a leaf position is
  present), else `segments[..len]` (no leaf position — the pattern governs
  every document in that literal-or-wildcard-scoped collection uniformly,
  the direct generalization of 4a's own `[Literal(coll)]` shape). Ancestor
  length is **always odd** by this construction, mirroring `DocumentPath.
  collection_path`'s own existing shape exactly.
- **Leaf** = `segments[len-1]` if `len` is even — `Wildcard(name)`,
  `Literal(value)`, or, per this feature's own general "arbitrary-length,
  alternating literal-collection/wildcard-or-literal-document-ID" scope, is
  never a third shape.

**Routing (US-02/03) touches ONLY the ancestor.** The leaf — if a wildcard —
continues to be resolved via ADR-062's own existing, completely unmodified
mechanism (condition-rewrite at decompose time, `document_id` threaded
directly into `evaluate()`'s existing `path_variable_value: Option<&str>`
parameter). **Zero new mechanism for the leaf.** This is the single fact that
keeps this feature's genuinely new surface small: only ancestor wildcards
(e.g. `expeditionId`) are new; entry-level wildcards (e.g. `entryId`) are 4a's
mechanism, reused verbatim.

## Decision — Schema

```sql
-- migrations/0032_access_rule_patterns.sql
CREATE TABLE access_rule_patterns (
    project_id              TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    collection_path_pattern TEXT NOT NULL,   -- ancestor only, e.g. "expeditions/{expeditionId}/journal_entries"
    ancestor_segment_count  SMALLINT NOT NULL,   -- always odd; segments in collection_path_pattern
    literal_skeleton        TEXT NOT NULL,       -- even-position literal collection names only, e.g. "expeditions/journal_entries"
    leaf_variable           TEXT,                -- Some("entryId") if this pattern captures a leaf wildcard; NULL = applies uniformly to every document in the ancestor-scoped collection (4a's [Literal(coll)] generalization)
    read_condition          TEXT,
    write_condition         TEXT,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, collection_path_pattern)
);

-- Narrows routing/overlap candidates to typically 0-1 rows before any
-- in-memory wildcard/literal compatibility check runs (Decision Driver 2).
CREATE INDEX idx_access_rule_patterns_routing
    ON access_rule_patterns (project_id, ancestor_segment_count, literal_skeleton);
```

```sql
-- migrations/0033_access_rule_pattern_history.sql (ADR-035 precedent, extended)
CREATE TABLE access_rule_pattern_history (
    id                       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    project_id               TEXT NOT NULL REFERENCES projects(id),
    collection_path_pattern  TEXT NOT NULL,
    leaf_variable            TEXT,
    read_condition           TEXT,
    write_condition          TEXT,
    actor_account_id         UUID NOT NULL,
    captured_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_access_rule_pattern_history_lookup
    ON access_rule_pattern_history (project_id, collection_path_pattern, id DESC);
```

**One row per pattern SHAPE, not per operation — a departure from ADR-028/
030's disjoint-table precedent, evidenced not mirrored.** `access_rules`/
`write_access_rules` are separate tables because they have separate,
independent *authoring* paths (`define_access_rule` vs
`define_write_access_rule` — two distinct admin actions, AC-17-43's
independence guarantee exists because either can be defined without the
other). Multi-segment patterns have exactly **one** authoring path (file
import, producing one `MatchBlock` per pattern with both verb buckets already
resolved in memory as `DecomposedRule`/`DecomposedPatternRule`'s own shape) —
splitting read/write into two tables here would (a) duplicate the ancestor
shape/routing-index columns in two places, (b) require overlap detection to
run twice (once per table) for what is structurally one routing decision, and
(c) buys no independence guarantee this feature's own scope needs (nothing
hand-authors a write-only or read-only multi-segment pattern independently of
the other today). `read_condition IS NULL` on a matched pattern row means
exactly what an absent row means in `write_access_rules` today —
**unrestricted for that operation**, not fail-closed-deny — this is a locked
composition rule (§ Decision — Composition, below), not an implicit
inference.

`PRIMARY KEY (project_id, collection_path_pattern)` makes re-import of a
byte-identical pattern idempotent (mirrors AC-17-176/195's own precedent) —
the wildcard's own captured NAME is part of `collection_path_pattern`'s text,
so two imports using *different* names at the same structural position
(`{expeditionId}` vs `{expId}`) do **not** collide on this key — that
collision is caught by the separate, explicit overlap-detection pass below
(§ Decision — Overlap Detection), never relied upon as a side effect of PK
uniqueness.

## Decision — Shared Matching Primitives (Decision Driver 3)

New pure, zero-IO module, `crates/embyr-core/src/access_control/
path_routing.rs`, operating entirely on `rules_file::PathSegment` (reused,
not reinvented — a concrete request's own ancestor segments are represented
as `Vec<PathSegment>` too, always `Literal`, via the SAME
`rules_file::parse_path_segments` splitter already proven on stored pattern
text):

```rust
use super::rules_file::PathSegment;
use std::collections::BTreeMap;

/// Even-position (collection-name) segments only, joined — the routing-index
/// narrowing key. Identical computation whether `segments` came from a
/// stored pattern or a concrete request's own ancestor path.
pub fn literal_skeleton(ancestor_segments: &[PathSegment]) -> String;

/// Routing primitive (US-02/03, request-time; also US-04's per-candidate
/// check, see below). `concrete_ancestor` is always `Literal`-only —
/// requests never carry wildcards. `Some(bindings)` iff every position is
/// compatible (`Literal == Literal` exactly, `Wildcard` always matches and
/// captures its own name). `None` = no structural match.
pub fn bind_ancestor(
    pattern_ancestor: &[PathSegment],
    concrete_ancestor: &[PathSegment],
) -> Option<BTreeMap<String, String>>;

/// Overlap-detection primitive (US-04, import-time). Two ancestor shapes
/// structurally overlap iff SOME concrete instantiation could satisfy both:
/// same length, literal positions equal (guaranteed by the caller having
/// already narrowed on `literal_skeleton`, re-checked here defensively), and
/// every document-ID position pairwise compatible (both wildcard, one
/// wildcard, or equal literals). Built on the SAME per-position compatibility
/// test `bind_ancestor` uses internally — one primitive, two call shapes,
/// never two independently-maintained matching routines.
pub fn structurally_overlap(a: &[PathSegment], b: &[PathSegment]) -> bool;
```

**Why this satisfies Decision Driver 3 structurally, not just by
convention**: `bind_ancestor` and `structurally_overlap` are the only two
functions in the codebase that inspect wildcard/literal compatibility at a
path position. Both are implemented in terms of one private per-position
predicate (`fn positions_compatible(a: &PathSegment, b: &PathSegment) ->
bool`) — a request-time "does this concrete value satisfy this segment" call
and an import-time "could these two segments both be satisfied by some
value" call are the same question asked two ways, not two questions.

## Decision — Routing Composition (US-02/03, all 6 locked call sites)

Every one of the 6 call sites DISCUSS's Resolution 3 locks (`GetDocument`,
`CreateDocument`, `UpdateDocument`, `DeleteDocument`, and `handle_add_target`'s
`Changed`/`Removed` arms) gains the **identical** two-step composition, via
one new shared private helper on the handler
(`resolve_access_rule_pattern(&self, project_id, collection_path) ->
Result<Option<RoutedPattern>, CoreError>`, `crates/embyr-server/src/grpc/
handler.rs`, EXTEND) — never duplicated per call site (mirrors ADR-029's own
"two call sites, one function" discipline, now at the composition layer, not
only the pure-function layer):

1. **Existing exact-match lookup, byte-for-byte unchanged**:
   `get_access_rule`/`get_write_access_rule(project_id, collection_path)`.
   `Some` → **UNCHANGED behavior** (US-05 guardrail) — evaluate using the
   existing `path_variable_value: Option<&str>` slot for any leaf capture,
   and an EMPTY `ancestor_path_variable_values` map (mechanical, new 6th
   `evaluate()` parameter, see § Decision — `evaluate()` Signature).
2. **Only if step 1 returns `None`**: query
   `list_access_rule_patterns_by_skeleton(project_id, ancestor_segment_count,
   literal_skeleton)` — computed from the request's own `collection_path`,
   zero new I/O beyond this one indexed lookup (mirrors ADR-062's own
   zero-new-I/O framing, now for a new but equally cheap query). For each
   candidate row (typically 0 or 1, occasionally a handful of mutually
   -exclusive literal-value variants — see § Complexity, below), re-parse
   `collection_path_pattern` via `rules_file::parse_path_segments` (reused)
   and call `path_routing::bind_ancestor`. Resolution 1's own import-time
   guarantee means AT MOST ONE candidate returns `Some` — the composition
   takes that one match; if more than one match is ever found (a bug or a
   concurrent-import race), fail closed, deny, and emit a structured
   `security_rules.routing_invariant_violated` log event rather than
   guessing — this should never be reachable given import-time enforcement,
   and is a defensive assertion, not a new control-flow branch client
   behavior depends on.
3. If a pattern matches: bind the leaf (if `leaf_variable.is_some()`, thread
   `document_id` via the EXISTING `path_variable_value` slot, unchanged from
   ADR-062) and thread the ancestor bindings from step 2 via the NEW
   `ancestor_path_variable_values` map. Evaluate `read_condition`/
   `write_condition` per operation, exactly as an exact-match row would —
   **`None` on the relevant condition column means unrestricted for that
   operation** (the locked composition rule from § Decision — Schema),
   never a fail-closed deny.
4. If no pattern matches either: fall through to whatever pre-existing
   default already governs (unrestricted, if truly no rule of any kind
   exists) — unchanged (US-05).

### Complexity (Decision Driver 2, stated explicitly — this runs on every read/write/query/Listen call)

- **Common case (no multi-segment pattern touches this request at all)**:
  cost = the existing exact-match lookup (unchanged) **plus one additional
  indexed lookup** on the miss path (`idx_access_rule_patterns_routing`).
  This is a real, named regression relative to today's single-query cost for
  a collection with literally no rule of any kind (e.g. `app_config`) — a
  second index lookup, not a scan, not unbounded. Named explicitly as a
  Performance-vs-Simplicity trade-off (§ Consequences), not hidden. A
  per-project "has any patterns at all" cache to skip step 2 entirely is a
  named, deferred optimization (mirrors OQ-SR-05's own precedent: build only
  if profiling warrants, not speculatively — Principle 8).
- **Pattern-governed case**: one indexed lookup narrows to a small candidate
  set (bounded by how many mutually non-overlapping literal-value patterns a
  human plausibly authors at the same shape+depth — realistically dozens at
  most, never unbounded, because Resolution 1 forbids a wildcard pattern from
  coexisting with ANY other pattern at its own shape+depth at all — a wildcard
  pattern, once stored, is *always* alone in its `(ancestor_segment_count,
  literal_skeleton)` bucket). In-memory compatibility check is
  O(depth × candidates), both small constants in realistic usage.
- **Overall**: O(1) indexed Postgres round-trips (1 or 2) plus O(depth ×
  small-K) pure CPU — no per-project or per-collection unbounded scan
  anywhere in this design.

## Decision — Overlap Detection (US-04)

The admin import handler (`import_rules_file`, EXTEND) runs, for every
multi-segment pattern the current file's `decompose()` pass produces:

1. **Intra-file**: pairwise `structurally_overlap` against every other
   pattern in the SAME import sharing `(ancestor_segment_count,
   literal_skeleton)` — an in-memory nested loop bounded by file size.
2. **Cross-import**: `list_access_rule_patterns_by_skeleton` against ALREADY
   -STORED patterns sharing the same key, `structurally_overlap` against
   each.

Both run **before any storage write**, mirroring ADR-062's own
validate-then-apply atomicity discipline exactly (no new transaction
machinery — Principle 8, same pragmatic call 4a made for the identical
class of risk). Any overlap → reject the WHOLE import, `OffendingBlock`
naming both colliding pattern texts, reason `PATTERN_OVERLAP` (new taxonomy
entry, distinguishable from `RECURSIVE_WILDCARD`/`NESTED_PATH`/etc., per
System Constraints).

**Scope boundary, named explicitly**: overlap detection checks new patterns
against `access_rule_patterns` ONLY, never against `access_rules`/
`write_access_rules` rows. A `collection_path` value on those tables
containing `/`-separated segments is a pre-existing, unvalidated loophole
(the direct hand-authoring JSON API has zero CHECK constraint preventing it,
confirmed by DISCUSS's own direct read) that predates this feature and is not
its concern to close — flagged as **OQ-PM-06** (§ Open Questions), not
silently ignored.

## Decision — `evaluate()` Signature (extends ADR-027/030/062)

```rust
pub fn evaluate(
    condition: &Condition,
    auth: Option<&AuthContext>,
    resource_fields: &BTreeMap<String, FieldValue>,
    request_resource_fields: &BTreeMap<String, FieldValue>,
    path_variable_value: Option<&str>,               // UNCHANGED (ADR-062) — the leaf capture, never touched by this ADR
    ancestor_path_variable_values: &BTreeMap<String, String>,   // NEW (6th parameter) — ancestor captures only
) -> EvaluationOutcome
```

`resolve_field_value`'s `PathVariable(name)` arm becomes:

```rust
Operand::PathVariable(name) => ancestor_path_variable_values
    .get(name)
    .cloned()
    .map(FieldValue::String)
    .or_else(|| path_variable_value.map(|v| FieldValue::String(v.to_string())))
    .ok_or(FieldMissing),
```

**Why two parameters, not one map (an evaluated departure from the
"map is the natural extension point" framing ADR-062 itself floated)**: an
exact-name-keyed map lookup for the leaf capture would require every
EXISTING `access_rules`/`write_access_rules` row's own re-parsed condition to
resolve correctly WITHOUT knowing what name Alex originally used for the
wildcard (that name is never stored separately from the condition text
itself, by ADR-062's own design — re-deriving it would mean re-parsing the
condition tree just to find its own `PathVariable` name, extra work for zero
new capability). Keeping the leaf slot as the SAME `Option<&str>`
ADR-062 already shipped means **zero behavior change and zero new reasoning**
for any of 4a's own already-delivered rows — they gain one new,
mechanically-inserted empty-map argument and nothing else. The NEW ancestor
map is exact-name-keyed because ancestor bindings are ALWAYS produced by
THIS feature's own routing step, which always knows the real captured names
(recovered directly from the stored `collection_path_pattern` text via
`rules_file::parse_path_segments`) — no ambiguity to paper over.

All 6 locked call sites, plus every one of 4a's own 4 already-wired
real-enforcement call sites (which now also pass the new 6th argument, always
`&BTreeMap::new()` on the exact-match branch) — a mechanical, one-line edit,
identical in kind to ADR-062's own "Rust has no default arguments" mechanical
edit at Listen's 2 per-event arms.

**Listen's per-event re-check (`OQ-CP-04`, closed by this feature per
Resolution 3)**: `handle_add_target`'s `Changed`/`Removed` arms now run the
SAME `resolve_access_rule_pattern` two-step composition (§ Decision — Routing
Composition) using the changed/removed document's own already-known path —
zero new I/O beyond the routing lookup itself, matching every other call
site's own zero-new-I/O argument. This closes the gap ADR-062 deliberately
left open, exactly as DISCUSS's Resolution 3 locks.

## Decision — Structural Re-Verification (Decision Driver 5, OQ-PM-02)

**Confirmed by direct inspection, not re-cited**: `decompose_decidable`
(`crates/embyr-core/src/access_control/mod.rs`) matches on `Operand`
*variant* shape only (`AuthNullSentinel`, `AuthUid`, `ResourceField`) — it
never inspects a `PathVariable`'s own captured `String` name. This means the
wildcard catch-all (`_ => Err(Undecidable)`) rejects **any** `Compare`
naming `Operand::PathVariable`, regardless of how many *distinct* names a
single condition references (e.g. `request.path.expeditionId ==
request.path.entryId`, grammar-legal, nonsensical) — the multi-variable case
this feature introduces changes nothing about which shapes
`decompose_decidable` recognizes, because names were never part of its own
match arms to begin with. **Zero code change required, confirmed at the
variant level, not the value level** — this holds for both `RunQuery`'s
non-group arm and Listen's subscribe-time gate (both call
`check_query_compliance`, unchanged) wherever they encounter an
`access_rules`/`write_access_rules` row (never `access_rule_patterns` — see
§ Out-of-Scope Boundary below) referencing `PathVariable`.

## Decision — Out-of-Scope Boundary: `RunQuery` / Listen Subscribe-Time Compliance Against Patterns

DISCUSS's own locked call-site list (§ System Constraints, Handoff Package
flag 3) names exactly 6 call sites — `GetDocument`, 3 write handlers, and
Listen's 2 per-event arms. `handle_run_query`'s non-group arm and Listen's
own subscribe-time gate (`handle_add_target`'s initial-snapshot compliance
check) are **not** in that list, and DISCUSS's own Out of Scope section
bounds "`RunQuery`/Listen subscribe-time compliance under multiple
simultaneously-captured variables" to the *structural* re-verification above
— not new wiring. Consequence, stated explicitly: **a `RunQuery` or a
Listen subscription's initial snapshot against a collection governed
EXCLUSIVELY by a multi-segment pattern (no `access_rules` exact-match row)
is NOT gated by that pattern in this feature** — `handle_run_query`'s
existing `get_access_rule(project_id, collection.collection_path)` exact
lookup returns `None` for any nested `collection_path`, and (per the
pre-existing "no rule ⇒ unrestricted" default, unchanged) the query proceeds
unrestricted. This is a real, named behavioral asymmetry between
`GetDocument`/writes/Listen-per-event (pattern-aware) and
`RunQuery`/Listen-subscribe (pattern-blind) — flagged as **OQ-PM-07** (§ Open
Questions), not silently built nor silently ignored. No domain example or
acceptance scenario in this feature's own DISCUSS exercises `RunQuery`
against a multi-segment-pattern-governed collection, so this is consistent
with "ship the evidenced slice" (Principle 8), not an oversight.

## Decision — Nested Match-Block Flattening (US-01, parser-only, `rules_file.rs` EXTEND)

`parse_match_blocks` becomes recursive: a match block's own body is either
**exclusively** `allow` clauses (a leaf block, parsed as today) **or**
**exclusively** further nested `match /<pattern> { ... }` blocks (parsed by
recursing, prepending the parent's own already-parsed `PathSegment`s onto
each nested block's own segments before continuing) — never a mix. A block
whose body contains both an `allow` clause and a nested `match` is rejected,
`SYNTAX_ERROR` (no domain example anywhere in this feature's own text
requires mixing; this is a DESIGN-owned scoping decision, flagged as
**OQ-PM-08** for DISTILL to add explicit acceptance coverage — mirrors
ADR-062's own `CONFLICTING_VERB_CONDITIONS` precedent for the identical class
of decision). This satisfies Resolution 2's own locked claim (§ System
Constraints: "a pure parser-flattening concern... introduces no new routing/
precedence concept") exactly: the flattened segment sequence a nested block
produces is byte-for-byte identical to what the equivalent flat multi-segment
syntax produces, feeding the SAME widened `decompose_block` shape-check
described below.

## Decision — Widened `decompose_block` Shape-Check (US-01)

Replaces the existing 2-entry allow-list with a general rule: `segments` is
valid iff every even index holds `Literal` and every odd index holds
`Wildcard` or `Literal` (never `RecursiveWildcard`, rejected as
`RECURSIVE_WILDCARD` unchanged), for any length ≥ 1. Ancestor/leaf split
per § Decision — The Ancestor/Leaf Split. `decompose()`'s return type gains
one new variant, additively:

```rust
pub enum DecomposedTarget {
    SingleCollection(DecomposedRule),          // UNCHANGED type (ADR-027/062) — ancestor_segments.len() == 1
    MultiSegmentPattern(DecomposedPatternRule), // NEW — ancestor_segments.len() > 1
}

pub struct DecomposedPatternRule {
    pub collection_path_pattern: String,
    pub ancestor_segment_count: u16,
    pub literal_skeleton: String,
    pub leaf_variable: Option<String>,
    pub read_condition: Option<String>,
    pub write_condition: Option<String>,
}

pub fn decompose(blocks: Vec<MatchBlock>) -> Result<Vec<DecomposedTarget>, RulesFileError>;
```

`DecomposedRule` (existing, tested type from 4a) is **unchanged** — every
existing test and every existing call site pattern-matching on it keeps
working; `decompose()`'s signature is the only touch point, additive
(`Vec<DecomposedRule>` → `Vec<DecomposedTarget>`), and `import_rules_file`
(EXTEND) branches on the variant to call either the existing
`upsert_access_rule`/`upsert_write_access_rule` (unchanged) or the new
`upsert_access_rule_pattern` (§ Decision — Adapter, below), after the
idempotency-check-before-upsert pattern the handler already applies for the
single-collection case (AC-17-195 precedent, extended identically for
patterns — a re-import comparing the SAME `collection_path_pattern`'s
existing `read_condition`/`write_condition` before writing).

The condition-rewrite step (ADR-062's `rewrite_path_variable`) generalizes
from "rewrite the block's one wildcard name" to "rewrite every distinct
wildcard name across ALL of the block's own segments (ancestor and leaf)" —
a loop over `Wildcard` segments instead of a single optional one; the
per-name whole-word substitution logic itself is unchanged.

## Decision — Adapter (`crates/embyr-server/src/adapters/system_db.rs`, EXTEND)

```rust
pub struct AccessRulePatternRow {
    pub collection_path_pattern: String,
    pub leaf_variable: Option<String>,
    pub read_condition: Option<String>,
    pub write_condition: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn upsert_access_rule_pattern(
    &self, project_id: &str, collection_path_pattern: &str,
    ancestor_segment_count: i16, literal_skeleton: &str,
    leaf_variable: Option<&str>, read_condition: Option<&str>,
    write_condition: Option<&str>, actor_account_id: uuid::Uuid,
) -> Result<(), CoreError>;

/// Exact PK lookup — idempotency check before `upsert_access_rule_pattern`
/// (mirrors `get_access_rule`'s own role in `import_rules_file` today).
pub async fn get_access_rule_pattern(
    &self, project_id: &str, collection_path_pattern: &str,
) -> Result<Option<AccessRulePatternRow>, CoreError>;

/// Routing (US-02/03) AND overlap-detection (US-04) candidate narrowing —
/// the ONE new query shape this feature introduces, deliberately narrowed
/// (never "list everything for project") per the NFR obligation DISCUSS
/// flagged (§ System Constraints).
pub async fn list_access_rule_patterns_by_skeleton(
    &self, project_id: &str, ancestor_segment_count: i16, literal_skeleton: &str,
) -> Result<Vec<AccessRulePatternRow>, CoreError>;

pub async fn get_access_rule_pattern_history(
    &self, project_id: &str, collection_path_pattern: &str,
) -> Result<Vec<AccessRulePatternHistoryRow>, CoreError>;
```

`upsert_access_rule_pattern` fuses history capture into the same transaction,
mirroring `upsert_access_rule`/`upsert_write_access_rule`'s own exact shape
(ADR-035 precedent, extended to a 3rd/4th sibling table — every prior rule
table in this initiative has a history sibling; omitting one here would be an
unjustified, inconsistent gap for `security-rules-operations` tooling).

## Decision — Admin Surface Extensions

**`import_rules_file` (existing route, `POST /admin/v1/projects/:project_id/
access_rules/import`, EXTEND)**: `ImportedBlockSummary`'s existing
`collection_path: String` field is reused unchanged to carry a pattern's own
`collection_path_pattern` text for `MultiSegmentPattern` targets (the field's
existing semantic — "what collection(s) this targets" — generalizes to a
template string without a type change; structurally distinguishable by
containing `{`). `OffendingBlock.construct` gains one new value,
`"PATTERN_OVERLAP"`, `detail` naming both colliding pattern texts
(AC-17-218/223).

**New route, `POST /admin/v1/projects/:project_id/access_rules/
simulate_route` (US-06, Release 2)** — an evaluated departure from
ADR-062's own precedent of extending `SimulateAccessRuleBody` additively,
in the SAME direction ADR-032/033 already established for a genuinely
different response contract: the existing `simulate_access_rule`'s
`{outcome: Allow|Deny}` response cannot express "no matching pattern" (a
THIRD state, AC-17-230, distinguishable from a routed-but-denied outcome) or
the resolved binding map (AC-17-228) real routing produces. New handler,
reusing `resolve_access_rule_pattern`'s own step 2 (skeleton lookup +
`bind_ancestor`) directly — never a second, independently-maintained routing
implementation for simulation:

```rust
pub struct SimulateRoutedAccessRuleBody {
    pub candidate_pattern: String,        // e.g. "expeditions/{expeditionId}/journal_entries/{entryId}"
    pub candidate_read_condition: Option<String>,
    pub candidate_write_condition: Option<String>,
    pub synthetic_concrete_path: String,  // e.g. "expeditions/test-expedition/journal_entries/test-entry"
    pub auth: Option<SimulatedAuth>,
    #[serde(default)]
    pub resource: BTreeMap<String, serde_json::Value>,
}

pub enum RoutedSimulationOutcome { Allow, Deny, NoMatchingPattern }

pub struct SimulateRoutedAccessRuleResponse {
    pub outcome: RoutedSimulationOutcome,
    pub resolved_bindings: BTreeMap<String, String>,
}
```

Any role, read-only, zero writes (mirrors `simulate_access_rule`'s own gate).

## Consequences

### Positive

- Routing touches only the genuinely new part (ancestor wildcards); the leaf
  capture mechanism (ADR-062) is reused byte-for-byte unchanged.
- One shared pure-function primitive (`positions_compatible`, via
  `bind_ancestor`/`structurally_overlap`) serves both import-time overlap
  detection and request-time routing — structurally, not by convention.
- `access_rules`/`write_access_rules`/`group_access_rules` and their history
  tables receive zero schema or row changes.
- The routing query is a genuine indexed lookup (typically 0-1 candidate
  rows), not a per-project scan, by construction of the schema's own
  narrowing columns.
- `decompose_decidable`'s "zero new code" claim is re-verified at the variant
  level, closing OQ-PM-02 with high confidence, not re-cited.
- Zero new workspace dependency.

### Negative / Trade-offs

- A collection with literally no rule of any kind now costs 2 indexed
  lookups instead of 1 on the `GetDocument`/write/Listen-per-event path (the
  exact-match miss triggers the new pattern-skeleton lookup). Named
  explicitly, not hidden; a per-project "has any patterns" cache is a
  deferred, evidence-gated optimization (mirrors OQ-SR-05's precedent).
- `RunQuery` and Listen's subscribe-time (initial snapshot) compliance gate
  remain pattern-blind for this feature (OQ-PM-07) — a real, named behavioral
  asymmetry against `GetDocument`/writes/Listen-per-event, consistent with
  DISCUSS's own locked call-site list, not an oversight.
- `access_rule_patterns` combines read+write into one row per pattern shape,
  a departure from the `access_rules`/`write_access_rules` disjoint-table
  precedent — evidenced (single authoring path), not free of risk (a future
  feature adding an independent multi-segment-pattern authoring path would
  need to revisit this).
- Overlap detection does not extend to `access_rules`/`write_access_rules`'
  own pre-existing, unvalidated `/`-containing `collection_path` loophole
  (OQ-PM-06) — a pre-existing gap, not this feature's to close, but
  co-existing with it un-remediated.
- Nested-match-block bodies may not mix `allow` clauses with further nested
  `match` blocks (OQ-PM-08) — a DESIGN-owned scoping decision, not
  DISCUSS-locked, flagged for DISTILL confirmation.

## Enforcement

Style: Hexagonal (ports-and-adapters), unchanged. BC-4 Access Control
(ADR-029) gains one new pure submodule (`path_routing`), one new
`rules_file` type (`DecomposedPatternRule`/`DecomposedTarget`), and
`evaluate()`'s 6th parameter; no new bounded context.

**No new driven port, no new Earned Trust probe (Principle 12 discipline,
mirroring ADR-027/029/030/031/033/062 § Enforcement verbatim)**:
`path_routing::bind_ancestor`/`structurally_overlap` and the widened
`rules_file::decompose` are pure, deterministic CPU computation over
in-memory values — the identical "no environment can lie to a pure function"
reasoning applies unmodified. The new adapter methods execute through the
existing, already-probed `SystemDb` connection pool — the identical substrate
every other BC-4 write/read already uses. The substrate this feature adds new
reliance on is exactly zero.

`cargo-deny`/`deny.toml` unaffected — no new workspace dependency.
`embyr_core::access_control`'s existing zero-IO enforcement covers both new
submodules unchanged.

## References

- `docs/feature/security-rules-cel-path-matching/feature-delta.md` §§ Job
  Discovery Framing Resolution (Resolutions 1-3), System Constraints, Handoff
  Package (flags 1-7), User Stories (US-01 through US-06).
- `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md`,
  `adr-028-access-rule-storage-and-lifecycle.md`,
  `adr-029-access-control-composition-and-bounded-context.md`,
  `adr-030-write-path-grammar-storage-and-composition.md`,
  `adr-031-query-shape-compliance-check.md`,
  `adr-032-collection-group-rule-storage-and-composition.md`,
  `adr-033-listen-compliance-composition-and-collection-scoping.md`,
  `adr-035-access-rule-history-storage-and-capture-mechanism.md`,
  `adr-062-rules-file-import-parser-path-variable-and-decomposition.md` — the
  machinery this ADR extends, never replaces.
- `crates/embyr-core/src/access_control/rules_file.rs` (full, read during
  DESIGN) — exact current `PathSegment`/`MatchBlock`/`decompose_block`
  shapes this ADR widens.
- `crates/embyr-core/src/access_control/mod.rs` (targeted, read during
  DESIGN) — exact current `Operand`/`evaluate`/`resolve_field_value`/
  `decompose_decidable` shapes confirmed before designing this extension.
- `crates/embyr-server/src/adapters/system_db.rs` (targeted, read during
  DESIGN) — exact current `get_access_rule`/`upsert_access_rule` shapes;
  confirmed no "list all rules for a project" method exists for any of the 3
  existing rule tables.
- `crates/embyr-server/src/admin/handlers/access_rules.rs` (targeted, read
  during DESIGN, `import_rules_file` full) — exact current
  idempotency-check-before-upsert shape this ADR's pattern path mirrors.
- `crates/embyr-server/src/grpc/handler.rs` (targeted, read during DESIGN)
  — exact current `handle_get_document`/write-handler/`handle_add_target`
  call shapes this ADR's `resolve_access_rule_pattern` helper composes with.
- `crates/embyr-core/src/domain/document.rs` — `DocumentPath{collection_path,
  document_id}`, the structural fact (§ Context) this whole ADR's
  ancestor/leaf split is built on.
- `migrations/0022_access_rules.sql`, `0023_write_access_rules.sql`,
  `0024_group_access_rules.sql`, `0025-0027_*_history.sql` — schema-style
  precedent this ADR's own migrations follow.
