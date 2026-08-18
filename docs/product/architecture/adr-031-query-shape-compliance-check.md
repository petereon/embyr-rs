# ADR-031: Query-Shape Compliance Check for RunQuery

## Status

Accepted

## Context

`security-rules` (Epic 2a, ADR-027/028/029) gave Alex per-collection read-path
enforcement on `GetDocument` only. `security-rules-write-path` (Epic 2b,
ADR-030) extended the same mechanism to `CreateDocument`/`UpdateDocument`/
`DeleteDocument`. Neither epic touched `RunQuery` — confirmed by direct code
read (`grpc/handler.rs::handle_run_query`, lines 1131-1267): it translates the
proto `StructuredQuery` into the domain `StructuredQuery` and calls
`adapter.run_query()` directly, with zero consultation of `access_rules`. This
is a genuine, actively-exploitable bypass: a client denied a document via
`GetDocument` can read the identical data by querying for it instead.

`security-rules-query-path` (Epic 2c, this ADR) closes that gap. DISCUSS
(`docs/feature/security-rules-query-path/feature-delta.md`, Resolution 1 +
Resolution 2) locked the following, which this ADR implements, not
re-litigates:

1. **A closed set of 5 statically-decidable `Condition` shapes**, checked
   against a query's filter tree *before* execution. Everything else —
   `Condition::Or`, `Condition::Not`, any reference to
   `Operand::RequestResourceField`, and any other `Condition::Compare` shape
   not named in the locked set — is rejected outright, for the entire
   collection, never partially enforced, never silently allowed.
2. **`evaluate()` is not reusable** for this mechanism — it requires an
   already-fetched document's field map (`resource_fields: &BTreeMap<String,
   FieldValue>`), which does not exist at query-planning time. A new, pure
   sibling function is required inside `embyr_core::access_control`, reusing
   `parse_condition()`/`Condition`/`Operand`/`AuthContext` unchanged.
3. **This feature reads `access_rules` only** (the existing read-condition
   table, `get_access_rule`, unchanged) — never `write_access_rules`.
4. **No change to `embyr-pg-storage`'s SQL-building layer** — confirmed by
   direct code read (`crates/embyr-pg-storage/src/backend_adapter.rs::run_query`,
   line 530+) that it builds SQL directly from the already-translated domain
   `StructuredQuery`. The enforcement point is upstream, in
   `grpc::handler::handle_run_query`, before `adapter.run_query()` is called.

DISCUSS's own confirmed evidence (`crates/embyr-core/src/domain/query.rs::
QueryFilter`, `crates/embyr-server/src/grpc/handler.rs::translate_filter`)
establishes that `QueryFilter` is AND-only — no OR representation exists
anywhere in embyr's query pipeline — which is the structural fact that makes
the 5-shape decidable set tractable at all (a general Or/Not compliance
prover would require inventing UNION-of-queries execution machinery in
`embyr-pg-storage`, explicitly rejected in DISCUSS's Resolution 1, Option A).

This ADR combines the algorithm, the composition (call-site ordering,
including relative to the existing composite-index check), the rejection
response shape, and the Release-2 simulation extension into one ADR, mirroring
ADR-030's own "smaller, bounded decision surface" precedent: each axis below
is either a single new pure function or a small, additive composition change
to one existing call site — not three independently wide option spaces.

## Decision Drivers

1. **US-05's undecidable-shape default arm must reject, never silently
   allow** — this feature's single highest-consequence design risk (a
   false-allow would look like enforcement while providing none). Designated
   mutation-testing surface (per-feature strategy, CLAUDE.md).
2. **The filter-value-matches-caller's-own-uid property (AC-17-51) is
   load-bearing** — a compliance check that merely requires "a filter on the
   right field," without binding its value to the caller's own
   server-verified `request.auth.uid`, lets any signed-in caller enumerate
   another user's data by writing a filter with someone else's id as the
   literal value.
3. **No modification to `embyr-pg-storage`'s SQL-building layer, or to any
   part of `security-rules`/`security-rules-write-path`'s already-shipped
   scope** (`access_rules`, `get_access_rule`, `handle_get_document`,
   `write_access_rules` and its call sites) — per this feature's own scope
   boundary.
4. **`parse_condition()`/`Condition`/`Operand`/`AuthContext` remain BC-4's
   sole grammar/type surface** (ADR-027, extended ADR-030) — the new
   function pattern-matches over the existing `Condition` enum; it invents no
   parallel condition representation.
5. **Simplest solution first** (Principle 8) — no new crate, no new
   dependency, no new storage, no new bounded context. The new function is a
   pure, zero-IO sibling to `evaluate()`.
6. **Enforceable, not conventional** — the undecidable-shape reject-default
   (Driver 1) must be structurally the *only* path to `Admitted`, i.e. total
   pattern-matching with an explicit catch-all, not an allow-list check that
   could be bypassed by an unhandled `Condition` variant.

## Decision — Algorithm and Types

### New types (`crates/embyr-core/src/access_control/mod.rs`, extended)

```rust
/// Outcome of a query-shape compliance check (security-rules-query-path).
/// Distinct from `EvaluationOutcome` (Allow/Deny only): this feature's own
/// AC-17-56/68 require the REASON a query was rejected to be distinguishable,
/// not just the binary admit/reject fact — and US-07 (simulation) requires
/// reporting WHICH conjunct(s) failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryComplianceOutcome {
    /// The query's filter tree, together with the caller's auth context,
    /// satisfies every decidable conjunct of the rule. The query may proceed.
    Admitted,
    /// The rule is a fully decidable shape, but the query does not satisfy
    /// one or more conjuncts. Carries every conjunct that failed.
    Rejected { unsatisfied_conjuncts: Vec<UnsatisfiedConjunct> },
    /// The rule's Condition tree contains a shape outside the locked
    /// decidable set (Or, Not, RequestResourceField, or any other
    /// unnamed Compare shape) ANYWHERE in the tree. The entire rule is
    /// undecidable — every query against the collection is rejected,
    /// regardless of filter shape.
    RejectedUnsupportedRuleShape,
}

/// One AND-conjunct that failed to be satisfied by the query's filter tree
/// and/or auth context. `reason_code()` gives the stable, machine-checkable
/// vocabulary shared by the gRPC rejection message (embedded, this feature's
/// own message-string-only convention — see § Decision — Rejection Response
/// Shape) and the Release-2 simulation JSON response (structured field) —
/// ONE vocabulary, two renderings, never two independently-maintained copies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsatisfiedConjunct {
    /// `Condition::Literal(false)` anywhere in the tree — deny-all. No
    /// filter shape could ever satisfy this; short-circuits the whole
    /// decision the moment it is found (mirrors the locked shape-5
    /// exception: "excluding Literal(false), which short-circuits the whole
    /// rule to always-deny").
    DenyAll,
    /// `request.auth != null` conjunct unmet — caller is not signed in.
    AuthRequired,
    /// `request.auth == null` conjunct unmet — caller is unexpectedly
    /// signed in (the inverse idiom; grammar-legal, rare, still decidable).
    AuthForbidden,
    /// `request.auth.uid == resource.data.<field>` conjunct unmet — no
    /// query filter binds `<field>` with `==` to the caller's own verified
    /// uid. Carries the field name for the rejection message (AC-17-56).
    OwnershipFilterMissing { field_path: String },
}

impl UnsatisfiedConjunct {
    /// Stable reason vocabulary — never changes shape based on caller
    /// (gRPC message text vs. Release-2 JSON `reason` field), mirroring
    /// `ConditionParseError`'s SYNTAX_ERROR/UNSUPPORTED_CONSTRUCT
    /// discipline (`admin::handlers::access_rules::
    /// condition_parse_error_response`) at the same abstraction level.
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::DenyAll => "RULE_DENIES_ALL",
            Self::AuthRequired => "AUTH_REQUIRED",
            Self::AuthForbidden => "AUTH_FORBIDDEN",
            Self::OwnershipFilterMissing { .. } => "OWNERSHIP_FILTER_MISSING",
        }
    }
}
```

`QueryComplianceOutcome::RejectedUnsupportedRuleShape`'s own reason code is a
module-level constant, `"UNSUPPORTED_RULE_SHAPE"` — not an `UnsatisfiedConjunct`
variant, since it is a whole-rule verdict, not a per-conjunct one (US-05's own
framing: "the entire collection is rejected," not "one conjunct among several
failed").

### The new function

```rust
/// Decide whether a `RunQuery`'s filter tree, together with the caller's
/// auth context, satisfies a rule's `Condition` — WITHOUT fetching or
/// inspecting any document (security-rules-query-path). Total and
/// infallible by construction: every `Condition` variant is matched, with
/// an explicit catch-all that resolves to `RejectedUnsupportedRuleShape`
/// (Decision Driver 1 — reject is the ONLY reachable outcome for a shape
/// this function does not name, not an accidental allow-list gap).
///
/// `filter` is the query's ALREADY-TRANSLATED domain `QueryFilter` (or
/// `None` for an unfiltered query) — this function performs zero proto
/// translation and zero IO.
///
/// `auth` is the caller's server-VERIFIED identity (or `None`), the
/// identical `Option<&AuthContext>` `evaluate()` already takes — this
/// function never derives or accepts an identity from anywhere else. This
/// is the load-bearing security property behind AC-17-51: the caller's own
/// uid used for the ownership-equality comparison always comes from this
/// parameter, NEVER from the query filter's own bound value. The filter's
/// bound value is only ever the thing being CHECKED against `auth.uid`,
/// never trusted as proof of `auth.uid`'s own value.
pub fn check_query_compliance(
    condition: &Condition,
    filter: Option<&QueryFilter>,
    auth: Option<&AuthContext>,
) -> QueryComplianceOutcome {
    let atoms = match decompose_decidable(condition) {
        Err(Undecidable) => return QueryComplianceOutcome::RejectedUnsupportedRuleShape,
        Ok(atoms) => atoms,
    };

    let mut unsatisfied = Vec::new();
    for atom in &atoms {
        match atom {
            Atom::AlwaysTrue => {}
            Atom::AlwaysFalse => {
                // Locked shape-5 exception: Literal(false) anywhere
                // short-circuits the WHOLE rule to always-deny, regardless
                // of any other conjunct's own satisfiability.
                return QueryComplianceOutcome::Rejected {
                    unsatisfied_conjuncts: vec![UnsatisfiedConjunct::DenyAll],
                };
            }
            Atom::AuthPresence { required: true } if auth.is_none() => {
                unsatisfied.push(UnsatisfiedConjunct::AuthRequired);
            }
            Atom::AuthPresence { required: false } if auth.is_some() => {
                unsatisfied.push(UnsatisfiedConjunct::AuthForbidden);
            }
            Atom::AuthPresence { .. } => {}
            Atom::OwnershipEquality(field) => {
                let satisfied = auth
                    .map(|a| filter_binds_field_to_uid(filter, field, &a.uid))
                    .unwrap_or(false);
                if !satisfied {
                    unsatisfied.push(UnsatisfiedConjunct::OwnershipFilterMissing {
                        field_path: field.clone(),
                    });
                }
            }
        }
    }

    if unsatisfied.is_empty() {
        QueryComplianceOutcome::Admitted
    } else {
        QueryComplianceOutcome::Rejected { unsatisfied_conjuncts: unsatisfied }
    }
}
```

### `decompose_decidable` — the fail-closed default-arm gate (Decision Driver 1)

```rust
struct Undecidable;

enum Atom {
    AlwaysTrue,
    AlwaysFalse,
    AuthPresence { required: bool }, // true = "!= null", false = "== null"
    OwnershipEquality(String),       // field name
}

/// Recursively decomposes an AND-only tree of the 5 locked decidable shapes
/// into a flat list of atoms. Returns `Err(Undecidable)` the MOMENT any
/// `Condition::Or`, `Condition::Not`, or any `Condition::Compare` shape
/// outside the two named pairings is encountered — anywhere in the tree,
/// including nested inside an `And`. This is the ONLY path to
/// `RejectedUnsupportedRuleShape`, and it is reached by an explicit
/// wildcard match arm, not by the absence of a match (mirrors `eval_bool`'s
/// own internal `Result`-as-control-flow pattern in this same module —
/// architectural consistency, not a new idiom).
fn decompose_decidable(condition: &Condition) -> Result<Vec<Atom>, Undecidable> {
    match condition {
        Condition::Literal(true) => Ok(vec![Atom::AlwaysTrue]),
        Condition::Literal(false) => Ok(vec![Atom::AlwaysFalse]),

        // Shape 3: `request.auth != null` / `request.auth == null`,
        // either operand order (both grammar-legal; the DISCUSS-locked
        // shape names "Ne|Eq" explicitly — both directions decidable).
        Condition::Compare(Operand::AuthNullSentinel, op, Operand::NullLiteral)
        | Condition::Compare(Operand::NullLiteral, op, Operand::AuthNullSentinel) => {
            Ok(vec![Atom::AuthPresence { required: *op == CompareOp::Ne }])
        }

        // Shape 4: `request.auth.uid == resource.data.<field>`, either
        // operand order. CompareOp::Eq ONLY — Ne on this pairing is NOT in
        // the locked set (falls through to the catch-all below).
        Condition::Compare(Operand::AuthUid, CompareOp::Eq, Operand::ResourceField(f))
        | Condition::Compare(Operand::ResourceField(f), CompareOp::Eq, Operand::AuthUid) => {
            Ok(vec![Atom::OwnershipEquality(f.clone())])
        }

        // Shape 5: AND of independently-decidable shapes 1-4. Recurses;
        // `?` propagates `Undecidable` from either side without inventing a
        // new short-circuit.
        Condition::And(left, right) => {
            let mut atoms = decompose_decidable(left)?;
            atoms.extend(decompose_decidable(right)?);
            Ok(atoms)
        }

        // Everything else: Condition::Or, Condition::Not, any
        // Condition::Compare pairing not named above (e.g.
        // RequestResourceField in a read rule, Ne against a ResourceField,
        // two ResourceFields compared to each other, AuthUid vs
        // NullLiteral). This arm is the ENTIRE undecidable-shape contract —
        // total, explicit, never reached by omission.
        _ => Err(Undecidable),
    }
}
```

### Filter-tree walk — the caller's-own-uid binding (Decision Driver 2)

```rust
/// Does the query's filter tree (recursively, through `QueryFilter::
/// Composite`'s AND structure — no other composite shape exists,
/// `crates/embyr-core/src/domain/query.rs`) contain an equality filter on
/// `field_path` whose bound VALUE equals `caller_uid`?
///
/// SECURITY-CRITICAL: `caller_uid` is ALWAYS `auth.uid` — the server
/// -verified identity threaded in from `check_query_compliance`'s own
/// `auth` parameter, which itself is only ever constructed from
/// `VerifiedEndUserIdentity` at the `handle_run_query` call site (never
/// from anything client-supplied). This function reads the filter's bound
/// value ONLY to compare it against that already-server-verified uid — it
/// never treats the filter's presence, or its field name matching, as
/// sufficient proof of entitlement on its own. A filter reading
/// `owner_id == "maria-santos"` issued by Dana does NOT match here, because
/// `caller_uid` is `"dana-kim"` — the field name is right, the VALUE is
/// wrong, and value is what this check binds on (AC-17-51).
///
/// Field-path matching is exact-string, case-sensitive (AC-17-52) — plain
/// `==` on `field_path: String`, no normalization.
fn filter_binds_field_to_uid(
    filter: Option<&QueryFilter>,
    field_path: &str,
    caller_uid: &str,
) -> bool {
    match filter {
        None => false,
        Some(QueryFilter::Field(ff)) => {
            ff.field_path == field_path
                && ff.op == FilterOp::Equal
                && ff.value == FieldValue::String(caller_uid.to_string())
        }
        Some(QueryFilter::Composite(filters)) => filters
            .iter()
            .any(|f| filter_binds_field_to_uid(Some(f), field_path, caller_uid)),
    }
}
```

### Why this is genuinely different from `evaluate()`, not a rename

`evaluate()` resolves operand VALUES against an already-fetched document's
field map (`resource_fields.get(name)`) and compares them. This function
never resolves a document field value at all — there is no document yet. It
instead asks a structurally different question: "does the QUERY's own filter
SHAPE, as written by the caller, already carry a proof of entitlement that a
server-side re-check of `auth.uid` can confirm?" This is why `evaluate()`'s
`resource_fields`/`request_resource_fields` parameters have no analog here,
and why the two functions are siblings with different signatures, not one
function overloaded on an `Option`.

## Decision — Composition (Placement in `handle_run_query`)

### Exact insertion points (`crates/embyr-server/src/grpc/handler.rs::handle_run_query`, lines 1131-1267)

1. **Identity attach** (new call site, function itself unchanged) — inserted
   immediately after the existing `suspended` check (after line 1149),
   mirroring `handle_get_document`'s own placement exactly:
   ```rust
   let verified_identity = self
       .attach_client_identity_if_present(&request, &project_id_str)
       .await;
   ```
2. Existing structured-query extraction, `translate_filter`, `order_by`,
   `limit`, cursor translation, and `domain_query`/`collection` construction
   — **entirely unchanged** (lines 1151-1222).
3. **Rule lookup** (new call site, cheap indexed PK lookup, identical
   shape/cost to `handle_get_document`'s own `get_access_rule` call) —
   inserted immediately after `domain_query`/`collection` are built (after
   line 1222), **before** the existing `requires_composite_index` check:
   ```rust
   let rule_row = self
       .system_db
       .get_access_rule(&project_id_str, &collection.collection_path)
       .await
       .map_err(|e| Status::internal(e.to_string()))?;

   if let Some(rule_row) = rule_row {
       let condition = embyr_core::access_control::parse_condition(&rule_row.condition_source)
           .map_err(|e| Status::internal(format!("stored access rule failed to re-parse: {e:?}")))?;
       let auth_ctx = verified_identity
           .as_ref()
           .map(|v| embyr_core::access_control::AuthContext { uid: v.end_user_id.clone() });

       match embyr_core::access_control::check_query_compliance(
           &condition,
           domain_query.filter.as_ref(),
           auth_ctx.as_ref(),
       ) {
           embyr_core::access_control::QueryComplianceOutcome::Admitted => {}
           outcome => return Err(query_compliance_rejection(&outcome)),
       }
   }
   ```
   **`None`** (no read rule defined for this collection) → the `if let`
   simply does not execute — the composite-index check and `adapter
   .run_query()` calls immediately below are reached completely unmodified,
   the EXACT pre-feature code path (AC-17-69/70, the structural mechanism
   behind US-06's guardrail, identical in shape to ADR-029's own
   `get_access_rule() -> None` short-circuit).
4. **Existing `requires_composite_index`/`index_manager.is_index_ready`
   check — entirely unchanged, but now runs strictly AFTER step 3.** See §
   OQ-SRQ-03 Resolution below for why this ordering, not the reverse, is
   correct.
5. **Existing `adapter.run_query()` call and response-stream construction —
   entirely unchanged.**

No change to `translate_filter`, `requires_composite_index`,
`collect_filter_fields`, or any other existing free function in this file.
No change to `embyr-pg-storage::backend_adapter::run_query` at all —
confirmed by direct code read during DISCUSS and re-confirmed here: the
compliance check runs entirely inside `grpc::handler::handle_run_query`,
strictly before `adapter.run_query()` is ever called.

### OQ-SRQ-03 Resolution — compliance check runs BEFORE the composite-index check

**Resolved by this DESIGN pass** (was flagged unresolved by DISCUSS).
**Compliance-checking runs strictly before `requires_composite_index`/
`is_index_ready`.**

Reasoning, verified against the actual code (not assumed):

- A query that is BOTH non-compliant with the collection's rule AND missing
  a required composite index must surface the compliance rejection, not the
  index rejection. Revealing "this query requires a composite index" to a
  caller who was never entitled to run any shape of query against this
  collection at all is itself a minor information leak — it confirms the
  collection's general query-ability and index topology to a caller a
  correctly-enforced rule would refuse before any of that becomes visible.
- This ordering costs nothing extra: `get_access_rule` is a single indexed
  PK lookup on `(project_id, collection_path)` — no more expensive than the
  `is_index_ready` check it now precedes, and for the common case (`rule_row
  == None`) it is the ONLY added cost on the unarmed-collection path,
  satisfying DISCUSS's own NFR note ("a collection with no read rule must
  add effectively zero overhead").
- This ordering mirrors `handle_get_document`'s own precedent exactly: the
  rule lookup and evaluation happen before the (cheaper, single) document
  fetch's result is used to decide the response — access control is decided
  before any other request-specific information is computed or revealed.
- The composite-index rejection (`Status::failed_precondition`) and the
  compliance rejection (`Status::permission_denied`, see below) are already
  different gRPC status CODES, so ordering does not need to invent any new
  distinguishability mechanism beyond what already exists — it only needs to
  decide which check runs first when both would fail, and the fail-closed,
  least-information-leaked answer is: compliance first.

## Decision — Rejection Response Shape

**gRPC status code: `Status::permission_denied`, mirroring
`handle_get_document`'s own precedent exactly** (`Status::permission_denied
("access denied by rule")`) — not `Status::invalid_argument` (reserved, per
existing precedent, for malformed request shapes the client can fix by
changing the request syntax — e.g. `translate_filter`'s "unsupported
composite operator") and not a new status code. This is a rule-driven access
decision, the same class of rejection `GetDocument` already produces for the
identical `access_rules` row.

**No new structured-reason machinery at the gRPC layer** — this codebase's
existing precedent (confirmed by direct grep across `grpc/handler.rs`) is
message-string-only at the gRPC boundary; the `SYNTAX_ERROR`/
`UNSUPPORTED_CONSTRUCT` structured-`reason`-field taxonomy exists ONLY at the
Admin HTTP layer (`admin::handlers::access_rules::
condition_parse_error_response`, a JSON body), never through tonic
`Status` metadata. This ADR does not invent a new mechanism inconsistent with
that precedent. Instead, distinguishability is achieved the same way
`handle_get_document`'s rejection is already distinguishable from
`authenticate()`'s own rejections and from `translate_filter`'s
`invalid_argument`: by gRPC status CODE first, then by a stable,
`UnsatisfiedConjunct::reason_code()`-driven message-text convention for the
finer-grained distinctions AC-17-56/68 require within the `PermissionDenied`
family itself:

```rust
fn query_compliance_rejection(
    outcome: &embyr_core::access_control::QueryComplianceOutcome,
) -> Status {
    use embyr_core::access_control::QueryComplianceOutcome;
    let message = match outcome {
        QueryComplianceOutcome::RejectedUnsupportedRuleShape => {
            "query rejected [UNSUPPORTED_RULE_SHAPE]: this collection's access rule \
             is not a shape supported for query enforcement".to_string()
        }
        QueryComplianceOutcome::Rejected { unsatisfied_conjuncts } => {
            let reasons: Vec<String> = unsatisfied_conjuncts
                .iter()
                .map(|c| match c {
                    embyr_core::access_control::UnsatisfiedConjunct::OwnershipFilterMissing { field_path } => {
                        format!(
                            "[{}] missing required equality filter on '{field_path}' bound to the caller's own identity",
                            c.reason_code()
                        )
                    }
                    other => format!("[{}]", other.reason_code()),
                })
                .collect();
            format!("query rejected by access rule: {}", reasons.join("; "))
        }
        QueryComplianceOutcome::Admitted => unreachable!("Admitted never reaches this function"),
    };
    Status::permission_denied(message)
}
```

The bracketed `[REASON_CODE]` token is the stable, grep/substring-checkable
distinguishing marker DISTILL's acceptance scenarios need for AC-17-56
("distinguishable from `authenticate()`-level and composite-index
rejections" — already true by status code alone) and AC-17-68
("distinguishable from the 'missing required filter' rejection" — true by
reason code: `UNSUPPORTED_RULE_SHAPE` vs. `OWNERSHIP_FILTER_MISSING`/
`AUTH_REQUIRED`/`AUTH_FORBIDDEN`/`RULE_DENIES_ALL`), without adding any new
tonic `Status` metadata mechanism this codebase does not already use
elsewhere at the gRPC boundary. `Status::internal` is reserved, as
elsewhere in this file, for a stored condition that fails to re-parse (a
server-side data-integrity fault, not a caller-facing decision) — mirrors
`handle_get_document`'s identical `Status::internal(format!("stored access
rule failed to re-parse: {e:?}"))` handling verbatim.

## Decision — Release 2 Simulation Extension (US-07)

**A new, distinct admin handler `simulate_query_compliance`, not a further
extension of `simulate_access_rule`'s existing body/response.** DISCUSS's own
Technical Note suggested extending `simulate_access_rule`; this ADR makes a
deliberate, reasoned departure, for the same reason ADR-030's DDD-SRW-6
rejected a `rule_type`-discriminated single handler for `define_access_rule`/
`define_write_access_rule`: `simulate_access_rule`'s response contract
(`{outcome: "allow"|"deny"}`) and this feature's own required response
contract (admit/reject PLUS which conjunct(s) failed, PLUS the
whole-rule-undecidable case) are genuinely different shapes, not an optional
field away from each other. Branching a single handler's RESPONSE TYPE on
which optional input field was present would reintroduce exactly the
runtime-table-selection-style risk ADR-030 already rejected once in this same
module. What IS reused, per Decision Driver 4 and DISCUSS's own explicit
requirement: the identical `check_query_compliance()` function real
enforcement calls — never a second, independently-maintained shape-compliance
implementation.

```rust
/// Body for POST /admin/v1/projects/:project_id/access_rules/simulate_query
/// (US-07, Release 2). A candidate condition (never read from or written to
/// access_rules), a candidate identity (or none, anonymous), and a candidate
/// query filter shape — the SAME `QueryFilter`-shaped input `handle_run_query`
/// produces from a real proto query, expressed here as a flat JSON list for
/// admin-API ergonomics (translated to `QueryFilter::Composite` internally,
/// mirroring the AND-only domain shape exactly — no OR input accepted here
/// either, consistent with the locked v1 scope).
#[derive(Deserialize)]
pub struct SimulateQueryComplianceBody {
    pub condition: String,
    pub auth: Option<SimulatedAuth>,
    #[serde(default)]
    pub query_filters: Vec<SimulatedQueryFilter>, // [{field_path, op, value}], AND-composed
}

#[derive(Deserialize)]
pub struct SimulatedQueryFilter {
    pub field_path: String,
    pub op: String,   // "==" only meaningfully checked by v1's decidable set; others pass through
    pub value: serde_json::Value,
}

/// Response — GENUINELY DIFFERENT CONTRACT from `SimulateAccessRuleResponse`
/// (shape compliance, not an allow/deny document-evaluation outcome).
#[derive(Serialize)]
pub struct SimulateQueryComplianceResponse {
    pub compliant: bool,
    /// Present iff `compliant == false`. One reason_code per unsatisfied
    /// conjunct, or exactly `["UNSUPPORTED_RULE_SHAPE"]` for a whole-rule
    /// undecidable verdict — the SAME `UnsatisfiedConjunct::reason_code()`
    /// vocabulary the gRPC rejection message embeds, never a second
    /// vocabulary.
    #[serde(default)]
    pub reasons: Vec<&'static str>,
}
```

`simulate_query_compliance` mirrors `simulate_access_rule`'s handler shape
exactly otherwise: session auth, ANY role (read-only, zero writes — mirrors
AC-17-18/47's precedent), `parse_condition` validation using the SAME
`condition_parse_error_response`/`SYNTAX_ERROR`/`UNSUPPORTED_CONSTRUCT`
taxonomy for a malformed candidate condition, zero effect on live traffic
(AC-17-76). New route: `POST /admin/v1/projects/:project_id/access_rules/
simulate_query`, registered alongside the existing `.../simulate` route in
`admin::router::build_admin_router`'s session sub-router.

## Consequences

### Positive

- The RunQuery bypass is closed using a genuinely new mechanism appropriate
  to the problem (static filter-shape comparison), not a bolted-on reuse of
  `evaluate()` that would have required either fetching every candidate
  document first (the explicitly-rejected antipattern) or silently limiting
  scope.
- `access_rules`, `get_access_rule`, `handle_get_document`,
  `write_access_rules`, and every write-path handler receive ZERO code
  changes from this feature — verifiable by diff.
- `embyr-pg-storage::backend_adapter::run_query`'s SQL-building layer
  receives ZERO code changes — the enforcement point is entirely upstream.
- The undecidable-shape reject-default (Decision Driver 1) is structurally
  the only reachable path for an unrecognized `Condition` shape — a missing
  match arm is a compile error (Rust's exhaustiveness check on the explicit
  named patterns plus wildcard), not a silent runtime gap.
- Collections with no read rule pay exactly one new indexed PK lookup
  (`get_access_rule` returning `None`) and zero additional logic — the
  identical "no overhead when unarmed" guarantee `security-rules` and
  `security-rules-write-path` already established.

### Negative / Trade-offs

- The 5-shape decidable set is a genuine capability ceiling, not just a v1
  simplification: a real, common Firestore pattern ("owner OR public") is
  unenforceable for queries under this design and every query against such a
  rule is rejected outright (US-05) rather than partially served. Accepted
  deliberately — DISCUSS's Resolution 1, Option A showed the alternative
  requires inventing UNION-of-queries execution machinery in
  `embyr-pg-storage`, a materially larger undertaking than this feature's own
  scope (OQ-SRQ-02, deferred to Product Discovery).
- Two independent boolean-condition-consuming functions
  (`evaluate`/`check_query_compliance`) now exist side by side in the same
  module rather than one — accepted because they answer structurally
  different questions (document-value truth vs. query-shape provability) and
  forcing them into one signature would require a document-fetch this
  feature exists specifically to avoid.
- The reason-code-in-message-text convention (§ Decision — Rejection
  Response Shape) is a weaker machine-contract than gRPC richer-error
  metadata (`google.rpc.ErrorInfo`) would provide — accepted as the
  minimal-new-mechanism choice consistent with this codebase's existing,
  unanimous message-string-only precedent at the gRPC boundary; a future ADR
  can introduce structured gRPC error details project-wide if evidence
  warrants, but this feature does not unilaterally invent that mechanism for
  itself alone.

## Enforcement

Style: Hexagonal (ports-and-adapters), unchanged project-wide pattern. No new
crate, no new bounded context — BC-4 Access Control (ADR-029) gains a third
pure function (`check_query_compliance`, alongside `parse_condition`/
`evaluate`), consumed from one new `grpc::handler` call site
(`handle_run_query`) and (Release 2) one new admin call site.

Rules enforced (existing, applying unchanged):
- `embyr-core::access_control` retains zero IO imports (`cargo-deny`,
  `deny.toml`, already covers all of `embyr-core`) — `check_query_compliance`
  adds no import; it consumes only in-memory `Condition`/`QueryFilter`/
  `AuthContext` values.
- `embyr-core` defines the value-type/function surface; `embyr-server`
  consumes it — dependency direction inward, unchanged.

**No new driven port, no new probe (Principle 12 discipline, explicit
reasoning required, mirroring ADR-029/030 § Enforcement verbatim):**

- `get_access_rule` (new call site, unchanged method) executes through the
  existing, already-probed `SystemDb` connection pool.
- `check_query_compliance`/`decompose_decidable`/`filter_binds_field_to_uid`
  are pure, deterministic CPU computation over values already resident in
  memory (a `Condition` AST, an `Option<QueryFilter>`, an
  `Option<AuthContext>`) — the identical "no environment can lie to a pure
  function" reasoning ADR-029 § Enforcement established applies unmodified.
  There is no filesystem, network, subprocess, clock, or vendor-SDK
  dependency anywhere in this function's call graph for Earned Trust
  (Principle 12) to probe against — the substrate this feature adds new
  reliance on is exactly zero.

`cargo-deny`/`deny.toml` unaffected — no new workspace dependency.

## References

- `docs/feature/security-rules-query-path/feature-delta.md` § Job Discovery
  — Framing Resolution (Resolutions 1-2, the locked 5-shape decidable set), §
  System Constraints, § Handoff Package, § Open Questions (OQ-SRQ-03).
- `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md`,
  `adr-028-access-rule-storage-and-lifecycle.md`,
  `adr-029-access-control-composition-and-bounded-context.md`,
  `adr-030-write-path-grammar-storage-and-composition.md` — the machinery
  this ADR extends (types, storage, composition precedent), not replaces.
- `crates/embyr-core/src/access_control/mod.rs` (full, read during DESIGN) —
  exact current `Condition`/`Operand`/`CompareOp`/`AuthContext` shapes,
  `eval_bool`'s internal `Result`-as-control-flow pattern (the direct
  structural precedent for `decompose_decidable`'s own `Result<_,
  Undecidable>` internal signaling).
- `crates/embyr-core/src/domain/query.rs` (full, read during DESIGN) —
  confirmed `QueryFilter::Composite` is AND-only ("Composite AND filter" doc
  comment), the structural fact this whole ADR's tractability rests on.
- `crates/embyr-server/src/grpc/handler.rs` (targeted full reads:
  `attach_client_identity_if_present` lines 358-383, `handle_get_document`
  lines 506-616, `handle_run_query` lines 1131-1267, `requires_composite_index`
  lines 446-458, `translate_filter` lines 1504-1553) — the exact current
  ordering and call shapes this ADR's § Decision — Composition inserts into,
  confirmed by direct read, not assumed.
- `crates/embyr-server/src/adapters/system_db.rs:346-368` (`get_access_rule`)
  — the exact, unmodified adapter method this feature's new call site reuses.
- `crates/embyr-server/src/admin/handlers/access_rules.rs` — exact current
  `simulate_access_rule`/`condition_parse_error_response`/
  `ConditionRejectionResponse` shapes, the direct structural precedent for
  the Release-2 `simulate_query_compliance` handler.
- `crates/embyr-pg-storage/src/backend_adapter.rs:530+` (`run_query`) —
  confirmed builds SQL directly from the already-translated domain
  `StructuredQuery`; zero change from this feature.
