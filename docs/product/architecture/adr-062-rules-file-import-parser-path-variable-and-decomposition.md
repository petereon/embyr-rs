# ADR-062: Rules-File Import — Outer-Syntax Parser, Canonical Path-Variable Operand, and Atomic Decomposition

## Status

Accepted

## Context

`security-rules-cel-parity` (Epic 4a, JOB-17's 8th realization) gives Alex a
way to import a real Firestore `.rules` file — `service cloud.firestore {
match /databases/{database}/documents { match /<collection>/{<var>}? { allow
<verbs>: if <condition>; } } }` — for single top-level collections with at
most one leaf-level path-variable capture, decomposed into embyr's existing
per-collection admin API. DISCUSS
(`docs/feature/security-rules-cel-parity/feature-delta.md`, Resolution 1
Option C, Resolution 2, Resolution 3) locked the observable behavior; this
ADR decides the mechanism, per Handoff Package flag 7's explicit charge:
"the path-variable-name-to-value binding is a genuinely new parsing concern
... flagged explicitly for DESIGN's own architecture design, not resolved
here."

Confirmed by direct code read (`crates/embyr-core/src/access_control/mod.rs`,
full): the locked v1 grammar (ADR-027, extended ADR-030/031/034) has zero
path-structure awareness — `Operand` has 8 variants, none referencing a
document's own path or ID. `access_rules`/`write_access_rules` (ADR-028/030)
are `TEXT NOT NULL` condition-source columns with **zero spare columns** —
DISCUSS's own System Constraints forbid any schema change. `decompose_decidable`
(ADR-031) and `handle_add_target`'s per-event `evaluate()` calls (ADR-033)
are the two other consumers of `Condition`/`Operand` this ADR must reason
about without modifying their own locked scope.

## Decision Drivers

1. **Zero new storage shape** (DISCUSS System Constraints, hard boundary) —
   no new column on `access_rules`/`write_access_rules`, no new table.
2. **No context-dependent parsing** — Handoff Package flag 7 flagged, but did
   not mandate, a parser that must receive "the enclosing match block's own
   captured-variable set as an explicit parameter." A design that avoids this
   entirely is preferable to one that builds it, if the locked scope (exactly
   one leaf-level variable, DISCUSS Resolution 1 Option C) makes it
   avoidable.
3. **Do not silently widen the locked condition grammar** (ADR-027 Decision
   Driver 1, reapplied) — extending `Operand` must follow the existing
   dot-prefixed-family precedent (`resource.data.`, `request.resource.data.`,
   `request.auth.token.`), not invent a second addressing scheme.
4. **`RunQuery`/Listen subscribe-time "zero new code" claim must be verified,
   not assumed** (DISCUSS System Constraints, Handoff Package flag 5).
5. **All-or-nothing import, every offending block named** (DISCUSS Resolution
   2, hard boundary) — achieved without inventing new cross-call transactional
   machinery unless evidence requires it (Principle 8).
6. **Simplest solution first** — no new crate, no new bounded context, reuse
   the existing hand-rolled-parser style (ADR-027 Option C) for the same
   "must not silently widen" reason, now applied to the outer file grammar.

## Considered Options — Path-Variable Binding Mechanism

### Option A: Context-parameterized parsing — `parse_condition` accepts a per-block set of valid variable names

`parse_condition(source, known_path_vars: &[String])`, threading the
enclosing `match` block's own captured name through both the import-time
parse and every future re-parse of the stored `condition_source`.

**Rejected.** Two structural problems: (1) the RE-PARSE call sites
(`handle_get_document` and the 3 write handlers, ADR-029/030) re-parse from
the stored `condition_source` string alone — they would need the block's
own captured variable name available at evaluate time too, which means
storing it somewhere, directly contradicting Decision Driver 1 (no new
storage shape) unless re-derived by some other means; (2) it changes
`parse_condition`'s signature, a function 5 existing call sites already use
(`define_access_rule`, `define_write_access_rule`, `handle_get_document`, 3
write handlers, `simulate_access_rule`) with the OLD, context-free
signature — every one of them would need a wiring decision ("what variable
set applies here?") that has no good answer for a plain JSON-API-authored
rule, which has no enclosing `match` block at all.

### Option B: A parallel `PathVariableCondition` type, evaluated by a second function

A structurally separate representation for path-variable-bearing rules,
evaluated by a new function alongside `evaluate()`.

**Rejected.** Directly repeats the exact anti-pattern ADR-030 Decision Driver
3 and ADR-029 DDD-SR-8 already rejected once (`evaluate_write()`) — a second,
independently-maintained evaluation routine is the single highest drift risk
this initiative's own precedent explicitly avoids everywhere else.

### Option C: Canonical dot-prefixed rewrite at import time — Accepted

The importer rewrites the raw file's bare wildcard-name occurrences (e.g.
`userId`) into embyr's own canonical, self-describing operand text
(`request.path.userId`) **before** calling the existing, unmodified
`upsert_access_rule`/`upsert_write_access_rule`. `Operand` gains
`PathVariable(String)`, parsed via one new, ordinary, unconditional
dot-prefix arm in `word_to_operand` — uniform with `resource.data.`,
`request.resource.data.`, and `request.auth.token.`. The stored
`condition_source` is therefore fully self-contained: every future re-parse
(real enforcement, simulation) reconstructs the identical `Operand::PathVariable`
node from the text alone, with **zero side-channel context and zero new
storage column**.

**Accepted.** Resolves Handoff Package flag 7 by making the "genuinely new
parsing concern" not exist as a parsing concern at all — it becomes an
ordinary token-prefix addition, the same one-line-per-family shape ADR-030/034
already established twice. `parse_condition`'s signature is completely
unchanged; all 5 existing call sites need zero modification to their own
call shape (`parse_condition(source)`), only `word_to_operand`'s internal
match arms grow by one.

## Decision — Grammar Extension

### `Operand` enum (extends ADR-027/030/034)

```
pub enum Operand {
    AuthUid,
    AuthNullSentinel,
    ResourceField(String),
    RequestResourceField(String),
    AuthTokenClaim(String),
    BoolLiteral(bool),
    NullLiteral,
    StringLiteral(String),
    PathVariable(String),   // NEW — parsed from "request.path.<name>"
}
```

`word_to_operand` gains one match arm:

```
w if w.starts_with("request.path.") => {
    let name = &w["request.path.".len()..];
    if name.is_empty() { return Err(syntax_error(...)); }
    Ok(Operand::PathVariable(name.to_string()))
}
```

No tokenizer change (identical reasoning to ADR-030: `.` is already a
word-continuation character). `detect_unsupported_construct`'s `**`/`{`/
call-syntax pre-scan is unaffected — `request.path.userId` contains none of
those markers.

The variable's **name** is retained in the AST purely for fidelity/future
use (Epic 4b's multiple-wildcards will need it to disambiguate); this
feature's own locked scope (exactly one leaf-level wildcard) means every
`PathVariable` reference in a given condition resolves to the **same**
single value regardless of name, so `evaluate()` does not need a name-keyed
lookup structure — see § Decision — Evaluation below.

### `evaluate()` signature (extends ADR-027/030)

```
pub fn evaluate(
    condition: &Condition,
    auth: Option<&AuthContext>,
    resource_fields: &BTreeMap<String, FieldValue>,
    request_resource_fields: &BTreeMap<String, FieldValue>,
    path_variable_value: Option<&str>,   // NEW parameter
) -> EvaluationOutcome
```

`resolve_field_value` gains one arm:

```
Operand::PathVariable(_) => path_variable_value
    .map(|id| FieldValue::String(id.to_string()))
    .ok_or(FieldMissing),
```

Fail-closed by construction: a call site that passes `None` denies any rule
referencing `PathVariable`, via the identical `FieldMissing` short-circuit
every other operand family already uses — no new error class, no new
control-flow shape (Decision Driver 6).

**Why `Option<&str>`, not `BTreeMap<String, String>`**: this feature's own
locked scope (DISCUSS Resolution 1 Option C) guarantees at most one wildcard
segment per collection, structurally making "the value" unambiguous
regardless of how many times or under what name the condition references
it. A map keyed by variable name is the natural extension point for Epic
4b (multiple wildcards) but is not built now (YAGNI) — introducing it today
would be speculative machinery for a case this feature cannot reach.

## Decision — Structural Verification (Decision Driver 4)

**Confirmed by direct inspection, not assumed**: `decompose_decidable`
(`crates/embyr-core/src/access_control/mod.rs`, ADR-031) matches only
`Condition::Literal`, `Compare(AuthNullSentinel, _, NullLiteral)` (both
orders), `Compare(AuthUid, Eq, ResourceField(_))` (both orders), and `And`;
every other shape — including any `Compare` naming `Operand::PathVariable`
on either side, in any position, under any `CompareOp` — falls through the
existing, unmodified wildcard arm `_ => Err(Undecidable)`. This is the
identical mechanism that already, today, rejects `AuthTokenClaim`/
`StringLiteral`/`RequestResourceField`-referencing rules for `RunQuery`
compliance with zero code change. Adding `PathVariable` requires **zero**
modification to `decompose_decidable`, `check_query_compliance`, or
`handle_run_query` — confirming DISCUSS's own flagged claim exactly.

Listen's subscribe-time gate (`handle_add_target`, ADR-033) calls the
identical `check_query_compliance` — the same zero-code-change guarantee
holds there too.

## Decision — Listen's Per-Event Re-Check Is Deliberately Not Wired

`handle_add_target`'s `Changed`/`Removed` arms (ADR-033) each call
`evaluate(condition, auth_ctx.as_ref(), &doc.fields, &empty_fields)` — 4
positional arguments. Rust has no default arguments, so adding `evaluate`'s
5th parameter requires a **mechanical, one-line edit at these 2 call
sites**, unavoidably, even though DISCUSS explicitly scoped Listen's
per-event re-check as out of bounds for this feature (§ Out of Scope: "no
domain example in this feature requires a live-updating, path-variable-bound
subscription"). Both call sites pass `path_variable_value: None` —
deliberately, not by omission. Consequence, stated explicitly rather than
left implicit: for the lifetime of this feature, a `PathVariable`-referencing
rule's per-event Listen re-check always resolves `PathVariable` to
`FieldMissing`, which fails the whole condition closed (`Deny`) for every
delivered event on such a subscription. This is safe (fail-closed, never a
false-allow) but not a working feature — flagged as **OQ-CP-04** (§ Open
Questions, feature-delta.md) for whichever future epic (most likely 4b,
which already touches path-matching machinery) prioritizes wiring it.

## Decision — Outer-Syntax Parser (`embyr-core::access_control::rules_file`, new module)

### Considered and rejected: a grammar-file-driven parser (`pest`/`nom`/similar)

Rejected for the identical reason ADR-027 Option A rejected one for the
condition grammar, reapplied one layer out: a `.pest`-file grammar makes
"accept one more path shape" (nested collections, recursive wildcards — Epic
4b/4c/4d/4e's own deferred scope) a one-line grammar-file edit, which is
exactly the "silently widen the locked outer grammar" risk this feature must
resist even more than the condition grammar does, since the outer syntax IS
the scope boundary DISCUSS's Resolution 1 Option C locked. Also a new
workspace dependency for a genuinely small, closed grammar (Principle 8).

### Accepted: hand-rolled scanner + recursive block/verb parser, zero IO, zero new dependency

New file `crates/embyr-core/src/access_control/rules_file.rs`:

- `PathSegment` — `Literal(String) | Wildcard(String) | RecursiveWildcard`.
- `MatchBlock` — `{ path_pattern: String (raw, for error messages), segments: Vec<PathSegment>, allow_clauses: Vec<(Vec<Verb>, String /* raw condition text */)> }`.
- `Verb` — `Read | Get | List | Write | Create | Update | Delete` (Firestore's
  own granular vocabulary, bucketed below).
- `parse_rules_file(source: &str) -> Result<Vec<MatchBlock>, RulesFileError>`
  — validates the literal `service cloud.firestore { match
  /databases/{database}/documents { ... } }` wrapper (rejected as a plain
  syntax error if malformed — this fixed shell is not itself a grammar
  concern), then scans each inner `match /<pattern> { allow <verbs>: if
  <condition>; ... }` block via `/`-delimited path-segment splitting and
  brace/semicolon-delimited clause splitting — the identical
  hand-rolled-scanner discipline ADR-027's condition tokenizer already
  established, applied one syntactic layer out.
- `decompose(blocks: Vec<MatchBlock>) -> Result<Vec<DecomposedRule>, RulesFileError>`
  — the validation + rewrite + verb-bucketing pass (§ Decision — Decomposition
  below). `RulesFileError` carries **every** offending block found (a `Vec`,
  never fails fast on the first), per DISCUSS Resolution 2/US-04.

Zero new workspace dependency; zero IO; module lives beside the existing
`mod.rs` inside `embyr-core::access_control`, covered by the same
`deny.toml` zero-IO rule unchanged (a submodule, not a new crate).

## Decision — Decomposition (Path-Shape Validation, Verb-Bucketing, Rewrite)

### Path-shape validation

Only two `segments` shapes are valid: `[Literal(coll)]` (no wildcard, e.g.
`journal_entries`, `app_config`) or `[Literal(coll), Wildcard(var)]` (one
leaf-level capture, e.g. `profiles/{userId}`). Any `RecursiveWildcard`
anywhere → reject, reason `RECURSIVE_WILDCARD`. Any other shape (length > 2,
two `Literal`s, a `Wildcard` not in the last position, more than one
`Wildcard`) → reject, reason `NESTED_PATH`. This is the entire mechanism
behind AC-17-188/189 — an explicit allow-list of exactly 2 shapes, not a
disallow-list, mirroring `decompose_decidable`'s own "reject is the only
reachable outcome for an unnamed shape" discipline (ADR-031 Decision Driver
6) at the path layer.

### Condition rewrite

For a block with a captured `Wildcard(var)`, every **whole-word** occurrence
of `var` in each `allow` clause's raw condition text is rewritten to
`request.path.<var>` before parsing (a plain string substitution over
tokenized word boundaries — reuses the same word-character class the
condition tokenizer already defines, so `userIdSuffix` is never partially
matched inside `userId`). A block with no captured wildcard performs no
rewrite. The rewritten text is then validated via the existing, unmodified
`parse_condition` — reusing its `UnsupportedConstruct::{CrossDocumentRead,
CustomFunction}` detection verbatim for AC-17-190, and its plain
`SyntaxError` catch-all for any bare identifier that is NOT the block's own
captured name (a typo, e.g. `uid` when the path captured `userId`) — this
already satisfies AC-17-193 ("distinguishable from a runtime missing-field
denial") because it fires at import-time parsing, a structurally different
code path/response type than any runtime `PermissionDenied`. No new
construct/error variant is introduced for this case (Principle 8).

### Verb-bucketing

`{Read, Get, List}` → the read bucket (`upsert_access_rule` /
`access_rules`); `{Write, Create, Update, Delete}` → the write bucket
(`upsert_write_access_rule` / `write_access_rules`) — embyr's existing
2-condition-per-collection model (ADR-028/030) has no finer granularity
than read/write, so Firestore's more granular verb vocabulary is
intentionally collapsed onto it. If a single block's `allow` clauses assign
**two different** (post-rewrite) condition texts to the same bucket (e.g. a
`get`-specific condition differing from a `list`-specific condition), the
import is rejected in full, naming that block, reason
`CONFLICTING_VERB_CONDITIONS` — embyr's storage has no way to express two
conditions for one bucket, and silently picking one would silently narrow
Alex's actual intent. This is a DESIGN-owned scoping decision, not
pre-locked by DISCUSS; flagged as **OQ-CP-05** for DISTILL to add explicit
acceptance coverage (feature-delta.md § Open Questions).

## Decision — Import Atomicity (No New Transaction)

The admin handler (§ Decision — Admin Surface, below) runs `parse_rules_file`
then `decompose` over the **entire** file first. If either returns any
error, the handler returns the rejection response and calls **zero**
storage methods — this alone satisfies DISCUSS Resolution 2's locked
requirement ("rejected in full... before any existing rule is touched").
Only once every block validates does the handler loop over
`Vec<DecomposedRule>`, calling the existing, byte-for-byte unmodified
`upsert_access_rule`/`upsert_write_access_rule` once per (collection,
bucket) pair — each call already transactional internally (its own
history-capture fusion, ADR-035), just not wrapped in one shared,
cross-call transaction.

**Considered and rejected: a new `Transaction`-taking sibling of
`upsert_access_rule`/`upsert_write_access_rule`, wrapping the whole loop in
one Postgres transaction.** Rejected for this feature's own scope: no domain
example or acceptance scenario (Slices 04/05) exercises a mid-loop
infrastructure failure — every one of Resolution 2's own scenarios is about
rejecting on invalid FILE CONTENT, fully closed by validate-then-apply
above. Building new transaction-taking plumbing for an untested,
unevidenced failure class is exactly the over-building Principle 8 and this
initiative's own "flag, don't invent absent evidence" discipline
(re-applied throughout `security-rules-*`) both argue against, and it would
make the decomposition target genuinely modified rather than the literally
unmodified calls DISCUSS's own System Constraints ask for. Flagged as a
named residual risk, not silently accepted: a Postgres connection failure
between block N and N+1 of a multi-block import could leave a prefix of the
file's collections updated and the rest not — identical in kind to every
other multi-step admin action in this codebase today, none of which has
cross-call transactional atomicity either.

## Decision — Admin Surface

**New route, `POST /admin/v1/projects/:project_id/access_rules/import`** —
Owner/Admin role gate (mirrors `define_access_rule`'s gate exactly, since
this writes rules; not `simulate_access_rule`'s any-role, read-only gate).
Sibling of the existing `access_rules` route group.

```rust
pub struct ImportRulesFileBody {
    pub rules_file: String,   // raw .rules file text
}

pub struct ImportedBlockSummary {
    pub collection_path: String,
    pub read_condition: Option<String>,    // rewritten condition stored in access_rules, if any
    pub write_condition: Option<String>,   // rewritten condition stored in write_access_rules, if any
}

pub struct ImportRulesFileResponse {
    pub project_id: String,
    pub imported: Vec<ImportedBlockSummary>,
}

pub struct OffendingBlock {
    pub path_pattern: String,     // raw path text as written, robust even for invalid shapes
    pub construct: &'static str,  // "NESTED_PATH" | "RECURSIVE_WILDCARD" | "CROSS_DOCUMENT_READ"
                                   // | "CUSTOM_FUNCTION" | "CONFLICTING_VERB_CONDITIONS" | "SYNTAX_ERROR"
    pub detail: String,
}

pub struct RulesFileRejectionResponse {
    pub reason: &'static str,     // "IMPORT_REJECTED"
    pub offending_blocks: Vec<OffendingBlock>,
}
```

200 on success (every block applied); 400 with `RulesFileRejectionResponse`
naming every offending block (AC-17-191) on any rejection, mirroring
`condition_parse_error_response`'s existing `SYNTAX_ERROR`/
`UNSUPPORTED_CONSTRUCT` taxonomy shape one level up, at the whole-file
granularity. Idempotent re-import (AC-17-176/195) requires zero special
handling — it is the identical `upsert_*` calls with identical rewritten
text, and ADR-035's own "single-redefine-equals-single-history-entry"
mechanism is reused unmodified.

### `simulate_access_rule` extension (US-06)

`SimulateAccessRuleBody` gains one new, additive (`#[serde(default)]`)
field:

```rust
pub struct SimulateAccessRuleBody {
    pub condition: String,
    pub auth: Option<SimulatedAuth>,
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub resource: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub request_resource: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub path_variable: Option<String>,   // NEW (US-06) — synthetic document ID
}
```

`simulate_access_rule`'s existing `evaluate(...)` call gains one new
argument, `body.path_variable.as_deref()` — mirrors ADR-030/031's own
additive-field extension precedent (not ADR-032's "new handler," since the
response contract `{outcome}` is completely unchanged — the genuinely
-different-contract test that justified a new handler there does not apply
here). No new route.

## Consequences

### Positive

- `parse_condition`'s signature is completely unchanged — every one of its 6
  existing call sites (5 pre-existing + `rules_file::decompose`'s own new
  use) needs zero modification to how it is called.
- The path-variable binding is resolved entirely through the existing
  dot-prefixed-operand-family mechanism — no context object, no
  two-pass parser-with-side-channel, no new storage column.
- `decompose_decidable`/`check_query_compliance`/`handle_run_query`/
  `handle_add_target`'s subscribe-time gate require zero code change —
  verified by direct inspection, not assumed, closing Handoff Package flag 5.
- Import atomicity for the evidenced risk (invalid content) is structurally
  complete (validate-before-any-write); the unevidenced risk (mid-loop infra
  failure) is named, not hidden.
- Zero new workspace dependency, zero new bounded context, zero new
  migration.

### Negative / Trade-offs

- `evaluate()`'s 5th parameter is a mechanical touch to 2 call sites (Listen's
  per-event arms) that do not use it meaningfully in this feature — an
  unavoidable consequence of Rust's lack of default arguments, not new logic.
- Listen's per-event re-check fails closed, not correctly, for
  `PathVariable`-bearing rules — a real, if safe, capability gap, named as
  OQ-CP-04.
- The verb-bucketing/conflict-detection rule (`CONFLICTING_VERB_CONDITIONS`)
  is a DESIGN-introduced scoping decision beyond DISCUSS's own enumerated
  rejection taxonomy — flagged as OQ-CP-05 for DISTILL to add explicit
  acceptance coverage, rather than silently asserting DISCUSS already
  decided it.
- No cross-block transactional atomicity against infrastructure failure
  mid-import — accepted, evidenced-absent risk, named explicitly rather than
  built against speculatively.

## Enforcement

Style: Hexagonal (ports-and-adapters), unchanged. BC-4 Access Control
(ADR-029) gains one new submodule (`rules_file`) and one new `Operand`
variant; no new bounded context.

**No new driven port, no new probe (Principle 12 discipline, mirroring
ADR-027/029/030/031/033 § Enforcement verbatim):** `rules_file::parse_rules_file`/
`decompose` are pure, deterministic CPU computation over an in-memory
`&str` — the identical "no environment can lie to a pure function"
reasoning already established applies unmodified. The new admin handler's
only I/O is the existing, already-probed `SystemDb` connection pool, via
calls to `upsert_access_rule`/`upsert_write_access_rule` that are
byte-for-byte unmodified. The substrate this feature adds new reliance on
is exactly zero.

`cargo-deny`/`deny.toml` unaffected — no new workspace dependency.
`embyr_core::access_control`'s existing zero-IO enforcement covers the new
submodule unchanged.

## References

- `docs/feature/security-rules-cel-parity/feature-delta.md` § Job Discovery
  Framing Resolution (Resolutions 1-3), § System Constraints, § Handoff
  Package (flags 1-8), § User Stories (US-01 through US-06).
- `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md`,
  `adr-028-access-rule-storage-and-lifecycle.md`,
  `adr-029-access-control-composition-and-bounded-context.md`,
  `adr-030-write-path-grammar-storage-and-composition.md`,
  `adr-031-query-shape-compliance-check.md`,
  `adr-033-listen-compliance-composition-and-collection-scoping.md`,
  `adr-034-custom-claims-representation-and-grammar-extension.md`,
  `adr-035-access-rule-history-storage-and-capture-mechanism.md` — the
  machinery this ADR extends, not replaces.
- `crates/embyr-core/src/access_control/mod.rs` (full, read during DESIGN) —
  exact current `Operand`/`evaluate`/`word_to_operand`/`decompose_decidable`
  shapes confirmed before designing this extension.
- `crates/embyr-server/src/admin/handlers/access_rules.rs` (targeted, read
  during DESIGN) — exact current `DefineAccessRuleBody`/`define_access_rule`/
  `SimulateAccessRuleBody`/`simulate_access_rule` shapes, the direct
  structural precedent for the new import handler and the extended
  simulation body.
- `crates/embyr-server/src/realtime/listen_handler.rs` (via ADR-033) — exact
  current `evaluate()` call shape at the 2 per-event call sites this ADR's §
  Decision — Listen's Per-Event Re-Check touches mechanically.
- `crates/embyr-core/src/domain/document.rs` — `DocumentPath { collection_path,
  document_id }`, the source of the value threaded into `evaluate()`'s new
  `path_variable_value` parameter at all 4 real-enforcement call sites.
