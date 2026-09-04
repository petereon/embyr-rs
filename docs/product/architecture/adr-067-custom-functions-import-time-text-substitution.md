# ADR-067: Custom Functions — Import-Time Text Substitution

## Status

Accepted

## Context

`security-rules-cel-functions` (JOB-17, 13th realization, "Epic 4e") lets Alex factor a repeated
boolean check in a real `.rules` file into a named, zero-parameter helper —
`function isEditor() { return request.auth.uid == resource.data.editor_id; }` — and call it from
one or more `match` blocks (`allow write: if isEditor();`). This is the LAST of the 3 CEL-parity
epics `security-rules-cel-parity`'s own original DISCUSS split named (4c, 4d, 4e), and, per that
DISCUSS's own framing, the lowest-risk of the four originally-deferred epics.

DISCUSS (`docs/feature/security-rules-cel-functions/feature-delta.md`, Resolutions 1–5) locked the
central architectural finding this ADR designs the mechanism for: **a function call needs NOTHING
new at evaluation time.** Unlike every prior CEL-parity epic (each of which needed a genuinely NEW
piece of information at evaluation time — a fetched cross-document result, a wall-clock read, a
numeric/timestamp operand), a function call's own body, once resolved, is ordinary, already
-supported condition grammar over the SAME `resource`/`request` bindings the calling clause
already has. There is nothing to "evaluate differently" — only something to EXPAND, once, before
evaluation ever begins.

Confirmed by direct code read (`crates/embyr-core/src/access_control/mod.rs`,
`detect_unsupported_construct`): every call-shaped identifier not in the fixed exemption list
(`get`/`exists`/`duration.value`) is ALREADY, unconditionally, rejected as
`UnsupportedConstruct::CustomFunction` — this is the mechanism's own safety net, not something
this feature builds or modifies.

## Decision Drivers

1. **`embyr-core`'s `access_control/mod.rs` stays byte-for-byte unchanged** — the strongest
   possible form of "lowest risk of the four deferred epics": no new `Operand`/`Condition` variant,
   no new `evaluate()` parameter, no per-call-site rollout across `embyr-server` (contrast every
   prior CEL-parity epic, each of which touched 12+ call sites in `handler.rs` alone).
2. **Reuse `rules_file.rs`'s own existing outer-syntax parsing machinery** — `find_matching_close`
   (brace-depth counting), the `RulesFileError`/`OffendingBlock` error shape, the
   `path_pattern`-threading discipline `parse_allow_clauses` already has — never new, parallel
   machinery for a structurally similar problem.
3. **Recursion/nesting-proof by construction, not by a separately-built detector** — validating a
   function's own body via the UNMODIFIED `parse_condition` (which itself rejects any call-shaped
   identifier) means a function body containing a nested call is rejected by code this feature
   does not touch, for free.
4. **Simplest solution first**: zero-parameter functions, a single linear (non-recursive)
   expansion pass, verbatim text splicing (no AST-to-string serializer, since none exists in this
   codebase and building one would be new machinery a narrowly-scoped feature does not justify).

## Decision — Types (`rules_file.rs`, EXTEND, no `embyr-core` change)

No new PUBLIC type is added to this feature's own scope beyond two new pure functions and their
private helpers — the function-definition map is a plain `BTreeMap<String, String>`, never a
dedicated struct (nothing beyond `name → body text` is ever needed).

```rust
/// Scans the LEADING portion of `service_body` (the content directly inside
/// `service cloud.firestore { ... }`, before the `match
/// /databases/{database}/documents { ... }` sub-block) for zero or more
/// `function <name>() { return <expr>; }` blocks — siblings of the match
/// sub-block, mirroring real Firestore's own declaration position (§
/// Reading Confirmation). Reuses `find_matching_close` unchanged for the
/// `{...}` body. Returns the consumed function definitions (name -> body
/// text, `return`/`;` already stripped) plus the REMAINING unconsumed
/// `service_body` slice (handed on, unmodified, to the existing `match
/// /databases/{database}/documents { ... }` shell check).
///
/// A function body is validated via the UNCHANGED `parse_condition` at THIS
/// scan step (fail-fast, before any `match` block is even reached) — this is
/// what makes nesting/recursion structurally impossible (Resolution 3):
/// `parse_condition`'s own existing `detect_unsupported_construct` rejects
/// any call-shaped identifier inside a function body, since NO expansion
/// pass is ever applied to a function body itself.
///
/// A non-empty parameter list (`function name(x) { ... }`) is a named
/// rejection (`FUNCTION_PARAMETERS_UNSUPPORTED`, Resolution 2). A duplicate
/// function name is a named rejection (`DUPLICATE_FUNCTION`). A body that is
/// not exactly `return <expr>;` (e.g. containing a `let` binding, multiple
/// statements, or no `return` at all) is a named rejection
/// (`SYNTAX_ERROR`, Resolution 4).
fn parse_function_blocks(service_body: &str) -> Result<(BTreeMap<String, String>, &str), RulesFileError>;

/// A single quote-aware linear scan over `condition_text` (the raw text
/// `parse_allow_clauses` already extracts between `if` and the clause's own
/// trailing `;`), splicing `(<body text>)` in place of each `<name>()`
/// occurrence found OUTSIDE a `"..."` or `'...'` literal, for every `name`
/// present in `functions`. Mirrors `tokenize`'s own quote-handling
/// discipline as a DESIGN PRINCIPLE (track literal-state, skip scanning
/// inside it) — NOT shared code; this scanner operates on raw text at a
/// structurally earlier pipeline stage, before any tokenization, and only
/// ever needs to recognize an identifier immediately followed by `(`/`)`,
/// never a full token stream.
///
/// A bare identifier matching a defined function name, NOT immediately
/// followed by `()` (e.g. followed by `(<non-empty args>)`, or referenced
/// without any call syntax at all as a plain word) is left untouched by
/// this scanner — `parse_condition`'s own existing grammar handles (or
/// rejects) it exactly as it already does today, since this feature only
/// ever recognizes the EXACT zero-argument call shape `name()`.
///
/// An identifier immediately followed by `(<non-empty>)`, where the
/// identifier matches a defined function name, is a named rejection
/// (`FUNCTION_PARAMETERS_UNSUPPORTED`, Resolution 2) — a call WITH
/// arguments to a KNOWN zero-parameter function is distinguishable from an
/// ordinary unrecognized custom-function call (which `detect_unsupported_
/// construct` already rejects, unchanged, as a plain `CustomFunction`).
///
/// Applied EXACTLY ONCE per condition, never recursively: a function body
/// is pre-validated flat (`parse_function_blocks`'s own `parse_condition`
/// call already proved it contains no call-shaped identifier), so the
/// SPLICED-IN body text can never itself contain a further `name()` needing
/// expansion — one linear pass is provably sufficient, not merely assumed
/// sufficient.
fn expand_function_calls(
    condition_text: &str,
    functions: &BTreeMap<String, String>,
    path_pattern: &str,
) -> Result<String, RulesFileError>;
```

## Decision — Threading (`parse_rules_file` → `parse_allow_clauses`)

`parse_rules_file` calls `parse_function_blocks` on `service_body` BEFORE the existing
`match /databases/{database}/documents { ... }` shell-check strips its own sub-block off — the
returned `BTreeMap<String, String>` then threads down through the EXISTING call chain
(`parse_match_blocks` → `parse_nested_match_blocks` → `parse_block_body` → `parse_allow_clauses`),
mirroring exactly how `full_path_pattern`/`full_segments` are already threaded down the identical
chain today (no new threading DISCIPLINE, just one more parameter riding the same path).
`parse_allow_clauses` calls `expand_function_calls` immediately after extracting
`condition_text`, before pushing `(verbs, condition_text)` into its own `clauses` vector — the
LAST possible point before the string becomes `MatchBlock`'s own stored, owned data.

`decompose`, `decompose_block`, `DecomposedTarget`, and every `embyr-server` consumer:
**unchanged, zero lines touched** — they already operate on `MatchBlock.allow_clauses`'s own
condition strings, which are now, by construction, always fully self-contained by the time they
arrive.

## Decision — Error Taxonomy (new `construct` tags, existing `RulesFileError`/`OffendingBlock` shape)

| `construct` | When | Reused mechanism |
|---|---|---|
| `DUPLICATE_FUNCTION` | Two `function` blocks share the same name | `RulesFileError::single`, shell-level (`path_pattern` empty) |
| `FUNCTION_PARAMETERS_UNSUPPORTED` | A function definition or call site carries a non-empty parameter/argument list | `RulesFileError::single`, threaded `path_pattern` for a call-site error (per-block naming), empty for a definition-site error (shell-level, before any block is reached) |
| `UNDEFINED_FUNCTION` | A call-shaped `name()` matches no defined function | `RulesFileError::single`, threaded `path_pattern` (the SAME per-match-block naming `parse_allow_clauses`'s own existing errors already use) |
| `SYNTAX_ERROR` (reused, unchanged tag) | A malformed `function` block shell, or a body that is not exactly `return <expr>;` | `shell_syntax_error`, unchanged |
| `CUSTOM_FUNCTION` (reused, unchanged — `embyr-core`'s own existing tag) | A function body itself calls another function/itself (Resolution 3), OR any un-expanded call site this feature's own scanner did not recognize (the safety-net corollary) | `detect_unsupported_construct`, completely unmodified |

## Enforcement

Unit tests for both new pure functions (`parse_function_blocks`, `expand_function_calls`) are
written DURING DELIVER, starting with Slice 01 — applying
`security-rules-cel-cross-document-reads`'s own QUALITY_GATE lesson
(`docs/evolution/2026-09-04-security-rules-cel-cross-document-reads.md` § Lessons Learned)
proactively: during-slice unit testing reduces, but does not eliminate, mutation-testing gaps, so a
dedicated `cargo-mutants --in-diff` pass is still budgeted at QUALITY_GATE regardless of test
discipline during DELIVER.

## Consequences

**Positive**:
- Zero `embyr-core` runtime change — the only CEL-parity epic in this initiative with that
  property, confirmed by construction, not merely claimed.
- Read, write, and simulation parity are a structural CONSEQUENCE of this design, not separately
  -built slices — Slices 02/03 need zero production code, the strongest form of the
  "confirmatory slice" precedent this initiative has established.
- Recursion and nesting are impossible by construction (Resolution 3), not by a separately
  -maintained cycle detector that could itself have a bug.

**Negative / accepted trade-offs**:
- A function's own body text is parsed twice in the general case (once, alone, at definition time
  for validation; again, spliced into each call site's own fully-expanded condition, when THAT
  condition is later parsed by `decompose_block`) — accepted, named, mirrors ADR-066's own
  identical "path template resolved twice" trade-off (D3): pure computation, not I/O, non
  -load-bearing.
- Verbatim text splicing (not AST-level substitution) means a function body's own internal
  operator precedence is trusted to already be correct as written (mirrors how any hand-authored
  parenthesized sub-expression already works) — the splice always wraps the body in an outer
  `(...)`, so this is safe regardless of the body's own internal structure.
- Zero-parameter-only, no-nesting, no-`let` are real, evidenced-by-Firestore's-own-docs
  capabilities this feature deliberately does NOT build (Resolutions 2–4) — named, deferred, not
  silently unsupported.
