# ADR-065: Expression Grammar — Numeric Literals, Relational Comparison, `in`/List Literals, Timestamp/Duration

## Status

Accepted

## Context

`security-rules-cel-expression-grammar` (JOB-17, 11th realization, "Epic 4c") closes the largest
remaining category of real Firestore `.rules`-file rejections after path/wildcard support (4b/4b′)
shipped: numeric-bound validation (`resource.data.photo_count <= 20`), whitelist/enum validation
(`request.resource.data.status in ["draft", "published", "archived"]`), and time-window validation
(`request.time < resource.data.created_at + duration.value(24, 'h')`). DISCUSS
(`docs/feature/security-rules-cel-expression-grammar/feature-delta.md`, Resolutions 1-4) locked:
one feature, 3 dependency-ordered releases (numeric+relational, then `in`/list, then
timestamp+duration — the latter needs the first release's numeric-literal tokenizer work and a new,
narrowly-scoped arithmetic operator); list literals in scope, map literals and nested map-field
traversal explicitly out (`OQ-CEG-01`); `request.time` reuses the existing `FieldValue::Timestamp`
with zero new I/O; arithmetic locked to `+`/`-` only, non-nested, timestamp+duration or
matching-numeric-type pairs only.

Confirmed by direct read (`access_control/mod.rs` full; `domain/field_value.rs` targeted): the
CURRENT grammar's `CompareOp` is `Eq`/`Ne` ONLY; `Operand` has no numeric/list/timestamp variant;
`tokenize()` has no digit-handling branch at all (a numeric literal is today an undifferentiated
`SyntaxError`); `detect_unsupported_construct` names only 3 constructs. `FieldValue` ALREADY has
`Integer(i64)`, `Double(f64)`, `Timestamp(i64, i32)`, `Array(Vec<FieldValue>)` — every domain VALUE
shape this feature needs already exists; the entire gap is the condition GRAMMAR (tokenizer,
`Operand`, `Condition`, parser, `evaluate()`'s comparison logic) only, zero new storage schema,
zero new admin route (the existing `define_access_rule`/`simulate_access_rule` routes already
accept arbitrary condition TEXT — only what `parse_condition` accepts widens).

## Decision Drivers

1. **Reuse the existing `Operand`/`Condition`/tokenizer/parser architecture unchanged in shape** —
   every new construct is an ADDITIVE variant/branch, never a parallel grammar or a second parser
   (mirrors every prior JOB-17 epic's own "one grammar, one parser" discipline).
2. **Zero new I/O anywhere** — `embyr-core` stays zero-IO (`deny.toml`-enforced); `request.time`'s
   "now" value is threaded as a caller-supplied parameter, mirroring `path_variable_value`'s own
   precedent (ADR-062) exactly.
3. **Zero new storage schema** — `FieldValue::Integer`/`Double`/`Timestamp`/`Array` already exist;
   condition TEXT is already stored as an opaque string (`access_rules.condition`/
   `access_rule_patterns.read_condition`/`write_condition`) — a widened grammar changes nothing
   about how it's stored, only how it's parsed and evaluated.
4. **Arithmetic generality stays a strict, evidenced subset, designed to widen additively later**
   (DISCUSS Resolution 4) — the AST shape must not need a rewrite if `*`/`/`/`%`/nesting are ever
   evidenced and added in a future feature.
5. **Every recognized-but-out-of-scope shape (map literals, nested map-field traversal, `in`
   against a non-list, unsupported arithmetic operators/nesting, unrecognized duration units) is a
   NAMED, distinguishable rejection** — never a bare `SyntaxError` (AC-17-03's distinguishability
   discipline, reapplied at 5 new named-construct sites).
6. **Simplest solution first**: extend the existing single-pass recursive-descent parser; no
   `pest`/`nom`, no operator-precedence-table rewrite beyond what the 2 new precedence LEVELS
   (relational comparison already sits at the SAME precedence as `==`/`!=`; `in` sits there too;
   arithmetic is a strictly narrower, non-nested sub-grammar within a comparison operand position,
   not a new precedence level in the general expression grammar) require.

## Decision — Types (`access_control/mod.rs`, EXTEND)

### `Operand`, 5 new variants

```rust
pub enum Operand {
    // ...existing variants unchanged...
    IntLiteral(i64),
    DoubleLiteral(f64),
    ListLiteral(Vec<Operand>),
    RequestTime,
    /// `<numeric-or-timestamp operand> +/- <numeric-or-duration operand>` —
    /// ADR-065 § Decision Driver 4: non-nested (both sides are LEAF
    /// operands, never another `Arithmetic`), the ONLY arithmetic shape
    /// this feature builds. `ArithmeticOp` is `Add`/`Sub` only.
    Arithmetic(Box<Operand>, ArithmeticOp, Box<Operand>),
    /// `duration.value(N, unit)` — never resolves to a `FieldValue` on its
    /// own (real Firestore's own `duration` is not a stored/returned type);
    /// only ever legal as `Arithmetic`'s own right-hand operand against a
    /// `Timestamp`-typed left operand. Reachable from the parser ONLY in
    /// that position — `resolve_field_value` panics-never, `Err
    /// (FieldMissing)`s if reached standalone, an unreachable-in-practice
    /// defensive branch (the parser never PRODUCES a standalone one).
    DurationLiteral(i64, DurationUnit),
}

pub enum ArithmeticOp { Add, Sub }
pub enum DurationUnit { Seconds, Minutes, Hours, Days }
```

### `Condition`, 1 new variant

```rust
pub enum Condition {
    // ...existing variants unchanged...
    /// `<operand> in <list literal>` — membership, not equality/relational
    /// comparison (DISCUSS Resolution 2/US-04). The RHS `Operand` is
    /// grammar-constrained to `ListLiteral` by the PARSER, not the type
    /// system (mirrors `Condition::Compare`'s own untyped-`Operand`-pair
    /// shape) — an `In` node whose RHS is not a `ListLiteral` is a parser
    /// bug, never a real runtime state; `eval_bool`'s own `In` arm still
    /// handles it defensively (falls closed) rather than panicking,
    /// consistent with `evaluate()`'s total/infallible-by-construction
    /// guarantee.
    In(Operand, Operand),
}
```

### `CompareOp`, 4 new variants

```rust
pub enum CompareOp {
    Eq, Ne,
    Lt, Le, Gt, Ge, // NEW
}
```

### `UnsupportedConstruct`, 1 new variant

```rust
pub enum UnsupportedConstruct {
    CrossDocumentRead, CustomFunction, WildcardPath,
    /// Map literals (`{"role": "admin"}`), nested map-field traversal
    /// (structurally indistinguishable from a flat dotted field name at
    /// parse time — `OQ-CEG-01`, not disambiguated by this feature),
    /// `in` against a non-list-literal RHS, an unsupported arithmetic
    /// operator (`*`/`/`/`%`) or nested arithmetic, and an unrecognized
    /// `duration.value(...)` unit — DISCUSS's own 5 named-rejection sites,
    /// one shared variant (they share the identical "recognized shape,
    /// deliberately out of this feature's own locked scope" semantics;
    /// `detail` distinguishes which).
    UnsupportedExpressionGrammar,
}
```

## Decision — Tokenizer (`tokenize`, EXTEND)

New branches, added alongside the existing `c.is_ascii_alphabetic()`/`'"'` arms:

- `c.is_ascii_digit()`, or `-` immediately followed by a digit in a position where the PREVIOUS
  token is not an operand-producing token (i.e. at the start of a primary, not immediately after a
  `Word`/literal/`)` — mirrors how `!` is already disambiguated from `!=` by lookahead, not
  position-tracking state) — scans a maximal digit run, optionally one `.` followed by more digits
  (a SECOND `.` or a `.` with no following digit is a syntax error, not a truncated number).
  Produces `Token::IntLiteral(i64)` or `Token::DoubleLiteral(f64)` (presence of `.` selects which).
- `[`, `]`, `,` — new structural tokens (`Token::LBracket`/`RBracket`/`Comma`) for list-literal
  syntax.
- `<`, `<=`, `>`, `>=` — new comparison tokens, mirroring the existing `==`/`!=` two-character
  lookahead pattern exactly.
- `+`, `-` (in an operand position, not the already-handled negative-literal position) — new
  arithmetic tokens (`Token::Plus`/`Minus`).

## Decision — Parser (EXTEND)

- `parse_comparison`'s existing `Eq`/`Ne` match widens to also accept `Lt`/`Le`/`Gt`/`Ge`,
  producing the SAME `Condition::Compare` shape with the new `CompareOp` variant — zero new
  `Condition` shape needed for relational comparison (it composes with the EXISTING `Compare`
  node, only the operator set widens).
- A NEW `parse_operand` helper (factored out of `word_to_operand`'s existing call sites, both of
  which currently inline `match self.advance() { Word(w) => word_to_operand(w)?, StringLiteral(s)
  => ... }`) also recognizes `Token::IntLiteral`/`DoubleLiteral` directly, `[` as the start of a
  list literal (parses comma-separated operands until `]`), and an operand immediately followed by
  `+`/`-` then another operand as `Operand::Arithmetic` (checked ONCE, non-recursively — the
  right-hand side of THAT `+`/`-` is itself parsed via `parse_operand` but a SECOND consecutive
  `+`/`-` after building one `Arithmetic` node is a parse-time rejection into
  `UnsupportedExpressionGrammar`, enforcing Resolution 4's non-nesting rule structurally, not by
  convention).
- After building the comparison's left operand via `parse_operand`, a NEW check: if the next token
  is the identifier-like word `"in"` (recognized the same way `true`/`false` are — a `Token::Word`
  whose text is checked, not a new dedicated token), parse a `ListLiteral` operand and produce
  `Condition::In` instead of continuing into `parse_comparison`'s existing `==`/`!=`/relational
  path.
- `duration.value(N, unit)` is recognized via a NEW branch that shares
  `detect_unsupported_construct`'s existing identifier-then-`(` scan shape but runs at the
  TOKENIZER/parser level (not the pre-tokenize scan) — reached ONLY from `parse_operand` when
  parsing the right-hand side of an `Arithmetic` node whose left side already resolved as a
  `Timestamp`-shaped operand (`RequestTime`, a `ResourceField`, or `RequestResourceField` — parser
  cannot know the RUNTIME type, so this is a SYNTACTIC position check: `duration.value(...)` is
  legal syntax only in arithmetic RHS position, full type-mismatch fail-closed handling happens at
  `evaluate()` time, mirroring how `resource.data.<field>`'s runtime type is never parse-time
  checked either). An unrecognized unit string is `UnsupportedExpressionGrammar`, not a silent
  seconds-default.
- `detect_unsupported_construct`'s existing pre-tokenize scan gains 2 new checks (run BEFORE the
  existing `**`/`{`/call-syntax scan, same quote-aware masking discipline): a bare `{` not already
  caught by the wildcard-path check but appearing in a position that looks like map-literal syntax
  (`{` followed eventually by `:`) is named `UnsupportedExpressionGrammar` with a
  map-literal-specific `detail`, distinguishable in message text (not a new `UnsupportedConstruct`
  variant, per Decision Driver 5's "one shared variant, `detail` distinguishes" choice above) from
  the pre-existing wildcard-path `{` rejection — ordering: wildcard-path's own `**`/bare-`{` check
  still runs first (unchanged priority), this new check only fires for a `{` that survives that
  scan AND contains a subsequent `:` before its matching `}`.

## Decision — `evaluate()` Signature (EXTEND, 7th parameter)

```rust
pub fn evaluate(
    condition: &Condition,
    auth: Option<&AuthContext>,
    resource_fields: &BTreeMap<String, FieldValue>,
    request_resource_fields: &BTreeMap<String, FieldValue>,
    path_variable_value: Option<&str>,
    ancestor_path_variable_values: &BTreeMap<String, String>,
    // NEW (security-rules-cel-expression-grammar, Slice 06, ADR-065):
    // request.time's own resolved value — mirrors path_variable_value's
    // zero-new-I/O precedent exactly. `None` at every call site not yet
    // wired to a real "now" value fails RequestTime closed via the
    // existing FieldMissing short-circuit, never a new control-flow
    // shape. Every REAL call site (GetDocument, 3 write handlers,
    // simulate_access_rule) already computes chrono::Utc::now() or
    // equivalent for other purposes (confirmed by direct grep before
    // Slice 06 locks this signature) — threading it here is zero new I/O.
    request_time: Option<FieldValue>,
) -> EvaluationOutcome
```

Every pre-existing call site not yet reached by Slice 06/07's own dispatch passes `None`,
mechanically, zero behavior change — the IDENTICAL rollout discipline ADR-062's `path_variable_
value` and ADR-063's `ancestor_path_variable_values` both established for their own new parameter.

## Decision — Rejecting Recognized-but-Out-of-Scope Shapes, Structurally

Every one of the 5 named out-of-scope shapes (map literals, nested map-field traversal, `in`
against non-list, unsupported arithmetic, unrecognized duration unit) is caught by an EXPLICIT
branch, never by falling through to the generic `SyntaxError` catch-all — mirrors
`detect_unsupported_construct`'s own existing "named constructs get a specific reason" discipline
(AC-17-03) at every one of these 5 new sites, re-verified per-site, not assumed to be inherited
automatically.

## Consequences

**Positive**: closes the single largest remaining category of real `.rules`-file rejections;
every new construct is additive to the existing `Operand`/`Condition` shape (confirmed above,
site-by-site) — no rewrite of `eval_bool`/`compare_operands`/`resolve_field_value`'s own existing
match arms, only new arms added; zero new storage schema, zero new admin route, zero new I/O.

**Negative / accepted trade-offs**: `evaluate()`'s parameter list grows to 7 (from 6) — named,
accepted, mirrors the SAME growth pattern every prior grammar-extension epic has produced (started
at 3 params pre-ADR-030, now 7); `OQ-CEG-01` (nested map-field traversal) remains a real, unfixed
gap, explicitly not silently absorbed into this feature's own scope; the arithmetic AST node
(`Operand::Arithmetic`) is deliberately non-general (Decision Driver 4) — a future feature widening
to `*`/`/`/`%`/nesting will need to revisit the parser's own non-nesting enforcement (currently a
structural parse-time rejection, not a type-level impossibility), a known, accepted, explicitly
named limitation of this ADR's own chosen shape.

## Enforcement

- `deny.toml` (unchanged) continues to enforce `embyr-core`'s zero-IO boundary — every new type/
  function in this ADR is pure data + pure computation.
- Mutation testing (per project CLAUDE.md, per-feature): the new grammar/evaluator logic is 100%
  `embyr-core`, pure and zero-IO — the SAME fast, `--in-place`, module-filtered mutation-testing
  approach 4b′'s own QUALITY_GATE proved out applies directly, with zero Docker/testcontainers
  exposure for this feature's own new logic.

## References

- `docs/feature/security-rules-cel-expression-grammar/feature-delta.md` (DISCUSS, all 4
  Resolutions, `OQ-CEG-01`)
- `docs/product/architecture/adr-062-rules-file-import-parser-path-variable-and-decomposition.md`
  (`path_variable_value` precedent, reused for `request_time`)
- `docs/product/architecture/adr-063-multi-segment-path-pattern-routing-and-storage.md`
  (`ancestor_path_variable_values` — the SAME "new parameter, `None` at unwired call sites"
  rollout discipline)
- `docs/product/architecture/adr-034-custom-claims-representation-and-grammar-extension.md`
  (`StringLiteral`/`BoolLiteral` tokenizer-addition precedent, and the ORIGINAL `OQ-SR-04` this
  feature's numeric-literal work finally resolves)
