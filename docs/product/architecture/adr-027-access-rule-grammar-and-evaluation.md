# ADR-027: Access-Rule Expression Grammar and Evaluation Mechanism

## Status

Accepted

## Context

`security-rules` (Epic 2a) gives Alex a way to define, per collection, a boolean
condition that gates `GetDocument` reads by identity (`request.auth`, sourced from
`client-auth`'s `VerifiedEndUserIdentity`) and document content (`resource.data`).
DISCUSS (`docs/feature/security-rules/feature-delta.md` § Job Discovery Framing
Resolution, Resolution 1) already locked the expressiveness ceiling: comparison
(`==`, `!=`) and boolean combinators (`&&`, `||`, `!`) over `request.auth` (`null`
or an object exposing `.uid`) and `resource.data.<field>`, plus literal `true`/
`false`. Explicitly excluded: cross-document reads (`get()`/`exists()`), custom
functions, wildcard/recursive path matching, custom claims. This ADR is scoped to
**how** that locked grammar is parsed and evaluated — it does not reopen Resolution
1's scope.

DISCUSS's own Walking Skeleton Evaluation (feature-delta.md, Decision 2) confirms:
"no existing mechanism evaluates a boolean condition over identity + document data
— that computation does not exist anywhere in the codebase today." This is
confirmed CREATE NEW territory (see § Reuse Analysis, feature-delta.md DESIGN
section).

## Decision Drivers

1. **Do not silently widen the locked grammar** (§ Handoff Package flag 1). Any
   parser/evaluator technology choice that makes it *easy* to accidentally support
   more than the locked grammar (e.g., a general-purpose expression-language
   library) is a structural risk to this constraint holding over time.
2. **Fail-closed on missing document field, never crash** (AC-17-09). A condition
   referencing an absent field must deny, never panic or return an internal error.
3. **Zero IO** — `embyr-core` must never import `tokio`/`sqlx`/`tonic`/`axum`
   (workspace-wide, `deny.toml`). Parsing and evaluation are pure, CPU-only
   transformations over already-fetched data.
4. **Simplest solution first** (Principle 8) — the grammar is small and closed by
   design (Resolution 1 explicitly rejected Option A's "unbounded scope" precisely
   because full CEL-like parity is a multi-week undertaking with a materially
   different risk profile). A parser sized to match a small, closed grammar is the
   correct default; a general-purpose parser-generator sized for an open-ended
   grammar is over-engineering for a grammar that is deliberately capped.
5. **Shared-artifact integrity** (§ Handoff Package flag 8) — simulation (US-05)
   must invoke the exact same evaluation routine as real enforcement (US-02/03),
   not a second, independently-maintained copy.
6. **Distinguishable rejection reasons at define-time** (AC-17-03 vs. AC-17-04) — a
   condition using an out-of-grammar construct (e.g., `get()`) must be
   distinguishable from a condition with plain invalid syntax (e.g., unbalanced
   parentheses).

## Considered Options

### Option A: Adopt a general-purpose expression/parser-generator crate (e.g., `pest`, `nom`, or a CEL-in-Rust crate)

A parser-combinator or grammar-file-driven library builds the AST from a formal
grammar definition.

**Rejected.** The locked grammar (Resolution 1, Option C) has exactly two
production families (boolean combinators, equality comparison) over four operand
shapes (`request.auth.uid`, `request.auth` compared to `null`, `resource.data.
<field>`, `true`/`false`). A parser-generator's entire value proposition —
expressive grammars that are easy to extend — is a liability here, not an asset:
extending the `.pest` grammar file to add `get()`/`exists()` support later would be
a *one-line grammar change*, which is exactly the "silently widen the locked
grammar" risk Decision Driver 1 flags. It also adds a new workspace dependency
(violating Principle 8's simplicity default) for a grammar too small to need one.
This is a case where "resume-driven development" (adopting mature tooling because
it exists) would be the wrong call — the grammar's smallness is a deliberate,
evidence-backed constraint (Resolution 1), not a temporary limitation to engineer
around in advance.

### Option B: A fixed enum of named condition "shapes" evaluated by a match statement (no parsing at all)

Mirrors Resolution 1's rejected Option B (fixed enum of rule shapes) at the
grammar-mechanism level: Alex would submit a `ConditionShape` enum tag
(`OwnerEquals("owner_id")`, `AuthRequired`, `PublicRead`, `DenyAll`) instead of free
text.

**Rejected — already rejected at the DISCUSS layer, and rejecting it again here
would contradict a locked decision.** Resolution 1 explicitly rejected the
enum-of-shapes approach because Alex is migrating an existing Firebase app with a
real `firestore.rules` file expressed as boolean conditions; forcing a
re-expression into a fixed enum is friction DISCUSS already ruled out. DESIGN's job
is to implement Option C, not re-litigate Option B.

### Option C: Hand-rolled recursive-descent parser + typed AST + pure evaluation function — Accepted

A small, hand-written recursive-descent parser (~2 production families, no
generator, no new dependency) builds a typed `Condition` AST. A separate pure
function evaluates the AST against an `EvaluationContext` (optional auth + resource
field map).

**Accepted.** Matches the grammar's actual size (small, closed, locked). Adds zero
new workspace dependencies — `embyr-core::access_control` uses only `std` and the
existing `embyr_core::domain::field_value::FieldValue` type, consistent with
`embyr-core::client_identity`'s existing zero-new-crypto-dependency shape (ADR-024
reused `jsonwebtoken`, already present; this ADR needs no new crate at all). The
grammar is structurally capped at exactly what the parser's match arms implement —
widening it later requires a deliberate code change (new AST variant, new parser
branch, new evaluator arm), not a one-line grammar-file edit, which is the
structural defense Decision Driver 1 requires.

## Decision

### Grammar (locked scope, EBNF-shaped for implementation reference only — not a widening)

```
condition   := or_expr
or_expr     := and_expr ( '||' and_expr )*
and_expr    := unary ( '&&' unary )*
unary       := '!' unary | primary
primary     := comparison | 'true' | 'false' | '(' condition ')'
comparison  := operand ( '==' | '!=' ) operand
operand     := 'request.auth.uid' | 'request.auth' | 'null' | 'resource.data.' IDENT
```

### Types (`embyr-core::access_control`, new module, zero IO)

- `Condition` — AST: `Literal(bool)`, `Compare(Operand, CompareOp, Operand)`,
  `And(Box<Condition>, Box<Condition>)`, `Or(Box<Condition>, Box<Condition>)`,
  `Not(Box<Condition>)`.
- `Operand` — `AuthUid`, `AuthNullSentinel`, `ResourceField(String)`,
  `BoolLiteral(bool)`, `NullLiteral`.
- `CompareOp` — `Eq`, `Ne`.
- `AuthContext { uid: String }` — the *only* shape `request.auth` presents to the
  evaluator. Constructed 1:1 from `VerifiedEndUserIdentity.end_user_id` at the call
  site (see ADR-029) — this module never constructs its own identity, it only
  consumes an already-verified one.
- `EvaluationOutcome` — `Allow | Deny` (no third state; the function is total).
- `ConditionParseError` — `SyntaxError { detail: String }` |
  `UnsupportedConstruct { construct: UnsupportedConstruct, detail: String }`, where
  `UnsupportedConstruct` is `CrossDocumentRead | CustomFunction | WildcardPath`.
  This is the AC-17-03/AC-17-04 distinguishability mechanism: the tokenizer/parser
  specifically recognizes `identifier(` call-shaped syntax and `{...}`/`**`
  wildcard-path shapes as *named* out-of-grammar constructs, distinct from generic
  token/parse failures (unbalanced parens, unrecognized operator), which fall
  through to `SyntaxError`.

### Functions (the two shared entry points — see ADR-029 for call sites)

```
fn parse_condition(source: &str) -> Result<Condition, ConditionParseError>
fn evaluate(
    condition: &Condition,
    auth: Option<&AuthContext>,
    resource_fields: &BTreeMap<String, FieldValue>,
) -> EvaluationOutcome
```

`evaluate` is infallible and total by construction — it never returns an error, it
returns `Deny` for every case that would otherwise be an error (see Fail-Closed
Semantics below). This is what "never crashes" (AC-17-09) means structurally: there
is no `Result::Err` branch for a caller to forget to handle, because there is no
`Result` in the return type.

### Fail-Closed Semantics on Missing Field (AC-17-09)

A field reference (`resource.data.<field>`) that is absent from `resource_fields`
is a **top-level evaluation short-circuit to `Deny`**, not a per-comparison
special case. Concretely: field lookup is internally `Result<&FieldValue,
FieldMissing>`; the *first* `FieldMissing` encountered anywhere in the condition
tree collapses the entire evaluation to `Deny`, regardless of `&&`/`||`/`!`
structure.

This is a deliberate simplification over per-operator null-propagation semantics
(which real CEL-like languages define precisely, and which real Firestore Rules
also defines precisely for its `resource`/`request.resource` distinction) — DISCUSS's
locked grammar does not specify per-operator null semantics, and inventing them
would expand the test matrix (what does `resource.data.missing != resource.data.
owner_id` mean? real CEL says "error," this design says "deny" uniformly) for zero
evidenced need. Every domain example in DISCUSS only exercises the "the whole
condition should deny" case (US-02 Domain Example 3, AC-17-09). If a future epic
needs finer-grained null semantics, that is new evidence, not a gap in this design.

### Comparison Semantics (which operand pairings are meaningful)

The parser accepts any `operand == operand` / `operand != operand` syntactically
(matching the grammar above); the evaluator defines semantics only for the
pairings DISCUSS's domain examples exercise:

- `AuthUid == ResourceField(_)` / `!=` — string equality against `VerifiedEndUserIdentity.end_user_id`; `Deny` (via fail-closed) if `auth` is `None` (no uid to compare) or the field is missing.
- `AuthUid == NullLiteral` / `AuthNullSentinel == NullLiteral` and the `!=` forms — the `request.auth == null` / `request.auth != null` idiom (AC-17-11/12).
- `ResourceField(_) == BoolLiteral(_)` / `!=` — for rules that gate on a boolean document field (not in DISCUSS's concrete examples, but syntactically reachable within the locked grammar and semantically unambiguous).
- Any other pairing accepted by the grammar but not semantically meaningful (e.g., comparing two `ResourceField`s) evaluates via ordinary `FieldValue` equality — no special-casing needed, since `embyr_core::domain::field_value::FieldValue` already implements `PartialEq`.

### Grammar Gap Flagged for DISTILL (not silently resolved)

The locked grammar (Resolution 1, Option C) admits only `true`/`false` as literal
operands — **not** arbitrary string/number literals. A rule such as
`resource.data.status == "published"` is therefore **not expressible in v1** under
a literal reading of DISCUSS's own grammar text. None of the 5 user stories'
domain examples require a string/number literal comparand (only uid-equality,
null-checks, and the bare `true` literal are exercised). This is flagged as
**OQ-SR-04** (see feature-delta.md § Open Questions) rather than silently widened
(which would violate Decision Driver 1) or silently narrowed further — DISTILL
should confirm this reading matches its acceptance-scenario expectations before
DELIVER locks the parser's literal-operand support to booleans only.

## Consequences

### Positive

- Zero new workspace dependency. `embyr-core::access_control` has an identical
  zero-IO shape to `embyr-core::client_identity` (ADR-024) — a proven pattern in
  this codebase, not a new one.
- The grammar cannot silently grow: adding `get()` support later requires a new
  AST variant, a new parser branch, and a new evaluator arm — a deliberate,
  reviewable code change, not a one-line grammar-file edit.
- `evaluate()`'s totality (no `Result`, no panic) makes AC-17-09's "never crashes"
  claim a type-level guarantee, not a tested convention.
- One evaluation function serves three call sites (real enforcement × 2 outcomes,
  simulation × 1) — see ADR-029 — eliminating the drift risk DISCUSS flagged as
  HIGH.

### Negative / Trade-offs

- Hand-rolled parsers require more hand-written test coverage of edge cases
  (operator precedence, parenthesization) than a grammar-file-driven parser would;
  mitigated by the grammar's small size (2 production families) and mutation
  testing (project's `per-feature` strategy, CLAUDE.md).
- The literal-operand gap (booleans only, no strings) may prove too restrictive
  once real Trailmark-style rules beyond ownership-equality are attempted — flagged
  as OQ-SR-04, not silently resolved either direction.

## Enforcement

Style: Hexagonal (ports-and-adapters), unchanged. Language: Rust.
`embyr-core::access_control` has zero IO imports — enforced by the existing
`cargo-deny`/`deny.toml` rule already covering all of `embyr-core` (no new
crate-specific configuration needed, since this is a submodule, not a new crate).
No new adapter, no new `probe()` — see ADR-029 § Driven Ports for the explicit
Earned Trust reasoning (no new substrate dependency is introduced by this ADR).

## References

- `docs/feature/security-rules/feature-delta.md` § Job Discovery Framing
  Resolution (Resolution 1), § System Constraints, § Handoff Package flags 1, 8.
- `docs/product/architecture/adr-024-client-identity-verification-mechanism.md` —
  the zero-IO pure-module shape this ADR's module structure mirrors.
- `crates/embyr-core/src/client_identity/mod.rs` — direct structural precedent.
- `crates/embyr-core/src/domain/field_value.rs` — `FieldValue` type reused
  unchanged as the resource-field value representation.

## Changed Assumptions (appended by feature `custom-claims`, DESIGN wave, 2026-08-27)

**Original assumption #1, quoted verbatim (Context, above):**

> Explicitly excluded: cross-document reads (`get()`/`exists()`), custom
> functions, wildcard/recursive path matching, custom claims.

**Original assumption #2, quoted verbatim (§ Grammar Gap Flagged for DISTILL,
above):**

> The locked grammar (Resolution 1, Option C) admits only `true`/`false` as
> literal operands — **not** arbitrary string/number literals... This is
> flagged as **OQ-SR-04**... DISTILL should confirm this reading matches its
> acceptance-scenario expectations before DELIVER locks the parser's
> literal-operand support to booleans only.

**Original assumption #3, quoted verbatim (§ Comparison Semantics, above) —
CORRECTED, not merely extended, by this amendment:**

> `ResourceField(_) == BoolLiteral(_)` / `!=` — for rules that gate on a
> boolean document field (not in DISCUSS's concrete examples, but
> syntactically reachable within the locked grammar and semantically
> unambiguous).

**Why this is being appended, not reopened:** custom claims were explicitly out
of this ADR's own locked scope (assumption #1) — `security-rules`' own
Out-of-Scope deferral named this exact gap as a future, cross-epic concern, not
a defect in this ADR. OQ-SR-04 (assumption #2) was deliberately flagged, not
silently resolved either direction — this amendment is the resolution DISCUSS's
own flag anticipated. Assumption #3, however, was **not accurate as written**:
direct code verification during `custom-claims`' own DESIGN pass found
`word_to_operand()` has no `"true"`/`"false"` match arm at all — `BoolLiteral`
was NOT, in fact, syntactically reachable from `parse_comparison` before this
feature's own fix. This amendment corrects that claim rather than perpetuating
it.

**New assumptions**:
1. `Operand` gains `AuthTokenClaim(String)` (parsed via a new
   `"request.auth.token."`-prefix branch in `word_to_operand()`, mirroring
   `RequestResourceField`'s own precedent, ADR-030) and `StringLiteral(String)`
   (Release 2, US-06 — requires a genuinely new tokenizer branch, the first
   quote-character handling this tokenizer has had).
2. `AuthContext` gains `claims: BTreeMap<String, FieldValue>`.
3. `word_to_operand()` gains `"true" => Ok(Operand::BoolLiteral(true))` /
   `"false" => Ok(Operand::BoolLiteral(false))` — a zero-regression fix making
   `BoolLiteral` genuinely reachable as a comparison operand for the first
   time, required for `custom-claims`' own `request.auth.token.is_moderator ==
   true` walking-skeleton domain example to parse at all.

Full decision, verified-zero-regression argument, and the corresponding
`client-auth`-side claims-representation extension:
`docs/product/architecture/adr-034-custom-claims-representation-and-grammar-extension.md`
§ Decision — Grammar Extension.

**Reference**: `docs/feature/custom-claims/feature-delta.md` § Job Discovery
Framing Resolution (Resolutions 2–3).
