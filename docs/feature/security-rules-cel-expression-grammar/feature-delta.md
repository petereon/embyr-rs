# Feature Delta: security-rules-cel-expression-grammar

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml` — JOB-17 (`document-access-control`) read in full, including every
NOTE appended by 4a (`security-rules-cel-parity`) and 4b′ (`security-rules-cel-recursive-wildcards`).
Same job, same persona (P1 Alex) — no new job created.
✓ `docs/feature/security-rules-cel-parity/feature-delta.md` — this feature's own naming origin:
§ Scope Assessment "4c — candidate `security-rules-cel-expression-grammar`" and § Out of Scope,
both cite the identical scope: "arithmetic operators, `in`, list/map literals, numeric literals
(`OQ-SR-04`, still open after 3 prior epics), timestamp/duration types."
✓ `docs/feature/security-rules-cel-recursive-wildcards/feature-delta.md` § Out of Scope — carries
one additional scope note forward to this feature: real Firestore's `path`-type segment-indexing
(`path[0]`) for referencing a recursive wildcard's own captured remainder was deliberately deferred
here, "a scope note added to 4c's own naming, not built here."
✓ `crates/embyr-core/src/access_control/mod.rs` — direct code read (not assumed from naming) of
the CURRENT grammar: `CompareOp` is `Eq`/`Ne` ONLY (no `<`/`<=`/`>`/`>=` at all); `Operand` has no
numeric-literal, list, map, or timestamp/duration variant; `tokenize()` has no digit-handling arm
at all (a bare numeric literal is currently an undifferentiated `SyntaxError`, not even a named
`UnsupportedConstruct`); `detect_unsupported_construct` names only 3 constructs
(`CrossDocumentRead`, `CustomFunction`, `WildcardPath`) — no numeric/arithmetic-specific rejection
category exists yet.
✓ `crates/embyr-core/src/domain/field_value.rs` — direct code read: `FieldValue` ALREADY has
`Integer(i64)`, `Double(f64)`, `Timestamp(i64, i32)`, `Array(Vec<FieldValue>)`,
`Map(BTreeMap<String, FieldValue>)` — every domain VALUE shape this feature needs already exists
at the storage/wire level. The entire gap is in the condition GRAMMAR (parser + `Operand` +
`evaluate()`'s comparison logic) only — confirms the epic's own naming ("extends `evaluate()`'s
operand/expression tree; still zero I/O") was accurate, not merely asserted.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** (Decision 1) — extends an existing pure-computation grammar/evaluator,
  zero new user-facing surface beyond the admin API's existing import/define/simulate actions.
- JTBD: **reuse JOB-17** (Decision 4 = "Yes", existing job) — this is the 11th realization of the
  SAME job (`document-access-control`), not a new job. A NOTE is appended to `jobs.yaml`'s JOB-17
  entry (§ SSOT Updates), mirroring 4a's/4b's/4b′'s own identical precedent.
- Walking Skeleton: **Yes** (Decision 2) — this codebase already has a real, running composition
  root; the skeleton here is the SMALLEST slice of this feature's own new grammar surface, not a
  from-scratch app skeleton.
- UX Research Depth: **Lightweight** (Decision 3) — a backend grammar extension with one persona
  (Alex, already fully profiled across 5 prior epics), no new emotional arc, no new journey shape
  beyond "the same real `.rules` file Alex already has, now with one more previously-rejected
  clause accepted."

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P1 Alex (SDK Developer), unchanged — see `docs/product/personas/` and every prior
JOB-17 feature-delta.md for the full profile.

**Job**: JOB-17 `document-access-control`, unchanged job_story. This feature's own realization:
Alex's real Firestore `.rules` file references numeric bounds (`resource.data.photo_count <= 20`),
enum/status whitelisting (`request.resource.data.status in ["draft", "published", "archived"]`),
or time-based freshness windows (`request.time < resource.data.created_at + duration.value(24,
'h')`) — today, EVERY one of these is rejected: relational operators don't exist in the grammar at
all, numeric literals fail as an undifferentiated `SyntaxError` at the FIRST digit, `in` and list
literals don't exist, and there is no timestamp/duration operand family. After this feature, Alex's
same file imports and enforces correctly for these idioms.

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

### Resolution 1 (THE central scope question) — Is "the remaining full CEL expression surface" one coherent feature, or does it need its own further Elephant Carpaccio split?

**Answer, confirmed by direct code read, not assumed from the epic's own naming**: the 5
sub-surfaces named by 4a's/4b's own Out-of-Scope entry — arithmetic operators, relational
comparisons (implied by "expression grammar" though not separately named — confirmed missing by
direct `CompareOp` read), `in`, list/map literals, numeric literals, timestamp/duration types — are
NOT independently shippable end-to-end value on their own in most cases: relational comparison is
USELESS without numeric literals to compare against (`resource.data.count <= 20` needs BOTH `<=`
and `20` to parse), and timestamp/duration comparison needs arithmetic (`created_at +
duration.value(24, 'h')`) to express any freshness-window idiom at all — real Firestore's OWN
canonical rate-limiting/expiry examples always compose these together. Splitting each sub-surface
into its OWN feature (mirroring 4d's/4e's own isolation) would produce features with no
independently-testable value, violating the SAME Elephant Carpaccio discipline that justified
splitting 4d (I/O) and 4e (composability-dependent) OUT of this feature in the first place.

**What DOES independently split, confirmed by direct evidence-fit analysis below**: numeric
literals + relational comparisons + arithmetic operators form one coherent, self-contained
capability (numeric bounds validation — the single most common real-Firestore idiom evidenced by
this initiative's own "real customers migrating real `.rules` files" trigger, reapplied). `in` +
list literals form a second, independently valuable capability (enum/status whitelisting) that
needs NONE of the numeric machinery. Timestamp/duration forms a third, which NEEDS arithmetic
(already built by the first) but nothing from the second — a genuine dependency order, not an
arbitrary one.

**Resolution**: **one feature, 3 releases, ordered by genuine dependency** (not 3 separate
features) — mirrors 4a's/4b's/4b′'s own identical "one feature, multiple releases" shape exactly.
Map literals (construction, e.g. `{"role": "admin"}` as a condition-side operand) and nested
map-field traversal (`resource.data.address.city`, a MAP-VALUED field's own nested key) are
BOTH deliberately narrowed OUT of this feature's own locked scope — see Resolution 2.

### Resolution 2 — Do "list/map literals" (the epic's own naming) both belong in this feature's locked scope?

The epic's own naming bundles "list/map literals" as one phrase, inherited unchanged from 4a's
original Out-of-Scope entry. Direct evidence-fit check, not assumed:

| Construct | Real-world evidenced need | Fit |
|---|---|---|
| **List literals** (`["draft", "published", "archived"]`) as the RHS of an `in` check | Canonical, extremely common real Firestore idiom — status/role/category whitelisting. Directly evidenced by this initiative's own "real customers migrating real `.rules` files" trigger (the SAME trigger that justified 4a's own charter reversal) | **In scope** |
| **Map literals** (`{"role": "admin", "level": 3}`) as a condition-side operand, compared or constructed inline | No domain example in this feature's own text, in any of the 5 prior JOB-17 features' own accumulated text, or in `OQ-SR-04`'s own original framing requires constructing a literal map INSIDE a condition — real Firestore rules overwhelmingly READ from existing map-valued fields (`resource.data.settings.theme`), never construct one to compare against | **Rejected for this feature** — zero domain evidence, would repeat the unevidenced-scope mistake this initiative has consistently avoided (Principle 8, reapplied) |
| **Nested map-field traversal** (`resource.data.address.city` — a MAP-VALUED field's own nested key, not a literal) | Real and evidenced in principle, but a STRUCTURALLY DIFFERENT gap (operand resolution against `FieldValue::Map`'s own nested structure, not literal-construction syntax) from what "list/map literals" actually names. Today's tokenizer already accepts dotted `Word`s; `word_to_operand`'s `resource.data.` arm treats the WHOLE remainder as one flat field name — silently wrong (looks up a field literally named `"address.city"`) rather than loudly rejected, an existing latent gap this feature did not introduce | **Named, deferred, NOT silently fixed as a "while we're here" addition** — flagged as `OQ-CEG-01` (§ Open Questions) for a future slice/feature; fixing it correctly needs its own DISCUSS (does `.` in a resource-field position mean nested-map-traversal or a literal dotted key? Real Firestore's answer is unambiguous — nested traversal — but embyr's own existing `resource.data.<field>` grammar production has never disambiguated this, and changing it retroactively could change behavior for any rule already using a literal dotted key, however unlikely) |

**Resolution**: **"list literals" is locked in scope (Release 2, this feature). "Map literals" and
nested map-field traversal are BOTH out of scope** — the epic's own inherited naming
("list/map literals") is narrowed here with direct justification, not silently trimmed. Named,
not silently dropped (§ Out of Scope).

### Resolution 3 — Timestamp/duration: a new `FieldValue`-level type, or reuse what already exists?

Direct code read (`field_value.rs`): `FieldValue::Timestamp(i64, i32)` (seconds + subsecond nanos)
ALREADY exists — used today for document metadata (`create_time`/`update_time`, confirmed via
`domain/document.rs`), just never exposed to the condition grammar. Real Firestore's own
`request.time` (the request's own server timestamp) and a document field of Firestore's native
`timestamp` type both map cleanly onto this existing representation — no new domain type needed.

Real Firestore's `duration` is NOT a stored value type at all — it exists only as a `duration.
value(amount, unit)` CONSTRUCTOR function whose result can be ADDED to or SUBTRACTED from a
timestamp (`resource.data.created_at + duration.value(24, 'h')`), never stored, compared, or
returned on its own. Confirmed by direct analogy to `detect_unsupported_construct`'s own existing
call-syntax scan (identifier immediately followed by `(` — the SAME shape `get()`/`exists()`
already use) — `duration.value(...)` fits the identical call-syntax shape, requiring no new
tokenizer branch beyond what already exists for named-call detection, just a NEW recognized name
(`duration.value`) instead of an automatic `CustomFunction` rejection.

**Resolution**: **`request.time` resolves to a NEW `Operand::RequestTime` variant, evaluated as
`FieldValue::Timestamp` from a caller-supplied "now" value (threaded exactly like every other
already-established call-site value — ADR-062's/ADR-063's own "value already known at the call
site, zero new I/O" precedent, reapplied)`. `duration.value(N, unit)` is parsed as a NEW,
narrowly-scoped arithmetic-RHS-only construct** (never a standalone operand — it can only ever
appear as the right-hand operand of a NEW `+`/`-` arithmetic operator against a timestamp-typed
left-hand operand), not a general-purpose function-call grammar (Resolution 4 locks arithmetic
scope narrowly for the identical "don't build unevidenced generality" reason 4e was isolated as
its own, later, composable-functions epic).

### Resolution 4 — Arithmetic operators: full expression-tree generality, or scoped to what the evidenced idioms actually need?

Real Firestore's own arithmetic surface is `+`/`-`/`*`/`/`/`%`, usable in any expression position
(deeply nestable, e.g. `(a + b) * c`). This feature's own evidenced domain examples need only:
(a) numeric comparison against a literal or another numeric field (`resource.data.count <= 20`,
no arithmetic at all — pure relational), and (b) ONE arithmetic composition shape, timestamp +
duration, for freshness-window checks (Resolution 3).

| Option | Description | Fit against evidence |
|---|---|---|
| **(A) Full arithmetic expression tree — `+`/`-`/`*`/`/`/`%`, any nesting, any numeric operand pairing** | Complete real-Firestore parity for this sub-surface | **Rejected for this feature's own locked scope** — no domain example anywhere in this initiative's accumulated text needs multiplication, division, modulo, or nested arithmetic (`(a + b) * c`); building full generality with zero evidence repeats the exact mistake this initiative has consistently avoided |
| **(B) `+`/`-` only, between two numeric operands OR a timestamp and a duration constructor, non-nested (one operator per comparison side, never `a + b + c` or `(a+b)*c`)** | Directly covers every evidenced idiom: numeric bound checks need no arithmetic at all (pure comparison); the ONE evidenced arithmetic idiom (timestamp freshness windows) needs exactly one `+`/`-` between a field/literal and a duration | **Strongest fit.** A strict, evidenced subset — `*`/`/`/`%` and nested arithmetic are named, deferred, not silently built |

**Resolution**: **(B) is locked.** `+` and `-` only, exactly one operator per comparison operand
(no nested arithmetic expressions), valid operand pairings: `Integer +/- Integer`, `Double +/-
Double`, `Timestamp +/- Duration` (Duration only ever from `duration.value(...)`, never stored or
compared standalone). `*`, `/`, `%`, and any nested/nested arithmetic are named, deferred (§ Out of
Scope) — zero evidence, and each is independently addable later without touching this feature's own
locked grammar shape (an `Operand::Arithmetic` AST node, not a special-cased pairwise combinator,
so deepening it later is additive, confirmed by design in § Architecture Design).

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? Borderline (9, see § User Stories — under threshold). >3
bounded contexts/modules? No — entirely within BC-4 Access Control, `embyr-core` only for
grammar/evaluation, `embyr-server` only for the 2 mechanical admin-response wiring points
(rejection-reason vocabulary needs no NEW field, `ConditionParseError`'s existing shape already
carries a `detail` string). Walking skeleton >5 integration points? No — 1 (parser + evaluator,
zero I/O, zero new call sites beyond what `parse_condition`/`evaluate` already have). Estimated
effort >2 weeks? No — 3 releases, each ≤2 days by slice estimate (§ Elephant Carpaccio Slices).
Multiple independent user outcomes that could ship separately? Partially — Resolution 1 already
addressed this by locking a 3-release single feature over a 3-feature split, with an explicit,
evidenced dependency-order justification (arithmetic is a genuine prerequisite for the
timestamp/duration release, not an arbitrary bucketing choice).

**Scope Assessment: PASS** (0-1 signals fired, both addressed above) — right-sized as one feature.

## Wave: DISCUSS / [REF] Journey — Alex's "The File I Already Have Now Just Works" Arc

### Mental model

Alex has a REAL, already-written `firestore.rules` file from his existing Firebase app (the SAME
premise every prior JOB-17 CEL epic starts from). He is not learning a new authorization model —
he is discovering, clause by clause across 4 prior epics, which parts of his OWN already-correct
file embyr now honors. This feature closes the single largest remaining category: any clause
involving a number, a whitelist, or a time window.

### Import flow (extends 4a's/4b's/4b′'s own identical import arc)

1. Alex re-imports the SAME `firestore.rules` file he's tried at every prior epic's own finalize
   (habit: he keeps a running mental checklist of "which of my clauses does embyr honor now").
2. Before this feature: any clause containing `<=`, `<`, `>=`, `>`, a bare number, `in [...]`, or
   `request.time`/`duration.value(...)` is rejected — either as an undifferentiated `SyntaxError`
   (numeric literals — the WORST case, indistinguishable from an actual typo) or simply never
   parses past the first offending token.
3. After this feature: those clauses import and enforce correctly. A clause using a construct
   still out of scope (map literals, nested map-field traversal, `*`/`/`/`%`, multiplication-style
   duration composition) is STILL rejected, but with the SAME distinguishable-reason discipline
   every prior epic established (never a bare `SyntaxError` for a recognized-but-unsupported shape).

### Failure modes (feeds DISTILL scenario generation)

- A numeric literal appears in a position the tokenizer has never had a digit-handling branch for
  at all — before this feature, ANY digit anywhere in a condition is an immediate, generic
  `SyntaxError`, indistinguishable from a real typo. This feature must specifically prove numeric
  literals now parse successfully, not merely that SOME numbers work.
- Real Firestore's own `duration.value(24, 'h')` unit vocabulary (`s`, `m`, `h`, `d` at minimum) —
  an unrecognized unit string must be rejected distinguishably, not silently misinterpreted as
  seconds.
- `in` against an operand that is NOT a list literal (e.g. `request.auth.uid in resource.data`,
  membership against a map) is a DIFFERENT real Firestore idiom (map key membership) this feature
  does NOT build (Resolution 2) — must be rejected as a named unsupported shape, not silently
  misevaluated as always-false or crash.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Alex writes/imports a `.rules` file containing a numeric-bound, whitelist, or time-window clause
→ embyr parses it → embyr evaluates it correctly on a real request → (Release 2/3) embyr does the
same for `in`/list and timestamp/duration clauses → Alex simulates any of these before publishing.

### Walking Skeleton

**Release 1, Slice 01**: a single numeric-literal comparison (`resource.data.photo_count <= 20`)
imports, and a real `GetDocument` request is allowed/denied correctly based on it. Smallest
possible end-to-end proof that the numeric-literal + relational-comparison grammar extension is
real, not merely parsed-and-discarded.

### Release 1 — Numeric Bounds Validation Works End-to-End (Slices 01–03)

Numeric literals (`Integer`/`Double`), relational comparison operators (`<`, `<=`, `>`, `>=`),
read+write+simulation parity — mirrors every prior epic's own "parity within the same feature"
precedent (Resolution established once per epic, reapplied here without re-litigating).

### Release 2 — Whitelist/Enum Validation Works End-to-End (Slices 04–05)

`in` operator + list literals, read+write+simulation parity.

### Release 3 — Time-Window Validation Works End-to-End (Slices 06–07)

`request.time`, `duration.value(...)`, the scoped `+`/`-` arithmetic operator (Resolution 4),
read+write+simulation parity. Depends on Release 1's own numeric-literal tokenizer work (duration
amounts are themselves numeric literals) but nothing from Release 2.

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 | 1 (Walking Skeleton) | 1 day | Disproves: a numeric-literal + relational-comparison grammar extension cannot share the existing `Operand`/`Condition`/tokenizer architecture without either a second, parallel parser or a disruptive rewrite of the existing string/bool-literal handling | Mirrors 4a's own Slice 01 (first grammar-widening slice of a CEL-parity epic) |
| 02 | US-02 | 1 | 1 day | Disproves: write-path parity for numeric comparisons cannot reuse `evaluate()`'s existing `request_resource_fields` parameter unchanged | Mirrors every prior epic's own write-parity slice |
| 03 | US-03 | 1 | 1 day | Disproves: simulation cannot share the identical numeric-comparison evaluation path real enforcement uses without a second, independently-maintained numeric-comparison routine | Mirrors 4a's own US-05 (simulation-shares-evaluation precedent) |
| 04 | US-04 | 2 | 1 day | Disproves: `in` + list-literal support cannot be expressed as ONE new `Operand::ListLiteral` + ONE new `CompareOp`-adjacent membership check without a second, parallel comparison mechanism | New primitive, no direct predecessor within this initiative — nearest reference class: `AuthTokenClaim`'s own "one new operand family" precedent (4a's custom-claims epic) |
| 05 | US-05 | 2 | 0.5 day | Disproves: `in`'s own write-path + simulation parity needs anything beyond the identical mechanical extension US-02/03 already proved for numeric comparisons | Mirrors Slices 02/03 |
| 06 | US-06 | 3 | 1.5 day | Disproves: `request.time` + `duration.value(...)` + scoped `+`/`-` arithmetic cannot be expressed as an `Operand::RequestTime` + a narrowly-scoped `Operand::Arithmetic` AST node without either a general-purpose expression-evaluator rewrite or silently unbounded arithmetic generality | New primitive family, nearest reference class: 4b′'s own `bind_recursive_prefix` (a genuinely new primitive, built once, reused at every call site) |
| 07 | US-07 | 3 | 0.5 day | Disproves: timestamp/duration's own write-path + simulation parity needs anything beyond the identical mechanical extension already proved twice | Mirrors Slices 02/03, 05 |

## Wave: DISCUSS / [REF] Prioritization

Ordered by learning leverage (highest-uncertainty first) AND genuine dependency (Resolution 1):
Slice 01 first (proves the core "new literal/operator kind fits the existing AST shape" question
with the LOWEST-complexity new construct); Slices 02-03 mechanically extend it (low uncertainty,
proven pattern from every prior epic); Slice 04 (the FIRST genuinely new operand family, `in`
+ list) carries the next-highest uncertainty and unblocks the highest-evidenced whitelist idiom;
Slices 06-07 (timestamp/duration) come last both because they carry the highest complexity (a new
operand family AND a new, narrowly-scoped arithmetic AST node) and because they structurally depend
on Slice 01's own numeric-literal tokenizer work (duration amounts are numeric literals).

## Wave: DISCUSS / [REF] System Constraints

- `embyr-core` remains zero-IO (`deny.toml`-enforced) — every new primitive (`Operand::IntLiteral`,
  `Operand::DoubleLiteral`, `Operand::ListLiteral`, `Operand::RequestTime`, `Operand::Arithmetic`)
  is pure data + pure computation, identical to every existing `Operand` variant.
- `request.time` needs a caller-supplied "current time" value threaded into `evaluate()` — mirrors
  `path_variable_value`'s own "value already known at the call site" precedent (ADR-062) exactly;
  every real call site already computes `chrono::Utc::now()` or equivalent for other purposes
  (e.g. `created_at`/`updated_at` stamping) — zero new I/O, confirmed by direct grep of existing
  `Utc::now()` call sites in `grpc/handler.rs`.
- `ConditionParseError::UnsupportedConstruct`'s existing 3-variant `UnsupportedConstruct` enum
  gains at least 1 new named variant (map literals / nested-field traversal — Resolution 2) for
  the SAME distinguishability discipline (AC-17-03) every prior epic maintained — never a bare
  `SyntaxError` for a recognized-but-out-of-scope shape.
- Mutation-testing lesson from 4b′'s own QUALITY_GATE (`docs/evolution/2026-09-04-security-rules-
  cel-recursive-wildcards.md` § Lessons Learned): this feature's own new logic lives entirely in
  `embyr-core` (pure, zero-IO) — the SAME fast, `--in-place`, module-filtered mutation-testing
  approach that worked cleanly there applies directly here, with no Docker/testcontainers exposure
  at all for the grammar/evaluator work itself.

## Wave: DISCUSS / [REF] User Stories

### US-01: Alex's `.rules` File's Numeric-Bound Clause Now Parses and Enforces Correctly (Walking Skeleton)

**job_id**: JOB-17 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `allow read: if resource.data.photo_count <= 20;` is rejected — `<=` doesn't exist as a
grammar token at all, and even if it did, `20` is an immediate `SyntaxError` at the first digit.
After: Alex imports that exact clause via `POST /admin/v1/projects/:project_id/access_rules`,
sees a 200 with the condition stored verbatim, then a real `GetDocument` call against a document
whose `photo_count` field satisfies (or doesn't satisfy) the bound is allowed (or denied)
correctly.
Decision enabled: Alex confirms his real numeric-bound validation rule now behaves identically to
how it behaved under real Firebase, before any real end user hits it.

#### Acceptance Criteria
- [ ] AC-CEG-01: `resource.data.<field> <= <integer literal>` (and `<`, `>`, `>=`) parses into a
      new `Condition::Compare` variant carrying the new relational operator.
- [ ] AC-CEG-02: a bare integer literal (e.g. `20`, `0`, a negative literal `-5`) parses into a new
      `Operand::IntLiteral(i64)` — distinguishable from the pre-existing `SyntaxError` catch-all a
      digit currently always hits.
- [ ] AC-CEG-03: a real `GetDocument` request against a document whose numeric field satisfies the
      bound succeeds; one whose field does not satisfy it is denied `PermissionDenied`.
- [ ] AC-CEG-04: a `Double` (floating-point) literal (e.g. `19.5`) parses and compares correctly
      against a `FieldValue::Double` resource field, using the SAME relational operators.
- [ ] AC-CEG-05: comparing a numeric operand against a non-numeric resource field (type mismatch —
      e.g. `resource.data.name <= 20` where `name` is a string) is a well-defined `Deny`, never a
      panic — mirrors `evaluate()`'s existing total/infallible-by-construction guarantee.

### US-02: The Same Numeric-Comparison Grammar Gates Writes

**job_id**: JOB-17 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: a write-path numeric-bound rule (`allow write: if request.resource.data.cost_usd >= 0;`)
is rejected identically to the read-path case.
After: Alex imports that clause, and a real `CreateDocument`/`UpdateDocument` call proposing a
document that violates the bound is denied; one that satisfies it succeeds.
Decision enabled: Alex confirms write-side numeric validation (the single most common real
Firestore write-rule idiom — non-negative amounts, max-length counters, bounded ratings) works
before it reaches a real end user's write.

#### Acceptance Criteria
- [ ] AC-CEG-06: `request.resource.data.<field>` participates in the new relational-comparison
      grammar identically to `resource.data.<field>` (US-01) — reuses the existing
      `RequestResourceField` operand family, only the comparison OPERATOR set widens.
- [ ] AC-CEG-07: a real `CreateDocument`/`UpdateDocument` call proposing a document violating the
      bound is denied; one satisfying it succeeds — real I/O, not merely parsed.

### US-03: Alex Simulates a Numeric-Bound Candidate Rule Before Publishing It

**job_id**: JOB-17 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: no way to test a numeric-bound candidate condition without publishing it live.
After: Alex calls the existing `simulate_access_rule` action with a candidate numeric-comparison
condition and a synthetic resource, sees the resolved allow/deny outcome.
Decision enabled: Alex catches an off-by-one or wrong-direction comparison (`<=` vs `>=`) during
his own testing.

#### Acceptance Criteria
- [ ] AC-CEG-08: `simulate_access_rule` evaluates a numeric-comparison candidate condition against
      a synthetic resource via the IDENTICAL `evaluate()` routine real enforcement uses — never a
      second, independently-maintained numeric-comparison implementation.

### US-04: Alex's `.rules` File's Whitelist Clause Now Parses and Enforces Correctly

**job_id**: JOB-17 | **Release**: 2 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `allow write: if request.resource.data.status in ["draft", "published", "archived"];` is
rejected — `in` and list-literal syntax (`[...]`) don't exist in the grammar at all.
After: Alex imports that exact clause, and a real write proposing a `status` value inside the list
succeeds; one proposing a value outside it is denied.
Decision enabled: Alex confirms enum/status-whitelist validation — the single most common
real-Firestore `in` idiom — works correctly.

#### Acceptance Criteria
- [ ] AC-CEG-09: a bracketed, comma-separated list of string (and/or numeric) literals (e.g.
      `["draft", "published", "archived"]`) parses into a new `Operand::ListLiteral(Vec<Operand>)`.
- [ ] AC-CEG-10: a new `in` grammar production (`<operand> in <list literal>`) parses into a new
      `Condition` shape distinct from `Compare` (membership, not equality/relational comparison).
- [ ] AC-CEG-11: a real write whose proposed field value is a member of the list succeeds; one
      whose value is not a member is denied.
- [ ] AC-CEG-12: `in` against an operand that is NOT a list literal (e.g. membership against a
      map-valued field — Resolution 2's own explicitly out-of-scope idiom) is rejected as a NAMED
      unsupported construct, never silently misevaluated.

### US-05: The Same Whitelist Grammar Gates Reads and Is Simulatable

**job_id**: JOB-17 | **Release**: 2 | **Persona**: P1 Alex

#### Elevator Pitch
Before: same rejection as US-04, for the read-path/simulation surfaces.
After: a real `GetDocument` call against a document whose field is (or isn't) a list member is
allowed/denied correctly, and Alex can simulate an `in` candidate before publishing.
Decision enabled: full read+write+simulation confidence for whitelist rules, mirroring every prior
epic's own parity discipline.

#### Acceptance Criteria
- [ ] AC-CEG-13: `resource.data.<field> in [...]` gates a real `GetDocument` call correctly.
- [ ] AC-CEG-14: `simulate_access_rule` evaluates an `in` candidate condition via the identical
      `evaluate()` routine.

### US-06: Alex's `.rules` File's Time-Window Clause Now Parses and Enforces Correctly

**job_id**: JOB-17 | **Release**: 3 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `allow update: if request.time < resource.data.created_at + duration.value(24, 'h');` is
rejected on every front — no `request.time` operand, no `duration.value(...)` recognition (falls
into the existing call-syntax scan as an undifferentiated `CustomFunction` rejection), no `+`
arithmetic operator, and the numeric literal `24` was rejected before this feature even started
(US-01 fixes that half).
After: Alex imports that exact clause, and a real `UpdateDocument` call is allowed within the
24-hour window and denied outside it.
Decision enabled: Alex confirms his real edit-window/freshness validation — a canonical real
Firestore rate-limiting idiom — works correctly, closing the LAST of the 3 named remaining
CEL-parity epics' own shared "arithmetic/timestamp/duration" naming.

#### Acceptance Criteria
- [ ] AC-CEG-15: `request.time` parses into a new `Operand::RequestTime`, resolving to a
      caller-supplied "now" `FieldValue::Timestamp` threaded into `evaluate()` exactly like
      `path_variable_value` (zero new I/O — the value is already computed at every real call
      site for other purposes, confirmed by direct grep before this AC is implemented).
- [ ] AC-CEG-16: `duration.value(<integer>, '<unit>')` (units: at minimum `s`, `m`, `h`, `d`)
      parses ONLY as the right-hand side of a new, narrowly-scoped `+`/`-` arithmetic operand
      pairing against a `Timestamp`-typed left-hand operand — never as a standalone operand
      (Resolution 3/4).
- [ ] AC-CEG-17: `<timestamp operand> +/- duration.value(...)` evaluates to a new `FieldValue::
      Timestamp`, correctly offset, then participates in the relational-comparison grammar US-01
      already built (`<`, `<=`, `>`, `>=` against `request.time` or another timestamp field).
- [ ] AC-CEG-18: an unrecognized duration unit string is a NAMED, distinguishable rejection —
      never silently treated as seconds or any other default.
- [ ] AC-CEG-19: nested arithmetic (`a + b + c`, `(a + b) * c`) or an unsupported operator (`*`,
      `/`, `%`) is rejected as a NAMED unsupported construct (Resolution 4's own locked boundary),
      never a bare `SyntaxError`.

### US-07: The Same Time-Window Grammar Gates Every Surface and Is Simulatable

**job_id**: JOB-17 | **Release**: 3 | **Persona**: P1 Alex

#### Elevator Pitch
Before: same rejection as US-06, for write/simulation.
After: real writes and simulation both correctly evaluate timestamp/duration conditions.
Decision enabled: full read+write+simulation confidence for time-window rules — the LAST parity
gate this whole 3-release feature closes.

#### Acceptance Criteria
- [ ] AC-CEG-20: a real `CreateDocument`/`UpdateDocument` call is gated correctly by a
      timestamp/duration condition.
- [ ] AC-CEG-21: `simulate_access_rule` evaluates a timestamp/duration candidate condition via the
      identical `evaluate()` routine, accepting a caller-supplied synthetic "now" value (mirrors
      `simulate_access_rule`'s own existing synthetic-input discipline for every other operand
      family).

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: security-rules-cel-expression-grammar

### Objective
Close the single largest remaining category of real customer `.rules`-file rejections after
path/wildcard support (4b/4b′) shipped — numeric bounds, whitelists, and time-window validation.

### Outcome KPIs
| KPI | Target | Measurement |
|---|---|---|
| Real `.rules`-file constructs newly accepted | 3 idiom families (numeric bound, `in`/whitelist, timestamp/duration window) | Direct: this feature's own acceptance-test suite proves each family imports and enforces correctly |
| Regression | 0 | Full `security_rules_*` baseline re-run clean after every slice (established discipline, 468+ tests as of this feature's start) |
| Mutation-testing kill rate on new `embyr-core` logic | >= 80% (per project CLAUDE.md, WARN 70-80%) | `cargo-mutants`, `--in-place`, module-filtered (per 4b′'s own lesson) |

## Wave: DISCUSS / [REF] Out of Scope

- **Map literals** (`{"role": "admin"}` as a condition-side operand) — zero domain evidence
  (Resolution 2). Named, deferred, no candidate feature id assigned.
- **Nested map-field traversal** (`resource.data.address.city` reaching INTO a map-valued field's
  own key, as distinct from a flat field name containing a literal dot) — a real, structurally
  different gap this feature does not fix, flagged `OQ-CEG-01`. Named, deferred.
- **`*`, `/`, `%`, and any nested arithmetic expression** — zero domain evidence beyond the ONE
  evidenced idiom (timestamp + duration, Resolution 4). Named, deferred.
- **`in` against a map (key-membership) or against `resource.data` itself** — a different real
  Firestore idiom from list-membership; not built here (Resolution 2/US-04's own AC-CEG-12).
- **Real Firestore's `path`-type segment-indexing** (`path[0]`) — carried forward from 4b′'s own
  scope note, still not built here; would need its own DISCUSS given it interacts with the
  recursive-wildcard feature's own locked "no captured-remainder binding" decision.
- **Cross-document reads (`get()`/`exists()`)** — unchanged, candidate id
  `security-rules-cel-cross-document-reads`, "Epic 4d." **Next after this feature.**
- **Custom `function` definitions and invocation** — unchanged, candidate id
  `security-rules-cel-functions`, "Epic 4e." Composes over this feature's own grammar surface.
- **Re-opening any part of 4a's/4b's/4b′'s own already-shipped scope** — done, out of bounds.

## Wave: DISCUSS / [REF] WS Strategy

**Strategy A** (real, minimal, end-to-end) — Slice 01 is a real numeric-literal + real relational
operator + real `GetDocument` enforcement proof, not a mock or a config-switch skeleton. Mirrors
every prior JOB-17 epic's own WS strategy.

## Wave: DISCUSS / [REF] Driving Ports

Admin HTTP `:9090` only (`define_access_rule`/`simulate_access_rule`'s existing routes, condition
TEXT unchanged in shape — only the grammar `parse_condition` accepts widens) + gRPC `:8080`
(`GetDocument`, `CreateDocument`, `UpdateDocument` — existing routes, zero new RPCs). No new admin
route, no new RPC — every user story is reachable through surfaces that already exist.

## Wave: DISCUSS / [REF] Pre-requisites

- `security-rules-cel-parity` (4a), `security-rules-cel-path-matching` (4b),
  `security-rules-cel-recursive-wildcards` (4b′) — all finalized, provide the `Operand`/
  `Condition`/`evaluate()`/parser architecture this feature extends.
- No new external dependency, no new bounded context, no new storage schema.

## Wave: DISCUSS / [REF] Handoff Package

Handed to `nw-solution-architect` (DESIGN): this feature-delta.md, the 4 Resolutions above (esp.
Resolution 4's locked narrow-arithmetic boundary — DESIGN should design the `Operand::Arithmetic`
AST shape to make widening to `*`/`/`/`%`/nesting later ADDITIVE, not a rewrite, mirroring 4b′'s
own "structurally doesn't preclude a future widening" soft-constraint discipline exactly), and
`OQ-CEG-01` (nested map-field traversal, flagged not built).

## Wave: DISCUSS / [REF] SSOT Updates

`docs/product/jobs.yaml`, JOB-17 entry: append a new dated NOTE (mirroring 4b′'s own identical
NOTE-append convention, § functional dimension) — "JOB-17 now also covers numeric-bound,
whitelist/`in`, and timestamp/duration-window validation clauses in a real Firestore `.rules`
file, same job, same persona, not a new job (11th realization)."

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.97**

### DoR Checklist (9-item hard gate)
1. [x] Every story traces to a job_id (JOB-17, all 7 stories)
2. [x] Every story has a complete Elevator Pitch (Before/After/Decision enabled)
3. [x] Every AC is testable without ambiguity
4. [x] Walking Skeleton identified (US-01)
5. [x] Scope Assessment passed
6. [x] No slice contains only `@infrastructure` stories (every slice has a direct Alex-facing
   value story)
7. [x] Out of Scope explicitly named (7 items)
8. [x] Outcome KPIs have numeric targets and measurement methods
9. [x] Prior-wave artifacts read and reconciled (no contradictions found)

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Open Questions

- `OQ-CEG-01`: nested map-field traversal (`resource.data.address.city`) — real gap, not built
  this feature, needs its own DISCUSS on whether `.` in a resource-field position should mean
  nested-map-traversal (matching real Firestore) given the existing grammar's current flat-field
  treatment of any dotted remainder.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] One feature, 3 dependency-ordered releases, not 3 separate features (Resolution 1):
  relational comparison needs numeric literals; timestamp/duration needs arithmetic — genuine
  dependency chains, not arbitrary bucketing.
- [D2] List literals + `in` locked in scope; map literals and nested map-field traversal locked
  OUT (Resolution 2): zero domain evidence for either, and nested traversal is a structurally
  different, separately-flagged gap (`OQ-CEG-01`).
- [D3] `request.time` reuses the existing `Timestamp` `FieldValue`, threaded as a caller-supplied
  value with zero new I/O (Resolution 3) — the identical `path_variable_value` precedent.
- [D4] Arithmetic locked to `+`/`-` only, non-nested, only between matching numeric types or a
  timestamp and a duration constructor (Resolution 4) — a strict, evidenced subset designed to
  widen additively later, never rebuilt.

### Requirements Summary
- Primary need: real customer `.rules` files still get rejected on numeric bounds, `in`/whitelist,
  and timestamp/duration clauses — the largest remaining CEL-parity gap after path/wildcard
  support shipped.
- Walking skeleton scope: one numeric-literal relational comparison, real end-to-end enforcement.
- Feature type: Backend.

### Constraints Established
- Zero new I/O anywhere (embyr-core stays zero-IO; `request.time` reuses an already-computed
  value, mirroring `path_variable_value`).
- Zero new storage schema (`FieldValue::Integer`/`Double`/`Timestamp`/`Array` already exist).
- Arithmetic generality deliberately narrow (Resolution 4), designed for additive widening later.

### Upstream Changes
- None — no DISCOVER assumptions from a prior feature are contradicted; this feature is a named,
  evidenced continuation of 4a's own original Out-of-Scope entry.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 4 locked Resolutions, `OQ-CEG-01`, 3-release/7-slice plan

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ This feature-delta.md's own DISCUSS sections in full, all 4 Resolutions.
✓ `crates/embyr-core/src/access_control/mod.rs` — full re-read (already done during DISCUSS,
re-confirmed here per this project's own "DESIGN re-verifies structurally, never assumes DISCUSS's
own summary" discipline) — the exact `Operand`/`Condition`/`CompareOp`/tokenizer/parser shape to
extend, and every existing call site of `evaluate()` (6, confirmed by grep: `handle_get_document`,
3 write handlers, `handle_add_target`'s 2 Listen arms, `simulate_access_rule`) that will need the
new 7th parameter threaded through, `None` at every site not wired this feature.
✓ `crates/embyr-core/src/domain/field_value.rs` — confirmed `Timestamp(i64, i32)` representation
(seconds + subsecond nanos) for the `Operand::Arithmetic` timestamp-offset design below.
✓ `docs/product/architecture/adr-062-...md`, `adr-063-...md`, `adr-034-...md` — the 3 direct
precedents this design reuses (new-parameter rollout discipline, new-operand-family precedent,
tokenizer-addition precedent).

## Wave: DESIGN / [REF] Reuse Analysis

| Existing mechanism | Reused unchanged for this feature? |
|---|---|
| `Condition::Compare` (the AST node) | Yes — relational comparison (`<`/`<=`/`>`/`>=`) is a NEW `CompareOp` variant on the SAME node, not a new `Condition` shape |
| `evaluate()`'s fail-closed `FieldMissing` short-circuit | Yes — every new operand (numeric literal excepted, which never fails) resolves through the SAME `resolve_field_value`/`compare_operands` fail-closed path |
| `parse_condition`'s 3-call-site sharing (define/redefine, real enforcement, simulation) | Yes — zero new call sites, the SAME 3 already share whatever `parse_condition` accepts |
| `detect_unsupported_construct`'s pre-tokenize, quote-aware scan | Yes, EXTENDED (2 new checks) — not replaced |
| `AccessRulePatternRow`/`access_rules`/`access_rule_patterns` storage shape | Unchanged — condition text is already an opaque string column |
| Admin routes (`define_access_rule`, `simulate_access_rule`, `import_rules_file`) | Unchanged — zero new routes, zero new request/response fields |

**Nothing in this feature requires a new bounded context, a new table, a new admin route, or a
change to any of the 5 prior JOB-17 epics' own already-shipped call sites beyond the SAME
mechanical `evaluate()` 7th-parameter threading every prior signature-widening epic has already
done twice (`path_variable_value`, ADR-062; `ancestor_path_variable_values`, ADR-063).**

## Wave: DESIGN / [REF] Architecture Design

See ADR-065 (`docs/product/architecture/adr-065-expression-grammar-numeric-in-list-timestamp-
duration.md`) for the full type/tokenizer/parser/`evaluate()`-signature design. Summary of the 6
extension points, each independently additive (confirmed in ADR-065 § Decision sections):

1. **Types**: 5 new `Operand` variants (`IntLiteral`, `DoubleLiteral`, `ListLiteral`,
   `RequestTime`, `Arithmetic`, `DurationLiteral` — 6, correction), 1 new `Condition` variant
   (`In`), 4 new `CompareOp` variants, 1 new `UnsupportedConstruct` variant (shared across all 5
   named out-of-scope rejection sites).
2. **Tokenizer**: digit-run scanning, `[`/`]`/`,`, `<`/`<=`/`>`/`>=`, `+`/`-` (position-
   disambiguated from the existing negative-literal case).
3. **Parser**: a factored `parse_operand` helper (replacing the 2 existing inline `Word => word_
   to_operand` call sites in `parse_comparison`), extended for numeric/list/arithmetic/`in`
   recognition; `duration.value(...)` recognized syntactically only in arithmetic-RHS position.
4. **`evaluate()` signature**: 7th parameter `request_time: Option<FieldValue>`, `None` at every
   pre-existing call site, mirroring `path_variable_value`'s/`ancestor_path_variable_values`'s own
   identical rollout discipline.
5. **Rejection discipline**: 5 named-but-out-of-scope shapes (map literals, nested map-field
   traversal, `in`-against-non-list, unsupported/nested arithmetic, unrecognized duration unit)
   each get an explicit branch into `UnsupportedExpressionGrammar`, never a fallthrough
   `SyntaxError`.
6. **Zero storage/admin-route change** — confirmed by Reuse Analysis above.

## Wave: DESIGN / [REF] Wave Decisions Summary

### Key Decisions
- [D1] `evaluate()` gains a 7th parameter (`request_time`), not a new function or a struct-of-
  params refactor — mirrors the SAME incremental-parameter-growth precedent 2 prior epics already
  established; a struct-of-params refactor is explicitly NOT triggered by this feature alone
  (named as a future-DESIGN reconsideration point if a 5th/6th grammar-extension epic ever pushes
  the parameter count materially higher, not decided here).
- [D2] `duration.value(...)`'s legality is checked SYNTACTICALLY (parse-time position), not
  semantically (runtime type) — consistent with how every other operand's runtime type is already
  never parse-time checked in this grammar.
- [D3] One shared `UnsupportedExpressionGrammar` variant for 5 distinct named-rejection sites,
  distinguished by `detail` text, not 5 new enum variants — smallest correct extension of
  `UnsupportedConstruct`, mirrors `ConditionParseError::SyntaxError`'s own single-variant-many-
  details shape.

### Constraints Established
- No new `embyr-core` dependency (timestamp arithmetic uses whatever the crate already has
  available for `FieldValue::Timestamp` construction/comparison — confirmed available, no new
  crate needed, per the Slice 06 Pre-Slice SPIKE).
- Parser non-nesting enforcement for arithmetic is structural (parse-time rejection), not type-
  level — an accepted, named limitation (ADR-065 § Consequences).

## Wave: DESIGN / [REF] Next Wave

**Handoff To**: nw-acceptance-designer (DISTILL wave)
**Deliverables**: this feature-delta.md's DESIGN section, ADR-065
