# ADR-034: Custom-Claims Representation, Grammar Extension, and Query-Path Safety

## Status

Accepted

## Context

`custom-claims` (JOB-17, 6th realization) closes the gap named explicitly by
`security-rules`' own Out-of-Scope deferral: no rule can reference anything about
the caller beyond their bare `uid`. DISCUSS (`docs/feature/custom-claims/feature-delta.md`
§ Job Discovery Framing Resolution) locked the central architectural question —
Resolution 1: claims are **mint-time-embedded** in the client-identity token's own
JWT payload, not a request-time lookup against a new admin-managed store — with HIGH
confidence, not escalated. It also locked Resolution 2 (a new `Operand::AuthTokenClaim(String)`
variant, mirroring `RequestResourceField`'s own precedent, ADR-030) and, as the one
genuine judgment call surfaced for DESIGN to receive already-confirmed
(§ Handoff Package flag 2, confirmed in scope by the user per this dispatch's own task
framing — not re-opened here), Resolution 3: string-literal support (resolving
`security-rules`' own OQ-SR-04) is in-scope, narrowly, for Release 2 (US-06).

This ADR is genuinely cross-bounded-context — it amends **two** prior epics' own
ADRs, not one: ADR-024 (`client-auth`, BC-1 Tenant Management — the claims
representation on the token/identity side) and ADR-027/029 (`security-rules`, BC-4
Access Control — the `Operand`/`AuthContext`/grammar extension). Per DISCUSS's
own Handoff Package flag 1, both are treated as amendments, with `§ Changed
Assumptions` sections appended to each (this ADR is the canonical new decision
record; the amendment notes on ADR-024/027/029 point here rather than duplicating
this ADR's own content).

Mirrors ADR-030's own "combine what would otherwise be several ADRs into one"
precedent: this feature's decision surface — while touching two bounded
contexts — is five bounded, additive extensions of already-accepted mechanisms
(ADR-024's claims decode, ADR-027's grammar, ADR-029's composition, ADR-030's
`RequestResourceField` precedent, ADR-031's query-path catch-all), not
independently wide option spaces of their own.

## Decision Drivers

1. **Zero-IO invariant is load-bearing across the ENTIRE feature** (§ Handoff
   Package flag 3) — every mechanism below must preserve `embyr-core::access_control`'s
   and `embyr-core::client_identity`'s existing zero-IO invariant (`deny.toml`
   -enforced). No new admin API, no new System DB table, no new I/O in `evaluate()`'s
   call path (Resolution 1, locked).
2. **Extra JWT fields must remain non-breaking to parse** (Finding 1) — the exact
   representation chosen for `ClientIdentityClaims` must not reintroduce
   `#[serde(deny_unknown_fields)]` anywhere in the decode path, and must preserve
   exact JSON-type round-tripping (no silent coercion, AC-17-137).
3. **Reuse `FieldValue`, do not invent a new claim-value type** (§ System
   Constraints, locked) — claim values are represented as the SAME `FieldValue`
   type `resource_fields`/`request_resource_fields` already use, so claim-to
   -resource-field comparisons (US-02's ABAC domain example) fall through to the
   existing `FieldValue::PartialEq` catch-all with zero new comparison logic.
4. **`Operand::AuthTokenClaim(String)` mirrors `RequestResourceField`'s own
   precedent exactly** (Resolution 2, locked) — one new operand within the
   existing `Condition`/`evaluate()` structure. No second AST, no second
   evaluator (Decision Driver 3 of ADR-027, reapplied).
5. **`check_query_compliance()` receives zero code changes** (Resolution 4,
   locked) — a claim-referencing rule must be provably, not just testedly,
   `RejectedUnsupportedRuleShape` via the ALREADY-EXISTING wildcard catch-all in
   `decompose_decidable()`. This is this feature's own designated
   mutation-testing surface (§ Handoff Package flag 4).
6. **String-literal support scoped narrowly to strings only** (Resolution 3,
   locked, Release 2) — no numeric-literal widening under this feature's own
   authority.
7. **US-03's falsifiability must be checked structurally, not assumed** (§
   Handoff Package flag 5) — if the identical `Operand::AuthTokenClaim` does
   NOT, in the actual code path, gate writes without additional production
   code, this feature's own scope-containment argument is disproven.
8. **Simulation shares the exact evaluation routine** (ADR-029 DDD-SR-8,
   reapplied) — US-07 must not duplicate `evaluate()`.

## Decision — Claims Representation (amends ADR-024)

### `ClientIdentityClaims` (`crates/embyr-core/src/client_identity/mod.rs`)

```rust
#[derive(serde::Deserialize)]
struct ClientIdentityClaims {
    sub: String,
    aud: String,
    exp: i64,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}
```

**Accepted: `#[serde(flatten)]` into a `BTreeMap<String, serde_json::Value>`.**
Three options were weighed:

| Option | Fit |
|---|---|
| **(A) `#[serde(flatten)] extra: BTreeMap<String, serde_json::Value>` — Accepted** | Serde's `flatten` attribute captures every top-level JSON key not already named (`sub`/`aud`/`exp`) into `extra`, preserving each value's exact JSON type (no coercion — a JSON string claim decodes as `serde_json::Value::String`, never silently becomes a `Value::Bool`, satisfying AC-17-137/Domain Example 3). Serde's `flatten` and `deny_unknown_fields` are mutually incompatible at the derive-macro level — choosing `flatten` **structurally forecloses** ever accidentally combining the two on this struct, a stronger guarantee than a code-review convention (Decision Driver 2). Zero change to `sub`/`aud`/`exp` extraction or to `jsonwebtoken::decode::<ClientIdentityClaims>`'s call shape (ADR-024's cryptographic path, algorithm pinning, and verification order are completely untouched). |
| **(B) An explicit, per-claim-name typed struct (e.g. `is_moderator: Option<bool>`, `department: Option<String>`)** | Rejected. Requires enumerating every possible claim name/type ahead of time — defeats the entire premise of "arbitrary claims Trailmark's own backend already chooses to embed" (Finding 2). A new Trailmark claim (e.g. `subscription_tier`) would require an embyr code change and redeploy before any rule could reference it — the opposite of what this feature exists to enable. |
| **(C) The WHOLE claims struct as a raw `serde_json::Value`, with `sub`/`aud`/`exp` extracted manually from the parsed `Value` at the call site** | Rejected. Discards `jsonwebtoken::decode::<T>`'s existing typed-field extraction and validation for `sub`/`aud`/`exp` (ADR-024's own decode-time claim shape), replacing proven, already-tested code with a hand-rolled `Value` field-lookup for no benefit over Option A — over-general for what this feature actually needs. |

`VerifiedEndUserIdentity` gains:

```rust
pub struct VerifiedEndUserIdentity {
    pub end_user_id: String,
    pub project_id: String,
    pub expires_at_unix: i64,
    pub claims: BTreeMap<String, FieldValue>,
}
```

`impl From<ClientIdentityClaims> for VerifiedEndUserIdentity` translates `extra`
via the NEW `FieldValue::from_json_value` function (see § Decision — Shared JSON
Translation below) — a token minted with zero extra claims produces an EMPTY
`claims` map (AC-17-138, zero regression for every existing test token).

**New production (non-dev) dependency: `serde_json` for `embyr-core`.** `embyr-core`'s
`Cargo.toml` currently lists `serde_json` only under `[dev-dependencies]`.
`deny.toml`'s IO-prohibition bans list (`tokio`, `tonic`, `axum`, `sqlx`, `hyper`
families) does not include `serde_json` — it is a pure serialization/data-model
crate with no filesystem, network, or async-runtime surface, so this addition
does not touch the zero-IO invariant `deny.toml` enforces (Decision Driver 1).
This is the smallest possible new dependency: already a transitive, already
-vetted, already-a-workspace-member crate (used today in `embyr-server`'s admin
handlers), promoted to direct-dependency status in one additional crate.

## Decision — Shared JSON Translation (new, cross-cutting reuse)

**`FieldValue::from_json_value(value: &serde_json::Value) -> FieldValue`** — a
new, pure associated function on `embyr-core::domain::field_value::FieldValue`,
with the IDENTICAL match logic `embyr-server::admin::handlers::access_rules::json_value_to_field_value`
already implements (`Null`->`Null`, `Bool`->`Boolean`, `String`->`String`,
`Number`->`Integer` if it fits an `i64` else `Double`, `Array`->`Array`
(recursive), `Object`->`Map` (recursive)).

**Accepted: promote the existing private helper into `embyr-core`, then have
`embyr-server`'s copy delegate to it.** This feature is the FIRST caller of
JSON-to-`FieldValue` translation from inside `embyr-core` (`client_identity`'s
claims decode) — reusing the exact logic `embyr-server::admin::handlers::access_rules`
already validated (used today for `simulate_access_rule`'s `resource`/
`request_resource` translation) rather than writing a second, independently
-maintained copy inside `client_identity`. `embyr-server`'s existing private
`json_value_to_field_value` function becomes a one-line delegate:
`fn json_value_to_field_value(v: &serde_json::Value) -> FieldValue { FieldValue::from_json_value(v) }`
(or its 3 call sites are updated to call `FieldValue::from_json_value` directly
and the private function is deleted) — single source of truth, zero duplicated
match arms, satisfying Reuse Analysis discipline (extend, don't duplicate) for a
transformation this feature newly needs in a SECOND crate.

**Rejected alternative: a second, `client_identity`-local JSON-to-`FieldValue`
function.** Would duplicate 6 match arms verbatim across two crates with no
reason for them to ever diverge — directly the kind of drift risk this
project's Reuse Analysis discipline exists to prevent.

## Decision — Grammar Extension (amends ADR-027/029)

### `Operand` enum (`crates/embyr-core/src/access_control/mod.rs`)

```rust
pub enum Operand {
    AuthUid,
    AuthNullSentinel,
    ResourceField(String),
    RequestResourceField(String),
    AuthTokenClaim(String),   // NEW (US-02)
    StringLiteral(String),    // NEW (US-06, Release 2)
    BoolLiteral(bool),
    NullLiteral,
}
```

`word_to_operand()` gains one new prefix arm, non-overlapping with every
existing prefix (`"request.resource.data."`, `"resource.data."`) — identical
ordering-irrelevance argument ADR-030 already established for
`RequestResourceField`:

```rust
w if w.starts_with("request.auth.token.") => {
    let claim = &w["request.auth.token.".len()..];
    if claim.is_empty() { return Err(syntax_error("'request.auth.token.' requires a claim name")); }
    Ok(Operand::AuthTokenClaim(claim.to_string()))
}
```

**No tokenizer change is required for `AuthTokenClaim`** — `tokenize()`'s
existing word-character class already consumes `request.auth.token.is_moderator`
as a single `Word` token, the identical zero-tokenizer-change property ADR-030
established for `request.resource.data.<field>`.

### Finding: a latent, pre-existing grammar gap discovered during this DESIGN pass

Direct code verification (per this dispatch's own "confirm structurally, not
just trust" instruction) surfaced a gap that ADR-027's own text did not
accurately describe. ADR-027 § Comparison Semantics claims `ResourceField(_) ==
BoolLiteral(_)` is "syntactically reachable within the locked grammar" — this is
**not true of the actual code**: `word_to_operand()` has no `"true"`/`"false"`
match arm at all. `Operand::BoolLiteral` is constructed nowhere by the parser
today; it exists only as a defensively-handled variant in `resolve_field_value`.
The ONLY place `"true"`/`"false"` are recognized is `parse_primary`'s
peek-based bare-literal special case (`Condition::Literal(true)` for a
whole-condition `"true"`) — which never fires from inside `parse_comparison`,
because `parse_comparison` is only reached once `parse_primary` has already
determined the current token is NOT a bare `"true"`/`"false"`.

**Consequence, verified by direct trace**: `request.auth.token.is_moderator ==
true` — US-02's own literal walking-skeleton domain example, the example this
entire feature is named after — **cannot be parsed by the pre-existing code**,
independent of whether `AuthTokenClaim` is added, because `word_to_operand("true")`
on the right-hand side of `==` hits the `other => Err(syntax_error(...))`
catch-all.

**Fix (required for US-02 to function at all, not optional, bundled into this
same grammar-extension commit)**: `word_to_operand()` gains two more arms:

```rust
"true" => Ok(Operand::BoolLiteral(true)),
"false" => Ok(Operand::BoolLiteral(false)),
```

**Verified zero-regression**: `parse_primary`'s existing peek-based guard
(`Some(Token::Word(w)) if w == "true"`) is checked BEFORE `parse_comparison` is
ever reached, and is unmodified by this fix — a bare top-level `"true"`/`"false"`
(US-03's `security-rules` public-read domain example) still resolves to
`Condition::Literal(bool)` exactly as before, never reaching the new
`word_to_operand` arms. The new arms are reachable ONLY from inside
`parse_comparison`'s LHS/RHS `word_to_operand` calls — a position `"true"`/`"false"`
could previously only reach as a guaranteed syntax error. This fix is therefore
strictly additive: every input that parsed successfully before still parses
identically; the only behavior change is that a previously-rejected input class
(`<operand> == true`/`<operand> == false`) now succeeds.

This finding is flagged prominently, not silently folded in, per Principle 12's
verify-don't-trust discipline — DISTILL and DELIVER should treat
`resource.data.<bool-field> == true`/`false` (an incidental side effect,
proving the fix is general, not `AuthTokenClaim`-specific — mirrors US-06's own
"prove the fix is general" domain example 2 discipline) as now independently
testable, alongside US-02's own `is_moderator == true`.

### `StringLiteral` and the tokenizer (US-06, Release 2)

`tokenize()` gains a genuinely new branch — the first quote-character handling
this tokenizer has ever had (confirmed absent by direct code read, DISCUSS
Finding 3):

```rust
'"' => {
    let start = i;
    i += 1;
    let content_start = i;
    while i < chars.len() && chars[i] != '"' { i += 1; }
    if i >= chars.len() {
        return Err(syntax_error(format!("unterminated string literal starting at position {start}")));
    }
    let content: String = chars[content_start..i].iter().collect();
    i += 1;
    tokens.push(Token::StringLiteral(content));
}
```

**Deliberately no escape-sequence handling** (Resolution 3's own narrow-scoping
discipline, reapplied) — no domain example requires a claim or field value
containing a literal `"` character. Flagged alongside the feature's other named
v1 exclusions (nested claim paths, bracket notation), not silently decided.

`parse_comparison`'s LHS/RHS operand resolution gains a second source, alongside
`word_to_operand`:

```rust
Some(Token::Word(w)) => word_to_operand(w)?,
Some(Token::StringLiteral(s)) => Operand::StringLiteral(s.clone()),
```

**Required companion fix: `detect_unsupported_construct` must become
quote-aware.** This function runs BEFORE tokenization and scans raw source
characters for `**`/`{` (wildcard-path) and `identifier(` (call-syntax) shapes.
Without a fix, a legitimate string-literal VALUE containing `**`, `{`, or a
`word(`-looking substring (e.g. a hypothetical claim value `"get(weird)"` or
`"a**b"`) would be misclassified as `UnsupportedConstruct` before tokenization
ever gets the chance to treat it as opaque string content — a genuine
correctness bug introduced BY string-literal support, not a pre-existing one
(pre-US-06, any `"` character was already rejected by `tokenize()`'s catch-all,
so this scan never had to be quote-aware before). The fix: `detect_unsupported_construct`
skips over the ENTIRE span between a `"` and its closing `"` (or end-of-input,
for the unterminated case — left for `tokenize()`'s own dedicated error to
report, per AC-17-153's distinguishability requirement) before continuing its
`**`/`{`/call-syntax scan. This is bundled into US-06's own scope, not a
separate story — it is a direct, necessary consequence of introducing quoted
content into a scanner that previously never had to reason about quoting.

### `resolve_field_value`/`compare_operands` (`evaluate()`, extends ADR-027/030)

```rust
fn resolve_field_value(...) -> Result<FieldValue, FieldMissing> {
    match operand {
        Operand::ResourceField(name) => resource_fields.get(name).cloned().ok_or(FieldMissing),
        Operand::RequestResourceField(name) => request_resource_fields.get(name).cloned().ok_or(FieldMissing),
        Operand::BoolLiteral(value) => Ok(FieldValue::Boolean(*value)),
        Operand::NullLiteral => Ok(FieldValue::Null),
        Operand::StringLiteral(value) => Ok(FieldValue::String(value.clone())),        // NEW
        Operand::AuthTokenClaim(key) => {                                              // NEW
            let auth = auth.ok_or(FieldMissing)?;
            auth.claims.get(key).cloned().ok_or(FieldMissing)
        }
        Operand::AuthUid | Operand::AuthNullSentinel => {
            auth.map(|a| FieldValue::String(a.uid.clone())).ok_or(FieldMissing)
        }
    }
}
```

**No new arm in `compare_operands` at all** — this is the load-bearing
confirmation of Resolution 2/3's own "falls through to the generic arm" claim,
verified structurally rather than asserted: neither `AuthTokenClaim` nor
`StringLiteral` appears in any of `compare_operands`'s NAMED special pairings
(`AuthUid`/`ResourceField`, `AuthUid`/`RequestResourceField`,
`AuthNullSentinel`/`NullLiteral`+variants), so every pairing involving them
(claim-vs-bool-literal, claim-vs-string-literal, claim-vs-resource-field,
resource-field-vs-string-literal) falls through to the existing generic `_`
arm — `resolve_field_value` both sides, compare via `FieldValue::PartialEq`.
Zero new comparison logic, confirming Decision Driver 3.

**Fail-closed semantics, verified**: `Operand::AuthTokenClaim`'s resolution
short-circuits to `FieldMissing` in exactly two cases — `auth.ok_or(FieldMissing)?`
first (anonymous session, AC-17-147, the identical mechanism `AuthUid` already
uses), then `auth.claims.get(key).ok_or(FieldMissing)` (claim present-identity,
absent-key, AC-17-146) — both collapsing the WHOLE evaluation to `Deny` via the
existing top-level `FieldMissing` short-circuit (`eval_bool`'s `?` propagation,
unmodified). No new error class, no new special case, exactly as DISCUSS's
System Constraints locked.

### `AuthContext` (extends ADR-027's type, ADR-029's construction)

```rust
pub struct AuthContext {
    pub uid: String,
    pub claims: BTreeMap<String, FieldValue>,
}
```

## Decision — Call-Site Propagation (amends ADR-029's "uniform construction" claim)

**Verified directly, not assumed**: 10 total `AuthContext { ... }` construction
sites exist in the codebase today (DISCUSS's own estimate of "5 already-shipped
call sites" undercounted the RunQuery group/non-group split and the 3 admin
-simulation call sites; the actual count, confirmed by `Grep` across
`crates/embyr-server/src`, is 10):

| # | Site | File | Pattern |
|---|---|---|---|
| 1 | `handle_get_document` | `grpc/handler.rs:584` | `verified_identity.as_ref().map(\|v\| AuthContext { uid: v.end_user_id.clone() })` |
| 2 | `handle_create_document` | `grpc/handler.rs:705` | identical |
| 3 | `handle_update_document` | `grpc/handler.rs:804` | identical |
| 4 | `handle_delete_document` | `grpc/handler.rs:915` | identical |
| 5 | `handle_run_query` (non-group) | `grpc/handler.rs:1267` | identical |
| 6 | `handle_run_query` (group) | `grpc/handler.rs:1303` | identical |
| 7 | `handle_add_target` (Listen) | `realtime/listen_handler.rs:99` | identical |
| 8 | `simulate_access_rule` | `admin/handlers/access_rules.rs:578` | `body.auth.map(\|a\| AuthContext { uid: a.uid })` |
| 9 | `simulate_query_compliance` | `admin/handlers/access_rules.rs:656` | identical |
| 10 | `simulate_group_query_compliance` | `admin/handlers/access_rules.rs:733` | identical |

**Sites 1–7 (production enforcement)**: all 7 construct from an
`Option<&VerifiedEndUserIdentity>` via the IDENTICAL one-line closure. Since
US-01 extends `VerifiedEndUserIdentity` with `claims: BTreeMap<String, FieldValue>`,
each of the 7 closures becomes a mechanical, identical one-line edit:

```rust
.map(|v| AuthContext { uid: v.end_user_id.clone(), claims: v.claims.clone() })
```

This is exactly the "uniform extension... gets support for free" property
Resolution 1/4 hypothesized — confirmed here to hold in PRACTICE (10 sites, 7 of
them a single, textually-identical mechanical edit), not merely in the abstract.

**Sites 8–10 (admin simulation)**: construct from a caller-supplied
`SimulatedAuth` (`admin/handlers/access_rules.rs`), currently `{ uid: String }`.
`SimulatedAuth` gains a claims field, `#[serde(default)]` for backward
compatibility (mirrors ADR-030's identical precedent for `SimulateAccessRuleBody`'s
`request_resource` field):

```rust
pub struct SimulatedAuth {
    pub uid: String,
    #[serde(default)]
    pub claims: BTreeMap<String, serde_json::Value>,
}
```

All 3 sites translate identically via `FieldValue::from_json_value` (§ Decision
— Shared JSON Translation):

```rust
let auth_ctx = body.auth.map(|a| AuthContext {
    uid: a.uid,
    claims: a.claims.iter().map(|(k, v)| (k.clone(), FieldValue::from_json_value(v))).collect(),
});
```

**`SimulatedAuth` remains ONE shared type across all 3 handlers**, not forked —
mirrors the existing precedent of `SimulatedAuth` already being shared before
this feature. `simulate_query_compliance`/`simulate_group_query_compliance`
gain the field mechanically (required by the shared type) but it is inert for
their own domain: a claim-referencing condition passed to either is rejected
via `check_query_compliance`'s unchanged catch-all regardless of what
`claims` carries (§ Decision — Query-Path Safety, below); only
`simulate_access_rule`'s own UAT scenarios (US-07) give the field observable
meaning.

## Decision — Query-Path Safety (US-05, confirms Resolution 4, zero code change)

**Verified directly, not assumed**: `decompose_decidable`'s match arms name
EXACT `Operand` variant patterns —

```rust
Condition::Compare(Operand::AuthNullSentinel, CompareOp::Ne, Operand::NullLiteral) => ...
Condition::Compare(Operand::AuthUid, CompareOp::Eq, Operand::ResourceField(f))
| Condition::Compare(Operand::ResourceField(f), CompareOp::Eq, Operand::AuthUid) => ...
Condition::And(left, right) => { decompose_decidable(left)?; decompose_decidable(right)?; ... }
_ => Err(Undecidable)
```

Since `Operand::AuthTokenClaim` and `Operand::StringLiteral` are STRUCTURALLY
DISTINCT enum variants from `AuthUid`/`ResourceField`/`AuthNullSentinel`/
`NullLiteral`, no `Condition::Compare` involving either new variant, in any
operator or operand-order combination, can ever match arm 2 or arm 3 — this is
a Rust exhaustive-match guarantee, not a runtime check that could regress. Every
such `Compare` falls through to the wildcard `_ => Err(Undecidable)` arm,
producing `RejectedUnsupportedRuleShape` (US-05's required outcome) with **zero
lines of code changed** in `decompose_decidable`, `check_query_compliance`, or
`filter_binds_field_to_uid`.

**`Condition::And` composition, verified**: a claim-referencing conjunct
combined with an otherwise-decidable conjunct (e.g. `request.auth.token.department
== resource.data.department && request.auth.uid == resource.data.owner_id`) is
ALSO safely rejected as a whole — `decompose_decidable`'s `And` arm propagates
`Err(Undecidable)` via `?` from either side, so one claim-referencing conjunct
anywhere in an AND-chain undecidabilizes the entire rule for query-path
purposes, exactly mirroring how any other unsupported shape inside an `And`
already behaves today.

**This is the mutation-testing-relevant surface (§ Handoff Package flag 4)**:
the property under test is "the wildcard match arm's exhaustiveness — not an
allow-list gap — is the sole path to rejection." DISTILL's acceptance scenarios
should include, independently: a bare claim-Compare, a claim-Compare inside an
`And`, and (Release 2) a `StringLiteral`-Compare, each proven rejected via
`RunQuery`, collection-group, AND `Listen` subscribe-time (mirrors the three
surfaces US-05's own UAT already names).

## Decision — Write-Path Falsifiability (US-03, confirms Resolution 4)

**Verified directly, not assumed**: `handle_create_document`/
`handle_update_document`/`handle_delete_document` (write-path, ADR-030) call
the SAME `evaluate()` function `handle_get_document` calls, with the SAME
`resolve_field_value`/`compare_operands` dispatch — there is no per-call-site
branch anywhere in `eval_bool`'s call tree keyed on which RPC handler invoked
it. Since `Operand::AuthTokenClaim`'s resolution is added ONCE, inside
`resolve_field_value` (§ Decision — Grammar Extension), and `compare_operands`
needs no new arm at all, **every** `evaluate()` call site — read, all three
write operations, Listen per-event recheck, and simulation — gains claim
support from that single change. Resolution 4's central hypothesis (the reason
this feature could stay a 7-story, 2-bounded-context feature rather than a
larger, per-surface epic split) is **confirmed true by direct code-path
tracing**, not merely asserted: US-03 requires zero production code beyond
US-02's own change, exactly as DISCUSS predicted.

## Decision — Simulation Extension (US-07)

**Accepted: extend `simulate_access_rule`'s existing `SimulatedAuth`/response
contract, not a new sibling handler.** Applying the SAME test ADR-032/033
already used to decide between "extend in place" and "new sibling handler":
is the REQUEST/RESPONSE CONTRACT genuinely different, or is this an additive,
backward-compatible field on an existing shape?

- `simulate_query_compliance` vs. `simulate_access_rule` (ADR-031): genuinely
  different response contract (`{compliant, reasons}` vs. `{outcome}`) — new
  sibling, correctly.
- `simulate_group_query_compliance` vs. `simulate_query_compliance` (ADR-032):
  genuinely different REQUEST contract (`group_condition: Option<String>` vs.
  required `condition`) — new sibling, correctly.
- **`SimulatedAuth` + `claims` field, this feature**: NOT a different contract
  shape — an additive, `#[serde(default)]` field on the EXISTING
  `SimulatedAuth` type, directly analogous to ADR-030's own precedent of adding
  `request_resource` to `SimulateAccessRuleBody` (which did NOT warrant a new
  handler). The response shape (`{outcome: "allow"|"deny"}`) is completely
  unchanged.

`simulate_access_rule` calls the identical `evaluate()` function real
enforcement uses (ADR-029 § Simulation shares the exact evaluation routine,
reapplied without modification) — no third, independently-maintained
evaluation implementation exists anywhere, exactly as before.

## Consequences

### Positive

- Zero new bounded context, zero new component file, zero new table, zero new
  admin route beyond what already exists (`simulate_access_rule`'s route is
  unchanged; only its body shape gains a field).
- `check_query_compliance()`/`decompose_decidable()`/`filter_binds_field_to_uid()`
  receive ZERO code changes — Resolution 4's safety claim is a property of
  Rust's exhaustive-match compilation, not a tested convention.
- A pre-existing, previously-undiscovered grammar gap (bare boolean-literal
  operands unreachable from `parse_comparison`) is fixed as a necessary,
  zero-regression side effect of this feature's own walking-skeleton domain
  example — caught by direct verification, not shipped as a silent surprise.
- `FieldValue::from_json_value` eliminates a duplicated JSON-translation
  implementation that would otherwise have been written a second time inside
  `embyr-core::client_identity`.

### Negative / Trade-offs

- `embyr-core` gains its first non-dev dependency on `serde_json` — a small,
  well-justified, non-IO addition, but a departure from its previous
  "`serde_json` only in `[dev-dependencies]`" shape.
- No escape-sequence support for string literals in v1 (Resolution 3's own
  narrow scoping) — a claim or field value containing a literal `"` character
  cannot be expressed. No domain example requires it; named as a candidate
  follow-up, not built speculatively.
- `SimulatedAuth`'s `claims` field is inert (present but meaningless) for
  `simulate_query_compliance`/`simulate_group_query_compliance` — an accepted
  minor type-shape imprecision in exchange for NOT forking a struct that was
  already shared across 3 handlers before this feature.

## Enforcement

Style: Hexagonal (ports-and-adapters), unchanged. No new bounded context — BC-1
(`client_identity`) and BC-4 (`access_control`) are both extended, not
replaced; BC-4's dependency on BC-1 remains read-only and indirect (ADR-029,
unchanged in shape — `AuthContext` is still constructed 1:1 from
`VerifiedEndUserIdentity`, now including `claims`, at the same 7 production call
sites, never re-derived).

**No new driven port, no new probe (Principle 12 discipline, explicit reasoning
required, mirroring ADR-027/029/030/031's own identical conclusion):**
`ClientIdentityClaims`'s `#[serde(flatten)]` decode reuses the EXISTING
`jsonwebtoken::decode::<ClientIdentityClaims>` call — no new substrate, no new
partial-trust surface (the same "signature already proved the claims are
genuine" reasoning ADR-024 already established covers the extra flattened
fields too, since they are part of the SAME signed payload). `FieldValue::from_json_value`,
`resolve_field_value`, `compare_operands`, `decompose_decidable` remain pure,
deterministic CPU computation over values already in memory — the identical
"no environment can lie to a pure function" reasoning ADR-027/029 § Enforcement
already established applies unmodified.

`cargo-deny`/`deny.toml`: `embyr-core` gains `serde_json` as a direct,
non-banned dependency (verified against `deny.toml`'s ban list above) —
`embyr-core`'s zero-IO enforcement is otherwise unaffected; no new crate-specific
configuration required.

## References

- `docs/feature/custom-claims/feature-delta.md` §§ Job Discovery Framing
  Resolution (Resolutions 1–4), § System Constraints, § Handoff Package flags
  1–6.
- `docs/product/architecture/adr-024-client-identity-verification-mechanism.md`
  § Changed Assumptions (appended by this feature).
- `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md` §
  Changed Assumptions (appended by this feature) — corrects the "syntactically
  reachable" claim re: `BoolLiteral`.
- `docs/product/architecture/adr-029-access-control-composition-and-bounded-context.md`
  § Changed Assumptions (appended by this feature).
- `docs/product/architecture/adr-030-write-path-grammar-storage-and-composition.md`
  — the `RequestResourceField`/`request_resource` precedents this ADR mirrors
  directly.
- `docs/product/architecture/adr-031-query-shape-compliance-check.md` — the
  `decompose_decidable` wildcard-catch-all mechanism this ADR relies on
  unmodified.
- `crates/embyr-core/src/client_identity/mod.rs`,
  `crates/embyr-core/src/access_control/mod.rs`,
  `crates/embyr-core/src/domain/field_value.rs`,
  `crates/embyr-server/src/admin/handlers/access_rules.rs`,
  `crates/embyr-server/src/grpc/handler.rs`,
  `crates/embyr-server/src/realtime/listen_handler.rs`,
  `crates/embyr-core/Cargo.toml`, `deny.toml` — all read in full or via
  targeted `Grep` during this DESIGN pass.
