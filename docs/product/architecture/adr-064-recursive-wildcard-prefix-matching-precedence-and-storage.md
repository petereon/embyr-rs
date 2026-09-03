# ADR-064: Recursive-Wildcard Prefix Matching, Precedence, and Storage

## Status

Accepted

## Context

`security-rules-cel-recursive-wildcards` (JOB-17, 10th realization, "Epic
4b-ii") lets Alex import even-prefix, terminal-position recursive-wildcard
`.rules` blocks (e.g. `match /{document=**} { allow read, write: if false; }`
and `match /expeditions/{expeditionId}/{path=**} { allow read: if
request.auth != null; }`) — real Firestore's own idiomatic
specific-override-plus-catch-all pattern, which 4a and 4b both rejected
outright (`RECURSIVE_WILDCARD`). DISCUSS
(`docs/feature/security-rules-cel-recursive-wildcards/feature-delta.md`,
Resolutions 1-4) locked: even-prefix/terminal-only scope, most-specific-wins
precedence scoped by structural containment (never real-Firestore
OR-composition, never `security-rules-cel-path-matching`'s own unmodified
reject-on-any-overlap), read+write+Listen-per-event parity, and — the
central architectural finding this ADR resolves — confirmed by direct code
read that ADR-063's own `bind_ancestor`/`structurally_overlap` have a hard
equal-length precondition (`path_routing.rs` lines 90-92) and
`access_rule_patterns`' own routing index is keyed on an EXACT
`ancestor_segment_count`, neither of which can represent a variable-depth
recursive wildcard. DISCUSS's own verdict: **augmentation, not replacement**
— reuse `PathSegment`/`positions_compatible` unchanged, build one new
primitive family (`bind_recursive_prefix`) operating on a concrete
document's own FULL path (not the ancestor alone — the zero-remaining-
segments case, locked "must succeed" per the post-DISCUSS
`OQ-RW-01` correction verifying real Firestore's `rules_version = '2'`
semantics, can land exactly at the prefix's own boundary document, which
`DocumentPath.collection_path` alone cannot represent), and design a new
storage/indexing shape (DISCUSS's own explicit, unlocked obligation for
DESIGN — two candidate directions named, § System Constraints).

Confirmed by direct read (`rules_file.rs` full; `path_routing.rs` full;
`grpc/handler.rs` targeted, `resolve_access_rule_pattern`/
`handle_get_document`; `access_control/mod.rs` targeted, `evaluate`/
`resolve_field_value`/`decompose_decidable`; `adapters/system_db.rs`
targeted, the `access_rule_patterns` CRUD; `admin/handlers/
access_rules.rs` targeted, `check_pattern_overlap`/`import_rules_file`/
`simulate_routed_access_rule`; `migrations/0032`/`0033`): 4a's own leaf-
capture mechanism, `structurally_overlap`, and `evaluate()`'s existing 6-
parameter shape are ALL directly reusable, unmodified, for this feature —
confirmed structurally below, not assumed.

## Decision Drivers

1. **Reuse the SAME `positions_compatible` predicate — never a third,
   independently-invented compatibility test** (DISCUSS's own explicit,
   locked instruction).
2. **Cheap on every read/write/query/Listen call, extended**: this feature
   adds a THIRD composition step behind ADR-063's own two; a project with
   zero recursive-wildcard patterns anywhere must not regress the common
   case beyond one additional indexed lookup on the already-degraded
   double-miss path.
3. **One shared implementation for import-time overlap/precedence
   validation and request-time precedence resolution** (DISCUSS Shared
   Artifact table, rated CRITICAL — higher consequence than ADR-063's own
   equivalent risk: a wrong WINNER, not just a wrong binding).
4. **Zero change to `evaluate()`'s signature, `AccessRulePatternRow`'s own
   shape, or any of the 6 already-wired call sites' own control flow** —
   re-verify, not assume, given Resolution 3 locks "no condition may
   reference the captured remainder."
5. **Request-time routing must never need to classify precedence between a
   4b fixed-depth pattern and a recursive-wildcard candidate** — the
   composition step ORDER alone must make "4b always wins" free, never a
   runtime containment check on the hot path.
6. **Simplest solution first**: extend `access_rule_patterns` (ADR-063's own
   table) with a discriminator, not a new disjoint table, unless evidence
   shows extension is unworkable.

## Considered Options — Storage/Routing Mechanism (OQ-RW-02)

### Option A: Extend `access_rule_patterns` with a discriminator column — Accepted

Add `is_recursive BOOLEAN NOT NULL DEFAULT false`. For a recursive row,
`ancestor_segment_count`/`literal_skeleton` are repurposed to describe the
pattern's own FIXED PREFIX (always even) instead of a full ancestor (always
odd, for 4b rows). `collection_path_pattern` stores the bare rendered prefix
text (`""` for the zero-length, project-wide-catch-all case).

**Structural non-collision proof (load-bearing for this decision, not
asserted)**: a 4b ancestor's own rendered text always has an ODD segment
count (ADR-063 § Decision — The Ancestor/Leaf Split); a recursive prefix's
own rendered text always has an EVEN segment count (Resolution 3, locked).
Segment count is recoverable from rendered text by counting `/` separators
(a segment value can never itself contain `/` — the parser's own
delimiter). An odd-count and an even-count rendering can therefore **never**
collide as raw text — `collection_path_pattern` alone already distinguishes
the two row kinds for any non-empty prefix; the empty-string case (`""`)
is trivially distinct from any 4a/4b row (whose own patterns are never
empty). The `is_recursive` column and its place in the primary key are kept
explicit anyway — Earned Trust discipline: never build correctness on an
implicit invariant when an explicit, DB-level one costs nothing (mirrors
ADR-032's own "enforce at the DB layer, not by convention" precedent).

**Accepted.** Reuses the SAME table, SAME `AccessRulePatternRow` struct, SAME
history-fusion transaction shape, SAME PK-based idempotency-check-before-
upsert convention 4b already proved — near-zero new adapter surface. Most
importantly, satisfies Decision Driver 5 for free: "4b always wins over a
recursive candidate" falls directly out of composition step ORDER (4b's own
step 2 query runs first, unconditionally, against the SAME table; step 3's
recursive scan only runs on a miss) — no cross-table join, no runtime
containment check needed on the request-time path at all.

### Option B: A new disjoint table, `access_rule_recursive_patterns`

**Rejected.** Duplicates ~90% of `access_rule_patterns`' own shape (project
scoping, pattern text, segment-count/skeleton narrowing columns, condition
columns, timestamps) for a construct that is structurally the SAME kind of
row with a different reach semantics — violates Decision Driver 6. Worse,
it reintroduces the EXACT risk Decision Driver 5 exists to avoid: "does a 4b
pattern in table X beat a recursive pattern in table Y" now requires
querying two tables and merging in application code, a genuine new
cross-table coupling point ADR-063's own single-table design never had to
reason about. The only real advantage (no compound-PK migration, no
column-meaning overload) is outweighed by the structural non-collision proof
above, which removes the overload risk Option B would have avoided.

## Decision — Schema

```sql
-- migrations/0034_access_rule_patterns_recursive.sql
ALTER TABLE access_rule_patterns ADD COLUMN is_recursive BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE access_rule_patterns
    DROP CONSTRAINT access_rule_patterns_pkey,
    ADD PRIMARY KEY (project_id, collection_path_pattern, is_recursive);

ALTER TABLE access_rule_patterns
    ADD CONSTRAINT access_rule_patterns_recursive_even_prefix
    CHECK (NOT is_recursive OR ancestor_segment_count % 2 = 0);

-- Narrows the recursive-wildcard routing/listing scan (US-02/03 step 3;
-- US-04 cross-checks) to ONLY recursive rows. A project with zero
-- recursive-wildcard patterns costs an empty partial-index scan, not a
-- table scan (Decision Driver 2).
CREATE INDEX idx_access_rule_patterns_recursive_routing
    ON access_rule_patterns (project_id, ancestor_segment_count)
    WHERE is_recursive;
```

```sql
-- migrations/0035_access_rule_pattern_history_recursive.sql
-- Symmetric audit-fidelity extension (ADR-035 precedent). History is
-- append-only, never queried for routing — no index needed.
ALTER TABLE access_rule_pattern_history ADD COLUMN is_recursive BOOLEAN NOT NULL DEFAULT false;
```

`list_access_rule_patterns_by_skeleton` (ADR-063's own step-2 query, EXTEND)
gains an explicit `AND NOT is_recursive` filter. **Not strictly required for
correctness** — a request's own concrete ancestor segment count is always
odd (`DocumentPath.collection_path`'s own invariant), a recursive row's
stored `ancestor_segment_count` is always even (the CHECK constraint above),
so the two can never satisfy the same equality predicate — but added anyway
for the identical Earned Trust reason the schema's own non-collision proof
is backed by an explicit column: don't rely solely on an implicit parity
argument when an explicit filter is free.

## Decision — New Pure Primitives (`path_routing.rs`, EXTEND)

All new functions are built on the SAME private `positions_compatible`
predicate ADR-063's own `bind_ancestor`/`structurally_overlap` already use —
confirmed reused, never reimplemented (Decision Driver 1).

```rust
/// Request-time routing primitive (US-02/03). `prefix` is a stored
/// recursive pattern's own FIXED PREFIX segments (always even length,
/// RecursiveWildcard already stripped). `concrete_full_path` is a CONCRETE
/// document's own FULL path segments (ancestor + document ID joined,
/// always Literal-only, always even length) — NOT the ancestor alone
/// (`bind_ancestor`'s own equal-length precondition cannot represent the
/// zero-remaining-segments case). `Some((bindings, remainder_len))` iff
/// `concrete_full_path` is at least as long as `prefix` and every position
/// is compatible (SAME `positions_compatible` predicate). `None` = no
/// structural match.
pub fn bind_recursive_prefix(
    prefix: &[PathSegment],
    concrete_full_path: &[PathSegment],
) -> Option<(BTreeMap<String, String>, usize)>;

/// Reconstructs a stored 4b fixed-depth pattern's own FULL concrete-
/// document reach shape (ancestor + one trailing document-ID position) —
/// for import-time containment/overlap comparison against a recursive
/// prefix ONLY (US-04). The trailing position is always `Wildcard`,
/// regardless of whether the stored pattern captured a named leaf
/// (`leaf_variable: Some(_)`) or governs every document uniformly
/// (`leaf_variable: None`) — both reach ANY concrete document ID there;
/// only the NAME differs, and `positions_compatible`/`generalizes` never
/// inspect a `Wildcard`'s own captured name.
pub fn fixed_depth_full_reach(ancestor: &[PathSegment]) -> Vec<PathSegment>;

/// Import-time precedence-containment classification (US-04) — Resolution
/// 2's own locked 3-way outcome for a pair of prefixes (either two
/// recursive prefixes, or a recursive prefix vs. a 4b pattern's own
/// `fixed_depth_full_reach`).
pub enum PrefixRelation {
    /// Reaches never intersect — safe, both coexist (generalizes 4b's own
    /// AC-17-221 "different leaf collection name never overlaps"
    /// precedent to unequal-length prefixes).
    Disjoint,
    /// One prefix's reach is a structural superset of the other's —
    /// precedence-resolvable; the DEEPER (longer) prefix wins wherever
    /// both reach.
    Contains { deeper_is_b: bool },
    /// Reaches intersect but neither structurally contains the other (or
    /// they are identical-length with any overlap at all) — Resolution 2's
    /// own "genuine tie / unrelated-shape overlap," rejected at import
    /// time, never resolved by precedence.
    AmbiguousOverlap,
}

pub fn classify_prefix_relation(a: &[PathSegment], b: &[PathSegment]) -> PrefixRelation;
```

**Why equal-length pairs reuse `structurally_overlap` unchanged, never a new
equal-length code path**: `classify_prefix_relation`'s own first branch, for
`a.len() == b.len()`, calls `structurally_overlap(a, b)` directly — the
IDENTICAL function ADR-063 already shipped and already tests — mapping
`true` to `AmbiguousOverlap` (Resolution 2's own explicit "same-specificity
conflicts... still rejected" carve-out, reusing 4b's own Option C exactly at
this narrower level) and `false` to `Disjoint`. This is why 4b's own fixed-
depth-vs-fixed-depth Option C is "completely unchanged and unreopened," not
merely claimed to be: the same-length branch of this feature's own new
precedence logic literally calls 4b's own function.

**Why containment needs a NEW, directional predicate
(`generalizes`), not `positions_compatible` reused for unequal lengths**:
`positions_compatible`'s own "Wildcard is always compatible with anything"
rule answers "could SOME concrete value satisfy both" (the right question
for OVERLAP existence) but not "does the general side's own constraint
SUBSUME the specific side's own reach" (the right question for
CONTAINMENT). A `Literal` position in the shorter (candidate-general)
prefix must match an IDENTICAL `Literal` in the longer prefix's
corresponding position to contain it; a `Wildcard` position in the shorter
prefix contains anything. Confirmed by direct construction (not assumed): a
2-segment prefix `expeditions/trek-2026` (a per-expedition literal
catch-all) and a 4-segment prefix `expeditions/{expeditionId}/
journal_entries/{entryId}` structurally OVERLAP (both reach
`expeditions/trek-2026/journal_entries/*`) but do NOT contain one another
(the 4-segment one also reaches `expeditions/OTHER-expedition/
journal_entries/*`, outside the 2-segment one's own reach; the 2-segment one
also reaches `expeditions/trek-2026/photos/*`, outside the 4-segment one's
own reach) — this is exactly Resolution 2's own "unrelated-shape overlap,"
which `classify_prefix_relation` correctly reports as `AmbiguousOverlap`,
not `Contains`, because `generalizes(Literal("trek-2026"),
Wildcard("expeditionId"))` is `false` (a literal can never generalize a
wildcard it doesn't control).

```rust
fn generalizes(general: &PathSegment, specific: &PathSegment) -> bool {
    match (general, specific) {
        (PathSegment::Literal(x), PathSegment::Literal(y)) => x == y,
        (PathSegment::Wildcard(_), _) => true,
        (PathSegment::Literal(_), PathSegment::Wildcard(_)) => false,
        (PathSegment::RecursiveWildcard, _) | (_, PathSegment::RecursiveWildcard) => false,
    }
}
```

## Decision — Request-Time Routing Never Needs Containment Classification

**The single most important simplification this ADR makes, confirmed by
direct reasoning about the EXISTING composition order, not assumed**: at
request time, ADR-063's own composition already runs step 1 (exact-match)
then step 2 (4b fixed-depth, exact `bind_ancestor` on the EXACT concrete
ancestor) before this feature's new step 3 (recursive scan) ever executes,
and step 3 only runs when step 2 returns `None` for THIS SPECIFIC concrete
path. If step 2 hits, step 3 never runs — "4b always wins over a containing
recursive wildcard" (Resolution 2's own ranking #2) is therefore satisfied
by mere ORDERING, with zero runtime containment check. `classify_prefix_
relation`/`generalizes`/`fixed_depth_full_reach` are used ONLY at import
time (US-04); request-time routing (US-02/03) uses ONLY `bind_recursive_
prefix` plus "pick the matching candidate with the largest prefix length,
fail closed on a depth-tie" — and import-time validation (below) guarantees
a depth-tie between two SIMULTANEOUSLY-matching recursive candidates can
never actually occur in stored data, so this is a defensive assertion
mirroring ADR-063's own `routing_invariant_violated` precedent exactly, not
a new control-flow branch client behavior depends on.

## Decision — `resolve_access_rule_pattern` Extended (`grpc/handler.rs`, EXTEND)

One new internal step, added to the SAME shared helper all 6 locked call
sites already call — **zero call-site changes beyond threading one already-
in-scope `document_id: &str` argument** (every call site already has the
target document's own ID in scope; this mirrors ADR-062's own "zero new
I/O" mechanical-argument precedent exactly):

```rust
pub(crate) async fn resolve_access_rule_pattern(
    system_db: &SystemDb,
    project_id: &str,
    collection_path: &str,
    document_id: &str,   // NEW — every call site already has this value
) -> Result<Option<(AccessRulePatternRow, BTreeMap<String, String>)>, Status> {
    // Step 1/2 (UNCHANGED): exact-match handled by the caller before this
    // function is even invoked (unchanged); this function's own step 2 is
    // ADR-063's own exact `bind_ancestor` lookup, byte-for-byte unchanged.
    // ... existing step-2 body, unmodified ...
    if matched.is_some() {
        return Ok(matched);
    }

    // Step 3 (NEW, this feature): recursive-wildcard scan — reached ONLY
    // on a step-2 miss (§ Decision — Request-Time Routing Never Needs
    // Containment Classification).
    let mut concrete_full_path = concrete_ancestor.clone();
    concrete_full_path.push(PathSegment::Literal(document_id.to_string()));

    let candidates = system_db
        .list_recursive_access_rule_patterns_up_to(project_id, concrete_full_path.len() as i16)
        .await?;

    let mut best: Option<(AccessRulePatternRow, BTreeMap<String, String>, usize)> = None;
    for candidate in candidates {
        let prefix_segments = candidate.parse_fixed_prefix()?; // "" => vec![]
        let Some((bindings, _remainder)) =
            path_routing::bind_recursive_prefix(&prefix_segments, &concrete_full_path)
        else { continue };
        match &best {
            Some((_, _, best_len)) if *best_len > prefix_segments.len() => {}
            Some((_, _, best_len)) if *best_len == prefix_segments.len() => {
                tracing::error!(
                    project_id, collection_path,
                    "security_rules.routing_invariant_violated: two recursive-wildcard \
                     patterns matched the same concrete path at the SAME depth"
                );
                return Err(Status::permission_denied("access denied: routing invariant violated"));
            }
            _ => best = Some((candidate, bindings, prefix_segments.len())),
        }
    }
    Ok(best.map(|(row, bindings, _)| (row, bindings)))
}
```

**Zero change to `evaluate()`, `AccessRulePatternRow`'s own field shape, or
any of the 6 call sites' own control flow — re-verified, not assumed**:
Resolution 3 locks "no condition may reference the captured remainder," so a
recursive pattern's own `read_condition`/`write_condition` only ever
references the FIXED PREFIX's own named wildcards (e.g. `{expeditionId}`) —
exactly what `ancestor_path_variable_values` (ADR-063's own 6th `evaluate()`
parameter) already threads through, unchanged. `AccessRulePatternRow`
carries the SAME `read_condition`/`write_condition`/`leaf_variable` fields
whether `is_recursive` or not — a recursive row's own `leaf_variable` is
always `None` (the recursive wildcard captures no usable name, structurally
— see § Decision — Parser below) and is simply never populated into
`path_variable_value`. Every one of the 6 call sites threads the SAME
`(pattern_row, ancestor_bindings)` tuple into `evaluate()` exactly as
ADR-063 already wired it.

### Complexity (this runs on every read/write/query/Listen call)

- **Common case (project has zero recursive-wildcard patterns anywhere)**:
  step 3's own query hits `idx_access_rule_patterns_recursive_routing`, a
  PARTIAL index containing zero rows for such a project — an empty index
  scan, negligible. Total worst case on the full double-miss path: 3 indexed
  Postgres round-trips (vs. ADR-063's own 2) — named explicitly as a further
  Performance-vs-Simplicity trade-off, only on the path ADR-063 already
  degraded once; unchanged (1 or 2 round-trips) whenever step 1 or step 2
  already hits.
- **Recursive-pattern-governed case**: step 3 returns a small, bounded
  candidate set (K = per-project recursive-pattern count — realistically
  single digits, since catch-all constructs are rare by nature; bounded the
  same "dozens at most" way ADR-063's own fixed-depth candidate set is).
  Each candidate costs one `bind_recursive_prefix` call, O(depth). Overall:
  O(K × depth) pure CPU, both small constants.
- **Import-time overlap/precedence validation (US-04)**: O(N²) intra-file
  pairwise (N = patterns in the current import) plus O(N × M) cross-check
  (M = total already-stored patterns for the project, "dozens at most") —
  admin-only, low-frequency, identical complexity class to ADR-063's own
  equivalent check.

## Decision — Parser (`rules_file.rs`, EXTEND)

**`validate_segment_shape`, widened** (the ONE function DISCUSS itself
identified as the exact widening point — confirmed, not merely asserted, by
this being a single-pass, position-indexed rewrite of the existing loop):

```rust
fn validate_segment_shape(segments: &[PathSegment], path_pattern: &str) -> Result<(), RulesFileError> {
    for (i, seg) in segments.iter().enumerate() {
        let is_last = i == segments.len() - 1;
        match seg {
            PathSegment::RecursiveWildcard if !is_last =>
                return Err(reject("RECURSIVE_WILDCARD_NOT_TERMINAL", ...)),
            PathSegment::RecursiveWildcard if !i.is_multiple_of(2) =>
                return Err(reject("RECURSIVE_WILDCARD_ODD_PREFIX", ...)),
            PathSegment::RecursiveWildcard => {} // terminal, even prefix — valid
            other if i.is_multiple_of(2) && !matches!(other, PathSegment::Literal(_)) =>
                return Err(reject("NESTED_PATH", ...)),
            _ => {}
        }
    }
    Ok(())
}
```

A recursive wildcard sitting at an EVEN index (like every collection-name
position) and terminal is valid — the direct generalization of the existing
"every even index holds `Literal`" rule, since prefix-length-even means the
recursive wildcard's own index equals the (even) prefix length. A recursive
wildcard at an ODD index is, by construction, always the odd-prefix case
(`RECURSIVE_WILDCARD_ODD_PREFIX`); anywhere but the final position is
`RECURSIVE_WILDCARD_NOT_TERMINAL`. **Taxonomy note, not a regression**: the
pre-existing test `decompose_still_rejects_a_recursive_wildcard_in_a_multi_
segment_pattern` (`rules_file.rs`) exercises `expeditions/{expeditionId}/
journal_entries/{name=**}` — a terminal recursive wildcard with a 3-segment
(ODD) prefix — whose expected `construct` string changes from the old bare
`"RECURSIVE_WILDCARD"` to `"RECURSIVE_WILDCARD_ODD_PREFIX"` under this
feature's own widened taxonomy. This is a known, correctly-flagged test-
string update for the crafter to make during DELIVER (US-05 proof
obligation), not a silent behavior change — the SHAPE is still rejected,
only the construct label narrows to name the specific reason.

**`decompose_block`, new branch (EXTEND)**: when `segments.last()` is
`RecursiveWildcard` (guaranteed terminal + even-prefix by the widened
`validate_segment_shape` above), split off the prefix (`&segments[..len-1]`)
and decompose it via the IDENTICAL wildcard-name-collection +
condition-rewrite + verb-bucketing loop the existing function already runs
for 4b's own ancestor — inlined into the new branch, not a new function.
**The recursive wildcard's own captured name is never added to the rewrite
list** — `PathSegment::RecursiveWildcard` carries no associated name data at
all (confirmed: the enum variant is unit-like, the scanner already discards
whatever name Alex wrote, e.g. `document` vs. `path`, at scan time) —
directly enforcing Resolution 3's own lock (no condition may reference the
captured remainder) structurally: there is nothing to add, not merely a
convention not to add it. This also means two recursive patterns at the
SAME fixed prefix, differently named (`{document=**}` vs. `{path=**}`),
render to the IDENTICAL stored text and are correctly treated as the SAME
pattern for idempotency purposes — a genuine simplification versus 4b's own
name-sensitive leaf/ancestor wildcards, worth naming explicitly.

```rust
pub struct DecomposedRecursivePattern {
    pub fixed_prefix_pattern: String,       // "" for the zero-length (project-wide) case
    pub fixed_prefix_segment_count: u16,    // always even, >= 0
    pub literal_skeleton_prefix: String,    // literal_skeleton() of the prefix only
    pub read_condition: Option<String>,
    pub write_condition: Option<String>,
}

pub enum DecomposedTarget {
    SingleCollection(DecomposedRule),           // UNCHANGED (4a)
    MultiSegmentPattern(DecomposedPatternRule),  // UNCHANGED (4b)
    RecursiveWildcardPattern(DecomposedRecursivePattern), // NEW, additive
}
```

## Decision — Adapter (`system_db.rs`, EXTEND)

`AccessRulePatternRow` gains `is_recursive: bool`. `upsert_access_rule_
pattern` gains an `is_recursive: bool` parameter, threaded into both the
`INSERT` and the `ON CONFLICT (project_id, collection_path_pattern,
is_recursive)` target (mechanical, mirrors the compound-PK change above).
`get_access_rule_pattern`/`list_access_rule_patterns_by_skeleton` read the
new column (`list_access_rule_patterns_by_skeleton` additionally filters
`AND NOT is_recursive`, § Decision — Schema). Two new methods:

```rust
/// Request-time routing (US-02/03, step 3) — narrowed via the partial
/// index to recursive rows whose own fixed-prefix length does not exceed
/// the concrete full path's own length (a longer prefix can never
/// structurally match a shorter concrete path).
pub async fn list_recursive_access_rule_patterns_up_to(
    &self, project_id: &str, max_prefix_segment_count: i16,
) -> Result<Vec<AccessRulePatternRow>, CoreError>;

/// Import-time overlap/precedence validation (US-04) ONLY — every pattern
/// (either kind) currently stored for a project, bounded by realistic
/// per-project pattern counts ("dozens at most," ADR-063's own precedent).
/// Never called on the read/write/query/Listen hot path.
pub async fn list_all_access_rule_patterns(
    &self, project_id: &str,
) -> Result<Vec<AccessRulePatternRow>, CoreError>;
```

## Decision — Overlap Detection, Generalized (`admin/handlers/access_rules.rs`, EXTEND)

`check_pattern_overlap` (ADR-063's own 4b-only intra-file + cross-import
overlap check) is **widened, not duplicated**, into the single shared
overlap-detection entry point for the WHOLE pattern family (4b + this
feature) — directly satisfying Decision Driver 3 at the strongest level
this initiative has applied it: a NEW 4b pattern must also be checked
against already-stored RECURSIVE patterns (the reverse direction from a NEW
recursive pattern's own checks), and both directions must derive "do these
overlap, and if so is it resolvable" via the identical classification, never
two independently-maintained rules that could silently disagree.

1. **Intra-file**: every pair of patterns in the current import (any kind)
   — same-length pairs via `structurally_overlap` (unchanged); different-
   length pairs (always at least one recursive pattern involved, since 4b
   ancestors are always odd and recursive prefixes always even, so two 4b
   patterns can never be different-length-comparable in a way this adds
   logic for) via `classify_prefix_relation`. `AmbiguousOverlap` → reject
   both, naming them (`PATTERN_OVERLAP`, reused unchanged). `Contains` or
   `Disjoint` → both import (this is where a specific-override-plus-
   catch-all pair is CORRECTLY accepted, unlike 4b's own reject-on-any-
   overlap).
2. **Cross-import**: each new pattern against `list_all_access_rule_
   patterns` (the new, bounded, project-wide scan) — identical
   classification logic, reused. A byte-identical already-stored row
   (`collection_path_pattern` + `is_recursive` both equal) is skipped —
   idempotent re-import, never an overlap (mirrors AC-17-205/222).
3. A 4b pattern's own comparison segments are its `fixed_depth_full_reach`
   (§ Decision — New Pure Primitives) whenever compared against a recursive
   candidate; its own bare ancestor whenever compared against another 4b
   pattern (unchanged from ADR-063).

## Decision — Admin Surface: Import + Simulation (EXTEND, no new routes)

`import_rules_file` gains one new match arm for `DecomposedTarget::
RecursiveWildcardPattern`, mirroring the existing `MultiSegmentPattern`
arm's identical idempotency-check-before-upsert shape against the same
`access_rule_patterns` table, with `is_recursive: true`.

`simulate_routed_access_rule` (US-06, Release 2, EXTEND — not a new
handler): candidate-pattern parsing is widened to detect a trailing
`RecursiveWildcard` segment (reusing `rules_file::parse_path_segments`
unchanged) and dispatch to `bind_recursive_prefix` instead of
`bind_ancestor`. Per AC-17-260 (a candidate must be shown as deferred to an
already-stored, more specific pattern, never silently applied), the handler
first re-runs the REAL 3-step precedence composition against STORED rows
for the synthetic path; only if no stored pattern wins does it fall back to
evaluating the CANDIDATE's own outcome — reusing the same composition logic
real enforcement uses, never a second, independently-maintained
implementation (mirrors 4b's own DDD-PM-9 precedent). The response contract
(`outcome: Allow|Deny|NoMatchingPattern`, `bindings`) needs no shape change.

## No new `Operand` variant, `decompose_decidable` unaffected (re-verified)

Confirmed by direct inspection: `decompose_decidable`'s match arms are keyed
on `Operand` variant only; a recursive pattern's own condition references
only pre-existing `Operand::PathVariable` (for prefix wildcards, identical
to 4b's own ancestor wildcards) — never a new variant, since Resolution 3
locks out any grammar reference to the captured remainder. Zero
re-verification risk beyond what ADR-063 already closed for
`decompose_decidable` (this feature introduces no new `Operand` shape at
all, a narrower footprint than ADR-063's own multi-variable extension).

## Consequences

### Positive

- "4b always wins over a containing recursive wildcard" costs nothing at
  request time — a structural consequence of composition step ORDER, not a
  runtime check.
- One table, one row shape, one set of pure functions serve both routing
  kinds — `access_rule_patterns` gains a discriminator, never a parallel
  schema.
- `evaluate()`, `AccessRulePatternRow`, and all 6 already-wired call sites'
  own control flow are completely unchanged in shape — only one new,
  already-in-scope `document_id` argument threads through.
- The recursive wildcard's own captured name being structurally
  unrepresentable (no data on the enum variant) directly enforces
  Resolution 3 at the type level, not by convention.
- Zero new workspace dependency, zero new driven port, zero new Earned
  Trust probe (see § Enforcement).

### Negative / Trade-offs

- A collection with no exact-match and no 4b fixed-depth pattern now costs a
  THIRD indexed lookup (vs. ADR-063's own second) on the full-miss
  read/write/Listen-per-event path — named explicitly, mirrors ADR-063's
  own identical, already-accepted trade-off one level further.
- `access_rule_patterns`' own primary key becomes a 3-column compound key —
  a real, if small, migration (`DROP CONSTRAINT` / `ADD PRIMARY KEY`), and
  `ancestor_segment_count`/`literal_skeleton` now carry a discriminator-
  dependent meaning (mitigated: DB-level `is_recursive` column + CHECK
  constraint, never convention-only; every consumer already branches on
  `DecomposedTarget`'s own variant regardless).
- `RunQuery`/Listen's own subscribe-time compliance gate remain pattern-
  blind for recursive-wildcard patterns too — unchanged scope boundary,
  carried forward from ADR-063's own `OQ-PM-07`, not reopened.
- Odd-prefix recursive wildcards and condition-grammar referencing of the
  captured remainder remain out of scope — named, deferred (Resolutions 2/3
  of DISCUSS), unaffected by this ADR.

## Enforcement

Style: Hexagonal (ports-and-adapters), unchanged. BC-4 Access Control
(ADR-029) gains new pure functions in the EXISTING `path_routing` submodule,
one new additive `rules_file` type
(`DecomposedRecursivePattern`/`DecomposedTarget::RecursiveWildcardPattern`),
and one new argument on an existing `pub(crate)` helper
(`resolve_access_rule_pattern`). No new bounded context.

**No new driven port, no new Earned Trust probe (Principle 12 discipline,
mirroring ADR-027/029/030/031/033/062/063 § Enforcement verbatim)**: the two
new adapter methods (`list_recursive_access_rule_patterns_up_to`,
`list_all_access_rule_patterns`) execute plain `sqlx` queries through the
EXISTING, already-probed `SystemDb` connection pool — the identical
substrate every other BC-4 read/write already uses, with zero new
reliance. `bind_recursive_prefix`, `classify_prefix_relation`,
`generalizes`, `fixed_depth_full_reach`, and the widened `rules_file::
decompose` are pure, deterministic CPU computation over in-memory
values — "no environment can lie to a pure function" applies unmodified.
The substrate this feature adds new reliance on is exactly zero, confirmed
by direct enumeration of every new function above, not asserted by
analogy alone.

`cargo-deny`/`deny.toml` unaffected — no new workspace dependency.
`embyr_core::access_control`'s existing zero-IO enforcement covers the
extended `path_routing`/`rules_file` submodules unchanged.

## References

- `docs/feature/security-rules-cel-recursive-wildcards/feature-delta.md`
  §§ Job Discovery Framing Resolution (Resolutions 1-4), System Constraints,
  User Stories (US-01 through US-06), Handoff Package.
- `docs/product/architecture/adr-063-multi-segment-path-pattern-routing-and-storage.md`
  — the mechanism this ADR augments; every decision above that says
  "unchanged" refers to this ADR's own shipped shape.
- `docs/product/architecture/adr-062-rules-file-import-parser-path-variable-and-decomposition.md`
  — the leaf-capture mechanism, reused unchanged.
- `crates/embyr-core/src/access_control/rules_file.rs` (full, read during
  DESIGN) — exact current `PathSegment`/`validate_segment_shape`/
  `decompose_block` shapes this ADR widens.
- `crates/embyr-core/src/access_control/path_routing.rs` (full, read during
  DESIGN) — exact current `positions_compatible`/`bind_ancestor`/
  `structurally_overlap` shapes this ADR's new primitives are built beside.
- `crates/embyr-core/src/access_control/mod.rs` (targeted, read during
  DESIGN) — exact current `Operand`/`evaluate`/`decompose_decidable` shapes
  confirmed unaffected before designing this extension.
- `crates/embyr-server/src/adapters/system_db.rs` (targeted, read during
  DESIGN) — exact current `access_rule_patterns` CRUD this ADR extends.
- `crates/embyr-server/src/grpc/handler.rs` (targeted, read during DESIGN)
  — exact current `resolve_access_rule_pattern`/`handle_get_document` call
  shapes this ADR's new step 3 composes with.
- `crates/embyr-server/src/admin/handlers/access_rules.rs` (targeted, read
  during DESIGN) — exact current `check_pattern_overlap`/`import_rules_
  file`/`simulate_routed_access_rule` shapes this ADR extends.
- `migrations/0032_access_rule_patterns.sql`,
  `0033_access_rule_pattern_history.sql` — the schema this ADR's own
  migrations (`0034`, `0035`) alter.
